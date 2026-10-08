//! A loopback-only Ollama runtime, installed into the application's data directory.

use std::{
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::process::{Child, Command};

pub const MODEL: &str = "qwen3:8b";
pub const ENDPOINT: &str = "http://127.0.0.1:11435";
const VERSION: &str = "v0.35.1";
const RELEASE: &str = "https://github.com/ollama/ollama/releases/download/v0.35.1";
const MAX_ARCHIVE: u64 = 4 * 1024 * 1024 * 1024;

#[derive(Clone, Default, Serialize)]
pub struct Progress {
    pub stage: String,
    pub completed: u64,
    pub total: u64,
    pub busy: bool,
    pub error: Option<String>,
}

pub type SharedProgress = Arc<Mutex<Progress>>;

pub async fn stop_process(mut child: Child) -> Result<(), String> {
    if let Some(pid) = child.id() {
        #[cfg(unix)]
        {
            // The leader may exit on TERM while model runners ignore it. Always
            // finish the group, not just the leader, before reporting success.
            signal_group(pid, libc::SIGTERM)?;
            let waited = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
            signal_group(pid, libc::SIGKILL)?;
            if let Ok(result) = waited {
                result.map_err(|e| e.to_string())?;
                return Ok(());
            }
        }
        #[cfg(windows)]
        {
            let result = tokio::time::timeout(
                Duration::from_secs(5),
                Command::new("taskkill.exe")
                    .args(["/PID", &pid.to_string(), "/T", "/F"])
                    .creation_flags(0x08000000)
                    .output(),
            )
            .await
            .map_err(|_| "Timed out stopping the managed Ollama process tree")?
            .map_err(|e| e.to_string())?;
            if !result.status.success() && child.try_wait().map_err(|e| e.to_string())?.is_none() {
                return Err("Could not stop the managed Ollama process tree".into());
            }
        }
    }
    child.start_kill().map_err(|e| e.to_string())?;
    tokio::time::timeout(Duration::from_secs(5), child.wait())
        .await
        .map_err(|_| "Timed out reaping the managed Ollama process")?
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(unix)]
fn signal_group(pid: u32, signal: i32) -> Result<(), String> {
    if unsafe { libc::kill(-(pid as i32), signal) } == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(()) // Already gone; never target an unrelated process by name.
    } else {
        Err(format!("Cannot signal managed Ollama process group: {error}"))
    }
}

/// Readiness is a successful version response, not merely a live process or
/// any HTTP response. This probe never follows redirects or uses a proxy.
pub async fn server_ready(endpoint: &str) -> bool {
    let Ok(client) = http(Duration::from_secs(3)) else {
        return false;
    };
    let Ok(response) = client.get(format!("{endpoint}/api/version")).send().await else {
        return false;
    };
    if !response.status().is_success() {
        return false;
    }
    response
        .json::<Value>()
        .await
        .ok()
        .is_some_and(|v| v["version"].as_str().is_some_and(|version| !version.trim().is_empty()))
}

pub fn progress(shared: &SharedProgress, stage: &str, completed: u64, total: u64) {
    let mut p = shared.lock().unwrap_or_else(|e| e.into_inner());
    p.stage = stage.into();
    p.completed = completed;
    p.total = total;
}

#[derive(Default, Serialize)]
pub struct Status {
    pub model: &'static str,
    pub endpoint: &'static str,
    pub installed: bool,
    pub server_ready: bool,
    pub model_downloaded: bool,
    pub model_loaded: bool,
    pub owned_server: bool,
    pub version: Option<String>,
    pub size_vram: u64,
}

#[derive(Debug, Serialize)]
pub struct Answer {
    pub response: String,
    pub elapsed_ms: u64,
    pub memories_used: usize,
}

pub struct Runtime {
    root: PathBuf,
    endpoint: String,
    child: Arc<Mutex<Option<Child>>>,
}

fn http(timeout: Duration) -> Result<Client, String> {
    Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(3))
        .timeout(timeout)
        .build()
        .map_err(|e| e.to_string())
}

fn executable(root: &Path) -> PathBuf {
    let dir = root.join(format!("runtime-{VERSION}"));
    if cfg!(target_os = "windows") {
        dir.join("ollama.exe")
    } else if cfg!(target_os = "macos") {
        dir.join("ollama")
    } else {
        dir.join("bin/ollama")
    }
}

