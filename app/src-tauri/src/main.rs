#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

use aegis_core::{
    strategy::{self, StrategyInfo},
    AccountSummary, BrokerId, BrokerInfo, Candle, ConnectOptions, Connector, Credentials, Timeframe,
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

#[derive(Default)]
struct AppState {
    session: Mutex<Option<Session>>,
    feed: std::sync::Mutex<Option<JoinHandle<()>>>,
    /// Bumped on every chart load; the window drops events from older feeds.
    generation: AtomicU64,
}

impl AppState {
    fn stop_feed(&self) {
        if let Some(feed) = self.feed.lock().unwrap_or_else(|e| e.into_inner()).take() {
            feed.abort();
        }
    }

    async fn close_session(&self) {
        self.stop_feed();
        self.generation.fetch_add(1, Ordering::SeqCst);
        if let Some(old) = self.session.lock().await.take() {
            old.connector.close().await;
        }
    }
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
fn brokers() -> Vec<BrokerInfo> {
    BrokerId::ALL.into_iter().map(BrokerId::info).collect()
}

#[tauri::command]
fn timeframes() -> Vec<Timeframe> {
    Timeframe::ALL.to_vec()
}

#[tauri::command]
fn strategies() -> Vec<StrategyInfo> {
    strategy::catalog()
}

#[tauri::command]
async fn session(state: State<'_, AppState>) -> Result<Option<AccountSummary>, String> {
    Ok(state.session.lock().await.as_ref().map(|s| s.summary.clone()))
}

/// Logs in to one broker. Any previous broker is logged out first, so the chart
/// can only ever show data from the broker that is connected now.
#[tauri::command]
async fn connect(
    app: AppHandle,
    state: State<'_, AppState>,
    credentials: Credentials,
) -> Result<AccountSummary, String> {
    state.close_session().await;
    let (connector, summary) = Connector::connect(credentials, &connect_options(&app))
        .await
        .map_err(|e| e.to_string())?;
    *state.session.lock().await = Some(Session {
        connector: Arc::new(connector),
        summary: summary.clone(),
    });
    Ok(summary)
}

#[tauri::command]
async fn disconnect(state: State<'_, AppState>) -> Result<(), String> {
    state.close_session().await;
    Ok(())
}

/// Loads history for the active broker and starts its live feed.
#[tauri::command]
async fn load_chart(app: AppHandle, state: State<'_, AppState>, timeframe: Timeframe) -> Result<ChartData, String> {
    let connector = state
        .session
        .lock()
        .await
        .as_ref()
        .map(|s| s.connector.clone())
        .ok_or("Connect a broker first")?;
    state.stop_feed();
    let generation = state.generation.fetch_add(1, Ordering::SeqCst) + 1;
    let candles = connector
        .candles(timeframe, HISTORY_BARS)
        .await
        .map_err(|e| e.to_string())?;
    if state.generation.load(Ordering::SeqCst) != generation {
        return Err("superseded".into());
    }
    let feed = async_runtime::spawn(run_feed(app, connector.clone(), timeframe, generation));
    if let Some(old) = state.feed.lock().unwrap_or_else(|e| e.into_inner()).replace(feed) {
        old.abort();
    }
    Ok(ChartData {
        generation,
        broker: connector.id(),
        symbol: connector.symbol().to_string(),
        timeframe,
        candles,
    })
}

/// Polls the last two bars so the closing bar gets its final values too.
async fn run_feed(app: AppHandle, connector: Arc<Connector>, timeframe: Timeframe, generation: u64) {
    let mut delay = POLL;
    let mut failing = false;
    loop {
        tokio::time::sleep(delay).await;
        match connector.candles(timeframe, 2).await {
            Ok(bars) => {
                for candle in bars {
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

/// `AEGIS_BINANCE_URL` / `AEGIS_BYBIT_URL` point the connectors at a testnet or a
/// local mock; `AEGIS_MT5_BRIDGE` overrides the bundled bridge script.
fn connect_options(app: &AppHandle) -> ConnectOptions {
    let env = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
    let bundled = app
        .path()
        .resolve("bridges/mt5_bridge.py", BaseDirectory::Resource)
        .ok();
    ConnectOptions {
        binance_url: env("AEGIS_BINANCE_URL"),
        bybit_url: env("AEGIS_BYBIT_URL"),
        mt5_bridge_script: env("AEGIS_MT5_BRIDGE").map(PathBuf::from).or(bundled),
    }
}

fn main() {
    let app = tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            brokers, timeframes, strategies, session, connect, disconnect, load_chart
        ])
        .build(tauri::generate_context!())
        .expect("failed to start AEGIS");

    app.run(|handle, event| {
        if let RunEvent::Exit = event {
            let state = handle.state::<AppState>();
            async_runtime::block_on(state.close_session());
        }
    });
}
