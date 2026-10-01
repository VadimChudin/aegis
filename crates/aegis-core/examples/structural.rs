//! Replay local daily Bybit gzip archives with the exact engine used by the desktop panel.
use aegis_core::structural::{self, Params};
fn main() {
    let root = std::env::args().nth(1).expect("trade archive folder");
    let mut ticks = Vec::new();
    for day in std::fs::read_dir(root).expect("archive folder").flatten() {
        let name = day.file_name().to_string_lossy().into_owned();
        if name.as_str() >= "XAUUSDT2026-04-01.csv.gz" && name.as_str() <= "XAUUSDT2026-09-30.csv.gz" {
            ticks.extend(structural::history::read_file(&day.path()).expect("trade archive"));
        }
    }
    ticks.sort_by(|a, b| a.time.total_cmp(&b.time));
    println!(
        "{}",
        serde_json::to_string_pretty(&structural::backtest(&ticks, &Params::default()).expect("backtest"))
            .expect("report json")
    );
}
