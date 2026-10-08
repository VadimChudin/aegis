//! RoboForex through MetaTrader 5. MT5 has no network API, so the app drives a
//! Python process (`python/aegis_lab/bridges/mt5_bridge.py`) that talks to the
//! local terminal with the official `MetaTrader5` package, one JSON line per message.

use std::{path::Path, process::Stdio, time::Duration};

use serde_json::{json, Value};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::Mutex,
    time::timeout,
};

use crate::{
    broker::{require, BrokerError, Probe, TradeRequest, TradeResult, TradingState},
    checks::{Check, Checklist},
    live_market::MarketSample,
    market::{Candle, Timeframe},
    market_depth::OrderBookSnapshot,
};

const PROTOCOL: i64 = 1;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

struct Io {
    child: Child,
    stdin: ChildStdin,
    lines: BufReader<ChildStdout>,
    next_id: u64,
}

pub struct Mt5Bridge {
    io: Mutex<Io>,
    symbol: String,
    raw_symbol: String,
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
                raw_symbol: String::new(),
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
        let result = self.request(req, CONNECT_TIMEOUT).await?;
        let symbol = result["symbol"]
            .as_str()
            .ok_or_else(|| BrokerError::Bridge(format!("connect reply without symbol: {result}")))?;
        self.raw_symbol = symbol.to_string();
        self.symbol = symbol.to_string();
        Ok(result)
    }

    pub(crate) fn symbol(&self) -> &str {
        &self.symbol
    }

    pub(crate) async fn candles(&self, tf: Timeframe, limit: usize) -> Result<Vec<Candle>, BrokerError> {
        let result = self
            .request(
                json!({"cmd": "candles", "timeframe": tf.as_str(), "limit": limit}),
                REQUEST_TIMEOUT,
            )
            .await?;
        serde_json::from_value(result).map_err(|e| BrokerError::Parse(format!("MT5 candles: {e}")))
    }

    pub(crate) async fn order_book(&self) -> Result<OrderBookSnapshot, BrokerError> {
        let result = self.request(json!({"cmd": "order_book"}), REQUEST_TIMEOUT).await?;
        let snapshot: OrderBookSnapshot =
            serde_json::from_value(result).map_err(|e| BrokerError::Parse(format!("MT5 order book: {e}")))?;
        snapshot
            .validate()
            .map_err(|e| BrokerError::Parse(format!("MT5 order book: {e}")))?;
        Ok(snapshot)
    }

    pub(crate) async fn market_sample(&self) -> Result<MarketSample, BrokerError> {
        let result = self.request(json!({"cmd": "market_snapshot"}), REQUEST_TIMEOUT).await?;
        let sample: MarketSample =
            serde_json::from_value(result).map_err(|e| BrokerError::Parse(format!("MT5 market snapshot: {e}")))?;
        if sample.symbol != self.raw_symbol {
            return Err(BrokerError::Parse("Market snapshot symbol changed".into()));
        }
        sample
            .validate(crate::sign::now_ms().max(0) as u64)
            .map_err(|e| BrokerError::Parse(format!("MT5 market snapshot: {e}")))?;
        Ok(sample)
    }

    pub(crate) async fn history_ticks(
        &self,
        start_ms: u64,
        end_ms: u64,
        max_ticks: usize,
    ) -> Result<crate::event_tools::HistoryTicks, BrokerError> {
        crate::event_tools::validate_history_request(start_ms, end_ms, max_ticks, crate::sign::now_ms().max(0) as u64)
            .map_err(BrokerError::Input)?;
        // Python binds the actual terminal symbol and checks account/server before and after.
        let result = self.request(json!({"cmd":"history_ticks","start_ms":start_ms,"end_ms":end_ms,"max_ticks":max_ticks,"symbol":self.raw_symbol}), Duration::from_secs(2)).await?;
        if serde_json::to_vec(&result)
            .map_err(|e| BrokerError::Parse(e.to_string()))?
            .len()
            > 384 * 1024
        {
            return Err(BrokerError::Parse("MT5 history reply exceeds byte budget".into()));
        }
        let mut history: crate::event_tools::HistoryTicks =
            serde_json::from_value(result).map_err(|e| BrokerError::Parse(e.to_string()))?;
        // MT5 exposes the raw broker suffix; normalize only the connected XAUUSD family.
        if !self.symbol.starts_with("XAUUSD") || history.symbol != self.raw_symbol {
            return Err(BrokerError::Parse(
                "Historical symbol does not match connected XAUUSD".into(),
            ));
        }
        let raw_symbol = history.symbol.clone();
        history
            .validate(
                &raw_symbol,
                start_ms,
                end_ms,
                max_ticks,
                crate::sign::now_ms().max(0) as u64,
            )
            .map_err(BrokerError::Parse)?;
        history.coverage_note = format!("raw_symbol={raw_symbol}; {}", history.coverage_note);
        history.symbol = "XAUUSD".into();
        Ok(history)
    }

    pub(crate) async fn trading_state(&self) -> Result<TradingState, BrokerError> {
        let value = self.request(json!({"cmd": "trading_state"}), REQUEST_TIMEOUT).await?;
        serde_json::from_value(value).map_err(|e| BrokerError::Parse(format!("MT5 trading state: {e}")))
    }

    pub(crate) async fn place_order(&self, request: &TradeRequest) -> Result<TradeResult, BrokerError> {
        let mut value =
            serde_json::to_value(request).map_err(|e| BrokerError::Parse(format!("MT5 trade request: {e}")))?;
        value["cmd"] = json!("place_order");
        let result = self.request(value, REQUEST_TIMEOUT).await?;
        serde_json::from_value(result).map_err(|e| BrokerError::Parse(format!("MT5 trade result: {e}")))
    }

    pub(crate) async fn close_position(&self, ticket: u64) -> Result<TradeResult, BrokerError> {
        self.reduce_position(ticket, 1.0).await
    }

    pub(crate) async fn close_all(&self) -> Result<Vec<TradeResult>, BrokerError> {
        let result = self.request(json!({"cmd": "close_all"}), REQUEST_TIMEOUT).await?;
        serde_json::from_value(result).map_err(|e| BrokerError::Parse(format!("MT5 close-all result: {e}")))
    }

    pub(crate) async fn reduce_position(&self, ticket: u64, fraction: f64) -> Result<TradeResult, BrokerError> {
        let result = self
            .request(
                json!({"cmd": "reduce_position", "ticket": ticket, "fraction": fraction}),
                REQUEST_TIMEOUT,
            )
            .await?;
        serde_json::from_value(result).map_err(|e| BrokerError::Parse(format!("MT5 reduce result: {e}")))
    }

    pub(crate) async fn modify_stop(
        &self,
        ticket: u64,
        stop: f64,
        target: Option<f64>,
    ) -> Result<TradeResult, BrokerError> {
        let result = self
            .request(
                json!({"cmd": "modify_stop", "ticket": ticket, "stop": stop, "target": target}),
                REQUEST_TIMEOUT,
            )
            .await?;
        serde_json::from_value(result).map_err(|e| BrokerError::Parse(format!("MT5 stop update: {e}")))
    }

    pub(crate) async fn shutdown(&self) {
        let _ = self.request(json!({"cmd": "shutdown"}), Duration::from_secs(5)).await;
        let _ = self.io.lock().await.child.start_kill();
    }

    /// Sends one request and waits for the reply with the same id. Replies to
    /// requests that were cancelled mid-flight are skipped.
    async fn request(&self, mut req: Value, wait: Duration) -> Result<Value, BrokerError> {
        let mut io = self.io.lock().await;
        io.next_id += 1;
        let id = io.next_id;
        req["id"] = json!(id);
        let mut line = req.to_string();
        line.push('\n');
        io.stdin.write_all(line.as_bytes()).await.map_err(|e| gone(&e))?;
        io.stdin.flush().await.map_err(|e| gone(&e))?;

        let read = async {
            loop {
                let Some(line) = bounded_line(&mut io.lines).await? else {
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
        timeout(wait, read)
            .await
            .map_err(|_| BrokerError::Bridge(format!("no reply within {}s", wait.as_secs())))?
    }
}

async fn bounded_line(reader: &mut BufReader<ChildStdout>) -> Result<Option<String>, BrokerError> {
    let mut bytes = Vec::new();
    loop {
        let chunk = reader.fill_buf().await.map_err(|e| gone(&e))?;
        if chunk.is_empty() {
            return if bytes.is_empty() {
                Ok(None)
            } else {
                Err(BrokerError::Bridge("Incomplete bridge reply".into()))
            };
        }
        let count = chunk.iter().position(|x| *x == b'\n').map_or(chunk.len(), |n| n + 1);
        if bytes.len() + count > 384 * 1024 {
            return Err(BrokerError::Bridge("Bridge reply exceeds bounded line budget".into()));
        }
        let done = chunk[count - 1] == b'\n';
        bytes.extend_from_slice(&chunk[..count]);
        reader.consume(count);
        if done {
            return String::from_utf8(bytes)
                .map(Some)
                .map_err(|_| BrokerError::Parse("Bridge reply is not UTF-8".into()));
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
        lines: BufReader::new(stdout),
        next_id: 0,
    })
}