fn system_executable() -> Option<PathBuf> {
    let name = if cfg!(windows) { "ollama.exe" } else { "ollama" };
    let mut paths: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).map(|d| d.join(name)).collect())
        .unwrap_or_default();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        paths.push(PathBuf::from(local).join("Programs/Ollama/ollama.exe"));
    }
    paths.into_iter().find(|p| p.is_file())
}

fn asset() -> Result<&'static str, String> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Ok("ollama-windows-amd64.zip"),
        ("windows", "aarch64") => Ok("ollama-windows-arm64.zip"),
        ("linux", "x86_64") => Ok("ollama-linux-amd64.tar.zst"),
        ("linux", "aarch64") => Ok("ollama-linux-arm64.tar.zst"),
        ("macos", "x86_64" | "aarch64") => Ok("ollama-darwin.tgz"),
        _ => Err("Automatic Ollama installation is not supported on this platform".into()),
    }
}

fn safe_relative(path: &Path) -> bool {
    let mut depth = 0usize;
    for c in path.components() {
        match c {
            Component::Normal(_) => depth += 1,
            Component::CurDir => (),
            Component::ParentDir if depth > 0 => depth -= 1,
            _ => return false,
        }
    }
    true
}

fn extract_tar<R: Read>(reader: R, dest: &Path) -> Result<(), String> {
    let mut archive = tar::Archive::new(reader);
    let mut expanded = 0u64;
    for entry in archive.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path().map_err(|e| e.to_string())?.into_owned();
        if !safe_relative(&path) {
            return Err("Unsafe path in runtime archive".into());
        }
        let kind = entry.header().entry_type();
        if kind.is_symlink() || kind.is_hard_link() {
            let link = entry
                .link_name()
                .map_err(|e| e.to_string())?
                .ok_or("Missing archive link target")?;
            let target = if kind.is_symlink() {
                path.parent().unwrap_or(Path::new("")).join(link)
            } else {
                link.into_owned()
            };
            if !safe_relative(&target) {
                return Err("Unsafe link in runtime archive".into());
            }
        } else if !kind.is_file() && !kind.is_dir() {
            return Err("Unsupported entry in runtime archive".into());
        }
        expanded = expanded.saturating_add(entry.size());
        if expanded > 12 * 1024 * 1024 * 1024 {
            return Err("Runtime archive is too large".into());
        }
        if !entry.unpack_in(dest).map_err(|e| e.to_string())? {
            return Err("Archive entry escaped runtime directory".into());
        }
    }
    Ok(())
}

