//! Window commands of the History panel: the local Dukascopy history for strategy backtests.

use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use aegis_core::{
    dataset::{self, dukascopy, DatasetConfig, Manifest},
    Timeframe,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::AppState;

const CONFIG_KEY: &str = "dataset";

#[derive(Default)]
pub struct HistoryState {
    running: AtomicBool,
    cancel: Arc<AtomicBool>,
}

/// Folder of one symbol's history.
pub fn root(state: &AppState, symbol: &str) -> PathBuf {
    state.cache_dir.join("dukascopy").join(symbol)
}

/// `AEGIS_DUKASCOPY_URL` points the loader at a mirror or a local mock.
fn base_url() -> String {
    std::env::var("AEGIS_DUKASCOPY_URL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| dukascopy::DEFAULT_URL.to_string())
}

pub async fn config(state: &AppState) -> DatasetConfig {
    let saved = state
        .settings
        .lock()
        .await
        .strategy(CONFIG_KEY)
        .and_then(|v| serde_json::from_value(v.clone()).ok());
    saved.unwrap_or_default()
}

#[derive(Serialize)]
pub struct HistoryInfo {
    config: DatasetConfig,
    manifest: Option<Manifest>,
    running: bool,
    folder: String,
    timeframes: Vec<&'static str>,
    earliest: &'static str,
    /// Yesterday, UTC: the last complete day.
    yesterday: String,
}

#[tauri::command]
pub async fn history_info(state: State<'_, AppState>) -> Result<HistoryInfo, String> {
    let config = config(&state).await;
    let dir = root(&state, &config.symbol);
    Ok(HistoryInfo {
        manifest: dataset::read_manifest(&dir),
        running: state.history.running.load(Ordering::SeqCst),
        folder: dir.display().to_string(),
        timeframes: Timeframe::ALL.iter().map(|t| t.as_str()).collect(),
        earliest: dataset::EARLIEST,
        yesterday: dataset::time::format_date(dataset::today() - 1),
        config,
    })
}

#[derive(Serialize)]
pub struct Plan {
    days: usize,
    files: usize,
    /// The range a Refresh would keep.
    from: String,
    to: String,
}

/// How many files a download or a Refresh with this configuration still needs.
#[tauri::command]
pub async fn history_plan(state: State<'_, AppState>, config: DatasetConfig, refresh: bool) -> Result<Plan, String> {
    let today = dataset::today();
    let c = if refresh { config.rolled(today) } else { config };
    let (days, files) = dataset::pending(&root(&state, &c.symbol), &c, today).map_err(|e| e.to_string())?;
    let (from, to) = c.range(today).map_err(|e| e.to_string())?;
    Ok(Plan {
        days,
        files,
        from: dataset::time::format_date(from),
        to: dataset::time::format_date(to),
    })
}

/// Downloads what is missing and rebuilds the timeframes. `refresh` first moves a rolling window
/// to end yesterday (deleting the oldest days).
#[tauri::command]
pub async fn history_sync(
    app: AppHandle,
    state: State<'_, AppState>,
    config: DatasetConfig,
    refresh: bool,
) -> Result<Manifest, String> {
    if state.history.running.swap(true, Ordering::SeqCst) {
        return Err("a download is already running".into());
    }
    state.history.cancel.store(false, Ordering::SeqCst);
    let res = async {
        let today = dataset::today();
        let c = if refresh { config.rolled(today) } else { config };
        c.range(today).map_err(|e| e.to_string())?;
        {
            let mut settings = state.settings.lock().await;
            settings.set_strategy(CONFIG_KEY, serde_json::to_value(&c).map_err(|e| e.to_string())?);
            state.save(&settings).await;
        }
        let dir = root(&state, &c.symbol);
        let a2 = app.clone();
        let m = dataset::sync(
            &dir,
            &base_url(),
            &c,
            today,
            move |p| {
                let _ = a2.emit("history_progress", p);
            },
            &state.history.cancel,
        )
        .await
        .map_err(|e| e.to_string())?;
        state.asia.invalidate().await;
        Ok(m)
    }
    .await;
    state.history.running.store(false, Ordering::SeqCst);
    res
}

#[tauri::command]
pub async fn history_cancel(state: State<'_, AppState>) -> Result<(), String> {
    state.history.cancel.store(true, Ordering::SeqCst);
    Ok(())
}
