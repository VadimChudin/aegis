//! Window commands of the Asia + FOMC strategy: settings, backtest and the auto search on the
//! local history.

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Instant,
};

use aegis_core::{
    asia_fomc::{self, ParamSpec, Params, Prepared, Report, SearchReport, SearchSpec},
    dataset, Timeframe,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::Mutex;

use crate::{history_cmd, AppState};

const PARAMS_KEY: &str = "asia_fomc";
const SPEC_KEY: &str = "asia_fomc_search";

#[derive(Default)]
pub struct AsiaState {
    /// (timeframe | history version, bars with their New York fields).
    prepared: Mutex<Option<(String, Arc<Prepared>)>>,
    searching: AtomicBool,
    cancel: Arc<AtomicBool>,
}

impl AsiaState {
    pub async fn invalidate(&self) {
        *self.prepared.lock().await = None;
    }
}

#[derive(Serialize)]
pub struct AsiaInfo {
    specs: Vec<ParamSpec>,
    defaults: Params,
    saved: Option<Params>,
    spec: SearchSpec,
    default_spec: SearchSpec,
    /// Timeframes built in the local history, smallest first.
    timeframes: Vec<&'static str>,
    first: Option<i64>,
    last: Option<i64>,
    has_spread: bool,
}

#[tauri::command]
pub async fn asia_info(state: State<'_, AppState>) -> Result<AsiaInfo, String> {
    let cfg = history_cmd::config(&state).await;
    let manifest = dataset::read_manifest(&history_cmd::root(&state, &cfg.symbol));
    let settings = state.settings.lock().await;
    let get = |k: &str| settings.strategy(k).cloned();
    Ok(AsiaInfo {
        specs: asia_fomc::param_specs(),
        defaults: Params::default(),
        saved: get(PARAMS_KEY).and_then(|v| serde_json::from_value(v).ok()),
        spec: get(SPEC_KEY).and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default(),
        default_spec: SearchSpec::default(),
        timeframes: Timeframe::ALL
            .iter()
            .filter(|t| manifest.as_ref().is_some_and(|m| m.bars.get(t.as_str()).is_some_and(|&n| n > 0)))
            .map(|t| t.as_str())
            .collect(),
        first: manifest.as_ref().and_then(|m| m.first),
        last: manifest.as_ref().and_then(|m| m.last),
        has_spread: manifest.as_ref().is_some_and(|m| m.config.ask),
    })
}

#[tauri::command]
pub async fn asia_save(state: State<'_, AppState>, params: Params, spec: SearchSpec) -> Result<(), String> {
    let mut settings = state.settings.lock().await;
    settings.set_strategy(PARAMS_KEY, serde_json::to_value(&params).map_err(|e| e.to_string())?);
    settings.set_strategy(SPEC_KEY, serde_json::to_value(&spec).map_err(|e| e.to_string())?);
    state.save(&settings).await;
    Ok(())
}

async fn prepared(state: &AppState, tf: &str) -> Result<Arc<Prepared>, String> {
    let tf = Timeframe::parse(tf).ok_or_else(|| format!("unknown timeframe {tf}"))?;
    let cfg = history_cmd::config(state).await;
    let dir = history_cmd::root(state, &cfg.symbol);
    let m = dataset::read_manifest(&dir).ok_or("no history yet: download it in the History panel")?;
    let key = format!("{}|{}", tf.as_str(), m.updated);
    let mut slot = state.asia.prepared.lock().await;
    if let Some((k, p)) = slot.as_ref() {
        if *k == key {
            return Ok(p.clone());
        }
    }
    let p = tauri::async_runtime::spawn_blocking(move || -> Result<Prepared, String> {
        let bars = dataset::load(&dir, tf).map_err(|e| e.to_string())?;
        if bars.is_empty() {
            return Err(format!("no {} bars in the history", tf.as_str()));
        }
        Ok(Prepared::new(&bars, tf.seconds()))
    })
    .await
    .map_err(|e| e.to_string())??;
    let p = Arc::new(p);
    *slot = Some((key, p.clone()));
    Ok(p)
}

#[tauri::command]
pub async fn asia_backtest(state: State<'_, AppState>, params: Params, tf: String) -> Result<Report, String> {
    let pr = prepared(&state, &tf).await?;
    tauri::async_runtime::spawn_blocking(move || pr.report(&params, 0, pr.len()))
        .await
        .map_err(|e| e.to_string())
}

#[derive(Serialize)]
pub struct SearchResult {
    report: SearchReport,
    seconds: f64,
}

#[tauri::command]
pub async fn asia_search(
    app: AppHandle,
    state: State<'_, AppState>,
    params: Params,
    spec: SearchSpec,
    tf: String,
) -> Result<SearchResult, String> {
    if state.asia.searching.swap(true, Ordering::SeqCst) {
        return Err("a search is already running".into());
    }
    state.asia.cancel.store(false, Ordering::SeqCst);
    let res = async {
        let pr = prepared(&state, &tf).await?;
        let cancel = state.asia.cancel.clone();
        let t0 = Instant::now();
        let report = tauri::async_runtime::spawn_blocking(move || {
            asia_fomc::search(
                &pr,
                &params,
                &spec,
                &|p| {
                    let _ = app.emit("asia_progress", p);
                },
                &cancel,
            )
        })
        .await
        .map_err(|e| e.to_string())?;
        Ok(SearchResult {
            report,
            seconds: t0.elapsed().as_secs_f64(),
        })
    }
    .await;
    state.asia.searching.store(false, Ordering::SeqCst);
    res
}

#[tauri::command]
pub async fn asia_cancel(state: State<'_, AppState>) -> Result<(), String> {
    state.asia.cancel.store(true, Ordering::SeqCst);
    Ok(())
}
