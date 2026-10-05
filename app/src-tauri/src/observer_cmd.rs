use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Mutex as StdMutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use aegis_core::{
    live_market::MarketSample,
    local_ai::{Runtime, ENDPOINT},
    observer::{self, FrameSummary, Journal, ModelDecision, ObserverConfig, PaperPosition, SimState, Snapshot},
    BrokerId, Connector, Timeframe,
};
use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager, State};
use tokio::sync::{watch, Mutex};

use crate::{local_ai_cmd::LocalAiState, AppState};

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(Clone, Serialize)]
pub struct LastDecision {
    strategy_id: String,
    decision: ModelDecision,
    accepted: bool,
    reason: String,
    elapsed_ms: u64,
}

#[derive(Clone, Serialize)]
pub struct ObserverStatus {
    running: bool,
    phase: String,
    error: Option<String>,
    sample: Option<MarketSample>,
    frames: Vec<FrameSummary>,
    last_decision: Option<LastDecision>,
    decisions: u64,
    outcomes: u64,
    position: Option<PaperPosition>,
    equity: f64,
    journal_path: String,
}

struct Inner {
    config: ObserverConfig,
    sim: SimState,
    journal: Option<Journal>,
    status: ObserverStatus,
    recovery_error: Option<String>,
}

pub struct ObserverState {
    inner: Mutex<Inner>,
    running: AtomicBool,
    epoch: AtomicU64,
    sequence: AtomicU64,
    changed: watch::Sender<u64>,
    root: PathBuf,
    tasks: StdMutex<Vec<tauri::async_runtime::JoinHandle<()>>>,
}

#[derive(Serialize)]
pub struct Info {
    config: ObserverConfig,
    defaults: ObserverConfig,
    status: ObserverStatus,
}

fn write_json(path: &std::path::Path, value: &impl Serialize) -> Result<(), String> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(&tmp).map_err(|e| e.to_string())?;
    file.write_all(&serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    drop(file);
    std::fs::rename(tmp, path).map_err(|e| e.to_string())
}

