//! Research CLI for the Bounce strategy (same engine as the app).
//!
//!   bounce touches  bars.csv touches.csv [params.json]   every touch, metrics and outcomes
//!   bounce backtest bars.csv [params.json]               backtest summary (JSON)
//!   bounce validate bars.csv [params.json]               statistical checks of the settings
//!   bounce optimize bars.csv [ga.json] [params.json]     walk-forward GA + checks
//!   bounce ga-noise bars.csv [ga.json] [params.json]     the same GA on permuted metrics
//!   bounce liveness bars.csv [params.json]               does every setting change the result?
//!   bounce seconds  aggTrades-dir out.sec                1-second candles for the position engine
//!
//! Set TRADES=path to write the trades of backtest/optimize as CSV, MINUTES=path for 1m candles
//! and SECONDS_FILE=path (a file from `seconds` or an aggTrades directory) for the 1-second engine.

use std::{io::Write, path::Path, sync::Arc, time::Instant};

use aegis_core::bounce::{self, param_specs, validate, Bar, Engine, GaSpec, Params, Trade, FEATURES, KINDS};

fn load<T: serde::de::DeserializeOwned + Default>(arg: Option<&String>) -> T {
    arg.map(|p| serde_json::from_str(&std::fs::read_to_string(p).expect("json file")).expect("json"))
        .unwrap_or_default()
}

/// Bounce label: price reaches `x`·ATR from the level before going `y`·ATR through it, within `n` bars.
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

fn write_trades(trades: &[Trade]) {
    if let Ok(path) = std::env::var("TRADES") {
        let mut out = std::fs::File::create(path).expect("trades file");
        writeln!(
            out,
            "entry_time,exit_time,dir,entry,exit,sl,tp,level,kinds,outcome,r,prob,part,flip_entry,flip_exit,flip_outcome,flip_r"
        )
        .unwrap();
        for t in trades {
            writeln!(
                out,
                "{},{},{},{},{},{},{},{},{},{},{:.4},{:.4},{},{},{},{},{:.4}",
                t.entry_time,
                t.exit_time,
                t.dir,
                t.entry,
                t.exit,
                t.sl,
                t.tp,
                t.level,
                t.kinds,
                t.outcome,
                t.r,
                t.prob,
                t.part,
                t.flip_entry,
                t.flip_exit,
                t.flip_outcome,
                t.flip_r
            )
            .unwrap();
        }
    }
}

