#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bounce_cmd;
mod density_cmd;

use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

use aegis_core::{
    strategy::{self, StrategyInfo},
    AccountSummary, BrokerId, BrokerInfo, Candle, ConnectOptions, ConnectReport, Connector, Credentials,
    PublicSettings, SettingsStore, Timeframe,
};
use serde::Serialize;
use tauri::{
    async_runtime::{self, JoinHandle},
    path::BaseDirectory,
    AppHandle, Emitter, Manager, RunEvent, State,
};
use tokio::sync::Mutex;

const HISTORY_BARS: usize = 500;
const POLL: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(15);

struct Session {
    connector: Arc<Connector>,
    summary: AccountSummary,
}

struct AppState {
    settings: Mutex<SettingsStore>,
    /// Every connected broker. Binance, Bybit and RoboForex can be open together.
    sessions: Mutex<HashMap<BrokerId, Session>>,
    /// Serialises connect/disconnect per broker so two clicks cannot race, while a slow
    /// MT5 login (up to a minute) does not hold up Binance or Bybit.
    connecting: [Mutex<()>; 3],
    feed: std::sync::Mutex<Option<(BrokerId, JoinHandle<()>)>>,
    /// Bumped on every chart load; the window drops events from older feeds.
    generation: AtomicU64,
    options: ConnectOptions,
    bounce: bounce_cmd::BounceState,
    cache_dir: PathBuf,
    /// Public futures API for the latest klines (no key needed).
    binance_public: String,
    density: density_cmd::DensityState,
}

impl AppState {
    fn stop_feed(&self, only: Option<BrokerId>) {
        let mut feed = self.feed.lock().unwrap_or_else(|e| e.into_inner());
        if feed.as_ref().is_some_and(|(b, _)| only.is_none_or(|o| o == *b)) {
            if let Some((_, handle)) = feed.take() {
                handle.abort();
            }
            self.generation.fetch_add(1, Ordering::SeqCst);
        }
    }

    async fn close(&self, broker: BrokerId) {
        self.stop_feed(Some(broker));
        self.density.invalidate(broker).await;
        if let Some(old) = self.sessions.lock().await.remove(&broker) {
            old.connector.close().await;
        }
    }

    async fn close_all(&self) {
        self.stop_feed(None);
        let sessions: Vec<_> = self.sessions.lock().await.drain().collect();
        for (_, s) in sessions {
            s.connector.close().await;
        }
    }

    fn connect_lock(&self, broker: BrokerId) -> &Mutex<()> {
        let i = BrokerId::ALL.iter().position(|b| *b == broker).unwrap_or(0);
        &self.connecting[i]
    }

    async fn summaries(&self) -> Vec<AccountSummary> {
        let mut list: Vec<_> = self.sessions.lock().await.values().map(|s| s.summary.clone()).collect();
        list.sort_by_key(|s| s.broker);
        list
    }

    async fn save(&self, settings: &SettingsStore) {
        if let Err(e) = settings.save() {
            eprintln!("AEGIS: cannot save {}: {e}", settings.path().display());
        }
    }

    /// Opens a session from credentials, replacing this broker's previous one.
    async fn open(&self, broker: BrokerId, fields: BTreeMap<String, String>) -> ConnectReport {
        let _guard = self.connect_lock(broker).lock().await;
        let report = match Credentials::from_fields(broker, &fields) {
            Err(e) => ConnectReport::input_error(broker, &e),
            Ok(credentials) => {
                let (connector, report) = Connector::connect(credentials, &self.options).await;
                if let Some(connector) = connector {
                    let summary = AccountSummary {
                        broker,
                        name: broker.name(),
                        symbol: connector.symbol().to_string(),
                        account: report.account.clone().unwrap_or_default(),
                    };
                    self.close(broker).await;
                    self.sessions.lock().await.insert(
                        broker,
                        Session {
                            connector: Arc::new(connector),
                            summary,
                        },
                    );
                }
                report
            }
        };
        let mut settings = self.settings.lock().await;
        settings.set_report(broker, report.clone());
        self.save(&settings).await;
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        net::TcpStream,
        process::{Child, Command},
        thread::sleep,
    };

    struct Mock(Child);

    impl Drop for Mock {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[test]
    fn feed_backfills_bars_missed_during_an_outage() {
        let now = 1_790_000_000;
        assert_eq!(feed_limit(Timeframe::M1, now - 30, now), 2);
        assert_eq!(feed_limit(Timeframe::M1, now - 600, now), 12);
        assert_eq!(feed_limit(Timeframe::M15, now - 3 * 3600, now), 14);
        assert_eq!(feed_limit(Timeframe::M1, now - 30 * 86_400, now), HISTORY_BARS);
        assert_eq!(feed_limit(Timeframe::H1, 0, now), 2);
        assert_eq!(feed_limit(Timeframe::H1, now + 50, now), 2);
    }

