//! Auto mode: find settings on the train part of the history, report them on the test part.
//!
//! 1. Coordinate search ("one at a time"): every chosen setting in turn is swept over its range
//!    with the others fixed, the best value is kept, and passes repeat until nothing improves.
//!    It costs the sum of the grid sizes per pass, not their product.
//! 2. Genetic algorithm: a population around the best settings so far (tournament selection,
//!    uniform crossover, mutation to random grid values, elitism).
//!
//! Either step can be switched off. The test part is never used for any choice.

use std::sync::atomic::{AtomicBool, Ordering};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use super::{param_specs, ParamSpec, Params, Prepared, Report, Stats};
use crate::bounce::ga::Rng;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchSpec {
    /// Ids of the settings to search; the rest stay as set.
    pub params: Vec<String>,
    pub coordinate: bool,
    pub passes: usize,
    /// Grid points per setting in the coordinate search (evenly spaced over its range).
    pub max_values: usize,
    pub ga: bool,
    pub population: usize,
    pub generations: usize,
    /// Chance that a gene mutates.
    pub mutation: f64,
    pub elite: usize,
    /// "sharpe", "return" (% a year), "calmar" (% a year ÷ max drawdown) or "pf" (profit factor).
    pub objective: String,
    /// Settings with fewer trades on the train part score last.
    pub min_trades: usize,
    /// Share of the history used for the search; the rest is the test.
    pub train_share: f64,
    pub seed: u64,
}

