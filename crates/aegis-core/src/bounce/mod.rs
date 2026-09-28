//! Bounce strategy: levels, touches, the metrics of every touch, a walk-forward probability
//! model, the backtest engine, a genetic algorithm for the settings and statistical checks.
//!
//! The same code runs the research CLI (`examples/bounce.rs`) and the app, so the numbers in
//! research reports and in the window agree.

mod bar;
pub mod engine;
mod features;
pub mod ga;
mod history;
mod model;
pub mod news;
pub mod params;
mod scan;
pub mod validate;

use std::sync::Arc;

pub use bar::{load_csv, load_minutes, parse_csv, parse_minutes, Bar};
pub use engine::{
    month_id, simulate, simulate_in, stats, Engine, ExpectedEntry, LiveReport, Minute, Report, SignalMark, Stats, Trade,
};
pub use features::{FeatureSpec, FEATURES, NF};
pub use ga::{optimize, GaSpec, OptimizeReport};
pub use history::{
    binance_extras, binance_history, binance_minutes, merge_extras, recent_klines, Extras, HistoryError,
    BINANCE_LISTING,
};
pub use model::MODEL_FEATURES;
pub use params::{param_specs, Filter, ParamSpec, Params, SESSIONS};
pub use scan::{scan, scan_full, LevelKind, ScanConfig, Touch, KINDS};
pub use validate::{validate, Validation};

pub(crate) fn backtest_month(time: i64) -> String {
    engine::month_id(time)
}

/// One-off backtest (builds an engine; keep an `Engine` to re-run with other settings).
pub fn evaluate(bars: &[Bar], p: &Params) -> Report {
    Engine::new(Arc::new(bars.to_vec()), &p.scan, p.on_close()).report(p)
}
