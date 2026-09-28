//! Window commands of the Bounce strategy: settings and presets, backtest, genetic optimisation,
//! statistical checks and expected entries.

use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Instant,
};

use aegis_core::bounce::{
    self, ga::DEFAULT_METRICS, Bar, Engine, FeatureSpec, GaSpec, LiveReport, Minute, OptimizeReport, ParamSpec, Params,
    Report, Validation, FEATURES, KINDS, SESSIONS,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::Mutex;

use crate::AppState;

const SYMBOL: &str = "XAUUSDT";
/// The archive is re-read at most this often.
const HISTORY_TTL_SECS: u64 = 3600;
const PRESETS_KEY: &str = "bounce_presets";

type Cached<T> = Mutex<Option<(String, Arc<T>)>>;
/// (loaded at, 5m bars, 1m candles, source).
type History = Mutex<Option<(Instant, Arc<Vec<Bar>>, Arc<Vec<Minute>>, String)>>;

#[derive(Default)]
pub struct BounceState {
    history: History,
    engine: Cached<Engine>,
    optimizing: AtomicBool,
}

#[derive(Serialize)]
pub struct BounceInfo {
    specs: Vec<ParamSpec>,
    features: &'static [FeatureSpec],
    model_features: Vec<&'static str>,
    kinds: Vec<(&'static str, &'static str)>,
    sessions: Vec<(&'static str, &'static str)>,
    /// Per-metric win rates by quintile from the research run (research.json).
    research: serde_json::Value,
    defaults: Params,
    saved: Option<Params>,
    presets: BTreeMap<String, Params>,
    ga: GaSpec,
    ga_metrics: Vec<&'static str>,
}

#[derive(Clone, Serialize)]
struct Progress {
    stage: String,
    done: usize,
    total: usize,
}

fn emit(app: &AppHandle, stage: &str, done: usize, total: usize) {
    let _ = app.emit(
        "bt_progress",
        Progress {
            stage: stage.into(),
            done,
            total,
        },
    );
}

fn presets(settings: &aegis_core::SettingsStore) -> BTreeMap<String, Params> {
    settings
        .strategy(PRESETS_KEY)
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default()
}

#[tauri::command]
pub async fn bounce_info(state: State<'_, AppState>) -> Result<BounceInfo, String> {
    let settings = state.settings.lock().await;
    Ok(BounceInfo {
        specs: bounce::param_specs(),
        features: FEATURES,
        model_features: bounce::MODEL_FEATURES.to_vec(),
        kinds: KINDS.iter().map(|k| (k.id(), k.label())).collect(),
        sessions: SESSIONS.to_vec(),
        research: serde_json::from_str(include_str!("../../../crates/aegis-core/src/bounce/research.json"))
            .unwrap_or_default(),
        defaults: Params::default(),
        saved: settings
            .strategy("bounce")
            .and_then(|v| serde_json::from_value(v.clone()).ok()),
        presets: presets(&settings),
        ga: GaSpec::default(),
        ga_metrics: DEFAULT_METRICS.to_vec(),
    })
}

#[tauri::command]
pub async fn bounce_save(state: State<'_, AppState>, params: Params) -> Result<(), String> {
    let mut settings = state.settings.lock().await;
    settings.set_strategy("bounce", serde_json::to_value(&params).map_err(|e| e.to_string())?);
    state.save(&settings).await;
    Ok(())
}

#[tauri::command]
pub async fn bounce_preset_save(
    state: State<'_, AppState>,
    name: String,
    params: Option<Params>,
) -> Result<BTreeMap<String, Params>, String> {
    let name = name.trim().chars().take(40).collect::<String>();
    let mut settings = state.settings.lock().await;
    let mut all = presets(&settings);
    match params {
        Some(p) if !name.is_empty() => {
            all.insert(name, p);
        }
        _ => {
            all.remove(&name);
        }
    }
    settings.set_strategy(PRESETS_KEY, serde_json::to_value(&all).map_err(|e| e.to_string())?);
    state.save(&settings).await;
    Ok(all)
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Binance XAUUSDT 5m history since the listing with open interest, long/short, taker ratio and
/// funding merged in. Cached in memory for an hour and on disk forever. `AEGIS_HISTORY_CSV`
/// loads a local bar CSV instead (offline development, research bars with order flow).
async fn history(app: &AppHandle, state: &AppState) -> Result<(Arc<Vec<Bar>>, Arc<Vec<Minute>>, String), String> {
    let b = &state.bounce;
    if let Some((at, bars, mins, source)) = b.history.lock().await.as_ref() {
        if at.elapsed().as_secs() < HISTORY_TTL_SECS {
            return Ok((bars.clone(), mins.clone(), source.clone()));
        }
    }
    let (bars, mins, source) = if let Ok(path) = std::env::var("AEGIS_HISTORY_CSV") {
        let mins = match std::env::var("AEGIS_MINUTES_CSV") {
            Ok(m) => bounce::load_minutes(std::path::Path::new(&m))?,
            Err(_) => Vec::new(),
        };
        (
            bounce::load_csv(std::path::Path::new(&path))?,
            mins,
            format!("local file {path}"),
        )
    } else {
        let days = (now_secs() - bounce::BINANCE_LISTING) / 86_400 + 2;
        let a2 = app.clone();
        let mut bars = bounce::binance_history(SYMBOL, days, &state.cache_dir, move |done, total| {
            emit(&a2, "Downloading Binance 5m archive", done, total)
        })
        .await
        .map_err(|e| e.to_string())?;
        let a3 = app.clone();
        match bounce::binance_extras(SYMBOL, bounce::BINANCE_LISTING, &state.cache_dir, move |done, total| {
            emit(&a3, "Downloading open interest and funding", done, total)
        })
        .await
        {
            Ok(ex) => bounce::merge_extras(&mut bars, &ex),
            Err(e) => eprintln!("AEGIS: derivatives archive unavailable: {e}"),
        }
        let a4 = app.clone();
        let mins = bounce::binance_minutes(SYMBOL, days, &state.cache_dir, move |done, total| {
            emit(&a4, "Downloading Binance 1m archive", done, total)
        })
        .await
        .map_err(|e| e.to_string())?;
        (bars, mins, "Binance public archive (data.binance.vision)".to_string())
    };
    let (bars, mins) = (Arc::new(bars), Arc::new(mins));
    *b.history.lock().await = Some((Instant::now(), bars.clone(), mins.clone(), source.clone()));
    *b.engine.lock().await = None;
    Ok((bars, mins, source))
}

/// The engine for these settings' level scan and entry mode, built once and reused.
async fn engine(app: &AppHandle, state: &AppState, p: &Params) -> Result<(Arc<Engine>, String), String> {
    let (bars, mins, source) = history(app, state).await?;
    let key = format!("{:?}|{}|{}", p.scan, p.on_close(), bars.len());
    if let Some((k, e)) = state.bounce.engine.lock().await.as_ref() {
        if *k == key {
            return Ok((e.clone(), source));
        }
    }
    emit(app, "Finding levels and training the model", 0, 1);
    let (scan, close) = (p.scan, p.on_close());
    let e = tauri::async_runtime::spawn_blocking(move || Arc::new(Engine::build(bars, mins, &scan, close)))
        .await
        .map_err(|e| e.to_string())?;
    *state.bounce.engine.lock().await = Some((key, e.clone()));
    Ok((e, source))
}

/// Share of touches that have each metric (0 = the loaded data has no such column).
fn available(e: &Engine, p: &Params) -> BTreeMap<&'static str, f64> {
    let n = e.touches.len().max(1) as f64;
    FEATURES
        .iter()
        .enumerate()
        .map(|(k, f)| {
            let vals: Vec<f64> = e
                .touches
                .iter()
                .map(|t| p.metrics(t)[k])
                .filter(|v| v.is_finite())
                .collect();
            // A metric with a single value (e.g. "closed back" before a limit fill) cannot filter.
            let constant = vals.windows(2).all(|w| w[0] == w[1]);
            (f.id, if constant { 0.0 } else { vals.len() as f64 / n })
        })
        .collect()
}

#[derive(Serialize)]
pub struct BacktestResult {
    report: Report,
    /// 5m bars as [time, open, high, low, close] for the backtest chart.
    bars: Vec<[f64; 5]>,
    symbol: &'static str,
    source: String,
    available: BTreeMap<&'static str, f64>,
}

fn bar_rows(e: &Engine) -> Vec<[f64; 5]> {
    e.bars
        .iter()
        .map(|b| [b.time as f64, b.open, b.high, b.low, b.close])
        .collect()
}

#[tauri::command]
pub async fn bounce_backtest(
    app: AppHandle,
    state: State<'_, AppState>,
    params: Params,
) -> Result<BacktestResult, String> {
    let (e, source) = engine(&app, &state, &params).await?;
    emit(&app, "Simulating trades", 0, 1);
    let e2 = e.clone();
    let p2 = params.clone();
    let report = tauri::async_runtime::spawn_blocking(move || e2.report(&p2))
        .await
        .map_err(|e| e.to_string())?;
    Ok(BacktestResult {
        report,
        bars: bar_rows(&e),
        symbol: SYMBOL,
        source,
        available: available(&e, &params),
    })
}

#[tauri::command]
pub async fn bounce_validate(app: AppHandle, state: State<'_, AppState>, params: Params) -> Result<Validation, String> {
    let (e, _) = engine(&app, &state, &params).await?;
    emit(&app, "Checks: bootstrap, random levels, shuffled metrics", 0, 1);
    tauri::async_runtime::spawn_blocking(move || bounce::validate(&e, &params, None))
        .await
        .map_err(|e| e.to_string())
}

#[derive(Serialize)]
pub struct OptimizeResult {
    report: OptimizeReport,
    validation: Validation,
    /// Full backtest of the settings the GA chose (in-sample over the recent window).
    backtest: Report,
    bars: Vec<[f64; 5]>,
    seconds: f64,
}

#[derive(Clone, Serialize)]
struct GaProgress {
    stage: String,
    window: usize,
    windows: usize,
    generation: usize,
    generations: usize,
    best: f64,
}

#[tauri::command]
pub async fn bounce_optimize(
    app: AppHandle,
    state: State<'_, AppState>,
    params: Params,
    spec: GaSpec,
) -> Result<OptimizeResult, String> {
    if state.bounce.optimizing.swap(true, Ordering::SeqCst) {
        return Err("an optimisation is already running".into());
    }
    let res = async {
        let (e, _) = engine(&app, &state, &params).await?;
        let a2 = app.clone();
        let t0 = Instant::now();
        tauri::async_runtime::spawn_blocking(move || {
            let report = bounce::optimize(&e, &params, &spec, &|p| {
                let _ = a2.emit(
                    "ga_progress",
                    GaProgress {
                        stage: p.stage,
                        window: p.window,
                        windows: p.windows,
                        generation: p.generation,
                        generations: p.generations,
                        best: p.best,
                    },
                );
            });
            let _ = a2.emit(
                "ga_progress",
                GaProgress {
                    stage: "checks".into(),
                    window: 0,
                    windows: 0,
                    generation: 0,
                    generations: 0,
                    best: 0.0,
                },
            );
            let validation = bounce::validate(&e, &report.params, Some(&report));
            let backtest = e.report(&report.params);
            OptimizeResult {
                bars: bar_rows(&e),
                report,
                validation,
                backtest,
                seconds: t0.elapsed().as_secs_f64(),
            }
        })
        .await
        .map_err(|e| e.to_string())
    }
    .await;
    state.bounce.optimizing.store(false, Ordering::SeqCst);
    res
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
    let (bars, mins, _) = history(&app, &state).await?;
    let mut merged: Vec<Bar> = bars.as_ref().clone();
    let (fresh, note) = match bounce::recent_klines(&state.binance_public, SYMBOL, 1500).await {
        Ok(recent) if !recent.is_empty() => {
            let from = recent[0].time;
            // Keep the archive's derivatives columns on the overlap; new bars carry the last value.
            let last = merged.iter().rev().find(|b| b.oi.is_finite()).copied();
            merged.retain(|b| b.time < from);
            merged.extend(recent.into_iter().map(|mut b| {
                if let Some(l) = last {
                    b.oi = l.oi;
                    b.ls_top = l.ls_top;
                    b.taker_ratio = l.taker_ratio;
                    b.funding = l.funding;
                }
                b
            }));
            (true, "Binance live klines".to_string())
        }
        Ok(_) => (false, "live klines empty; using the archive".to_string()),
        Err(e) => (false, format!("live klines unavailable ({e}); using the archive")),
    };
    let live = tauri::async_runtime::spawn_blocking(move || {
        Engine::build(Arc::new(merged), mins, &params.scan, params.on_close()).live(&params)
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(LiveResult { live, fresh, note })
}
