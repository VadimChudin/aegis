//! Permanent immutable records. Incomplete .pending files are preserved but never read.
//! One fsynced file per identifier avoids torn-tail truncation and unbounded replay RAM.
use serde::{de::DeserializeOwned, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
const MAX_RECORD: u64 = 16 * 1024 * 1024;
// Directory fsync is supported on Unix. Windows retains file fsync and atomic
// no-overwrite publication; std::fs::File cannot open directories there.
fn sync_directory(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        File::open(path).and_then(|f| f.sync_all()).map_err(|e| e.to_string())?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}
#[derive(serde::Serialize, serde::Deserialize)]
struct Envelope {
    sha256: String,
    payload: serde_json::Value,
}
pub struct Archive {
    root: PathBuf,
}
impl Archive {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, String> {
        fs::create_dir_all(root.as_ref()).map_err(|e| e.to_string())?;
        if let Some(parent) = root.as_ref().parent() {
            sync_directory(parent)?;
        }
        let archive = Self {
            root: root.as_ref().into(),
        };
        archive.visit::<serde_json::Value>(|_| Ok(()))?;
        Ok(archive)
    }
    pub fn contains(&self, id: &str) -> Result<bool, String> {
        let path = self.root.join(format!("{:x}.json", Sha256::digest(id.as_bytes())));
        if !path.exists() {
            return Ok(false);
        }
        let _: serde_json::Value = Self::read(&path)?;
        Ok(true)
    }
    pub fn append<T: Serialize>(&self, id: &str, record: &T) -> Result<(), String> {
        let payload = serde_json::to_value(record).map_err(|e| e.to_string())?;
        let hash = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&payload).map_err(|e| e.to_string())?)
        );
        let key = format!("{:x}", Sha256::digest(id.as_bytes()));
        let final_path = self.root.join(format!("{key}.json"));
        if final_path.exists() {
            let old: serde_json::Value = Self::read(&final_path)?;
            return if old == payload {
                Ok(())
            } else {
                Err("Immutable archive identity conflict".into())
            };
        }
        let bytes = serde_json::to_vec(&Envelope { sha256: hash, payload }).map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_RECORD {
            return Err("Archive record exceeds 16 MiB".into());
        }
        // Unique staging files preserve earlier torn attempts. Never overwrite an immutable record.
        let mut nonce = 0u64;
        let (pending, mut file) = loop {
            let path = self
                .root
                .join(format!("{key}.{}.{}.pending", std::process::id(), nonce));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => break (path, file),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => nonce += 1,
                Err(e) => return Err(e.to_string()),
            }
        };
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        drop(file);
        // Hard link publishes atomically without replacing a concurrent writer's record.
        match fs::hard_link(&pending, &final_path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let old: serde_json::Value = Self::read(&final_path)?;
                if old != serde_json::to_value(record).map_err(|e| e.to_string())? {
                    return Err("Archive identity conflict".into());
                }
            }
            Err(e) => return Err(e.to_string()),
        }
        sync_directory(&self.root)?;
        // Staging hardlinks intentionally retained: no evidence deletion or tail truncation.
        Ok(())
    }
    fn read<T: DeserializeOwned>(path: &Path) -> Result<T, String> {
        let file = File::open(path).map_err(|e| e.to_string())?;
        if file.metadata().map_err(|e| e.to_string())?.len() > MAX_RECORD {
            return Err("Oversize archive record".into());
        }
        let mut bytes = Vec::new();
        file.take(MAX_RECORD + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_RECORD {
            return Err("Oversize archive record".into());
        }
        let envelope: Envelope = serde_json::from_slice(&bytes).map_err(|e| format!("Corrupt archive: {e}"))?;
        let actual = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&envelope.payload).map_err(|e| e.to_string())?)
        );
        if actual != envelope.sha256 {
            return Err("Archive checksum mismatch".into());
        }
        serde_json::from_value(envelope.payload).map_err(|e| e.to_string())
    }
    pub fn visit<T: DeserializeOwned>(&self, mut f: impl FnMut(T) -> Result<(), String>) -> Result<(), String> {
        for item in fs::read_dir(&self.root).map_err(|e| e.to_string())? {
            let path = item.map_err(|e| e.to_string())?.path();
            if path.extension().is_some_and(|x| x == "json") {
                f(Self::read(&path)?)?;
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn immutable_restart_torn_and_corrupt() {
        let root = std::env::temp_dir().join(format!(
            "aegis-archive-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let a = Archive::open(&root).unwrap();
        a.append("a", &serde_json::json!({"net":12})).unwrap();
        a.append("a", &serde_json::json!({"net":12})).unwrap();
        assert!(a.append("a", &serde_json::json!({"net":13})).is_err());
        fs::write(root.join("torn.pending"), b"{incomplete").unwrap();
        let a = Archive::open(&root).unwrap();
        let mut count = 0;
        a.visit::<serde_json::Value>(|_| {
            count += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(count, 1);
        let path = fs::read_dir(&root)
            .unwrap()
            .map(|x| x.unwrap().path())
            .find(|x| x.extension().is_some_and(|e| e == "json"))
            .unwrap();
        fs::write(path, b"broken").unwrap();
        assert!(Archive::open(root).is_err());
    }
}