impl Default for SearchSpec {
    fn default() -> Self {
        SearchSpec {
            params: vec![
                "asia_enter".into(),
                "asia_leave".into(),
                "fomc_lead".into(),
                "fomc_exit".into(),
                "vol_target".into(),
            ],
            coordinate: true,
            passes: 3,
            max_values: 40,
            ga: true,
            population: 40,
            generations: 25,
            mutation: 0.15,
            elite: 4,
            objective: "sharpe".into(),
            min_trades: 50,
            train_share: 0.6,
            seed: 7,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct SearchProgress {
    /// "coordinate" or "ga".
    pub stage: &'static str,
    pub pass: usize,
    pub param: String,
    pub done: usize,
    pub total: usize,
    pub best: f64,
    pub evaluations: usize,
}

/// One improvement of the coordinate search.
#[derive(Clone, Debug, Serialize)]
pub struct Step {
    pub pass: usize,
    pub param: String,
    pub from: f64,
    pub to: f64,
    pub score: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct SearchReport {
    pub params: Params,
    pub start_train: Stats,
    pub start_test: Stats,
    pub train: Stats,
    pub test: Stats,
    pub full: Stats,
    pub score: f64,
    pub start_score: f64,
    pub steps: Vec<Step>,
    /// Best train score after each GA generation.
    pub generations: Vec<f64>,
    pub evaluations: usize,
    /// log10 of the number of combinations a full grid of the chosen settings would need.
    pub combinations_log10: f64,
    /// Backtests one coordinate pass needs.
    pub per_pass: usize,
    pub split_time: i64,
    pub cancelled: bool,
    /// Full backtest of the chosen settings (for the charts; includes the train part).
    pub report: Report,
}

/// Grid of one setting: toggles 0 / 1; sliders every step, thinned to `max` points.
fn grid(s: &ParamSpec, max: usize) -> Vec<f64> {
    if s.kind == "toggle" {
        return vec![0.0, 1.0];
    }
    let n = ((s.hi - s.lo) / s.step).round() as usize + 1;
    let k = n.min(max.max(2));
    let mut v: Vec<f64> = (0..k)
        .map(|i| {
            let raw = s.lo + (s.hi - s.lo) * i as f64 / (k - 1).max(1) as f64;
            snap(s, raw)
        })
        .collect();
    v.dedup();
    v
}

fn full_grid_size(s: &ParamSpec) -> f64 {
    if s.kind == "toggle" {
        2.0
    } else {
        ((s.hi - s.lo) / s.step).round() + 1.0
    }
}

fn snap(s: &ParamSpec, v: f64) -> f64 {
    let x = s.lo + ((v - s.lo) / s.step).round() * s.step;
    // Keep decimal steps (0.05) free of binary noise.
    (x.clamp(s.lo, s.hi) * 1e6).round() / 1e6
}

fn score(st: &Stats, spec: &SearchSpec) -> f64 {
    if st.trades < spec.min_trades {
        return -1e9 + st.trades as f64;
    }
    match spec.objective.as_str() {
        "return" => st.pct_per_year,
        "calmar" => st.pct_per_year / st.max_dd_pct.abs().max(0.1),
        "pf" => st.profit_factor,
        _ => st.sharpe,
    }
}

pub fn search(
    pr: &Prepared,
    start: &Params,
    spec: &SearchSpec,
    progress: &(dyn Fn(SearchProgress) + Sync),
    cancel: &AtomicBool,
) -> SearchReport {
    let specs: Vec<ParamSpec> = param_specs()
        .into_iter()
        .filter(|s| s.tunable && spec.params.iter().any(|p| p == s.id))
        .collect();
    let split = pr.split(spec.train_share);
    let n = pr.len();
    let train = |p: &Params| pr.stats(p, 0, split);
    let eval = |p: &Params| score(&train(p), spec);
    let mut evaluations = 0usize;
    let mut best = start.clone();
    let start_score = eval(start);
    let mut best_score = start_score;
    let mut steps = Vec::new();
    let grids: Vec<Vec<f64>> = specs.iter().map(|s| grid(s, spec.max_values)).collect();
    let per_pass: usize = grids.iter().map(Vec::len).sum();
    let stop = || cancel.load(Ordering::Relaxed);

    if spec.coordinate && !specs.is_empty() {
        'passes: for pass in 1..=spec.passes.max(1) {
            let mut improved = false;
            for (k, (s, g)) in specs.iter().zip(&grids).enumerate() {
                if stop() {
                    break 'passes;
                }
                progress(SearchProgress {
                    stage: "coordinate",
                    pass,
                    param: s.label.to_string(),
                    done: k,
                    total: specs.len(),
                    best: best_score,
                    evaluations,
                });
                let scored: Vec<(f64, f64)> = g
                    .par_iter()
                    .map(|&v| {
                        let mut p = best.clone();
                        p.set(s.id, v);
                        (v, eval(&p))
                    })
                    .collect();
                evaluations += scored.len();
                let from = best.get(s.id).unwrap_or(f64::NAN);
                // Ties keep the current value, so the search never wanders on a flat score.
                if let Some(&(v, sc)) = scored.iter().max_by(|a, b| a.1.total_cmp(&b.1)) {
                    if sc > best_score + 1e-9 && v != from {
                        best.set(s.id, v);
                        best_score = sc;
                        improved = true;
                        steps.push(Step {
                            pass,
                            param: s.id.to_string(),
                            from,
                            to: v,
                            score: sc,
                        });
                    }
                }
            }
            if !improved {
                break;
            }
        }
    }

    let mut generations = Vec::new();
    if spec.ga && !specs.is_empty() && !stop() {
        let mut rng = Rng::new(spec.seed);
        let pop_n = spec.population.max(4);
        let genome = |p: &Params| specs.iter().map(|s| p.get(s.id).unwrap_or(s.lo)).collect::<Vec<f64>>();
        let params_of = |g: &[f64]| {
            let mut p = best.clone();
            for (s, &v) in specs.iter().zip(g) {
                p.set(s.id, v);
            }
            p
        };
        let random_value = |s: &ParamSpec, rng: &mut Rng| {
            if s.kind == "toggle" {
                f64::from(u8::from(rng.u() < 0.5))
            } else {
                snap(s, s.lo + rng.u() * (s.hi - s.lo))
            }
        };
        let mut pop: Vec<Vec<f64>> = vec![genome(&best)];
        while pop.len() < pop_n {
            // Half the population starts near the best, half anywhere.
            let mut g = genome(&best);
            let wide = pop.len() % 2 == 0;
            for (j, s) in specs.iter().enumerate() {
                if wide || rng.u() < 0.3 {
                    g[j] = random_value(s, &mut rng);
                }
            }
            pop.push(g);
        }
        let mut scores: Vec<f64> = pop.par_iter().map(|g| eval(&params_of(g))).collect();
        evaluations += pop.len();
        for gen in 1..=spec.generations {
            if stop() {
                break;
            }
            let mut order: Vec<usize> = (0..pop.len()).collect();
            order.sort_by(|&a, &b| scores[b].total_cmp(&scores[a]));
            let mut next: Vec<Vec<f64>> = order.iter().take(spec.elite.min(pop_n)).map(|&i| pop[i].clone()).collect();
            let pick = |rng: &mut Rng| {
                let (a, b, c) = (rng.below(pop.len()), rng.below(pop.len()), rng.below(pop.len()));
                *[a, b, c].iter().max_by(|x, y| scores[**x].total_cmp(&scores[**y])).unwrap_or(&a)
            };
            while next.len() < pop_n {
                let (pa, pb) = (pick(&mut rng), pick(&mut rng));
                let child: Vec<f64> = specs
                    .iter()
                    .enumerate()
                    .map(|(j, s)| {
                        let v = if rng.u() < 0.5 { pop[pa][j] } else { pop[pb][j] };
                        if rng.u() < spec.mutation {
                            random_value(s, &mut rng)
                        } else {
                            v
                        }
                    })
                    .collect();
                next.push(child);
            }
            let elite = spec.elite.min(pop_n);
            let mut new_scores: Vec<f64> = order.iter().take(elite).map(|&i| scores[i]).collect();
            new_scores.extend(next[elite..].par_iter().map(|g| eval(&params_of(g))).collect::<Vec<_>>());
            evaluations += next.len() - elite;
            pop = next;
            scores = new_scores;
            let top = scores.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            generations.push(top);
            progress(SearchProgress {
                stage: "ga",
                pass: 0,
                param: String::new(),
                done: gen,
                total: spec.generations,
                best: top.max(best_score),
                evaluations,
            });
        }
        if let Some((i, &sc)) = scores.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)) {
            if sc > best_score + 1e-9 {
                best = params_of(&pop[i]);
                best_score = sc;
            }
        }
    }

    SearchReport {
        start_train: train(start),
        start_test: pr.stats(start, split, n),
        train: train(&best),
        test: pr.stats(&best, split, n),
        full: pr.stats(&best, 0, n),
        score: best_score,
        start_score,
        report: pr.report(&best, 0, n),
        params: best,
        steps,
        generations,
        evaluations,
        combinations_log10: specs.iter().map(|s| full_grid_size(s).log10()).sum(),
        per_pass,
        split_time: pr.time.get(split).copied().unwrap_or_default(),
        cancelled: stop(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::{time, HBar};

    /// Hourly bars where price rises only between 20:00 and 23:00 New York: the search must move
    /// the Asian window onto those hours.
    fn planted() -> Prepared {
        let (a, b) = (time::parse_date("2025-01-06").unwrap() * 86_400, time::parse_date("2026-06-30").unwrap() * 86_400);
        let mut v = Vec::new();
        let mut px = 1000.0f32;
        let mut k = 0u64;
        for t in (a..b).step_by(3600) {
            let ny = t + time::ny_offset(t);
            let min = ny.rem_euclid(86_400) / 60;
            let wd = time::weekday(ny.div_euclid(86_400));
            if wd == 5 || (wd == 4 && min >= 1020) || (wd == 6 && min < 1080) || (1020..1080).contains(&min) {
                continue;
            }
            k = k.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let noise = ((k >> 33) as f32 / (1u64 << 31) as f32 - 0.5) * 0.4;
            let drift = if (20 * 60..23 * 60).contains(&min) { 0.6 } else { -0.1 };
            let c = px + drift + noise;
            v.push(HBar {
                time: t,
                open: px,
                high: px.max(c),
                low: px.min(c),
                close: c,
                volume: 1.0,
                spread_open: 0.05,
                spread_close: 0.05,
            });
            px = c;
        }
        Prepared::new(&v, 3600)
    }

    #[test]
    fn coordinate_search_finds_the_planted_window_and_ga_keeps_it() {
        let pr = planted();
        let start = Params { asia_enter: 18.0 * 60.0, asia_leave: 360.0, fomc: false, vol_target: 0.0, ..Params::default() };
        let spec = SearchSpec {
            params: vec!["asia_enter".into(), "asia_leave".into()],
            max_values: 100,
            generations: 5,
            population: 12,
            min_trades: 20,
            ..SearchSpec::default()
        };
        let r = search(&pr, &start, &spec, &|_| {}, &AtomicBool::new(false));
        assert!(r.score > r.start_score, "{} vs {}", r.score, r.start_score);
        // Hourly bars: any entry in (19:00, 20:00] enters at 20:00; flat from midnight is best.
        assert!(r.params.asia_enter > 19.0 * 60.0 && r.params.asia_enter <= 20.0 * 60.0, "{:?}", r.params);
        assert_eq!(r.params.asia_leave, 0.0, "{:?}", r.params);
        assert!(r.test.pct_per_year > 0.0 && r.test.pct_per_year > r.start_test.pct_per_year);
        assert!(!r.steps.is_empty() && r.evaluations > 0 && r.generations.len() == 5);
        assert!(r.combinations_log10 > 3.0);
        assert!(r.split_time > pr.time[0]);
    }

    #[test]
    fn search_steps_can_be_switched_off_and_cancelled() {
        let pr = planted();
        let start = Params::default();
        let none = SearchSpec { coordinate: false, ga: false, ..SearchSpec::default() };
        let r = search(&pr, &start, &none, &|_| {}, &AtomicBool::new(false));
        assert_eq!((r.evaluations, r.params.clone()), (0, start.clone()));
        let cancelled = search(&pr, &start, &SearchSpec::default(), &|_| {}, &AtomicBool::new(true));
        assert!(cancelled.cancelled && cancelled.evaluations == 0);
    }

    #[test]
    fn grids_snap_to_steps() {
        let s = param_specs().into_iter().find(|s| s.id == "stop_pct").unwrap();
        let g = grid(&s, 7);
        assert_eq!(g.len(), 7);
        assert_eq!((g[0], g[6]), (0.0, 3.0));
        assert!(g.iter().all(|v| ((v / 0.05).round() * 0.05 - v).abs() < 1e-9));
        let t = param_specs().into_iter().find(|s| s.id == "mon").unwrap();
        assert_eq!(grid(&t, 40), vec![0.0, 1.0]);
    }
}