    #[tokio::test]
    async fn failed_reconnect_keeps_existing_session_and_feed() {
        let port = 18768;
        let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/mock_venues.py");
        let python = std::env::var("AEGIS_TEST_PYTHON").unwrap_or_else(|_| "python3".into());
        let _mock = Mock(Command::new(python).arg(script).arg(port.to_string()).spawn().unwrap());
        for _ in 0..50 {
            if TcpStream::connect(("127.0.0.1", port)).is_ok() {
                break;
            }
            sleep(Duration::from_millis(100));
        }
        let base = format!("http://127.0.0.1:{port}/binance");
        let path = std::env::temp_dir().join(format!("aegis-reconnect-{}.json", std::process::id()));
        let state = AppState {
            settings: Mutex::new(SettingsStore::open(&path)),
            sessions: Mutex::new(HashMap::new()),
            connecting: Default::default(),
            feed: std::sync::Mutex::new(None),
            generation: AtomicU64::new(0),
            options: ConnectOptions {
                binance_url: Some(base.clone()),
                binance_spot_url: Some(base),
                ..Default::default()
            },
            bounce: bounce_cmd::BounceState::default(),
            cache_dir: std::env::temp_dir(),
            binance_public: String::new(),
            density: density_cmd::DensityState::new(std::env::temp_dir().join("aegis-test-densities")),
        };
        let fields = |secret: &str| {
            BTreeMap::from([
                ("api_key".into(), "test-key".into()),
                ("api_secret".into(), secret.into()),
            ])
        };
        assert!(
            connect_form(&state, BrokerId::Binance, fields("test-secret"), true)
                .await
                .connected
        );
        *state.feed.lock().unwrap() = Some((
            BrokerId::Binance,
            async_runtime::spawn(async {
                std::future::pending::<()>().await;
            }),
        ));
        assert!(
            !connect_form(&state, BrokerId::Binance, fields("wrong"), true)
                .await
                .connected
        );
        assert_eq!(state.summaries().await.len(), 1);
        assert_eq!(state.generation.load(Ordering::SeqCst), 0);
        assert!(state.feed.lock().unwrap().is_some());
        assert_eq!(
            state.settings.lock().await.credentials(BrokerId::Binance)["api_secret"],
            "test-secret"
        );
        assert_eq!(
            SettingsStore::open(&path).credentials(BrokerId::Binance)["api_secret"],
            "test-secret"
        );
        state.close_all().await;
        let _ = std::fs::remove_file(path);
    }
}

#[derive(Serialize)]
struct Bootstrap {
    version: &'static str,
    brokers: Vec<BrokerInfo>,
    timeframes: Vec<Timeframe>,
    strategies: Vec<StrategyInfo>,
    settings: PublicSettings,
    sessions: Vec<AccountSummary>,
}

#[derive(Clone, Serialize)]
struct ChartData {
    generation: u64,
    broker: BrokerId,
    symbol: String,
    timeframe: Timeframe,
    candles: Vec<Candle>,
}

#[derive(Clone, Serialize)]
struct CandleEvent {
    generation: u64,
    candle: Candle,
}

#[derive(Clone, Serialize)]
struct FeedStatus {
    generation: u64,
    ok: bool,
    message: String,
}

#[tauri::command]
async fn bootstrap(state: State<'_, AppState>) -> Result<Bootstrap, String> {
    Ok(Bootstrap {
        version: env!("CARGO_PKG_VERSION"),
        brokers: BrokerId::ALL.into_iter().map(BrokerId::info).collect(),
        timeframes: Timeframe::ALL.to_vec(),
        strategies: strategy::catalog(),
        settings: state.settings.lock().await.public(),
        sessions: state.summaries().await,
    })
}

#[tauri::command]
async fn settings_get(state: State<'_, AppState>) -> Result<PublicSettings, String> {
    Ok(state.settings.lock().await.public())
}

#[tauri::command]
async fn sessions(state: State<'_, AppState>) -> Result<Vec<AccountSummary>, String> {
    Ok(state.summaries().await)
}

/// Saves the form (secrets encrypted, empty secret keeps the stored one) and connects.
#[tauri::command]
async fn broker_connect(
    state: State<'_, AppState>,
    broker: BrokerId,
    form: BTreeMap<String, String>,
    auto_connect: bool,
) -> Result<ConnectReport, String> {
    Ok(connect_form(&state, broker, form, auto_connect).await)
}

async fn connect_form(
    state: &AppState,
    broker: BrokerId,
    form: BTreeMap<String, String>,
    auto_connect: bool,
) -> ConnectReport {
    let was_connected = state.sessions.lock().await.contains_key(&broker);
    let fields = state.settings.lock().await.merged_form(broker, &form);
    let report = state.open(broker, fields).await;
    {
        let mut settings = state.settings.lock().await;
        if report.connected || !was_connected {
            settings.apply_form(broker, &form);
        }
        settings.set_auto_connect(broker, auto_connect);
        state.save(&settings).await;
    }
    report
}

/// Connects with the stored credentials (auto-connect on start). `None` if nothing is stored.
#[tauri::command]
async fn broker_connect_saved(state: State<'_, AppState>, broker: BrokerId) -> Result<Option<ConnectReport>, String> {
    let fields = {
        let settings = state.settings.lock().await;
        if !settings.has_credentials(broker) {
            return Ok(None);
        }
        settings.credentials(broker)
    };
    Ok(Some(state.open(broker, fields).await))
}

#[tauri::command]
async fn broker_disconnect(
    state: State<'_, AppState>,
    broker: BrokerId,
    forget: bool,
) -> Result<PublicSettings, String> {
    let _guard = state.connect_lock(broker).lock().await;
    state.close(broker).await;
    let mut settings = state.settings.lock().await;
    if forget {
        settings.forget(broker);
        state.save(&settings).await;
    }
    Ok(settings.public())
}

#[tauri::command]
async fn set_auto_connect(state: State<'_, AppState>, broker: BrokerId, on: bool) -> Result<(), String> {
    let mut settings = state.settings.lock().await;
    settings.set_auto_connect(broker, on);
    state.save(&settings).await;
    Ok(())
}

#[tauri::command]
async fn set_lang(app: AppHandle, state: State<'_, AppState>, lang: String) -> Result<(), String> {
    let mut settings = state.settings.lock().await;
    settings.set_lang(&lang);
    state.save(&settings).await;
    let public = settings.public();
    let _ = app.emit(
        "density_preferences",
        serde_json::json!({"lang": public.lang, "theme": public.theme}),
    );
    Ok(())
}

#[tauri::command]
async fn set_theme(app: AppHandle, state: State<'_, AppState>, theme: String) -> Result<(), String> {
    let mut settings = state.settings.lock().await;
    settings.set_theme(&theme);
    state.save(&settings).await;
    let public = settings.public();
    let _ = app.emit(
        "density_preferences",
        serde_json::json!({"lang": public.lang, "theme": public.theme}),
    );
    Ok(())
}

/// Loads history from one connected broker and starts its live feed.
/// The chart never mixes brokers: the previous feed is stopped first.
#[tauri::command]
async fn load_chart(
    app: AppHandle,
    state: State<'_, AppState>,
    broker: BrokerId,
    timeframe: Timeframe,
) -> Result<ChartData, String> {
    let connector = state
        .sessions
        .lock()
        .await
        .get(&broker)
        .map(|s| s.connector.clone())
        .ok_or_else(|| format!("{} is not connected", broker.name()))?;
    state.stop_feed(None);
    let generation = state.generation.fetch_add(1, Ordering::SeqCst) + 1;
    {
        let mut settings = state.settings.lock().await;
        settings.set_chart(Some(broker), timeframe);
        state.save(&settings).await;
    }
    let candles = connector
        .candles(timeframe, HISTORY_BARS)
        .await
        .map_err(|e| e.to_string())?;
    if state.generation.load(Ordering::SeqCst) != generation {
        return Err("superseded".into());
    }
    let last = candles.last().map_or(0, |c| c.time);
    let feed = async_runtime::spawn(run_feed(app, connector.clone(), timeframe, generation, last));
    if let Some((_, old)) = state
        .feed
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .replace((broker, feed))
    {
        old.abort();
    }
    Ok(ChartData {
        generation,
        broker,
        symbol: connector.symbol().to_string(),
        timeframe,
        candles,
    })
}

#[tauri::command]
async fn stop_chart(state: State<'_, AppState>) -> Result<(), String> {
    state.stop_feed(None);
    Ok(())
}

/// Bars to poll so nothing is skipped: normally the last two (the closing bar gets its
/// final values too), after an outage, a sleep of the computer or a slow venue every bar
/// since the last one received, so the chart and live strategies never see a hole.
fn feed_limit(timeframe: Timeframe, last_bar: i64, now: i64) -> usize {
    if last_bar <= 0 {
        return 2;
    }
    let missed = (now - last_bar).max(0) / timeframe.seconds();
    usize::try_from(missed)
        .unwrap_or(usize::MAX)
        .saturating_add(2)
        .min(HISTORY_BARS)
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

async fn run_feed(app: AppHandle, connector: Arc<Connector>, timeframe: Timeframe, generation: u64, last: i64) {
    let mut delay = POLL;
    let mut failing = false;
    let mut last_bar = last;
    loop {
        tokio::time::sleep(delay).await;
        match connector
            .candles(timeframe, feed_limit(timeframe, last_bar, unix_now()))
            .await
        {
            Ok(bars) => {
                for candle in bars {
                    last_bar = last_bar.max(candle.time);
                    let _ = app.emit("candle", CandleEvent { generation, candle });
                }
                if failing {
                    let _ = app.emit(
                        "feed_status",
                        FeedStatus {
                            generation,
                            ok: true,
                            message: "Live".into(),
                        },
                    );
                }
                failing = false;
                delay = POLL;
            }
            Err(err) => {
                failing = true;
                let _ = app.emit(
                    "feed_status",
                    FeedStatus {
                        generation,
                        ok: false,
                        message: err.to_string(),
                    },
                );
                delay = (delay * 2).min(MAX_BACKOFF);
            }
        }
    }
}

/// `AEGIS_BINANCE_URL`, `AEGIS_BINANCE_SPOT_URL`, `AEGIS_BYBIT_URL` point the connectors at
/// a testnet or a local mock; `AEGIS_MT5_BRIDGE` overrides the bundled bridge script;
/// `AEGIS_CONFIG_DIR` moves the settings file.
fn setup_state(app: &AppHandle) -> AppState {
    let env = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
    let bundled = app
        .path()
        .resolve("bridges/mt5_bridge.py", BaseDirectory::Resource)
        .ok();
    let options = ConnectOptions {
        binance_url: env("AEGIS_BINANCE_URL"),
        binance_spot_url: env("AEGIS_BINANCE_SPOT_URL"),
        bybit_url: env("AEGIS_BYBIT_URL"),
        mt5_bridge_script: env("AEGIS_MT5_BRIDGE").map(PathBuf::from).or(bundled),
    };
    let dir = env("AEGIS_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| app.path().app_config_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    let cache_dir = app
        .path()
        .app_cache_dir()
        .unwrap_or_else(|_| dir.join("cache"))
        .join("history");
    let binance_public = options
        .binance_url
        .clone()
        .unwrap_or_else(|| "https://fapi.binance.com".into());
    AppState {
        settings: Mutex::new(SettingsStore::open(dir.join("settings.json"))),
        sessions: Mutex::new(HashMap::new()),
        connecting: Default::default(),
        feed: std::sync::Mutex::new(None),
        generation: AtomicU64::new(0),
        options,
        bounce: bounce_cmd::BounceState::default(),
        cache_dir,
        binance_public,
        density: density_cmd::DensityState::new(
            app.path()
                .app_data_dir()
                .unwrap_or_else(|_| dir.clone())
                .join("densities"),
        ),
    }
}

fn main() {
    let app = tauri::Builder::default()
        .setup(|app| {
            let state = setup_state(app.handle());
            app.manage(state);
            density_cmd::setup_window(app.handle())?;
            async_runtime::spawn(density_cmd::run(app.handle().clone()));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            bootstrap,
            settings_get,
            sessions,
            broker_connect,
            broker_connect_saved,
            broker_disconnect,
            set_auto_connect,
            set_theme,
            load_chart,
            stop_chart,
            density_cmd::density_snapshot,
            density_cmd::density_select,
            density_cmd::density_open,
            density_cmd::density_hide,
            density_cmd::density_set_docked,
            density_cmd::density_settings_get,
            density_cmd::density_settings_save,
            bounce_cmd::bounce_info,
            bounce_cmd::bounce_save,
            bounce_cmd::bounce_backtest,
            bounce_cmd::bounce_live,
            bounce_cmd::bounce_validate,
            bounce_cmd::bounce_optimize,
            bounce_cmd::bounce_preset_save,
            set_lang
        ])
        .build(tauri::generate_context!())
        .expect("failed to start AEGIS");

    app.run(|handle, event| {
        if let RunEvent::Ready = event {
            density_cmd::dock(handle);
        }
        if let RunEvent::Exit = event {
            if let Some(state) = handle.try_state::<AppState>() {
                async_runtime::block_on(state.close_all());
            }
        }
    });
}
