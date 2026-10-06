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
    trading::{TradeRequest, TradeResult, TradingState},
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

fn agree(local: &ModelDecision, cloud: &ModelDecision) -> Result<(), String> {
    if local.snapshot_id != cloud.snapshot_id || local.action != cloud.action || local.position_id != cloud.position_id
    {
        return Err("Models disagree on the action, snapshot or position; no action accepted".into());
    }
    if local.stop != cloud.stop || local.target != cloud.target || local.quantity_fraction != cloud.quantity_fraction {
        return Err("Models disagree on stop, target or reduction size; no action accepted".into());
    }
    Ok(())
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
    account: Option<TradingState>,
    money_armed: bool,
    diagnostics: Vec<Diagnostic>,
    provider_ready: bool,
}

#[derive(Clone, Serialize)]
pub struct Diagnostic {
    name: String,
    ok: bool,
    latency_ms: u64,
    detail: String,
}

struct Inner {
    config: ObserverConfig,
    sim: SimState,
    journal: Option<Journal>,
    status: ObserverStatus,
    recovery_error: Option<String>,
    account_identity: Option<(u64, String)>,
    daily_day: u64,
    daily_equity: f64,
}

pub struct ObserverState {
    inner: Mutex<Inner>,
    running: AtomicBool,
    epoch: AtomicU64,
    sequence: AtomicU64,
    changed: watch::Sender<u64>,
    root: PathBuf,
    tasks: StdMutex<Vec<tauri::async_runtime::JoinHandle<()>>>,
    execution: Mutex<()>,
    mode_gate: Mutex<()>,
    provider_busy: AtomicBool,
}

