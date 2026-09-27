//! Window commands of the Bounce strategy: settings, backtest and expected entries.

use std::{sync::Arc, time::Instant};

use aegis_core::bounce::{self, Bar, FeatureSpec, LiveReport, ParamSpec, Params, Report, FEATURES, KINDS};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::AppState;

const SYMBOL: &str = "XAUUSDT";
/// The archive is re-read at most this often.
const HISTORY_TTL_SECS: u64 = 3600;

#[derive(Serialize)]
pub struct BounceInfo {
    specs: Vec<ParamSpec>,
    features: &'static [FeatureSpec],
    model_features: Vec<&'static str>,
    kinds: Vec<(&'static str, &'static str)>,
    /// Per-metric win rates by quintile from the research run (research.json).
    research: serde_json::Value,
    defaults: Params,
    saved: Option<Params>,
}

#[derive(Clone, Serialize)]
struct Progress {
    stage: &'static str,
    done: usize,
    total: usize,
}

#[derive(Serialize)]
pub struct BacktestResult {
    report: Report,
    /// 5m bars as [time, open, high, low, close] for the backtest chart.
    bars: Vec<[f64; 5]>,
    symbol: &'static str,
    source: String,
}

fn saved(settings: &aegis_core::SettingsStore) -> Option<Params> {
    settings
        .strategy("bounce")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
}

#[tauri::command]
pub async fn bounce_info(state: State<'_, AppState>) -> Result<BounceInfo, String> {
    let settings = state.settings.lock().await;
    Ok(BounceInfo {
        specs: bounce::param_specs(),
        features: FEATURES,
        model_features: bounce::MODEL_FEATURES.to_vec(),
        kinds: KINDS.iter().map(|k| (k.id(), k.label())).collect(),
        research: serde_json::from_str(include_str!("../../../crates/aegis-core/src/bounce/research.json"))
            .unwrap_or_default(),
        defaults: Params::default(),
        saved: saved(&settings),
    })
}

#[tauri::command]
pub async fn bounce_save(state: State<'_, AppState>, params: Params) -> Result<(), String> {
    let mut settings = state.settings.lock().await;
    settings.set_strategy("bounce", serde_json::to_value(&params).map_err(|e| e.to_string())?);
    state.save(&settings).await;
    Ok(())
}

/// Archive history since the XAUUSDT listing, cached in memory for an hour and on disk forever.
/// `AEGIS_HISTORY_CSV` loads a local bar CSV instead (offline development).
async fn history(app: &AppHandle, state: &AppState) -> Result<(Arc<Vec<Bar>>, String), String> {
    if let Some((at, bars, source)) = state.history.lock().await.as_ref() {
        if at.elapsed().as_secs() < HISTORY_TTL_SECS {
            return Ok((bars.clone(), source.clone()));
        }
    }
    let (bars, source) = if let Ok(path) = std::env::var("AEGIS_HISTORY_CSV") {
        let bars = bounce::load_csv(std::path::Path::new(&path))?;
        (bars, format!("local file {path}"))
    } else {
        let days = (now_secs() - bounce::BINANCE_LISTING) / 86_400 + 2;
        let app2 = app.clone();
        let bars = bounce::binance_history(SYMBOL, days, &state.cache_dir, move |done, total| {
            let _ = app2.emit(
                "bt_progress",
                Progress {
                    stage: "Downloading Binance 5m archive",
                    done,
                    total,
                },
            );
        })
        .await
        .map_err(|e| e.to_string())?;
        (bars, "Binance public archive (data.binance.vision)".to_string())
    };
    let bars = Arc::new(bars);
    *state.history.lock().await = Some((Instant::now(), bars.clone(), source.clone()));
    Ok((bars, source))
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[tauri::command]
pub async fn bounce_backtest(
    app: AppHandle,
    state: State<'_, AppState>,
    params: Params,
) -> Result<BacktestResult, String> {
    let (bars, source) = history(&app, &state).await?;
    let _ = app.emit(
        "bt_progress",
        Progress {
            stage: "Finding levels, training the model, simulating trades",
            done: 0,
            total: 1,
        },
    );
    let run = bars.clone();
    let report = tauri::async_runtime::spawn_blocking(move || bounce::evaluate(&run, &params))
        .await
        .map_err(|e| e.to_string())?;
    Ok(BacktestResult {
        report,
        bars: bars
            .iter()
            .map(|b| [b.time as f64, b.open, b.high, b.low, b.close])
            .collect(),
        symbol: SYMBOL,
        source,
    })
}

#[derive(Serialize)]
pub struct LiveResult {
    live: LiveReport,
    /// Whether the last day came from the live API (else the archive, up to a day old).
    fresh: bool,
    note: String,
}

/// Where the strategy would rest orders right now, with each order's probability.
#[tauri::command]
pub async fn bounce_live(app: AppHandle, state: State<'_, AppState>, params: Params) -> Result<LiveResult, String> {
    let (bars, _) = history(&app, &state).await?;
    let mut merged: Vec<Bar> = bars.as_ref().clone();
    let (fresh, note) = match bounce::recent_klines(&state.binance_public, SYMBOL, 1500).await {
        Ok(recent) if !recent.is_empty() => {
            let from = recent[0].time;
            merged.retain(|b| b.time < from);
            merged.extend(recent);
            (true, "Binance live klines".to_string())
        }
        Ok(_) => (false, "live klines empty; using the archive".to_string()),
        Err(e) => (false, format!("live klines unavailable ({e}); using the archive")),
    };
    let live = tauri::async_runtime::spawn_blocking(move || bounce::live(&merged, &params))
        .await
        .map_err(|e| e.to_string())?;
    Ok(LiveResult { live, fresh, note })
}
