//! Asia + FOMC on a data folder built by the History panel (or any folder with `<tf>.bin`).
//!
//!     cargo run --release -p aegis-core --example asia_fomc -- <folder> [tf] [params.json] [--search]
//!
//! Prints the backtest of the default (or given) settings; `--search` runs the auto mode with the
//! default search settings and prints train / test results.

use std::{path::Path, sync::atomic::AtomicBool, time::Instant};

use aegis_core::{
    asia_fomc::{search, Params, Prepared, SearchSpec, Stats},
    dataset, Timeframe,
};

fn line(name: &str, s: &Stats) {
    println!(
        "{name:<28} trades {:>5}  win {:>5.1}%  avg {:>+6.2} bp  {:>+6.2} %/yr  Sharpe {:>+5.2}  max DD {:>6.1}%  years up {}/{}",
        s.trades,
        s.win_rate * 100.0,
        s.avg_bp,
        s.pct_per_year,
        s.sharpe,
        s.max_dd_pct,
        s.years_up,
        s.years
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let folder = Path::new(args.first().map(String::as_str).unwrap_or("."));
    let tf = args.get(1).and_then(|s| Timeframe::parse(s)).unwrap_or(Timeframe::M1);
    let params: Params = args
        .iter()
        .find(|a| a.ends_with(".json"))
        .map(|f| serde_json::from_slice(&std::fs::read(f).expect("read params")).expect("params json"))
        .unwrap_or_default();
    let t0 = Instant::now();
    let bars = dataset::load(folder, tf).expect("load bars");
    let pr = Prepared::new(&bars, tf.seconds());
    drop(bars);
    println!("{} bars of {} prepared in {:.1} s", pr.len(), tf.as_str(), t0.elapsed().as_secs_f64());
    let t1 = Instant::now();
    let r = pr.report(&params, 0, pr.len());
    println!("backtest in {:.3} s", t1.elapsed().as_secs_f64());
    line("all", &r.stats);
    for y in &r.years {
        println!("  {}  {:>+6.2}%  trades {:>4}  Sharpe {:>+5.2}", y.year, y.pct, y.trades, y.sharpe);
    }
    let half = pr.split(0.5);
    line("first half", &pr.stats(&params, 0, half));
    line("second half", &pr.stats(&params, half, pr.len()));
    if args.iter().any(|a| a == "--search") {
        let t2 = Instant::now();
        let s = search(&pr, &params, &SearchSpec::default(), &|p| eprintln!("{} {} {}/{} best {:.3}", p.stage, p.param, p.done, p.total, p.best), &AtomicBool::new(false));
        println!("search: {} backtests in {:.1} s, full grid 10^{:.1}", s.evaluations, t2.elapsed().as_secs_f64(), s.combinations_log10);
        line("start, train", &s.start_train);
        line("start, test", &s.start_test);
        line("found, train", &s.train);
        line("found, test", &s.test);
        println!("{}", serde_json::to_string(&s.params).unwrap_or_default());
    }
}
