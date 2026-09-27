//! Bounce strategy: levels, touches, the metrics of every touch, backtest and walk-forward tuning.
//!
//! The same code runs the research CLI (`examples/bounce.rs`) and the Backtest button in the app,
//! so the numbers in research reports and in the window agree.

mod backtest;
mod bar;
mod features;
mod history;
mod model;
mod scan;

pub use backtest::{
    evaluate, live, optimize, param_specs, simulate, ExpectedEntry, Filter, LiveReport, OptimizeReport, ParamSpec,
    Params, Report, SignalMark, Stats, Trade, TuneSpec, WindowResult, SESSIONS,
};
pub use bar::{load_csv, parse_csv, Bar};
pub use features::{FeatureSpec, FEATURES, NF};
pub use history::{binance_history, recent_klines, HistoryError, BINANCE_LISTING};
pub use model::MODEL_FEATURES;
pub use scan::{scan, scan_full, LevelKind, ScanConfig, Touch, KINDS};

pub(crate) fn backtest_month(time: i64) -> String {
    backtest::month_id(time)
}
