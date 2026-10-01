//! RoboForex through MetaTrader 5. MT5 has no network API, so the app drives a
//! Python process (`python/aegis_lab/bridges/mt5_bridge.py`) that talks to the
//! local terminal with the official `MetaTrader5` package, one JSON line per message.

use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};

use serde_json::{json, Value};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::Mutex,
    time::timeout,
};

use crate::{
    broker::{require, BrokerError, Probe},
    checks::{Check, Checklist},
    market::{Candle, Timeframe},
    market_depth::OrderBookSnapshot,
};

const PROTOCOL: i64 = 1;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
/// A bridge that misses this many replies in a row is treated as hung and restarted.
const MAX_SILENT: u32 = 3;
/// Minimum pause between two restarts of a dead bridge.
const REVIVE_PAUSE: Duration = Duration::from_secs(5);

struct Io {
    child: Child,
    stdin: ChildStdin,
    lines: Lines<BufReader<ChildStdout>>,
    next_id: u64,
    /// Requests in a row that got no reply in time.
    silent: u32,
    /// When the bridge was last (re)started; paces restarts.
    started: Instant,
}

/// What the bridge needs to start again and log back in after it died.
struct Relaunch {
    python: String,
    script: PathBuf,
    login: Value,
}

pub struct Mt5Bridge {
    io: Mutex<Io>,
    symbol: String,
    relaunch: Option<Relaunch>,
}

fn default_python() -> &'static str {
    if cfg!(windows) {
        "python"
    } else {
        "python3"
    }
}

const AFTER_PYTHON: [(&str, &str); 3] = [
    ("package", "MetaTrader5 package"),
    ("login", "MT5 login"),
    ("terminal", "Terminal"),
];

impl Mt5Bridge {
    /// Starts the bridge, logs in and returns the terminal's own checklist.
    pub(crate) async fn probe(
        python: Option<&str>,
        script: &Path,
        login: &str,
        password: &str,
        server: &str,
        terminal_path: Option<&str>,
    ) -> Probe<Self> {
        let mut list = Checklist::default();
        if login.trim().parse::<u64>().is_err() {
            list.fail("input", "Credentials", "MT5 login must be a number");
            return Probe {
                conn: None,
                checks: list,
                account: None,
            };
        }
        let python = python
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .unwrap_or(default_python());
        let bridge = match spawn(python, script) {
            Ok(io) => Mt5Bridge {
                io: Mutex::new(io),
                symbol: String::new(),
                relaunch: None,
            },
            Err(e) => {
                list.fail("python", "Python", e.to_string());
                list.skip(&AFTER_PYTHON);
                return Probe {
                    conn: None,
                    checks: list,
                    account: None,
                };
            }
        };
        let hello = match bridge.request(json!({"cmd": "hello"}), REQUEST_TIMEOUT).await {
            Ok(h) if h["protocol"].as_i64() == Some(PROTOCOL) => h,
            Ok(h) => {
                list.fail("python", "Python", format!("unsupported bridge protocol: {h}"));
                list.skip(&AFTER_PYTHON);
                return bridge.fail(list).await;
            }
            Err(e) => {
                list.fail("python", "Python", e.to_string());
                list.skip(&AFTER_PYTHON);
                return bridge.fail(list).await;
            }
        };
        list.ok(
            "python",
            "Python",
            hello["python"].as_str().unwrap_or("found").to_string(),
        );
        match hello["mt5_package"].as_str() {
            Some(version) => list.ok("package", "MetaTrader5 package", version.to_string()),
            None => {
                let why = hello["mt5_error"]
                    .as_str()
                    .unwrap_or("not installed: run `pip install MetaTrader5`");
                list.fail("package", "MetaTrader5 package", why.to_string());
                list.skip(&AFTER_PYTHON[1..]);
                return bridge.fail(list).await;
            }
        }
        let mut bridge = bridge;
        match bridge.login(login, password, server, terminal_path).await {
            Ok(result) => {
                let mut again = json!({"cmd": "connect", "login": login.trim().parse::<u64>().unwrap_or_default(),
                    "password": password.trim(), "server": server.trim()});
                if let Some(path) = terminal_path.map(str::trim).filter(|p| !p.is_empty()) {
                    again["terminal_path"] = json!(path);
                }
                bridge.remember(python, script, again);
                list.ok("login", "MT5 login", format!("{login} · {server}"));
                if let Ok(checks) = serde_json::from_value::<Vec<Check>>(result["checks"].clone()) {
                    list.extend(checks);
                }
                let account = result["account"].as_str().unwrap_or("MT5 account").to_string();
                Probe {
                    conn: Some(bridge),
                    checks: list,
                    account: Some(account),
                }
            }
            Err(e) => {
                list.fail("login", "MT5 login", e.to_string());
                list.skip(&AFTER_PYTHON[2..]);
                bridge.fail(list).await
            }
        }
    }

