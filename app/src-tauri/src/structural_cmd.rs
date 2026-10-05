//! Independent experimental strategy panel. No broker credentials or live order placement.
use crate::AppState;
use aegis_core::structural::{self, Params, Tick};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tauri::{AppHandle, Emitter, State};
use tokio::sync::Mutex;

#[derive(Default)]
pub struct StructuralState {
    pub busy: AtomicBool,
    ticks: Mutex<Option<(String, Arc<Vec<Tick>>)>>,
}
#[tauri::command]
pub async fn structural_info(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let settings = state.settings.lock().await;
    Ok(
        serde_json::json!({"specs":structural::specs(),"defaults":Params::default(),
        "saved":settings.strategy("structural"),"presets":settings.strategy("structural_presets"),
        "research":{"total_r":0.8545230718742105,"trades":5,"from":"2026-04-01","to":"2026-09-30"}}),
    )
}
#[tauri::command]
pub async fn structural_save(state: State<'_, AppState>, params: Params, name: Option<String>) -> Result<(), String> {
    params.validate()?;
    let mut settings = state.settings.lock().await;
    if let Some(n) = name {
        let n = n.trim().chars().take(40).collect::<String>();
        if n.is_empty() {
            return Err("preset name is empty".into());
        }
        let mut all: BTreeMap<String, Params> = settings
            .strategy("structural_presets")
            .and_then(|x| serde_json::from_value(x.clone()).ok())
            .unwrap_or_default();
        all.insert(n, params.clone());
        settings.set_strategy(
            "structural_presets",
            serde_json::to_value(all).map_err(|e| e.to_string())?,
        );
    }
    settings.set_strategy("structural", serde_json::to_value(params).map_err(|e| e.to_string())?);
    state.save(&settings).await;
    Ok(())
}
#[tauri::command]
pub async fn structural_backtest(
    app: AppHandle,
    state: State<'_, AppState>,
    params: Params,
    from: String,
    to: String,
) -> Result<structural::Report, String> {
    params.validate()?;
    if state.structural.busy.swap(true, Ordering::SeqCst) {
        return Err("a structural backtest is already running".into());
    }
    let result = async {
        let key = format!("{from}|{to}");
        let mut cache = state.structural.ticks.lock().await;
        if cache.as_ref().is_none_or(|(k, _)| k != &key) {
            let dir = std::env::var("AEGIS_STRUCTURAL_DATA_DIR")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| state.cache_dir.join("bybit-trades"));
            let ticks = structural::history::load(&dir, &from, &to, move |done, total| {
                let _ = app.emit("structural_progress", serde_json::json!({"done":done,"total":total}));
            })
            .await?;
            *cache = Some((key, Arc::new(ticks)));
        }
        let ticks = cache.as_ref().ok_or("history unavailable")?.1.clone();
        drop(cache);
        tauri::async_runtime::spawn_blocking(move || structural::backtest(&ticks, &params))
            .await
            .map_err(|e| e.to_string())?
    }
    .await;
    state.structural.busy.store(false, Ordering::SeqCst);
    result
}