#[derive(Serialize)]
pub struct Info {
    config: ObserverConfig,
    defaults: ObserverConfig,
    status: ObserverStatus,
    provider_model: &'static str,
    provider_key_stored: bool,
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
                    let mut config: ObserverConfig = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
                    config.normalize();
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
        let mut pending_money = HashMap::new();
        let mut daily_day = now_ms() / 86_400_000;
        let mut daily_equity = config.initial_equity;
        let journal = match Journal::new(&journal_path) {
            Ok(journal) => {
                match journal.replay() {
                    Ok(records) => {
                        for record in records {
                            if record["kind"] == "broker_intent" {
                                if let Some(id) = record["request_id"].as_str() {
                                    pending_money.insert(id.to_owned(), true);
                                }
                            }
                            if record["kind"] == "broker_result" {
                                if let Some(id) = record["request_id"].as_str() {
                                    if record["result"]["status"] != "unknown" {
                                        pending_money.remove(id);
                                    }
                                }
                            }
                            if let (Some(day), Some(equity)) =
                                (record["daily_day"].as_u64(), record["daily_equity"].as_f64())
                            {
                                if equity.is_finite() && equity > 0. {
                                    daily_day = day;
                                    daily_equity = equity;
                                }
                            }
                            if record["kind"] == "decision" {
                                decisions += 1;
                            }
                            if record["kind"] == "outcome" || record.get("paper_outcome").is_some_and(|v| !v.is_null())
                            {
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
        if !pending_money.is_empty() {
            recovery_error=Some("Unresolved broker execution in journal. Inspect MT5 and reconcile the account before enabling new entries; closing positions remains available.".into());
        }
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
            account: None,
            money_armed: false,
            diagnostics: Vec::new(),
            provider_ready: false,
        };
        let (changed, _) = watch::channel(0);
        Self {
            inner: Mutex::new(Inner {
                config,
                sim,
                journal,
                status,
                recovery_error,
                account_identity: None,
                daily_day,
                daily_equity,
            }),
            running: AtomicBool::new(false),
            epoch: AtomicU64::new(0),
            sequence: AtomicU64::new(now_ms()),
            changed,
            root,
            tasks: StdMutex::new(Vec::new()),
            execution: Mutex::new(()),
            mode_gate: Mutex::new(()),
            provider_busy: AtomicBool::new(false),
        }
    }

    pub fn running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn spawn(&self, app: AppHandle) {
        let mut tasks = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
        tasks.push(tauri::async_runtime::spawn(sample_loop(app.clone())));
        tasks.push(tauri::async_runtime::spawn(decision_loop(app.clone())));
        tasks.push(tauri::async_runtime::spawn(account_loop(app)));
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
pub async fn observer_info(app: AppHandle, state: State<'_, ObserverState>) -> Result<Info, String> {
    let inner = state.inner.lock().await;
    Ok(Info {
        config: inner.config.clone(),
        defaults: ObserverConfig::default(),
        status: inner.status.clone(),
        provider_model: "anthropic/claude-fable-5",
        provider_key_stored: app.state::<AppState>().settings.lock().await.ai_key().is_some(),
    })
}

#[tauri::command]
pub async fn observer_status(state: State<'_, ObserverState>) -> Result<ObserverStatus, String> {
    Ok(state.inner.lock().await.status.clone())
}

#[tauri::command]
pub async fn observer_save(
    state: State<'_, ObserverState>,
    mut config: ObserverConfig,
) -> Result<ObserverConfig, String> {
    let _gate = state.mode_gate.lock().await;
    let _execution = state.execution.lock().await;
    config.normalize();
    config.validate()?;
    let mut inner = state.inner.lock().await;
    if state.running() {
        return Err("Stop new entries before editing SPA or configuration; position protection remains active".into());
    }
    if inner.sim.position.is_some()
        && (config.mode != inner.config.mode
            || config.commission_per_oz != inner.config.commission_per_oz
            || config.slippage != inner.config.slippage)
    {
        return Err("Cannot change mode or fill assumptions while a paper position is open".into());
    }
    if inner
        .status
        .account
        .as_ref()
        .is_some_and(|s| s.positions.iter().any(|p| p.magic == 26070552))
        && config.mode != inner.config.mode
    {
        return Err("Close AEGIS positions before changing Paper/Money mode".into());
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
        inner.daily_day = now_ms() / 86_400_000;
        inner.daily_equity = config.initial_equity;
    }
    inner.config = config.clone();
    inner.status.money_armed = false;
    Ok(config)
}

#[tauri::command]
pub async fn observer_start(app: AppHandle, state: State<'_, ObserverState>) -> Result<ObserverStatus, String> {
    let _gate = state.mode_gate.lock().await;
    let _execution = state.execution.lock().await;
    let appstate = app.state::<AppState>();
    let connector = connector(&appstate).await?;
    let sample = connector.market_sample().await.map_err(|e| e.to_string())?;
    sample.validate(now_ms())?;
    let model = Runtime::inspect(ENDPOINT).await;
    if !model.server_ready || !model.model_downloaded {
        return Err("Install Qwen and press Start server in Local AI first".into());
    }
    let trading = connector.trading_state().await.map_err(|e| e.to_string())?;
    let key_present = appstate.settings.lock().await.ai_key().is_some();
    if !key_present {
        return Err("Save an OpenRouter API key in Settings → AI first".into());
    }
    crate::ai_cmd::stop_auto(&appstate).await;
    let mut inner = state.inner.lock().await;
    if let Some(error) = &inner.recovery_error {
        return Err(error.clone());
    }
    inner.config.validate()?;
    if !inner.config.ai_enabled {
        return Err("Enable AI trading in Settings first".into());
    }
    if inner.config.mode == "money" && !inner.status.money_armed {
        return Err("Confirm real-money execution for this session in Settings → AI".into());
    }
    if inner.config.mode == "money" && !trading.account.trade_allowed {
        return Err("MT5 automated trading is not permitted on this account".into());
    }
    let identity = (trading.account.login, trading.account.server.clone());
    if inner.config.mode == "money" && inner.account_identity.as_ref() != Some(&identity) {
        inner.status.money_armed = false;
        state.running.store(false, Ordering::SeqCst);
        inner.status.running = false;
        return Err("The armed MT5 account changed. Confirm this account again before trading.".into());
    }
    if !state.running() {
        inner.account_identity = Some(identity);
    }
    inner.status.account = Some(trading);
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
        json!({"schema_version":1,"kind":"start","at_ms":now_ms(),"config":config,"sim_state":sim,"source":"RoboForex MT5 XAUUSD","mode":config.mode}),
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
    let _gate = state.mode_gate.lock().await;
    state.running.store(false, Ordering::SeqCst);
    state.epoch.fetch_add(1, Ordering::SeqCst);
    state.changed.send_replace(state.epoch.load(Ordering::SeqCst));
    // Stop returns only after an already-dispatched broker action has settled.
    let _execution = state.execution.lock().await;
    let mut inner = state.inner.lock().await;
    inner.status.running = false;
    inner.status.money_armed = false;
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

#[tauri::command]
pub async fn observer_account(app: AppHandle) -> Result<TradingState, String> {
    connector(&app.state::<AppState>())
        .await?
        .trading_state()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn observer_arm_money(
    app: AppHandle,
    state: State<'_, ObserverState>,
    confirmed: bool,
) -> Result<ObserverStatus, String> {
    let _gate = state.mode_gate.lock().await;
    let _execution = state.execution.lock().await;
    let account = connector(&app.state::<AppState>())
        .await?
        .trading_state()
        .await
        .map_err(|e| e.to_string())?;
    let mut inner = state.inner.lock().await;
    if state.running() {
        return Err("Stop AI before changing execution permission".into());
    }
    if confirmed && (inner.config.mode != "money" || !inner.config.ai_enabled) {
        return Err("Select AI real-money mode before arming".into());
    }
    if confirmed && !account.account.trade_allowed {
        return Err("MT5 trading permission is disabled".into());
    }
    inner.status.account = Some(account.clone());
    inner.account_identity = Some((account.account.login, account.account.server));
    inner.status.money_armed = confirmed;
    Ok(inner.status.clone())
}

#[tauri::command]
pub async fn observer_provider_save(
    app: AppHandle,
    state: State<'_, ObserverState>,
    key: String,
    model: String,
) -> Result<Info, String> {
    if model != "anthropic/claude-fable-5" {
        return Err("This mode uses anthropic/claude-fable-5".into());
    }
    if state.running() {
        return Err("Stop AI before changing provider credentials".into());
    }
    if !key.trim().is_empty() {
        if key.len() > 512 || key.chars().any(char::is_control) {
            return Err("Invalid provider key".into());
        }
        let appstate = app.state::<AppState>();
        let mut store = appstate.settings.lock().await;
        store.set_ai_key(key.trim());
        store.save().map_err(|e| e.to_string())?;
    }
    observer_info(app, state).await
}

#[tauri::command]
pub async fn observer_provider_test(app: AppHandle, state: State<'_, ObserverState>) -> Result<Diagnostic, String> {
    if state.running() {
        return Err("Stop new entries before testing the provider".into());
    }
    if state.provider_busy.swap(true, Ordering::SeqCst) {
        return Err("A provider test is already running".into());
    }
    let result = async {
        let key = app
            .state::<AppState>()
            .settings
            .lock()
            .await
            .ai_key()
            .ok_or("Save an OpenRouter key first")?;
        let answer = aegis_core::ai_provider::consult(
            &key,
            "Return {\"ready\":true}",
            "Connectivity test. Reply with JSON only.",
            25,
        )
        .await?;
        let value: Value = serde_json::from_str(&answer.response).map_err(|_| "Provider returned invalid JSON")?;
        if value["ready"] != true {
            return Err("Provider did not confirm readiness".into());
        }
        Ok(Diagnostic {
            name: "OpenRouter / Anthropic".into(),
            ok: true,
            latency_ms: answer.latency_ms,
            detail: "Actual provider response; billable API call".into(),
        })
    }
    .await;
    state.provider_busy.store(false, Ordering::SeqCst);
    state.inner.lock().await.status.provider_ready = result.is_ok();
    result
}

#[tauri::command]
pub async fn observer_check_cycle(app: AppHandle, state: State<'_, ObserverState>) -> Result<Vec<Diagnostic>, String> {
    if state.running() {
        return Err("Stop AI before testing the cycle; no orders will be sent".into());
    }
    let mut checks = Vec::new();
    let at = std::time::Instant::now();
    let connection = connector(&app.state::<AppState>()).await;
    match connection {
        Ok(conn) => {
            let result = conn.trading_state().await;
            checks.push(Diagnostic {
                name: "MT5 account / execution permissions".into(),
                ok: result.is_ok(),
                latency_ms: at.elapsed().as_millis() as u64,
                detail: result
                    .map(|s| {
                        format!(
                            "{}; equity {}; trading allowed {}; no order sent",
                            s.account.currency, s.account.equity, s.account.trade_allowed
                        )
                    })
                    .unwrap_or_else(|e| e.to_string()),
            });
            let at = std::time::Instant::now();
            let result = conn
                .market_sample()
                .await
                .map_err(|e| e.to_string())
                .and_then(|s| s.validate(now_ms()));
            checks.push(Diagnostic {
                name: "MT5 fresh quotes / depth fallback".into(),
                ok: result.is_ok(),
                latency_ms: at.elapsed().as_millis() as u64,
                detail: result.err().unwrap_or_else(|| "Fresh quote received".into()),
            });
        }
        Err(error) => checks.push(Diagnostic {
            name: "MT5".into(),
            ok: false,
            latency_ms: at.elapsed().as_millis() as u64,
            detail: error,
        }),
    }
    let at = std::time::Instant::now();
    let result = crate::local_ai_cmd::local_ai_ping(app.state::<LocalAiState>()).await;
    checks.push(Diagnostic {
        name: "Qwen".into(),
        ok: result.is_ok(),
        latency_ms: at.elapsed().as_millis() as u64,
        detail: result.map(|r| r.response).unwrap_or_else(|e| e),
    });
    match observer_provider_test(app.clone(), app.state::<ObserverState>()).await {
        Ok(check) => checks.push(check),
        Err(error) => checks.push(Diagnostic {
            name: "OpenRouter / Anthropic".into(),
            ok: false,
            latency_ms: 0,
            detail: error,
        }),
    }
    state.inner.lock().await.status.diagnostics = checks.clone();
    Ok(checks)
}

async fn stop_entries(state: &ObserverState) {
    state.running.store(false, Ordering::SeqCst);
    state.epoch.fetch_add(1, Ordering::SeqCst);
    state.changed.send_replace(state.epoch.load(Ordering::SeqCst));
    let mut inner = state.inner.lock().await;
    inner.status.running = false;
    inner.status.money_armed = false;
}

#[tauri::command]
pub async fn observer_close(app: AppHandle, state: State<'_, ObserverState>, ticket: u64) -> Result<Value, String> {
    stop_entries(&state).await;
    let _execution = state.execution.lock().await;
    let conn = connector(&app.state::<AppState>()).await?;
    let mode = state.inner.lock().await.config.mode.clone();
    let known_live = conn
        .trading_state()
        .await
        .map_err(|e| e.to_string())?
        .positions
        .iter()
        .any(|p| p.ticket == ticket && p.magic == 26070552 && p.symbol == "XAUUSD");
    if known_live || mode == "money" {
        let mut result = conn.close_position(ticket).await.map_err(|e| e.to_string())?;
        let current = conn.trading_state().await.map_err(|e| e.to_string());
        if result.status == "filled"
            && current
                .as_ref()
                .map_or(true, |s| s.positions.iter().any(|p| p.ticket == ticket))
        {
            result.status = "unknown".into();
            result.message =
                "Broker position remains open after close acknowledgement; reconcile MT5 before retrying".into();
        }
        let mut inner = state.inner.lock().await;
        inner.status.account = current.ok();
        journal(
            &mut inner,
            json!({"schema_version":1,"kind":"manual_close","at_ms":now_ms(),"result":result}),
        )?;
        Ok(json!(result))
    } else {
        let sample = conn.market_sample().await.map_err(|e| e.to_string())?;
        sample.validate(now_ms())?;
        let mut inner = state.inner.lock().await;
        if inner.sim.position.as_ref().map(|p| p.id) != Some(ticket) {
            return Err("Paper position is no longer open".into());
        }
        let mut next = inner.sim.clone();
        let result = next
            .exit_at(&sample, &inner.config, "manual_market_close")
            .ok_or("Cannot close simulation at this quote")?;
        journal(
            &mut inner,
            json!({"schema_version":1,"kind":"outcome","outcome":result,"sim_state":next}),
        )?;
        inner.sim = next;
        inner.status.position = None;
        inner.status.equity = inner.sim.equity;
        inner.status.outcomes += 1;
        Ok(json!(result))
    }
}

#[tauri::command]
pub async fn observer_close_all(app: AppHandle, state: State<'_, ObserverState>) -> Result<Value, String> {
    stop_entries(&state).await;
    let ticket = state.inner.lock().await.sim.position.as_ref().map(|p| p.id);
    let mut combined = Vec::new();
    if let Some(ticket) = ticket {
        let sample = connector(&app.state::<AppState>())
            .await?
            .market_sample()
            .await
            .map_err(|e| e.to_string())?;
        sample.validate(now_ms())?;
        let mut inner = state.inner.lock().await;
        let mut next = inner.sim.clone();
        let outcome = next
            .exit_at(&sample, &inner.config, "manual_close_all")
            .ok_or("No fresh Paper exit quote")?;
        let event =
            json!({"schema_version":1,"kind":"outcome","outcome":outcome,"sim_state":next,"position_id":ticket});
        journal(&mut inner, event)?;
        inner.sim = next;
        inner.status.position = None;
        inner.status.equity = inner.sim.equity;
        inner.status.outcomes += 1;
        combined.push(json!(outcome));
    }
    let _execution = state.execution.lock().await;
    let conn = connector(&app.state::<AppState>()).await?;
    let mut result = conn.close_all().await.map_err(|e| e.to_string())?;
    let current = conn.trading_state().await.map_err(|e| e.to_string());
    if let Ok(current) = &current {
        for outcome in &mut result {
            if outcome.status == "filled"
                && outcome
                    .request_id
                    .parse::<u64>()
                    .ok()
                    .is_some_and(|id| current.positions.iter().any(|p| p.ticket == id))
            {
                outcome.status = "unknown".into();
                outcome.message = "Position is still reported open; inspect MT5".into();
            }
        }
    } else {
        for outcome in &mut result {
            if outcome.status == "filled" {
                outcome.status = "unknown".into();
                outcome.message = "Fresh position reconciliation unavailable; inspect MT5".into();
            }
        }
    }
    let mut inner = state.inner.lock().await;
    inner.status.account = current.ok();
    journal(
        &mut inner,
        json!({"schema_version":1,"kind":"manual_close_all","at_ms":now_ms(),"results":result}),
    )?;
    combined.extend(result.iter().map(|r| json!(r)));
    Ok(json!(combined))
}

async fn account_loop(app: AppHandle) {
    let state = app.state::<ObserverState>();
    let appstate = app.state::<AppState>();
    let mut interval = tokio::time::interval(Duration::from_secs(2));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        let conn = match connector(&appstate).await {
            Ok(conn) => conn,
            Err(_) => {
                state.inner.lock().await.status.account = None;
                continue;
            }
        };
        match conn.trading_state().await {
            Ok(account) => {
                let identity = (account.account.login, account.account.server.clone());
                let mut inner = state.inner.lock().await;
                if (state.running() || inner.status.money_armed) && inner.account_identity.as_ref() != Some(&identity) {
                    state.running.store(false, Ordering::SeqCst);
                    state.epoch.fetch_add(1, Ordering::SeqCst);
                    state.changed.send_replace(state.epoch.load(Ordering::SeqCst));
                    inner.status.running = false;
                    inner.status.money_armed = false;
                    inner.status.error = Some("MT5 account changed; new entries stopped".into());
                }
                inner.status.account = Some(account);
            }
            Err(error) => {
                let mut inner = state.inner.lock().await;
                inner.status.account = None;
                inner.status.error = Some(error.to_string());
            }
        }
    }
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

async fn execute_money(
    app: &AppHandle,
    state: &ObserverState,
    strategy: &observer::StrategyConfig,
    config: &ObserverConfig,
    decision: &ModelDecision,
    context: (u64, &str, Option<(u64, String)>),
) -> Result<TradeResult, String> {
    let (epoch, request_id, identity) = context;
    let conn = connector(&app.state::<AppState>()).await?;
    let account = conn.trading_state().await.map_err(|e| e.to_string())?;
    if identity.as_ref() != Some(&(account.account.login, account.account.server.clone())) {
        return Err("MT5 account identity changed before execution".into());
    }
    if !state.running() || epoch != state.epoch.load(Ordering::SeqCst) {
        return Err("AI action cancelled before execution".into());
    }
    if matches!(decision.action.as_str(), "long" | "short") {
        let request = TradeRequest {
            request_id: request_id.into(),
            side: decision.action.clone(),
            risk_pct: strategy.risk_pct,
            stop: decision.stop.ok_or("Missing stop")?,
            target: decision.target.ok_or("Missing target")?,
            max_spread: config.max_spread,
            max_positions: strategy.max_positions,
            daily_loss_limit_pct: strategy.max_daily_loss_pct,
            confirm_real: true,
        };
        conn.place_order(&request).await.map_err(|e| e.to_string())
    } else {
        let ticket = decision
            .position_id
            .ok_or("Position management requires a broker ticket")?;
        if !account
            .positions
            .iter()
            .any(|p| p.ticket == ticket && p.symbol == "XAUUSD" && p.magic == 26070552)
        {
            return Err("Position is not owned by AEGIS on XAUUSD".into());
        }
        match decision.action.as_str() {
            "close" => conn.close_position(ticket).await.map_err(|e| e.to_string()),
            "reduce" => conn
                .reduce_position(ticket, decision.quantity_fraction.ok_or("Reduction fraction required")?)
                .await
                .map_err(|e| e.to_string()),
            "stop" => conn
                .modify_stop(ticket, decision.stop.ok_or("Stop required")?, decision.target)
                .await
                .map_err(|e| e.to_string()),
            _ => Err("Unsupported money action".into()),
        }
    }
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
            let day=now_ms()/86_400_000;
            if inner.daily_day!=day {
                inner.daily_day=day;inner.daily_equity=inner.sim.equity;
                let baseline=json!({"schema_version":1,"kind":"daily_baseline","daily_day":inner.daily_day,"daily_equity":inner.daily_equity,"sim_state":inner.sim});
                journal(&mut inner,baseline)?;
            }
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
                .filter(|f| {
                    f.validate_at(now_ms()).is_ok()
                        && (inner.config.strictness < 100 || strategy.timeframes.contains(&f.timeframe))
                })
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
        {
            let inner = state.inner.lock().await;
            let positions = if config.mode == "money" {
                inner.status.account.as_ref().map(|s| json!(s.positions.iter()
                    .filter(|p|p.magic==26070552 && p.symbol=="XAUUSD")
                    .map(|p|json!({"ticket":p.ticket,"side":p.side,"quantity_oz":p.quantity_oz,"entry":p.entry,"stop":p.stop,"target":p.target,"profit":p.profit}))
                    .collect::<Vec<_>>())).unwrap_or(json!([]))
            } else {
                json!(inner.sim.position)
            };
            input["positions"] = positions;
        }
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
        let response = match response {
            Ok(answer) => {
                let local_decision = observer::parse_decision(&answer.response);
                match local_decision {
                    Ok(local_decision) => {
                        let key = app.state::<AppState>().settings.lock().await.ai_key();
                        match key {
                            Some(key) => {
                                let cloud_payload = json!({"market":input,"local_proposal":local_decision,
                                    "instruction":"Independently check the SPA and supplied market. Return the same decision schema with the actual snapshot_id. Disagreement means wait."}).to_string();
                                let cloud = tokio::select! {
                                    value = aegis_core::ai_provider::consult(&key,&cloud_payload,&system,25) => value,
                                    _ = async {
                                        loop {
                                            if !state.running() || epoch!=state.epoch.load(Ordering::SeqCst) { break; }
                                            if cancel.changed().await.is_err() { break; }
                                        }
                                    } => {continue;}
                                };
                                cloud.and_then(|cloud| {
                                    let confirmed = observer::parse_decision(&cloud.response)?;
                                    agree(&local_decision, &confirmed)?;
                                    let fresh = state
                                        .inner
                                        .try_lock()
                                        .ok()
                                        .and_then(|inner| inner.status.sample.clone())
                                        .ok_or("Latest broker quote unavailable")?;
                                    observer::validate_decision(
                                        &snapshot,
                                        &config,
                                        &strategy,
                                        &confirmed,
                                        &fresh,
                                        now_ms(),
                                    )?;
                                    Ok(aegis_core::local_ai::Answer {
                                        response: answer.response,
                                        elapsed_ms: answer.elapsed_ms + cloud.latency_ms,
                                        memories_used: 0,
                                    })
                                })
                            }
                            None => Err("OpenRouter key unavailable; no new entries".into()),
                        }
                    }
                    Err(error) => Err(error),
                }
            }
            Err(error) => Err(error),
        };
        let _execution = state.execution.lock().await;
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
                let mut paper_outcome = None;
                let effective = ObserverConfig {
                    risk_pct: strategy.risk_pct,
                    ..config.clone()
                };
                let checked = fresh
                    .as_ref()
                    .ok_or_else(|| "Current MT5 quote is unavailable".to_owned())
                    .and_then(|sample| {
                        observer::validate_decision(&snapshot, &effective, &strategy, &decision, sample, now_ms())
                    });
                let result = if config.mode == "money" && decision.action != "wait" {
                    let armed = inner.status.money_armed;
                    let identity = inner.account_identity.clone();
                    if checked.is_err() {
                        checked.map(|_| None)
                    } else if !armed {
                        Err("Real-money execution is not armed".into())
                    } else {
                        let request_id = format!("{}-{}", snapshot.id, strategy.id);
                        let intent = json!({"schema_version":1,"kind":"broker_intent","at_ms":now_ms(),"request_id":request_id,"decision":decision,"account":identity});
                        match journal(&mut inner, intent) {
                            Err(error) => Err(error),
                            Ok(()) => {
                                drop(inner);
                                let execution = execute_money(
                                    &app,
                                    &state,
                                    &strategy,
                                    &effective,
                                    &decision,
                                    (epoch, &request_id, identity),
                                )
                                .await;
                                inner = state.inner.lock().await;
                                match execution {
                                    Ok(result) => {
                                        let executed = matches!(result.status.as_str(), "filled" | "partial");
                                        let unknown = result.status == "unknown";
                                        let event = json!({"schema_version":1,"kind":"broker_result","at_ms":now_ms(),"request_id":request_id,"result":result});
                                        if let Err(error) = journal(&mut inner, event) {
                                            inner.recovery_error = Some(error.clone());
                                            state.running.store(false, Ordering::SeqCst);
                                            inner.status.running = false;
                                            inner.status.money_armed = false;
                                            Err(format!("Broker response received but journal failed: {error}. Reconcile positions before restart."))
                                        } else if unknown {
                                            state.running.store(false, Ordering::SeqCst);
                                            inner.status.running = false;
                                            inner.status.money_armed = false;
                                            Err("Broker outcome is unknown; new entries stopped. Inspect MT5 before retrying.".into())
                                        } else if executed {
                                            Ok(None)
                                        } else {
                                            Err(format!("Broker rejected action: {}", result.message))
                                        }
                                    }
                                    Err(error) => {
                                        state.running.store(false, Ordering::SeqCst);
                                        inner.status.running = false;
                                        inner.status.money_armed = false;
                                        Err(error)
                                    }
                                }
                            }
                        }
                    }
                } else {
                    checked.and_then(|()| {
                        let sample = fresh.as_ref().expect("validated quote");
                        if matches!(decision.action.as_str(), "long" | "short") {
                            let unrealized = next_sim.position.as_ref().map_or(0., |p| {
                                let price = if p.side == "long" {
                                    sample.quote.bid
                                } else {
                                    sample.quote.ask
                                };
                                (price - p.entry) * p.quantity * if p.side == "long" { 1. } else { -1. }
                            });
                            let loss = inner.daily_equity - next_sim.equity - unrealized;
                            if loss >= inner.daily_equity * strategy.max_daily_loss_pct / 100. {
                                return Err("Daily simulation loss limit reached; new entries blocked".into());
                            }
                        }
                        match decision.action.as_str() {
                            "close" => {
                                let ticket = decision.position_id.ok_or("Close requires position_id")?;
                                if next_sim.position.as_ref().map(|p| p.id) != Some(ticket) {
                                    return Err("Unknown paper position".into());
                                }
                                let outcome = next_sim
                                    .exit_at(sample, &effective, "model_market_exit")
                                    .ok_or("Invalid paper exit")?;
                                paper_outcome = Some(outcome);
                                Ok(None)
                            }
                            "stop" => {
                                let ticket = decision.position_id.ok_or("Position id required")?;
                                let stop = decision.stop.ok_or("Stop required")?;
                                next_sim.tighten_stop(ticket, stop, sample)?;
                                if let Some(target) = decision.target {
                                    let position = next_sim.position.as_mut().ok_or("Position unavailable")?;
                                    if !target.is_finite()
                                        || (position.side == "long" && target <= sample.quote.ask)
                                        || (position.side == "short" && target >= sample.quote.bid)
                                    {
                                        return Err("Target must remain on the profit side of the current quote".into());
                                    }
                                    position.target = target;
                                }
                                Ok(None)
                            }
                            "reduce" => {
                                paper_outcome = Some(next_sim.reduce_at(
                                    decision.position_id.ok_or("Position id required")?,
                                    decision.quantity_fraction.ok_or("Fraction required")?,
                                    sample,
                                    &effective,
                                )?);
                                Ok(None)
                            }
                            _ => next_sim.apply(&snapshot, &effective, &strategy, &decision, sample, now_ms()),
                        }
                    })
                };
                let accepted = result.is_ok();
                let reason = result
                    .as_ref()
                    .err()
                    .cloned()
                    .unwrap_or_else(|| decision.reason.clone());
                let sim = next_sim.clone();
                let event = json!({"schema_version":1,"kind":"decision","at_ms":now_ms(),"snapshot_id":snapshot.id,"strategy_id":strategy.id,"decision":decision,"accepted":accepted,"reason":reason,"elapsed_ms":answer.elapsed_ms,"sim_state":sim,"paper_outcome":paper_outcome,"source":config.mode});
                if let Err(error) = journal(&mut inner, event) {
                    inner.status.error = Some(error);
                    state.running.store(false, Ordering::SeqCst);
                    inner.status.running = false;
                } else {
                    inner.sim = next_sim;
                    if paper_outcome.is_some() {
                        inner.status.outcomes += 1;
                    }
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

    #[test]
    fn models_must_agree_on_every_executable_parameter() {
        let local:ModelDecision=serde_json::from_value(json!({"snapshot_id":7,"action":"long","reason":"test","stop":99.,"target":102.,"used_timeframes":["1m"],"checks":[]})).unwrap();
        assert!(agree(&local, &local).is_ok());
        let mut cloud = local.clone();
        cloud.stop = Some(98.);
        assert!(agree(&local, &cloud).is_err());
        cloud = local.clone();
        cloud.target = Some(103.);
        assert!(agree(&local, &cloud).is_err());
        cloud = local.clone();
        cloud.quantity_fraction = Some(0.5);
        assert!(agree(&local, &cloud).is_err());
        cloud = local.clone();
        cloud.snapshot_id = 8;
        assert!(agree(&local, &cloud).is_err());
    }

    #[tokio::test]
    async fn unresolved_broker_intent_blocks_automatic_restart() {
        let root = root();
        let state = ObserverState::new(root.clone());
        {
            let mut inner = state.inner.lock().await;
            journal(&mut inner, json!({"kind":"broker_intent","request_id":"ambiguous-1"})).unwrap();
        }
        let restored = ObserverState::new(root.clone());
        assert!(restored
            .inner
            .lock()
            .await
            .recovery_error
            .as_deref()
            .unwrap()
            .contains("Unresolved broker execution"));
        assert!(!restored.running());
        assert!(!restored.inner.lock().await.status.money_armed);
        std::fs::remove_dir_all(root).unwrap();
    }
}
