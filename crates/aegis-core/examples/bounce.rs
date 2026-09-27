//! Research CLI for the Bounce strategy (same engine as the app).
//!
//!   cargo run --release -p aegis-core --example bounce -- touches bars.csv touches.csv
//!   cargo run --release -p aegis-core --example bounce -- backtest bars.csv [params.json]
//!   cargo run --release -p aegis-core --example bounce -- optimize bars.csv tune.json [params.json]

use std::{io::Write, path::Path};

use aegis_core::bounce::{self, Bar, Params, ScanConfig, TuneSpec, FEATURES, KINDS};

fn params(arg: Option<&String>) -> Params {
    arg.map(|p| serde_json::from_str(&std::fs::read_to_string(p).expect("params file")).expect("params json"))
        .unwrap_or_default()
}

/// Bounce label: from the level, price reaches `x`·ATR in the bounce direction before it goes
/// `y`·ATR through the level, within `n` bars after the touch bar. 1 bounce, 0 break, NaN neither.
fn label(bars: &[Bar], t: &bounce::Touch, x: f64, y: f64, n: usize) -> f64 {
    let d = t.dir as f64;
    let (target, fail) = (t.level + d * x * t.atr, t.level - d * y * t.atr);
    for b in bars.iter().skip(t.i + 1).take(n) {
        let (adv, fav) = if d > 0.0 { (b.low, b.high) } else { (b.high, b.low) };
        if d * (adv - fail) <= 0.0 {
            return 0.0;
        }
        if d * (fav - target) >= 0.0 {
            return 1.0;
        }
    }
    f64::NAN
}

fn excursions(bars: &[Bar], t: &bounce::Touch, n: usize) -> (f64, f64) {
    let d = t.dir as f64;
    let e = bars[t.i].close;
    let (mut mfe, mut mae) = (0.0f64, 0.0f64);
    for b in bars.iter().skip(t.i + 1).take(n) {
        let (adv, fav) = if d > 0.0 { (b.low, b.high) } else { (b.high, b.low) };
        mfe = mfe.max(d * (fav - e));
        mae = mae.max(d * (e - adv));
    }
    (mfe / t.atr, mae / t.atr)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let bars = bounce::load_csv(Path::new(&args[1])).expect("bars");
    match args[0].as_str() {
        "touches" => {
            let cfg: ScanConfig = params(args.get(3)).scan;
            let touches = bounce::scan(&bars, &cfg);
            let mut out = std::io::BufWriter::new(std::fs::File::create(&args[2]).expect("out"));
            let grid = [
                (0.5, 0.25, 12),
                (1.0, 0.5, 12),
                (1.0, 0.5, 24),
                (1.5, 0.5, 24),
                (2.0, 1.0, 48),
            ];
            write!(out, "time,dir,level,atr,close,fill").unwrap();
            for k in KINDS {
                write!(out, ",k_{}", k.id()).unwrap();
            }
            for f in FEATURES {
                write!(out, ",{}", f.id).unwrap();
            }
            for (x, y, n) in grid {
                write!(out, ",bounce_{x}_{y}_{n}").unwrap();
            }
            let tps = [0.5, 0.75, 1.0, 1.5, 2.0];
            for tp in tps {
                write!(out, ",r_{tp}").unwrap();
            }
            writeln!(out, ",risk,mfe12,mae12,mfe24,mae24").unwrap();
            let mut gross = Params {
                maker_bps: 0.0,
                taker_bps: 0.0,
                slippage: 0.0,
                min_risk_atr: 0.0,
                max_risk_atr: 1e9,
                sl_atr: 0.5,
                max_bars: 24,
                ..Params::default()
            };
            for t in &touches {
                write!(
                    out,
                    "{},{},{},{:.4},{},{}",
                    t.time, t.dir, t.level, t.atr, bars[t.i].close, t.fill
                )
                .unwrap();
                for k in KINDS {
                    write!(out, ",{}", u8::from(t.kinds & k.bit() != 0)).unwrap();
                }
                for v in t.pre {
                    write!(out, ",{v:.5}").unwrap();
                }
                for (x, y, n) in grid {
                    write!(out, ",{}", label(&bars, t, x, y, n)).unwrap();
                }
                let mut risk = f64::NAN;
                for tp in tps {
                    gross.tp_r = tp;
                    match bounce::simulate(&bars, t, &gross) {
                        Some((tr, _)) => {
                            risk = t.dir as f64 * (tr.entry - tr.sl);
                            write!(out, ",{:.4}", tr.r).unwrap();
                        }
                        None => write!(out, ",").unwrap(),
                    }
                }
                write!(out, ",{risk:.4}").unwrap();
                let (a, b) = excursions(&bars, t, 12);
                let (c, d) = excursions(&bars, t, 24);
                writeln!(out, ",{a:.4},{b:.4},{c:.4},{d:.4}").unwrap();
            }
            eprintln!("{} bars, {} touches", bars.len(), touches.len());
        }
        "backtest" => {
            let r = bounce::evaluate(&bars, &params(args.get(2)));
            if let Ok(path) = std::env::var("TRADES") {
                let mut out = std::fs::File::create(path).expect("trades file");
                writeln!(out, "entry_time,exit_time,dir,entry,exit,sl,tp,level,outcome,r,prob").unwrap();
                for t in &r.trades {
                    writeln!(
                        out,
                        "{},{},{},{},{},{},{},{},{},{:.4},{:.4}",
                        t.entry_time, t.exit_time, t.dir, t.entry, t.exit, t.sl, t.tp, t.level, t.outcome, t.r, t.prob
                    )
                    .unwrap();
                }
            }
            let brief = serde_json::json!({
                "stats": r.stats, "by_kind": r.by_kind, "by_session": r.by_session, "by_month": r.by_month,
            });
            println!("{}", serde_json::to_string_pretty(&brief).unwrap());
        }
        "optimize" => {
            let spec: TuneSpec =
                serde_json::from_str(&std::fs::read_to_string(&args[2]).expect("tune")).expect("tune json");
            let r = bounce::optimize(&bars, &params(args.get(3)), &spec);
            let brief = serde_json::json!({
                "out_of_sample": r.out_of_sample, "in_sample": r.in_sample, "params": r.params,
                "windows": r.windows.iter().map(|w| serde_json::json!({
                    "test_from": w.test_from, "train": [w.train.trades, w.train.win_rate, w.train.total_r],
                    "test": [w.test.trades, w.test.win_rate, w.test.total_r]})).collect::<Vec<_>>(),
            });
            println!("{}", serde_json::to_string_pretty(&brief).unwrap());
        }
        other => panic!("unknown command {other}"),
    }
}