fn brief(s: &bounce::Stats) -> String {
    format!(
        "n={} win={:.1}% (lo {:.1}%) avgR={:+.3} totR={:+.1} pf={:.2} dd={:.1} /day={:.1} sharpe={:.3}",
        s.trades,
        100.0 * s.win_rate,
        100.0 * s.win_rate_lo,
        s.avg_r,
        s.total_r,
        s.profit_factor,
        s.max_dd_r,
        s.trades_per_day,
        s.sharpe
    )
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args[0] == "seconds" {
        let s = bounce::load_seconds(Path::new(&args[1])).expect("aggTrades");
        bounce::save_seconds(Path::new(&args[2]), &s).expect("write");
        eprintln!("{} seconds", s.len());
        return;
    }
    let bars = Arc::new(bounce::load_csv(Path::new(&args[1])).expect("bars"));
    // MINUTES=path: 1m candles to resolve the order of events inside 5m bars.
    let minutes = Arc::new(
        std::env::var("MINUTES")
            .ok()
            .map(|p| bounce::load_minutes(Path::new(&p)).expect("minutes"))
            .unwrap_or_default(),
    );
    let seconds = Arc::new(
        std::env::var("SECONDS_FILE")
            .ok()
            .map(|p| bounce::load_seconds(Path::new(&p)).expect("seconds"))
            .unwrap_or_default(),
    );
    let engine =
        |p: &Params| Engine::build(bars.clone(), minutes.clone(), &p.scan, p.on_close()).with_seconds(seconds.clone());
    let t0 = Instant::now();
    match args[0].as_str() {
        "touches" => {
            let p: Params = load(args.get(3));
            let touches = bounce::scan(&bars, &p.scan);
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
            writeln!(out, ",risk").unwrap();
            let mut gross = Params {
                maker_bps: 0.0,
                taker_bps: 0.0,
                slippage: 0.0,
                min_risk_atr: 0.0,
                max_risk_atr: 1e9,
                ..p.clone()
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
                for v in p.metrics(t) {
                    write!(out, ",{v:.5}").unwrap();
                }
                for (x, y, n) in grid {
                    write!(out, ",{}", label(&bars, t, x, y, n)).unwrap();
                }
                let mut risk = f64::NAN;
                for tp in tps {
                    gross.tp_r = tp;
                    match bounce::simulate_in(&bars, &minutes, t, &gross) {
                        Some((tr, _)) => {
                            risk = t.dir as f64 * (tr.entry - tr.sl);
                            write!(out, ",{:.4}", tr.r).unwrap();
                        }
                        None => write!(out, ",").unwrap(),
                    }
                }
                writeln!(out, ",{risk:.4}").unwrap();
            }
            eprintln!("{} bars, {} touches", bars.len(), touches.len());
        }
        "backtest" => {
            let p: Params = load(args.get(2));
            let e = engine(&p);
            let r = e.report(&p);
            write_trades(&r.trades);
            let out = serde_json::json!({
                "stats": r.stats, "by_kind": r.by_kind, "by_session": r.by_session, "by_month": r.by_month,
                "scored_from": r.scored_from, "seconds": t0.elapsed().as_secs_f64(),
            });
            println!("{}", serde_json::to_string_pretty(&out).unwrap());
        }
        "validate" => {
            let p: Params = load(args.get(2));
            let e = engine(&p);
            let v = validate(&e, &p, None);
            println!("{}", serde_json::to_string_pretty(&v).unwrap());
        }
        "optimize" | "ga-noise" => {
            let spec: GaSpec = load(args.get(2));
            let p: Params = load(args.get(3));
            let real = engine(&p);
            let e = if args[0] == "ga-noise" {
                bounce::validate::permuted(&real, 99)
            } else {
                real
            };
            eprintln!(
                "engine ready in {:.1}s, {} touches",
                t0.elapsed().as_secs_f64(),
                e.touches.len()
            );
            let last = std::sync::Mutex::new(String::new());
            let r = bounce::optimize(&e, &p, &spec, &|pr| {
                let line = format!("{} {}/{}", pr.stage, pr.window, pr.windows);
                let mut l = last.lock().unwrap();
                if *l != line {
                    eprintln!("  {line} ({:.0}s)", t0.elapsed().as_secs_f64());
                    *l = line;
                }
            });
            write_trades(&r.oos_trades);
            let v = validate(&e, &r.params, Some(&r));
            eprintln!(
                "done in {:.1}s, {} evaluations",
                t0.elapsed().as_secs_f64(),
                r.evaluations
            );
            eprintln!("OOS: {}", brief(&r.out_of_sample));
            for w in &r.windows {
                eprintln!(
                    "  window {}: train {} | test {}",
                    bounce::month_id(w.test_from),
                    brief(&w.train),
                    brief(&w.test)
                );
            }
            let out = serde_json::json!({
                "out_of_sample": r.out_of_sample, "windows": r.windows, "params": r.params,
                "convergence": r.convergence, "baseline": r.baseline, "evaluations": r.evaluations,
                "validation": v, "seconds": t0.elapsed().as_secs_f64(),
            });
            println!("{}", serde_json::to_string_pretty(&out).unwrap());
        }
        "liveness" => {
            // Every setting must change the backtest for at least one value in its range.
            let base: Params = load(args.get(2));
            let mut engines: std::collections::HashMap<String, Engine> = Default::default();
            let engine_for = |engines: &mut std::collections::HashMap<String, Engine>, p: &Params| -> String {
                let key = format!("{:?}{}", p.scan, p.on_close());
                engines.entry(key.clone()).or_insert_with(|| engine(p));
                key
            };
            let fp = |e: &Engine, p: &Params| {
                let r = e.report(p);
                (r.stats.trades, (r.stats.total_r * 1e4).round() as i64, r.signals.len())
            };
            let k0 = engine_for(&mut engines, &base);
            let f0 = fp(&engines[&k0], &base);
            let (mut live, mut dead, mut no_data) = (0, Vec::new(), Vec::<String>::new());
            for s in param_specs() {
                let ids: Vec<String> = if s.kind == "filter" {
                    vec![format!("{}.on", s.id)]
                } else {
                    vec![s.id.clone()]
                };
                for id in ids {
                    let mut alive = false;
                    let candidates: Vec<f64> = if s.kind == "toggle" {
                        vec![1.0 - base.get(&id).unwrap_or(1.0)]
                    } else if s.kind == "filter" {
                        vec![0.25, 0.5, 0.75]
                    } else {
                        vec![0.0, 0.25, 0.5, 0.75, 1.0]
                            .into_iter()
                            .map(|q| s.lo + q * (s.hi - s.lo))
                            .collect()
                    };
                    for q in candidates {
                        let mut p = base.clone();
                        if id.starts_with("kinds.") {
                            // Higher-timeframe swings share prices with 5m swings, so a kind is
                            // checked alone: only this kind enabled.
                            for k in KINDS {
                                p.set(&format!("kinds.{}", k.id()), 0.0);
                            }
                            p.set(&id, 1.0);
                        } else if s.kind == "filter" {
                            // A filter keeping only the middle of the metric's observed range.
                            let m = s.id.trim_start_matches("filters.");
                            let k = FEATURES.iter().position(|f| f.id == m).unwrap();
                            let e = &engines[&k0];
                            let mut vals: Vec<f64> = e
                                .touches
                                .iter()
                                .map(|t| p.metrics(t)[k])
                                .filter(|v| v.is_finite())
                                .collect();
                            if vals.is_empty() || vals.windows(2).all(|w| w[0] == w[1]) {
                                no_data.push(id.clone());
                                break;
                            }
                            vals.sort_by(f64::total_cmp);
                            let lo = vals[((vals.len() - 1) as f64 * (q - 0.25).max(0.0)) as usize];
                            let hi = vals[((vals.len() - 1) as f64 * (q + 0.25).min(1.0)) as usize];
                            p.set(&format!("{}.on", s.id), 1.0);
                            p.set(&format!("{}.min", s.id), lo);
                            p.set(&format!("{}.max", s.id), hi.max(lo + 1e-9));
                        } else {
                            p.set(&id, q);
                        }
                        let key = engine_for(&mut engines, &p);
                        if fp(&engines[&key], &p) != f0 {
                            alive = true;
                            break;
                        }
                    }
                    if alive {
                        live += 1;
                    } else if !no_data.contains(&id) {
                        dead.push(id);
                    }
                }
            }
            println!(
                "{}",
                serde_json::json!({"live": live, "dead": dead, "no_data": no_data, "total": live + dead.len() + no_data.len(), "base": f0, "seconds": t0.elapsed().as_secs_f64()})
            );
        }
        other => panic!("unknown command {other}"),
    }
}