fn extract(archive: &Path, name: &str, dest: &Path) -> Result<(), String> {
    fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let file = fs::File::open(archive).map_err(|e| e.to_string())?;
    if name.ends_with(".zip") {
        let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
        let mut expanded = 0u64;
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
            let path = entry.enclosed_name().ok_or("Unsafe path in runtime archive")?;
            if entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
                return Err("Symlink in ZIP runtime archive".into());
            }
            expanded = expanded.saturating_add(entry.size());
            if expanded > 12 * 1024 * 1024 * 1024 {
                return Err("Runtime archive is too large".into());
            }
            let out = dest.join(path);
            if entry.is_dir() {
                fs::create_dir_all(out).map_err(|e| e.to_string())?;
            } else {
                if let Some(parent) = out.parent() {
                    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                let mut target = fs::File::create(out).map_err(|e| e.to_string())?;
                std::io::copy(&mut entry, &mut target).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    } else if name.ends_with(".zst") {
        extract_tar(zstd::stream::read::Decoder::new(file).map_err(|e| e.to_string())?, dest)
    } else {
        extract_tar(flate2::read::GzDecoder::new(file), dest)
    }
}

impl Runtime {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            endpoint: ENDPOINT.into(),
            child: Arc::new(Mutex::new(None)),
        }
    }

    pub fn process(&self) -> Arc<Mutex<Option<Child>>> {
        self.child.clone()
    }

    pub fn installed(&self) -> bool {
        executable(&self.root).is_file() || system_executable().is_some()
    }

    pub fn downloaded(&self) -> bool {
        let dir = self.root.join("models");
        let manifest = dir.join("manifests/registry.ollama.ai/library/qwen3/8b");
        let Ok(bytes) = fs::read(manifest) else { return false };
        let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
            return false;
        };
        let complete_blob = |blob: &Value| {
            let Some(digest) = blob["digest"]
                .as_str()
                .and_then(|d| d.strip_prefix("sha256:"))
                .filter(|d| d.len() == 64 && d.bytes().all(|b| b.is_ascii_hexdigit()))
            else {
                return false;
            };
            let Some(size) = blob["size"].as_u64().filter(|size| *size > 0) else {
                return false;
            };
            fs::metadata(dir.join("blobs").join(format!("sha256-{digest}")))
                .is_ok_and(|metadata| metadata.is_file() && metadata.len() == size)
        };
        complete_blob(&value["config"])
            && value["layers"]
                .as_array()
                .is_some_and(|layers| !layers.is_empty() && layers.iter().all(complete_blob))
    }

    pub fn owned(&mut self) -> bool {
        let mut child = self.child.lock().unwrap_or_else(|e| e.into_inner());
        if child.as_mut().is_some_and(|c| c.try_wait().ok().flatten().is_some()) {
            *child = None;
        }
        child.is_some()
    }

    pub async fn inspect(endpoint: &str) -> Status {
        let mut status = Status {
            model: MODEL,
            endpoint: ENDPOINT,
            ..Default::default()
        };
        let Ok(client) = http(Duration::from_secs(3)) else {
            return status;
        };
        let version = client.get(format!("{endpoint}/api/version")).send().await;
        let Ok(version) = version else { return status };
        if !version.status().is_success() {
            return status;
        }
        let Ok(version) = version.json::<Value>().await else {
            return status;
        };
        status.version = version["version"].as_str().map(str::to_owned);
        status.server_ready = status.version.as_ref().is_some_and(|v| !v.trim().is_empty());
        if let Ok(response) = client.get(format!("{endpoint}/api/tags")).send().await {
            if let Ok(tags) = response.error_for_status() {
                if let Ok(tags) = tags.json::<Value>().await {
                    status.model_downloaded = has_model(&tags);
                }
            }
        }
        if let Ok(response) = client.get(format!("{endpoint}/api/ps")).send().await {
            if let Ok(response) = response.error_for_status() {
                if let Ok(ps) = response.json::<Value>().await {
                    if let Some(model) = ps["models"]
                        .as_array()
                        .and_then(|m| m.iter().find(|m| m["name"] == MODEL || m["model"] == MODEL))
                    {
                        status.model_loaded = true;
                        status.size_vram = model["size_vram"].as_u64().unwrap_or(0);
                    }
                }
            }
        }
        status
    }

    pub async fn install(&mut self, p: &SharedProgress) -> Result<(), String> {
        if !self.installed() {
            self.install_runtime(p).await?;
        }
        self.start().await?;
        if !Self::inspect(&self.endpoint).await.model_downloaded {
            self.pull(p).await?;
        }
        progress(p, "Loading Qwen into memory", 0, 0);
        self.ping().await?;
        progress(p, "Ready", 1, 1);
        Ok(())
    }

    async fn install_runtime(&self, p: &SharedProgress) -> Result<(), String> {
        let name = asset()?;
        fs::create_dir_all(&self.root).map_err(|e| e.to_string())?;
        let client = http(Duration::from_secs(3600))?;
        progress(p, "Checking official runtime checksum", 0, 0);
        let sums = client
            .get(format!("{RELEASE}/sha256sum.txt"))
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .text()
            .await
            .map_err(|e| e.to_string())?;
        let expected = checksum(&sums, name)?;
        let archive = self.root.join(format!("{name}.part"));
        let mut response = client
            .get(format!("{RELEASE}/{name}"))
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?;
        let total = response.content_length().unwrap_or(0);
        if total > MAX_ARCHIVE {
            return Err("Runtime download is too large".into());
        }
        let mut file = fs::File::create(&archive).map_err(|e| e.to_string())?;
        let mut hash = Sha256::new();
        let mut completed = 0;
        while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
            completed += chunk.len() as u64;
            if completed > MAX_ARCHIVE {
                return Err("Runtime download is too large".into());
            }
            file.write_all(&chunk).map_err(|e| e.to_string())?;
            hash.update(&chunk);
            progress(p, "Downloading Ollama", completed, total);
        }
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        if hex::encode(hash.finalize()) != expected {
            let _ = fs::remove_file(&archive);
            return Err("Ollama checksum mismatch; runtime was not executed. Retry installation.".into());
        }
        progress(p, "Extracting verified Ollama runtime", 0, 0);
        let staging = self.root.join("runtime-staging");
        let final_dir = self.root.join(format!("runtime-{VERSION}"));
        let staged = staging.clone();
        let archived = archive.clone();
        tokio::task::spawn_blocking(move || {
            if staged.exists() {
                fs::remove_dir_all(&staged).map_err(|e| e.to_string())?;
            }
            extract(&archived, name, &staged)
        })
        .await
        .map_err(|e| e.to_string())??;
        let relative = executable(&self.root)
            .strip_prefix(&final_dir)
            .map_err(|e| e.to_string())?
            .to_owned();
        if !staging.join(relative).is_file() {
            return Err("Ollama executable is missing from archive".into());
        }
        if final_dir.exists() {
            fs::remove_dir_all(&final_dir).map_err(|e| e.to_string())?;
        }
        fs::rename(&staging, &final_dir).map_err(|e| e.to_string())?;
        let _ = fs::remove_file(archive);
        Ok(())
    }

    pub async fn start(&mut self) -> Result<(), String> {
        if server_ready(&self.endpoint).await {
            return Ok(());
        }
        // A slow-starting owned process must not be duplicated.
        if !self.owned() {
            let own = executable(&self.root);
            let exe = if own.is_file() {
                own
            } else {
                system_executable().ok_or("Install Ollama and Qwen first")?
            };
            fs::create_dir_all(self.root.join("models")).map_err(|e| e.to_string())?;
            let log = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.root.join("server.log"))
                .map_err(|e| e.to_string())?;
            let mut cmd = Command::new(exe);
            cmd.arg("serve")
                .env("OLLAMA_HOST", "127.0.0.1:11435")
                .env("OLLAMA_MODELS", self.root.join("models"))
                .env("OLLAMA_NO_CLOUD", "1")
                .env("OLLAMA_CONTEXT_LENGTH", "4096")
                .env("OLLAMA_NUM_PARALLEL", "1")
                .env("OLLAMA_MAX_LOADED_MODELS", "1")
                .env("OLLAMA_KEEP_ALIVE", "10m")
                .stdin(Stdio::null())
                .stdout(Stdio::from(log.try_clone().map_err(|e| e.to_string())?))
                .stderr(Stdio::from(log))
                .kill_on_drop(true);
            #[cfg(windows)]
            cmd.creation_flags(0x08000000);
            #[cfg(unix)]
            cmd.process_group(0);
            *self.child.lock().unwrap_or_else(|e| e.into_inner()) =
                Some(cmd.spawn().map_err(|e| format!("Cannot start Ollama: {e}"))?);
        }
        let readiness = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                // Propagate wait errors instead of claiming ownership/readiness.
                let alive = {
                    let mut slot = self.child.lock().unwrap_or_else(|e| e.into_inner());
                    match slot.as_mut() {
                        Some(child) => child.try_wait().map_err(|e| e.to_string())?.is_none(),
                        None => false,
                    }
                };
                if !alive {
                    return Err(format!("Ollama exited. See {}", self.root.join("server.log").display()));
                }
                if server_ready(&self.endpoint).await {
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        })
        .await
        .unwrap_or_else(|_| Err("Ollama did not become ready; check server.log or retry Start".into()));
        if readiness.is_err() {
            // A failed start must not leave a hidden server to collide with retry.
            let child = self.child.lock().unwrap_or_else(|e| e.into_inner()).take();
            if let Some(child) = child {
                stop_process(child).await?;
            }
        }
        readiness
    }

    pub async fn stop(&mut self) -> Result<(), String> {
        let child = self.child.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(child) = child {
            stop_process(child).await?;
        } else if Self::inspect(&self.endpoint).await.server_ready {
            return Err("This Ollama server was not started by AEGIS; it will not be stopped".into());
        }
        Ok(())
    }

    async fn pull(&self, p: &SharedProgress) -> Result<(), String> {
        progress(p, "Downloading Qwen (resumable)", 0, 0);
        let client = http(Duration::from_secs(7200))?;
        let mut response = client
            .post(format!("{}/api/pull", self.endpoint))
            .json(&json!({"model": MODEL, "stream": true}))
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?;
        let mut pending = Vec::new();
        let mut success = false;
        while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
            pending.extend_from_slice(&chunk);
            while let Some(end) = pending.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = pending.drain(..=end).collect();
                if !line.iter().all(u8::is_ascii_whitespace) {
                    success |= pull_event(&line, p)?;
                }
            }
            if pending.len() > 1024 * 1024 {
                return Err("Invalid Ollama progress stream".into());
            }
        }
        if !pending.is_empty() {
            success |= pull_event(&pending, p)?;
        }
        if !success {
            return Err("Qwen download ended before completion. Retry to resume.".into());
        }
        Ok(())
    }

    pub async fn ping(&self) -> Result<Answer, String> {
        self.ask(
            "Reply with the single word READY.",
            "You are a local connectivity test. Follow the user's instruction.",
            0,
            16,
        )
        .await
    }

    pub async fn observe(&self, prompt: &str, system: &str) -> Result<Answer, String> {
        if prompt.len().saturating_add(system.len()) > 14_000 {
            return Err("Observer request exceeds local context budget; shorten the strategy prompt".into());
        }
        let at = Instant::now();
        let snapshot_id = serde_json::from_str::<Value>(prompt)
            .map_err(|e| e.to_string())?
            .pointer("/snapshot/id")
            .and_then(Value::as_u64)
            .ok_or("Observer snapshot id missing")?;
        let schema = json!({"type":"object","additionalProperties":false,
            "properties":{
                "snapshot_id":{"const":snapshot_id},
                "action":{"type":"string","enum":["wait","long","short","close","reduce","stop"]},
                "reason":{"type":"string","maxLength":160},
                "stop":{"type":["number","null"]},"target":{"type":["number","null"]},
                "position_id":{"type":["integer","null"]},
                "quantity_fraction":{"type":["number","null"]},
                "used_timeframes":{"type":"array","minItems":1,"maxItems":6,"items":{"type":"string","enum":["1m","5m","15m","1h","4h","1d"]}},
                "checks":{"type":"array","maxItems":4,"items":{"type":"object","additionalProperties":false,
                    "properties":{"rule":{"type":"string"},"met":{"type":"boolean"},"evidence":{"type":"string","maxLength":80}},
                    "required":["rule","met","evidence"]}}
            },"required":["snapshot_id","action","reason","stop","target","used_timeframes","checks"]});
        let response = http(Duration::from_secs(28))?
            .post(format!("{}/api/chat", self.endpoint))
            .json(&json!({
                "model": MODEL, "stream": false, "think": false, "keep_alive": "10m", "format":schema,
                "messages": [{"role":"system","content":system},{"role":"user","content":prompt}],
                "options": {"num_ctx":4096,"num_predict":256,"temperature":0}
            }))
            .send()
            .await
            .map_err(|e| format!("Observer inference failed or exceeded deadline: {e}"))?;
        let code = response.status();
        let value: Value = response.json().await.map_err(|e| e.to_string())?;
        if !code.is_success() || value["error"].is_string() {
            return Err(format!(
                "Ollama: {}",
                value["error"].as_str().unwrap_or("Observer request failed")
            ));
        }
        if value["done_reason"] == "length" {
            return Err("Model output exceeded token limit; no action accepted".into());
        }
        if value["done"] != true || value["done_reason"] != "stop" {
            return Err("Observer inference did not finish successfully; no action accepted".into());
        }
        let used = value["prompt_eval_count"].as_u64().unwrap_or(0);
        if used > 3_700 {
            return Err("Model input exhausted context budget; no action accepted".into());
        }
        let response = value["message"]["content"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .ok_or("Ollama returned an empty observer decision")?
            .to_owned();
        Ok(Answer {
            response,
            elapsed_ms: at.elapsed().as_millis() as u64,
            memories_used: 0,
        })
    }

    pub async fn ask(&self, prompt: &str, system: &str, memories_used: usize, tokens: u32) -> Result<Answer, String> {
        if prompt.trim().is_empty() || prompt.len() > 2000 {
            return Err("Enter a prompt of 1–2000 UTF-8 bytes".into());
        }
        if prompt.len().saturating_add(system.len()) > 3000 {
            return Err("Prompt and memory exceed the local context budget".into());
        }
        let at = Instant::now();
        let response = http(Duration::from_secs(300))?
            .post(format!("{}/api/chat", self.endpoint))
            .json(&json!({
                "model": MODEL, "stream": false, "think": false, "keep_alive": "10m",
                "messages": [{"role":"system","content":system},{"role":"user","content":prompt}],
                "options": {"num_ctx":4096,"num_predict":tokens,"temperature":0.1}
            }))
            .send()
            .await
            .map_err(|e| format!("Local model request failed: {e}"))?;
        let code = response.status();
        let value: Value = response.json().await.map_err(|e| e.to_string())?;
        if !code.is_success() || value["error"].is_string() {
            return Err(format!(
                "Ollama: {}",
                value["error"].as_str().unwrap_or("Request failed")
            ));
        }
        let response = value["message"]["content"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .ok_or("Ollama returned an empty answer")?
            .to_owned();
        Ok(Answer {
            response,
            elapsed_ms: at.elapsed().as_millis() as u64,
            memories_used,
        })
    }
}

fn checksum(text: &str, asset: &str) -> Result<String, String> {
    text.lines()
        .find_map(|line| {
            let mut parts = line.split_whitespace();
            let sum = parts.next()?;
            let name = parts.next()?.trim_start_matches('*').trim_start_matches("./");
            (name == asset && sum.len() == 64 && sum.bytes().all(|b| b.is_ascii_hexdigit()))
                .then(|| sum.to_ascii_lowercase())
        })
        .ok_or_else(|| "Official checksum for this platform is unavailable".into())
}

fn has_model(tags: &Value) -> bool {
    tags["models"]
        .as_array()
        .is_some_and(|m| m.iter().any(|m| m["name"] == MODEL || m["model"] == MODEL))
}

#[derive(Deserialize)]
struct PullEvent {
    #[serde(default)]
    status: String,
    #[serde(default)]
    completed: u64,
    #[serde(default)]
    total: u64,
    error: Option<String>,
}

fn pull_event(line: &[u8], p: &SharedProgress) -> Result<bool, String> {
    let event: PullEvent = serde_json::from_slice(line).map_err(|e| e.to_string())?;
    if let Some(error) = event.error {
        return Err(format!("Qwen download: {error}"));
    }
    progress(p, &event.status, event.completed, event.total);
    Ok(event.status == "success")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader},
        net::TcpListener,
        thread,
    };

    fn mock(replies: Vec<(&'static str, u16, Value)>) -> (String, thread::JoinHandle<Vec<Value>>) {
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let mut bodies = Vec::new();
            for (path, code, reply) in replies {
                let (mut stream, _) = server.accept().unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                assert!(line.contains(path), "{line}");
                let mut length = 0usize;
                loop {
                    line.clear();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(v) = line.to_lowercase().strip_prefix("content-length:") {
                        length = v.trim().parse().unwrap();
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                if !body.is_empty() {
                    bodies.push(serde_json::from_slice(&body).unwrap());
                }
                let body = reply.to_string();
                write!(stream, "HTTP/1.1 {code} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
            bodies
        });
        (url, handle)
    }

    fn temp_dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "aegis-ai-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[tokio::test]
    async fn inference_is_local_bounded_and_preserves_errors() {
        let (endpoint, server) = mock(vec![
            ("/api/chat", 200, json!({"message":{"content":"READY"}})),
            ("/api/chat", 500, json!({"error":"not enough memory"})),
        ]);
        let mut runtime = Runtime::new(temp_dir("inference"));
        runtime.endpoint = endpoint;
        assert_eq!(runtime.ping().await.unwrap().response, "READY");
        assert!(runtime.ping().await.unwrap_err().contains("not enough memory"));
        assert!(runtime.ask("", "", 0, 1).await.is_err());
        assert!(runtime.ask(&"x".repeat(2001), "", 0, 1).await.is_err());
        assert!(runtime.ask(&"x".repeat(2000), &"x".repeat(1001), 0, 1).await.is_err());
        let requests = server.join().unwrap();
        assert_eq!(requests[0]["model"], MODEL);
        assert_eq!(requests[0]["think"], false);
        assert_eq!(requests[0]["options"]["num_ctx"], 4096);
        assert_eq!(requests[0]["options"]["num_predict"], 16);
    }

    #[tokio::test]
    async fn status_distinguishes_downloaded_loaded_and_gpu() {
        let (endpoint, server) = mock(vec![
            ("/api/version", 200, json!({"version":"test"})),
            ("/api/tags", 200, json!({"models":[{"name":MODEL}]})),
            ("/api/ps", 200, json!({"models":[{"name":MODEL,"size_vram":123}]})),
        ]);
        let status = Runtime::inspect(&endpoint).await;
        assert!(status.server_ready && status.model_downloaded && status.model_loaded);
        assert_eq!(status.size_vram, 123);
        server.join().unwrap();
    }

    #[tokio::test]
    async fn does_not_stop_a_server_started_elsewhere() {
        let (endpoint, server) = mock(vec![
            ("/api/version", 200, json!({"version":"test"})),
            ("/api/tags", 200, json!({"models":[]})),
            ("/api/ps", 200, json!({"models":[]})),
        ]);
        let mut runtime = Runtime::new(temp_dir("external"));
        runtime.endpoint = endpoint;
        assert!(runtime.stop().await.unwrap_err().contains("not started by AEGIS"));
        server.join().unwrap();
    }

    #[test]
    fn offline_manifest_requires_complete_config_and_layer_blobs() {
        let root = temp_dir("manifest");
        let manifest = root.join("models/manifests/registry.ollama.ai/library/qwen3/8b");
        fs::create_dir_all(manifest.parent().unwrap()).unwrap();
        let digest = "a".repeat(64);
        let config = "b".repeat(64);
        fs::write(
            &manifest,
            json!({
                "config":{"digest":format!("sha256:{config}"),"size":2},
                "layers":[{"digest":format!("sha256:{digest}"),"size":7}]
            })
            .to_string(),
        )
        .unwrap();
        let runtime = Runtime::new(root.clone());
        assert!(!runtime.downloaded());
        fs::create_dir_all(root.join("models/blobs")).unwrap();
        let layer_path = root.join("models/blobs").join(format!("sha256-{digest}"));
        fs::write(&layer_path, "weights").unwrap();
        assert!(!runtime.downloaded(), "missing config must not report downloaded");
        fs::write(root.join("models/blobs").join(format!("sha256-{config}")), "{}").unwrap();
        assert!(runtime.downloaded());
        fs::write(&layer_path, "weigh").unwrap();
        assert!(!runtime.downloaded(), "truncated layer must not report downloaded");
        fs::write(&layer_path, "weights-extra").unwrap();
        assert!(!runtime.downloaded(), "wrong-size layer must not report downloaded");
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn readiness_requires_successful_nonempty_version() {
        for (code, reply, expected) in [
            (200, json!({"version":"test"}), true),
            (500, json!({"version":"test"}), false),
            (200, json!({"version":"  "}), false),
            (200, json!({"error":"not Ollama"}), false),
        ] {
            let (url, server) = mock(vec![("/api/version", code, reply)]);
            assert_eq!(server_ready(&url).await, expected);
            server.join().unwrap();
        }
    }

    #[tokio::test]
    async fn failed_health_does_not_report_downloaded_or_loaded() {
        let (url, server) = mock(vec![
            ("/api/version", 200, json!({"version":"test"})),
            ("/api/tags", 500, json!({"models":[{"name":MODEL}]})),
            ("/api/ps", 500, json!({"models":[{"name":MODEL,"size_vram":123}]})),
        ]);
        let status = Runtime::inspect(&url).await;
        assert!(status.server_ready);
        assert!(!status.model_downloaded);
        assert!(!status.model_loaded);
        server.join().unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn failed_start_releases_owned_child_and_allows_retry() {
        use std::os::unix::fs::PermissionsExt;
        let root = temp_dir("failed-start");
        let binary = executable(&root);
        fs::create_dir_all(binary.parent().unwrap()).unwrap();
        fs::write(&binary, "#!/bin/sh\nexit 9\n").unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
        let (url, server) = mock(vec![("/api/version", 503, json!({"error":"offline"}))]);
        let mut runtime = Runtime::new(root.clone());
        runtime.endpoint = url;
        assert!(runtime.start().await.unwrap_err().contains("exited"));
        assert!(runtime.child.lock().unwrap().is_none());
        server.join().unwrap();
        assert!(runtime.start().await.unwrap_err().contains("exited"));
        assert!(runtime.child.lock().unwrap().is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn stop_kills_runner_even_when_leader_exits_on_term() {
        let root = temp_dir("process-tree");
        fs::create_dir_all(&root).unwrap();
        let pidfile = root.join("runner.pid");
        let script = "import os,signal,sys,time\npid=os.fork()\nif pid == 0:\n signal.signal(signal.SIGTERM,signal.SIG_IGN)\n open(sys.argv[1],'w').write(str(os.getpid()))\n while True: time.sleep(1)\nelse:\n while True: time.sleep(1)\n";
        let mut command = Command::new("python3");
        command
            .args(["-c", script])
            .arg(&pidfile)
            .process_group(0)
            .kill_on_drop(true)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let child = command.spawn().unwrap();
        let group = child.id().unwrap();
        for _ in 0..100 {
            if pidfile.is_file() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let runner = fs::read_to_string(&pidfile).unwrap();
        stop_process(child).await.unwrap();
        let mut stopped = false;
        for _ in 0..100 {
            stopped = fs::read_to_string(format!("/proc/{runner}/stat"))
                .map(|stat| stat.split_whitespace().nth(2) == Some("Z"))
                .unwrap_or(true);
            if stopped {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        if !stopped {
            let _ = signal_group(group, libc::SIGKILL);
        }
        assert!(stopped, "SIGTERM-ignoring model runner survived stop");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn extracts_zip_and_rejects_traversal() {
        let root = temp_dir("zip");
        fs::create_dir_all(&root).unwrap();
        let path = root.join("runtime.zip");
        for (name, safe) in [("bin/ollama", true), ("../escape", false)] {
            let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
            zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(b"binary").unwrap();
            zip.finish().unwrap();
            assert_eq!(extract(&path, "runtime.zip", &root.join("out")).is_ok(), safe);
        }
        assert_eq!(fs::read(root.join("out/bin/ollama")).unwrap(), b"binary");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_external_tar_symlinks() {
        let root = temp_dir("symlink");
        fs::create_dir_all(&root).unwrap();
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_path("bin/escape").unwrap();
        header.set_link_name("/tmp/outside").unwrap();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        header.set_mode(0o777);
        header.set_cksum();
        builder.append(&header, &[][..]).unwrap();
        assert!(extract_tar(&builder.into_inner().unwrap()[..], &root).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn checksums_require_exact_asset_and_valid_digest() {
        let hash = "a".repeat(64);
        assert_eq!(
            checksum(&format!("{hash}  ./ollama.zip\n"), "ollama.zip").unwrap(),
            hash
        );
        assert!(checksum("bad ollama.zip", "ollama.zip").is_err());
        assert!(checksum(&format!("{hash} other.zip"), "ollama.zip").is_err());
    }

    #[test]
    fn rejects_archive_escape_paths() {
        for p in ["../evil", "/tmp/evil", "bin/../../evil"] {
            assert!(!safe_relative(Path::new(p)), "{p}");
        }
        for p in ["bin/ollama", "lib/../bin/ollama", "./ollama"] {
            assert!(safe_relative(Path::new(p)), "{p}");
        }
    }

    #[test]
    fn pull_stream_handles_errors_and_completion() {
        let p = Arc::new(Mutex::new(Progress::default()));
        assert!(!pull_event(br#"{"status":"pulling","completed":12,"total":20}"#, &p).unwrap());
        assert_eq!(p.lock().unwrap().completed, 12);
        assert!(pull_event(br#"{"status":"success"}"#, &p).unwrap());
        assert!(pull_event(br#"{"error":"disk full"}"#, &p)
            .unwrap_err()
            .contains("disk full"));
        assert!(pull_event(b"garbage", &p).is_err());
    }

    #[test]
    fn model_detection_does_not_accept_other_models() {
        assert!(has_model(&json!({"models":[{"name":MODEL}]})));
        assert!(!has_model(&json!({"models":[{"name":"qwen3:4b"}]})));
    }
}

#[cfg(test)]
#[path = "local_ai_audit_tests.rs"]
mod audit_tests;