    async fn fail(self, checks: Checklist) -> Probe<Self> {
        self.shutdown().await;
        Probe {
            conn: None,
            checks,
            account: None,
        }
    }

    async fn login(
        &mut self,
        login: &str,
        password: &str,
        server: &str,
        terminal_path: Option<&str>,
    ) -> Result<Value, BrokerError> {
        let login: u64 = require(login, "MT5 login")?
            .parse()
            .map_err(|_| BrokerError::Input("MT5 login must be a number".into()))?;
        let mut req = json!({"cmd": "connect", "login": login, "password": password.trim(), "server": server.trim()});
        if let Some(path) = terminal_path.map(str::trim).filter(|p| !p.is_empty()) {
            req["terminal_path"] = json!(path);
        }
        let result = self.request(req.clone(), CONNECT_TIMEOUT).await?;
        let symbol = result["symbol"]
            .as_str()
            .ok_or_else(|| BrokerError::Bridge(format!("connect reply without symbol: {result}")))?;
        self.symbol = symbol.to_string();
        Ok(result)
    }

    /// Remembers how to start this bridge again, so a crashed Python process is replaced
    /// and logged back in instead of failing the session for good.
    fn remember(&mut self, python: &str, script: &Path, login: Value) {
        self.relaunch = Some(Relaunch {
            python: python.to_string(),
            script: script.to_path_buf(),
            login,
        });
    }

    /// Restarts a dead or hung bridge and logs it back in. Paced by `REVIVE_PAUSE`.
    async fn revive(&self) -> Result<(), BrokerError> {
        let Some(relaunch) = &self.relaunch else {
            return Err(BrokerError::Bridge("the bridge process exited".into()));
        };
        let mut io = self.io.lock().await;
        if io.alive() {
            return Ok(());
        }
        if io.started.elapsed() < REVIVE_PAUSE {
            return Err(BrokerError::Bridge("the bridge stopped; restarting it".into()));
        }
        log::warn!("mt5 bridge: restarting the Python process");
        io.started = Instant::now();
        let mut fresh = spawn(&relaunch.python, &relaunch.script)?;
        exchange(&mut fresh, json!({"cmd": "hello"}), REQUEST_TIMEOUT).await?;
        exchange(&mut fresh, relaunch.login.clone(), CONNECT_TIMEOUT).await?;
        *io = fresh;
        Ok(())
    }

    pub(crate) fn symbol(&self) -> &str {
        &self.symbol
    }

    pub(crate) async fn candles(&self, tf: Timeframe, limit: usize) -> Result<Vec<Candle>, BrokerError> {
        let result = self
            .call(json!({"cmd": "candles", "timeframe": tf.as_str(), "limit": limit}))
            .await?;
        serde_json::from_value(result).map_err(|e| BrokerError::Parse(format!("MT5 candles: {e}")))
    }

    pub(crate) async fn order_book(&self) -> Result<OrderBookSnapshot, BrokerError> {
        let result = self.call(json!({"cmd": "order_book"})).await?;
        let snapshot: OrderBookSnapshot =
            serde_json::from_value(result).map_err(|e| BrokerError::Parse(format!("MT5 order book: {e}")))?;
        snapshot
            .validate()
            .map_err(|e| BrokerError::Parse(format!("MT5 order book: {e}")))?;
        Ok(snapshot)
    }