impl ObserverState {
    pub fn new(root: PathBuf) -> Self {
        let config_path = root.join("config.json");
        let read = (|| -> Result<ObserverConfig, String> {
            match std::fs::read(&config_path) {
                Ok(bytes) => {
                    let config: ObserverConfig = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
                    config.validate()?;
                    Ok(config)
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ObserverConfig::default()),
                Err(e) => Err(e.to_string()),
            }
        })();
        let mut recovery_error = read.as_ref().err().cloned();
        let config = read.unwrap_or_default();
        let journal_path = root.join("decisions.jsonl");
        let mut sim = SimState::new(config.initial_equity).expect("default config is valid");
        let mut decisions = 0;
        let mut outcomes = 0;
        let journal = match Journal::new(&journal_path) {
            Ok(journal) => {
                match journal.replay() {
                    Ok(records) => {
                        for record in records {
                            if record["kind"] == "decision" {
                                decisions += 1;
                            }
                            if record["kind"] == "outcome" {
                                outcomes += 1;
                            }
                            if record.get("sim_state").is_some() {
                                let recovered = serde_json::from_value::<SimState>(record["sim_state"].clone());
                                match recovered.and_then(|s| s.validate().map(|_| s).map_err(serde::de::Error::custom))
                                {
                                    Ok(state) => sim = state,
                                    Err(e) => {
                                        recovery_error = Some(format!("Invalid saved simulator state: {e}"));
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => recovery_error = Some(e),
                }
                Some(journal)
            }
            Err(e) => {
                recovery_error = Some(e);
                None
            }
        };
        let status = ObserverStatus {
            running: false,
            phase: "Stopped; simulation only".into(),
            error: recovery_error.clone(),
            sample: None,
            frames: Vec::new(),
            last_decision: None,
            decisions,
            outcomes,
            position: sim.position.clone(),
            equity: sim.equity,
            journal_path: journal_path.display().to_string(),
        };
        let (changed, _) = watch::channel(0);
        Self {
            inner: Mutex::new(Inner {
                config,
                sim,
                journal,
                status,
                recovery_error,
            }),
            running: AtomicBool::new(false),
            epoch: AtomicU64::new(0),
            sequence: AtomicU64::new(now_ms()),
            changed,
            root,
            tasks: StdMutex::new(Vec::new()),
        }
    }

    pub fn running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn spawn(&self, app: AppHandle) {
        let mut tasks = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
        tasks.push(tauri::async_runtime::spawn(sample_loop(app.clone())));
        tasks.push(tauri::async_runtime::spawn(decision_loop(app)));
    }

    pub fn shutdown(&self) {
        self.running.store(false, Ordering::SeqCst);
        self.epoch.fetch_add(1, Ordering::SeqCst);
        for task in self.tasks.lock().unwrap_or_else(|e| e.into_inner()).drain(..) {
            task.abort();
        }
    }
}

#[tauri::command]
pub async fn observer_info(state: State<'_, ObserverState>) -> Result<Info, String> {
    let inner = state.inner.lock().await;
    Ok(Info {
        config: inner.config.clone(),
        defaults: ObserverConfig::default(),
        status: inner.status.clone(),
    })
}

#[tauri::command]
pub async fn observer_status(state: State<'_, ObserverState>) -> Result<ObserverStatus, String> {
    Ok(state.inner.lock().await.status.clone())
}

#[tauri::command]
pub async fn observer_save(state: State<'_, ObserverState>, config: ObserverConfig) -> Result<ObserverConfig, String> {
    config.validate()?;
    let mut inner = state.inner.lock().await;
    if state.running() || inner.sim.position.is_some() {
        return Err("Stop observation and wait for the simulated position to close before editing settings".into());
    }
    if let Some(e) = &inner.recovery_error {
        return Err(e.clone());
    }
    if config.initial_equity != inner.config.initial_equity && inner.status.decisions > 0 {
        return Err("Initial simulation equity cannot change after recording decisions".into());
    }
    write_json(&state.root.join("config.json"), &config)?;
    if inner.status.decisions == 0 {
        inner.sim = SimState::new(config.initial_equity)?;
        inner.status.equity = inner.sim.equity;
    }
    inner.config = config.clone();
    Ok(config)
}

#[tauri::command]
pub async fn observer_start(app: AppHandle, state: State<'_, ObserverState>) -> Result<ObserverStatus, String> {
    let appstate = app.state::<AppState>();
    let connector = connector(&appstate).await?;
    let sample = connector.market_sample().await.map_err(|e| e.to_string())?;
    sample.validate(now_ms())?;
    let model = Runtime::inspect(ENDPOINT).await;
    if !model.server_ready || !model.model_downloaded {
        return Err("Install Qwen and press Start server in Local AI first".into());
    }
    crate::ai_cmd::stop_auto(&appstate).await;
    let mut inner = state.inner.lock().await;
    if let Some(error) = &inner.recovery_error {
        return Err(error.clone());
    }
    inner.config.validate()?;
    if !inner.config.strategies.iter().any(|s| s.enabled) {
        return Err("Enable at least one strategy".into());
    }
    if state.running() {
        return Ok(inner.status.clone());
    }
    let config = inner.config.clone();
    let sim = inner.sim.clone();
    journal(
        &mut inner,
        json!({"schema_version":1,"kind":"start","at_ms":now_ms(),"config":config,"sim_state":sim,"source":"RoboForex MT5 XAUUSD; simulated only"}),
    )?;
    inner.status.sample = Some(sample);
    inner.status.frames.clear();
    inner.status.running = true;
    inner.status.phase = "Collecting RoboForex timeframes".into();
    inner.status.error = None;
    state.epoch.fetch_add(1, Ordering::SeqCst);
    state.running.store(true, Ordering::SeqCst);
    Ok(inner.status.clone())
}

#[tauri::command]
pub async fn observer_stop(state: State<'_, ObserverState>) -> Result<ObserverStatus, String> {
    state.running.store(false, Ordering::SeqCst);
    state.epoch.fetch_add(1, Ordering::SeqCst);
    state.changed.send_replace(state.epoch.load(Ordering::SeqCst));
    let mut inner = state.inner.lock().await;
    inner.status.running = false;
    inner.status.phase = "Stopped; pending inference cancelled, existing simulation remains protected".into();
    let sim = inner.sim.clone();
    journal(
        &mut inner,
        json!({"schema_version":1,"kind":"stop","at_ms":now_ms(),"sim_state":sim}),
    )?;
    Ok(inner.status.clone())
}

#[tauri::command]
pub async fn observer_journal(state: State<'_, ObserverState>) -> Result<String, String> {
    let inner = state.inner.lock().await;
    inner.journal.as_ref().ok_or("Journal is unavailable")?.export()
}

async fn connector(state: &AppState) -> Result<std::sync::Arc<Connector>, String> {
    let connector = state
        .sessions
        .lock()
        .await
        .get(&BrokerId::Roboforex)
        .map(|s| s.connector.clone())
        .ok_or("Connect RoboForex MT5 in Brokers first")?;
    if connector.symbol() != "XAUUSD" {
        return Err(format!(
            "Observer is restricted to XAUUSD, connected symbol is {}",
            connector.symbol()
        ));
    }
    Ok(connector)
}

fn journal(inner: &mut Inner, value: Value) -> Result<(), String> {
    inner.journal.as_mut().ok_or("Journal is unavailable")?.append(&value)
}

async fn sample_loop(app: AppHandle) {
    let state = app.state::<ObserverState>();
    let appstate = app.state::<AppState>();
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut frames_at = 0u64;
    let mut notified_at = 0u64;
    let mut notified_mid = 0.0f64;
    let mut notified_book = String::new();
    loop {
        interval.tick().await;
        let needs_quotes = state.running() || state.inner.lock().await.sim.position.is_some();
        if !needs_quotes {
            continue;
        }
        let result = async {
            let conn = connector(&appstate).await?;
            let sample = conn.market_sample().await.map_err(|e| e.to_string())?;
            sample.validate(now_ms())?;
            let mut inner = state.inner.lock().await;
            let config = inner.config.clone();
            let mut next_sim = inner.sim.clone();
            if let Some(outcome) = next_sim.mark(&sample, &config) {
                let sim = next_sim.clone();
                journal(&mut inner, json!({"schema_version":1,"kind":"outcome","outcome":outcome,"sim_state":sim,"source":"simulated MT5 quote fills"}))?;
                inner.sim = next_sim;
                inner.status.outcomes += 1;
            }
            inner.status.position = inner.sim.position.clone();
            inner.status.equity = inner.sim.equity;
            inner.status.sample = Some(sample);
            if state.running() && !inner.status.frames.is_empty() {
                let sample = inner.status.sample.as_ref().expect("sample was assigned");
                let mid = (sample.quote.bid + sample.quote.ask) / 2.;
                let book = sample.book.as_ref().map(|b| format!("{:?}:{:?}", b.bids.first(), b.asks.first())).unwrap_or_default();
                let threshold = inner.status.frames.iter().find(|f| f.timeframe == Timeframe::M1)
                    .map_or(0.1, |f| (f.atr * 0.1).max(0.02));
                if now_ms().saturating_sub(notified_at) >= 5_000
                    && ((mid - notified_mid).abs() >= threshold || book != notified_book) {
                    notified_at = now_ms();
                    notified_mid = mid;
                    notified_book = book;
                    state.changed.send_replace(state.sequence.fetch_add(1, Ordering::SeqCst));
                }
            }
            drop(inner);
            if state.running() && (now_ms().saturating_sub(frames_at) >= 15_000) {
                let mut frames = Vec::new();
                let mut failures = Vec::new();
                for tf in Timeframe::ALL {
                    match conn.candles(tf, 80).await.map_err(|e| e.to_string())
                        .and_then(|bars| observer::summarize(tf, &bars, now_ms())) {
                        Ok(frame) => frames.push(frame),
                        Err(e) => failures.push(format!("{}: {e}", tf.as_str())),
                    }
                }
                frames_at = now_ms();
                let mut inner = state.inner.lock().await;
                inner.status.frames = frames;
                if inner.status.frames.is_empty() { return Err(format!("No usable closed candles: {}", failures.join("; "))); }
                inner.status.error = if failures.is_empty() { None } else { Some(format!("Unavailable frames: {}", failures.join("; "))) };
                state.changed.send_replace(state.sequence.fetch_add(1, Ordering::SeqCst));
            }
            Ok::<_, String>(())
        }.await;
        if let Err(error) = result {
            let mut inner = state.inner.lock().await;
            inner.status.error = Some(error);
            inner.status.phase = "Waiting for fresh MT5 data; no new simulation entries".into();
            inner.status.sample = None;
            frames_at = 0;
        }
    }
}

async fn decision_loop(app: AppHandle) {
    let state = app.state::<ObserverState>();
    let mut changed = state.changed.subscribe();
    let mut strategy_cursor = 0usize;
    let mut last_keys = HashMap::new();
    loop {
        if changed.changed().await.is_err() {
            break;
        }
        if !state.running() {
            continue;
        }
        let epoch = state.epoch.load(Ordering::SeqCst);
        let (config, strategy, snapshot) = {
            let inner = state.inner.lock().await;
            let Some(sample) = inner.status.sample.clone() else {
                continue;
            };
            if sample.validate(now_ms()).is_err() || inner.status.frames.is_empty() {
                continue;
            }
            let enabled: Vec<_> = inner.config.strategies.iter().filter(|s| s.enabled).cloned().collect();
            if enabled.is_empty() {
                continue;
            }
            let strategy = enabled[strategy_cursor % enabled.len()].clone();
            strategy_cursor += 1;
            let frames: Vec<_> = inner
                .status
                .frames
                .iter()
                .filter(|f| inner.config.strictness < 100 || strategy.timeframes.contains(&f.timeframe))
                .cloned()
                .collect();
            if frames.is_empty() {
                continue;
            }
            let key = format!(
                "{epoch}:{}:{}:{:.2}:{:.2}",
                strategy.id,
                frames
                    .iter()
                    .map(|f| f.closed_at_ms.to_string())
                    .collect::<Vec<_>>()
                    .join(","),
                sample.quote.bid,
                sample.quote.ask
            );
            if last_keys.get(&strategy.id) == Some(&key) {
                continue;
            }
            last_keys.insert(strategy.id.clone(), key);
            let snapshot = Snapshot {
                schema_version: 1,
                id: state.sequence.fetch_add(1, Ordering::SeqCst),
                market: sample,
                frames,
            };
            (inner.config.clone(), strategy, snapshot)
        };
        let mut input = observer::request_payload(&snapshot, &strategy);
        // Full source data is journaled; the model gets bounded summaries, never hundreds of ticks.
        if let Some(market) = input.pointer_mut("/snapshot/market").and_then(Value::as_object_mut) {
            if let Some(ticks) = market.get_mut("ticks").and_then(Value::as_array_mut) {
                let keep = ticks.len().saturating_sub(6);
                ticks.drain(..keep);
            }
            if let Some(book) = market.get_mut("book").and_then(Value::as_object_mut) {
                for side in ["bids", "asks"] {
                    if let Some(levels) = book.get_mut(side).and_then(Value::as_array_mut) {
                        levels.truncate(5);
                    }
                }
            }
        }
        if let Some(frames) = input.pointer_mut("/snapshot/frames").and_then(Value::as_array_mut) {
            for frame in frames {
                let value = json!({
                    "timeframe":frame["timeframe"],"closed_at_ms":frame["closed_at_ms"],
                    "OHLC":[frame["last_closed"]["open"],frame["last_closed"]["high"],frame["last_closed"]["low"],frame["last_closed"]["close"]],
                    "S":frame["support"],"R":frame["resistance"],"ATR":frame["atr"],"trend":frame["trend"]
                });
                *frame = value;
            }
        }
        if let Some(market) = input.pointer_mut("/snapshot/market").and_then(Value::as_object_mut) {
            market.remove("ticks");
            market.remove("book_note");
        }
        let system = observer::system_prompt(&config, &strategy);
        let prompt = input.to_string();
        {
            let mut inner = state.inner.lock().await;
            inner.status.phase = format!("Qwen analysing {}", strategy.id);
            let version = aegis_core::local_ai::MODEL;
            if let Err(error) = journal(
                &mut inner,
                json!({"schema_version":1,"kind":"snapshot","source":"RoboForex MT5 XAUUSD","snapshot":snapshot,"config":config,"model_id":version,"strategy_id":strategy.id}),
            ) {
                inner.status.error = Some(error);
                state.running.store(false, Ordering::SeqCst);
                inner.status.running = false;
                continue;
            }
        }
        let local = app.state::<LocalAiState>();
        let mut cancel = state.changed.subscribe();
        let response = tokio::select! {
            answer = local.observe(&prompt, &system) => answer,
            _ = async {
                loop {
                    if !state.running() || epoch != state.epoch.load(Ordering::SeqCst) { break; }
                    if cancel.changed().await.is_err() { break; }
                }
            } => { continue; }
        };
        let mut inner = state.inner.lock().await;
        if !state.running() || epoch != state.epoch.load(Ordering::SeqCst) {
            continue;
        }
        let fresh = inner.status.sample.clone();
        let outcome =
            response.and_then(|answer| observer::parse_decision(&answer.response).map(|decision| (answer, decision)));
        match outcome {
            Ok((answer, decision)) => {
                let mut next_sim = inner.sim.clone();
                let result = fresh
                    .as_ref()
                    .ok_or_else(|| "Current MT5 quote is unavailable".to_owned())
                    .and_then(|sample| next_sim.apply(&snapshot, &config, &strategy, &decision, sample, now_ms()));
                let accepted = result.is_ok();
                let reason = result
                    .as_ref()
                    .err()
                    .cloned()
                    .unwrap_or_else(|| decision.reason.clone());
                let sim = next_sim.clone();
                let event = json!({"schema_version":1,"kind":"decision","at_ms":now_ms(),"snapshot_id":snapshot.id,"strategy_id":strategy.id,"decision":decision,"accepted":accepted,"reason":reason,"elapsed_ms":answer.elapsed_ms,"sim_state":sim,"source":"simulation only"});
                if let Err(error) = journal(&mut inner, event) {
                    inner.status.error = Some(error);
                    state.running.store(false, Ordering::SeqCst);
                    inner.status.running = false;
                } else {
                    inner.sim = next_sim;
                    inner.status.decisions += 1;
                    inner.status.last_decision = Some(LastDecision {
                        strategy_id: strategy.id.clone(),
                        decision,
                        accepted,
                        reason,
                        elapsed_ms: answer.elapsed_ms,
                    });
                    inner.status.phase = if accepted {
                        "Observing; decision recorded"
                    } else {
                        "Decision rejected; observing"
                    }
                    .into();
                    inner.status.error = None;
                    inner.status.position = inner.sim.position.clone();
                    inner.status.equity = inner.sim.equity;
                }
            }
            Err(error) => {
                inner.status.error = Some(error.clone());
                inner.status.phase = "Inference failed; no new simulation entry".into();
                let event = json!({"schema_version":1,"kind":"inference_error","at_ms":now_ms(),"snapshot_id":snapshot.id,"strategy_id":strategy.id,"error":error});
                if let Err(error) = journal(&mut inner, event) {
                    inner.status.error = Some(error);
                    state.running.store(false, Ordering::SeqCst);
                    inner.status.running = false;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        std::env::temp_dir().join(format!(
            "aegis-observer-state-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        ))
    }

    #[tokio::test]
    async fn restores_simulation_but_never_autostarts() {
        let root = root();
        let state = ObserverState::new(root.clone());
        {
            let mut inner = state.inner.lock().await;
            let mut sim = inner.sim.clone();
            sim.equity = 9999.;
            journal(&mut inner, json!({"kind":"decision","sim_state":sim})).unwrap();
        }
        let restored = ObserverState::new(root.clone());
        let inner = restored.inner.lock().await;
        assert!(!restored.running());
        assert_eq!(inner.sim.equity, 9999.);
        assert_eq!(inner.status.decisions, 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn corrupt_data_is_not_overwritten() {
        let root = root();
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("config.json"), "corrupt").unwrap();
        let state = ObserverState::new(root.clone());
        assert!(state.inner.lock().await.recovery_error.is_some());
        assert_eq!(std::fs::read_to_string(root.join("config.json")).unwrap(), "corrupt");
        std::fs::remove_dir_all(root).unwrap();
    }
}
