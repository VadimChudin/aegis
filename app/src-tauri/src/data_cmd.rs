//! Window commands of the DATA strategy (smart-money rules that passed the research):
//! settings, backtest on the Binance 1m history, today's signal and the paper journal.

use aegis_core::{
    bounce,
    data::{self, ParamSpec, Params},
};
use serde::Serialize;
use tauri::{AppHandle, State};

use crate::{bounce_cmd, AppState};

const SYMBOL: &str = "XAUUSDT";
const KEY: &str = "data";
/// Paper trades found in the live candles, kept across restarts (one per New York day).
const JOURNAL_KEY: &str = "data_paper";
const JOURNAL_MAX: usize = 500;

#[derive(Serialize)]
pub struct DataInfo {
    specs: Vec<ParamSpec>,
    defaults: Params,
    saved: Option<Params>,
    journal: Vec<serde_json::Value>,
}

fn journal(settings: &aegis_core::SettingsStore) -> Vec<serde_json::Value> {
    settings
        .strategy(JOURNAL_KEY)
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
}

#[tauri::command]
pub async fn data_info(state: State<'_, AppState>) -> Result<DataInfo, String> {
    let settings = state.settings.lock().await;
    Ok(DataInfo {
        specs: data::param_specs(),
        defaults: Params::default(),
        saved: settings
            .strategy(KEY)
            .and_then(|v| serde_json::from_value(v.clone()).ok()),
        journal: journal(&settings),
    })
}

#[tauri::command]
pub async fn data_save(state: State<'_, AppState>, params: Params) -> Result<(), String> {
    let mut settings = state.settings.lock().await;
    settings.set_strategy(KEY, serde_json::to_value(&params).map_err(|e| e.to_string())?);
    state.save(&settings).await;
    Ok(())
}

#[derive(Serialize)]
pub struct DataBacktest {
    report: data::Report,
    symbol: &'static str,
    source: String,
}

#[tauri::command]
pub async fn data_backtest(app: AppHandle, state: State<'_, AppState>, params: Params) -> Result<DataBacktest, String> {
    let (mins, source) = bounce_cmd::minutes(&app, &state).await?;
    let report = tauri::async_runtime::spawn_blocking(move || data::backtest(&mins, &params))
        .await
        .map_err(|e| e.to_string())?;
    Ok(DataBacktest {
        report,
        symbol: SYMBOL,
        source,
    })
}

#[derive(Serialize)]
pub struct DataLive {
    live: data::Live,
    pick: Option<data::AutoPick>,
    journal: Vec<serde_json::Value>,
    note: String,
}

/// Today's signal from the live Binance 1m candles. Completed trades of the last day are added
/// to the paper journal, so the journal grows every day the app is opened.
#[tauri::command]
pub async fn data_live(app: AppHandle, state: State<'_, AppState>, params: Params) -> Result<DataLive, String> {
    let recent = bounce::recent_minutes(&state.binance_public, SYMBOL, 1500)
        .await
        .map_err(|e| format!("live 1m candles unavailable: {e}"))?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let pick = if params.mode == data::Mode::Auto {
        let (mins, _) = bounce_cmd::minutes(&app, &state).await?;
        let p2 = params.clone();
        tauri::async_runtime::spawn_blocking(move || data::current_pick(&mins, &p2, now))
            .await
            .map_err(|e| e.to_string())?
    } else {
        None
    };
    let live = data::live(&recent, &params, pick.as_ref().map(|p| (p.entry_min, p.exit_min)), now);
    let mut settings = state.settings.lock().await;
    let mut j = journal(&settings);
    let mut added = false;
    for t in &live.closed {
        if !j.iter().any(|v| v.get("day").and_then(|d| d.as_i64()) == Some(t.day)) {
            j.push(serde_json::to_value(t).map_err(|e| e.to_string())?);
            added = true;
        }
    }
    if added {
        j.sort_by_key(|v| v.get("day").and_then(|d| d.as_i64()).unwrap_or(0));
        let extra = j.len().saturating_sub(JOURNAL_MAX);
        j.drain(..extra);
        settings.set_strategy(JOURNAL_KEY, serde_json::Value::Array(j.clone()));
        state.save(&settings).await;
    }
    Ok(DataLive {
        live,
        pick,
        journal: j,
        note: "Binance XAUUSDT live 1m candles".into(),
    })
}

#[tauri::command]
pub async fn data_journal_clear(state: State<'_, AppState>) -> Result<(), String> {
    let mut settings = state.settings.lock().await;
    settings.set_strategy(JOURNAL_KEY, serde_json::Value::Array(Vec::new()));
    state.save(&settings).await;
    Ok(())
}