    /// A session request that survives a crashed or hung bridge: the bridge is restarted,
    /// logged back in and the request sent once more.
    async fn call(&self, req: Value) -> Result<Value, BrokerError> {
        match self.request(req.clone(), REQUEST_TIMEOUT).await {
            Ok(result) => Ok(result),
            Err(e) => {
                let dead = self.relaunch.is_some() && !self.io.lock().await.alive();
                if !dead {
                    return Err(e);
                }
                log::warn!("mt5 bridge: {e}");
                self.revive().await?;
                self.request(req, REQUEST_TIMEOUT).await
            }
        }
    }

    pub(crate) async fn shutdown(&self) {
        let _ = self.request(json!({"cmd": "shutdown"}), Duration::from_secs(5)).await;
        let _ = self.io.lock().await.child.start_kill();
    }

    /// Kills the Python process, as a crash would (tests only).
    #[doc(hidden)]
    pub async fn kill_for_test(&self) {
        let mut io = self.io.lock().await;
        let _ = io.child.kill().await;
    }

    async fn request(&self, req: Value, wait: Duration) -> Result<Value, BrokerError> {
        let mut io = self.io.lock().await;
        exchange(&mut io, req, wait).await
    }
}

impl Io {
    /// The process is running and answered recently enough to be trusted.
    fn alive(&mut self) -> bool {
        self.silent < MAX_SILENT && matches!(self.child.try_wait(), Ok(None))
    }
}

/// Sends one request and waits for the reply with the same id. Replies to
/// requests that were cancelled mid-flight are skipped. A bridge that keeps
/// missing replies is killed so the next request restarts it.
async fn exchange(io: &mut Io, mut req: Value, wait: Duration) -> Result<Value, BrokerError> {
    io.next_id += 1;
    let id = io.next_id;
    req["id"] = json!(id);
    let mut line = req.to_string();
    line.push('\n');
    io.stdin.write_all(line.as_bytes()).await.map_err(|e| gone(&e))?;
    io.stdin.flush().await.map_err(|e| gone(&e))?;

    let read = async {
        loop {
            let Some(line) = io.lines.next_line().await.map_err(|e| gone(&e))? else {
                return Err(BrokerError::Bridge("the bridge process exited".into()));
            };
            let Ok(reply) = serde_json::from_str::<Value>(&line) else {
                log::warn!("mt5 bridge: {line}");
                continue;
            };
            if reply["id"].as_u64() != Some(id) {
                continue;
            }
            return if reply["ok"] == true {
                Ok(reply["result"].clone())
            } else {
                Err(BrokerError::Bridge(
                    reply["error"].as_str().unwrap_or("unknown error").to_string(),
                ))
            };
        }
    };
    match timeout(wait, read).await {
        Ok(reply) => {
            io.silent = 0;
            reply
        }
        Err(_) => {
            io.silent += 1;
            if io.silent >= MAX_SILENT {
                log::warn!("mt5 bridge: {MAX_SILENT} requests without a reply, stopping it");
                let _ = io.child.start_kill();
            }
            Err(BrokerError::Bridge(format!("no reply within {}s", wait.as_secs())))
        }
    }
}

fn gone(e: &std::io::Error) -> BrokerError {
    BrokerError::Bridge(format!("the bridge process is not running ({e})"))
}

fn spawn(python: &str, script: &Path) -> Result<Io, BrokerError> {
    let mut cmd = Command::new(python);
    cmd.arg("-u")
        .arg(script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console flashes up next to the app
    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            BrokerError::Bridge(format!(
                "Python was not found (`{python}`). Install Python 3, run `pip install MetaTrader5`, or set the Python path"
            ))
        } else {
            BrokerError::Bridge(format!("cannot start `{python}`: {e}"))
        }
    })?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| BrokerError::Bridge("no stdin".into()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| BrokerError::Bridge("no stdout".into()))?;
    Ok(Io {
        child,
        stdin,
        lines: BufReader::new(stdout).lines(),
        next_id: 0,
        silent: 0,
        started: Instant::now(),
    })
}
