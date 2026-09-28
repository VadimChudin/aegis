//! Is the edge real? Statistical checks of a backtest and of the GA that produced it.
//!
//! - Probabilistic and Deflated Sharpe Ratio (Bailey & López de Prado 2012, 2014): the chance the
//!   true Sharpe exceeds 0, or exceeds the best Sharpe expected from N useless trials.
//! - Probability of Backtest Overfitting by combinatorially symmetric cross-validation
//!   (Bailey, Borwein, López de Prado & Zhu 2017).
//! - Stationary bootstrap confidence intervals over days (Politis & Romano 1994).
//! - Control with levels moved to meaningless prices, and a permutation test that shuffles the
//!   metrics between touches (Aronson 2006, "Evidence-Based Technical Analysis", ch. 5).

use std::sync::Arc;

use serde::Serialize;

use super::{
    engine::{daily_r, stats, Engine, Stats, Trade},
    ga::{Baseline, OptimizeReport, Rng, Trial},
    params::Params,
    scan::ScanConfig,
};

const EULER_GAMMA: f64 = 0.577_215_664_901_532_9;

/// Standard normal CDF (erfc with Chebyshev fit, |error| < 1.2e-7; Numerical Recipes 6.2).
pub fn norm_cdf(x: f64) -> f64 {
    let z = x.abs() / std::f64::consts::SQRT_2;
    let t = 1.0 / (1.0 + 0.5 * z);
    let r = t
        * (-z * z - 1.265_512_23
            + t * (1.000_023_68
                + t * (0.374_091_96
                    + t * (0.096_784_18
                        + t * (-0.186_288_06
                            + t * (0.278_868_07
                                + t * (-1.135_203_98
                                    + t * (1.488_515_87 + t * (-0.822_152_23 + t * 0.170_872_77)))))))))
            .exp();
    if x >= 0.0 {
        1.0 - 0.5 * r
    } else {
        0.5 * r
    }
}

/// Inverse standard normal CDF (Acklam's rational approximation, relative error < 1.2e-9).
#[allow(clippy::excessive_precision)]
pub fn norm_ppf(p: f64) -> f64 {
    let p = p.clamp(1e-300, 1.0 - 1e-16);
    let a = [
        -3.969683028665376e1,
        2.209460984245205e2,
        -2.759285104469687e2,
        1.383577518672690e2,
        -3.066479806614716e1,
        2.506628277459239,
    ];
    let b = [
        -5.447609879822406e1,
        1.615858368580409e2,
        -1.556989798598866e2,
        6.680131188771972e1,
        -1.328068155288572e1,
    ];
    let c = [
        -7.784894002430293e-3,
        -3.223964580411365e-1,
        -2.400758277161838,
        -2.549732539343734,
        4.374664141464968,
        2.938163982698783,
    ];
    let d = [
        7.784695709041462e-3,
        3.224671290700398e-1,
        2.445134137142996,
        3.754408661907416,
    ];
    let pl = 0.02425;
    if p < pl {
        let q = (-2.0 * p.ln()).sqrt();
        (((((c[0] * q + c[1]) * q + c[2]) * q + c[3]) * q + c[4]) * q + c[5])
            / ((((d[0] * q + d[1]) * q + d[2]) * q + d[3]) * q + 1.0)
    } else if p <= 1.0 - pl {
        let q = p - 0.5;
        let r = q * q;
        (((((a[0] * r + a[1]) * r + a[2]) * r + a[3]) * r + a[4]) * r + a[5]) * q
            / (((((b[0] * r + b[1]) * r + b[2]) * r + b[3]) * r + b[4]) * r + 1.0)
    } else {
        let q = (-2.0 * (1.0 - p).ln()).sqrt();
        -(((((c[0] * q + c[1]) * q + c[2]) * q + c[3]) * q + c[4]) * q + c[5])
            / ((((d[0] * q + d[1]) * q + d[2]) * q + d[3]) * q + 1.0)
    }
}

/// (mean, std, skewness, kurtosis — not excess) of a sample.
pub fn moments(x: &[f64]) -> (f64, f64, f64, f64) {
    let n = x.len() as f64;
    if n < 2.0 {
        return (x.first().copied().unwrap_or(0.0), 0.0, 0.0, 3.0);
    }
    let m = x.iter().sum::<f64>() / n;
    let (mut m2, mut m3, mut m4) = (0.0, 0.0, 0.0);
    for v in x {
        let d = v - m;
        m2 += d * d;
        m3 += d * d * d;
        m4 += d * d * d * d;
    }
    let (m2, m3, m4) = (m2 / n, m3 / n, m4 / n);
    if m2 <= 0.0 {
        return (m, 0.0, 0.0, 3.0);
    }
    (m, (m2 * n / (n - 1.0)).sqrt(), m3 / m2.powf(1.5), m4 / (m2 * m2))
}

/// Probabilistic Sharpe Ratio: P(true SR > `sr_star`) for an observed per-trade `sr` over `n`
/// trades with the given skewness and kurtosis.
pub fn psr(sr: f64, n: usize, skew: f64, kurt: f64, sr_star: f64) -> f64 {
    if n < 2 {
        return 0.0;
    }
    let den = (1.0 - skew * sr + (kurt - 1.0) / 4.0 * sr * sr).max(1e-12).sqrt();
    norm_cdf((sr - sr_star) * ((n - 1) as f64).sqrt() / den)
}

/// Expected maximum Sharpe of `n_trials` independent trials with true SR 0 and the given
/// variance of their Sharpes (False Strategy Theorem).
pub fn expected_max_sharpe(n_trials: usize, var: f64) -> f64 {
    if n_trials < 2 || var <= 0.0 {
        return 0.0;
    }
    let n = n_trials as f64;
    var.sqrt()
        * ((1.0 - EULER_GAMMA) * norm_ppf(1.0 - 1.0 / n)
            + EULER_GAMMA * norm_ppf(1.0 - 1.0 / (n * std::f64::consts::E)))
}

/// Probability of Backtest Overfitting by CSCV. `perf[c][s]` holds (n, sum R, sum R²) of config
/// `c` in time block `s`; the statistic of a union of blocks is the t-stat of the mean R.
/// Returns (PBO, probability that the IS-best config loses out of sample, combinations).
pub fn pbo(perf: &[Vec<(f64, f64, f64)>]) -> (f64, f64, usize) {
    let nc = perf.len();
    let s = perf.first().map_or(0, |v| v.len());
    if nc < 2 || s < 2 || s % 2 != 0 {
        return (f64::NAN, f64::NAN, 0);
    }
    let tstat = |c: usize, blocks: &[usize]| {
        let (mut n, mut sum, mut sq) = (0.0, 0.0, 0.0);
        for &b in blocks {
            let v = perf[c][b];
            n += v.0;
            sum += v.1;
            sq += v.2;
        }
        if n < 2.0 {
            return f64::NEG_INFINITY;
        }
        let m = sum / n;
        let sd = ((sq - n * m * m) / (n - 1.0)).max(1e-12).sqrt();
        m / sd * n.sqrt()
    };
    let mut overfit = 0usize;
    let mut loss = 0usize;
    let mut combos = 0usize;
    // Every subset of s/2 blocks as in-sample (bitmask enumeration; s ≤ 16).
    for mask in 0u32..(1 << s) {
        if mask.count_ones() as usize != s / 2 {
            continue;
        }
        let is: Vec<usize> = (0..s).filter(|b| mask >> b & 1 == 1).collect();
        let os: Vec<usize> = (0..s).filter(|b| mask >> b & 1 == 0).collect();
        let is_perf: Vec<f64> = (0..nc).map(|c| tstat(c, &is)).collect();
        let best = (0..nc).max_by(|&a, &b| is_perf[a].total_cmp(&is_perf[b])).unwrap_or(0);
        let os_perf: Vec<f64> = (0..nc).map(|c| tstat(c, &os)).collect();
        // Relative rank of the IS winner out of sample, in (0, 1).
        let below = os_perf.iter().filter(|&&v| v < os_perf[best]).count() as f64;
        let omega = (below + 1.0) / (nc as f64 + 1.0);
        let logit = (omega / (1.0 - omega)).ln();
        combos += 1;
        overfit += usize::from(logit <= 0.0);
        loss += usize::from(os_perf[best] <= 0.0);
    }
    (overfit as f64 / combos as f64, loss as f64 / combos as f64, combos)
}

/// 95% stationary-bootstrap intervals over days: (win rate, mean R per trade, trades per day).
pub fn bootstrap(trades: &[Trade], reps: usize, mean_block: f64, seed: u64) -> [(f64, f64); 3] {
    use std::collections::BTreeMap;
    let mut days: BTreeMap<i64, (f64, f64, f64)> = BTreeMap::new();
    for t in trades {
        let d = days.entry(t.entry_time.div_euclid(86_400)).or_default();
        d.0 += 1.0;
        d.1 += f64::from(u8::from(t.r > 0.0));
        d.2 += t.r;
    }
    let days: Vec<(f64, f64, f64)> = days.into_values().collect();
    let n = days.len();
    if n < 5 {
        return [(f64::NAN, f64::NAN); 3];
    }
    let mut rng = Rng::new(seed);
    let p = 1.0 / mean_block.max(1.0);
    let (mut wr, mut mr, mut pd) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..reps {
        let (mut cnt, mut wins, mut sum) = (0.0, 0.0, 0.0);
        let mut i = rng.below(n);
        for _ in 0..n {
            let d = days[i];
            cnt += d.0;
            wins += d.1;
            sum += d.2;
            i = if rng.u() < p { rng.below(n) } else { (i + 1) % n };
        }
        if cnt > 0.0 {
            wr.push(wins / cnt);
            mr.push(sum / cnt);
            pd.push(cnt / n as f64);
        }
    }
    let ci = |v: &mut Vec<f64>| {
        v.sort_by(f64::total_cmp);
        let k = v.len();
        (v[k * 25 / 1000], v[(k * 975 / 1000).min(k - 1)])
    };
    [ci(&mut wr), ci(&mut mr), ci(&mut pd)]
}

/// Mean pairwise Pearson correlation of equal-length series, clamped to [0, 1].
pub fn mean_correlation(series: &[Vec<f64>]) -> f64 {
    let (mut sum, mut n) = (0.0, 0usize);
    for i in 0..series.len() {
        for j in i + 1..series.len() {
            let (a, b) = (&series[i], &series[j]);
            let (ma, sa, _, _) = moments(a);
            let (mb, sb, _, _) = moments(b);
            if sa > 0.0 && sb > 0.0 && a.len() == b.len() && a.len() > 2 {
                let cov = a.iter().zip(b).map(|(x, y)| (x - ma) * (y - mb)).sum::<f64>() / (a.len() as f64 - 1.0);
                sum += cov / (sa * sb);
                n += 1;
            }
        }
    }
    if n == 0 {
        0.0
    } else {
        (sum / n as f64).clamp(0.0, 1.0)
    }
}

/// Welch's t-test p-value (one-sided: mean of `a` > mean of `b`), normal approximation.
pub fn welch(a: &[f64], b: &[f64]) -> f64 {
    let (ma, sa, _, _) = moments(a);
    let (mb, sb, _, _) = moments(b);
    let se = (sa * sa / a.len().max(1) as f64 + sb * sb / b.len().max(1) as f64).sqrt();
    if a.len() < 2 || b.len() < 2 || se <= 0.0 {
        return f64::NAN;
    }
    1.0 - norm_cdf((ma - mb) / se)
}

/// Two-proportion z-test p-value (one-sided: a > b).
pub fn prop_test(wa: usize, na: usize, wb: usize, nb: usize) -> f64 {
    if na == 0 || nb == 0 {
        return f64::NAN;
    }
    let (pa, pb) = (wa as f64 / na as f64, wb as f64 / nb as f64);
    let p = (wa + wb) as f64 / (na + nb) as f64;
    let se = (p * (1.0 - p) * (1.0 / na as f64 + 1.0 / nb as f64)).sqrt();
    if se <= 0.0 {
        return f64::NAN;
    }
    1.0 - norm_cdf((pa - pb) / se)
}

#[derive(Clone, Debug, Serialize)]
pub struct Check {
    pub id: &'static str,
    pub pass: bool,
    pub value: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Validation {
    /// Trades the checks are about (out of sample for a GA run, the backtest otherwise).
    pub stats: Stats,
    pub sharpe: f64,
    pub psr: f64,
    /// Deflated Sharpe Ratio of the selected settings on the window they were chosen on.
    pub dsr: f64,
    pub selected_sharpe: f64,
    pub sr0: f64,
    pub trials: usize,
    pub pbo: f64,
    pub pbo_loss: f64,
    pub pbo_configs: usize,
    pub pbo_combinations: usize,
    pub boot_win_rate: (f64, f64),
    pub boot_avg_r: (f64, f64),
    pub boot_per_day: (f64, f64),
    /// Same settings, levels moved to random prices.
    pub control: Stats,
    /// Same settings, metrics shuffled between touches.
    pub permuted: Stats,
    /// t-statistic of the daily R (Harvey & Liu: a new effect needs t > 3).
    pub t_daily: f64,
    /// (fill rule, stats): limit fills on touch, as set, and only when traded $0.10 through.
    pub fills: Vec<(String, Stats)>,
    /// Every touch traded, no filters and no model: the base rate.
    pub base: Stats,
    pub baseline: Option<Baseline>,
    pub checks: Vec<Check>,
}

fn control_engine(e: &Engine) -> Engine {
    let scan = ScanConfig {
        control_shift: 1.0,
        ..e.scan
    };
    Engine::build(e.bars.clone(), e.minutes.clone(), &scan, e.close_entry).with_seconds(e.seconds.clone())
}

fn permuted_engine(e: &Engine, seed: u64) -> Engine {
    let mut touches = e.touches.clone();
    let mut rng = Rng::new(seed);
    let n = touches.len();
    // Fisher–Yates over the metric vectors only; outcomes stay with their bars.
    for i in (1..n).rev() {
        let j = rng.below(i + 1);
        let (fi, pi) = (touches[i].f, touches[i].pre);
        touches[i].f = touches[j].f;
        touches[i].pre = touches[j].pre;
        touches[j].f = fi;
        touches[j].pre = pi;
    }
    Engine::with_touches(
        e.bars.clone(),
        e.minutes.clone(),
        touches,
        Vec::new(),
        &e.scan,
        e.close_entry,
    )
    .with_seconds(e.seconds.clone())
}

fn scored_range(e: &Engine, p: &Params) -> (usize, usize) {
    let from = if p.use_model { e.index_at(e.scored_from()) } else { 0 };
    (from, e.bars.len())
}

/// Checks for settings: `opt` adds the GA-specific ones (DSR over its trials, PBO, baseline).
pub fn validate(e: &Engine, p: &Params, opt: Option<&OptimizeReport>) -> Validation {
    let (from, to) = scored_range(e, p);
    let (trades, st, st_from, st_to) = match opt {
        Some(o) => {
            let a = o.windows.first().map_or(e.bars[from].time, |w| w.test_from);
            let z = o.windows.last().map_or(e.bars[to - 1].time + 300, |w| w.test_to);
            (o.oos_trades.clone(), o.out_of_sample.clone(), a, z)
        }
        None => {
            let t = e.trades(p, from, to);
            let (a, z) = (e.bars[from.min(e.bars.len() - 1)].time, e.bars[to - 1].time + 300);
            let s = stats(&t, a, z);
            (t, s, a, z)
        }
    };
    let rs: Vec<f64> = trades.iter().map(|t| t.r).collect();
    let (_, _, skew, kurt) = moments(&rs);
    let sharpe = st.sharpe;
    let psr0 = psr(sharpe, rs.len(), skew, kurt, 0.0);

    // Deflated Sharpe of the selection: daily Sharpe of the chosen settings on the window they
    // were chosen on, against the expected maximum daily Sharpe of the trials evaluated there.
    let (dsr, selected, sr0, n_trials) = match opt {
        Some(o) => {
            let (a, z) = o.recent;
            let (ta, tz) = (e.bars[a].time, e.bars[z - 1].time + 300);
            let train_end = e.bars[a + (z - a) * 3 / 4].time;
            let sel = daily_r(&e.trades(&o.params, a, z), ta, tz);
            let _ = train_end;
            let (m, sd, sk, ku) = moments(&sel);
            let sr = if sd > 0.0 { m / sd } else { 0.0 };
            let valid: Vec<&Trial> = o.trials.iter().filter(|t| t.trades >= o.objective.min_trades).collect();
            let srs: Vec<f64> = valid.iter().map(|t| t.sharpe).collect();
            let (_, sd_trials, _, _) = moments(&srs);
            // GA trials are strongly correlated: effective trials N̂ = ρ̄ + (1 − ρ̄)·M, with ρ̄ the
            // mean pairwise correlation of the daily R of the final population (Bailey & López de
            // Prado 2014, section 3).
            let series: Vec<Vec<f64>> = o
                .final_population
                .iter()
                .take(30)
                .map(|c| daily_r(&e.trades(c, a, z), ta, tz))
                .collect();
            let rho = mean_correlation(&series);
            let n_eff = (rho + (1.0 - rho) * valid.len() as f64).round().max(1.0) as usize;
            let sr0 = expected_max_sharpe(n_eff, sd_trials * sd_trials);
            (psr(sr, sel.len(), sk, ku, sr0), sr, sr0, n_eff)
        }
        None => {
            let d = daily_r(&trades, st_from, st_to);
            let (m, sd, sk, ku) = moments(&d);
            let sr = if sd > 0.0 { m / sd } else { 0.0 };
            (psr(sr, d.len(), sk, ku, 0.0), sr, 0.0, 1)
        }
    };

    // PBO over the GA's final population, 10 time blocks of the scored range.
    let (mut pbo_v, mut pbo_loss, mut combos, mut configs) = (f64::NAN, f64::NAN, 0, 0);
    if let Some(o) = opt {
        let mut pool: Vec<Params> = o.final_population.clone();
        pool.push(o.params.clone());
        pool.truncate(40);
        configs = pool.len();
        let blocks = 10usize;
        let edges: Vec<usize> = (0..=blocks).map(|b| from + (to - from) * b / blocks).collect();
        let perf: Vec<Vec<(f64, f64, f64)>> = pool
            .iter()
            .map(|c| {
                (0..blocks)
                    .map(|b| {
                        let q = e.quick(c, edges[b], edges[b + 1]);
                        (q.n as f64, q.sum, q.sumsq)
                    })
                    .collect()
            })
            .collect();
        (pbo_v, pbo_loss, combos) = pbo(&perf);
    }

    let [bwr, bmr, bpd] = bootstrap(&trades, 2000, 5.0, 11);
    // Control and permutation over the same period as the checked trades.
    let window = |eng: &Engine| {
        let (a, z) = (eng.index_at(st_from), eng.index_at(st_to).max(1));
        let a = a.min(z - 1);
        let t = eng.trades(p, a, z);
        let r: Vec<f64> = t.iter().map(|x| x.r).collect();
        (stats(&t, st_from, st_to), r)
    };
    let (control, control_r) = window(&control_engine(e));
    let (permuted, permuted_r) = window(&permuted_engine(e, 23));
    let p_control = welch(&rs, &control_r);
    let t_daily = {
        let d = daily_r(&trades, st_from, st_to);
        let (m, sd, _, _) = moments(&d);
        if sd > 0.0 {
            m / sd * (d.len() as f64).sqrt()
        } else {
            0.0
        }
    };
    // Fill realism (hftbacktest's point: limit fills decide a bounce strategy). Same settings,
    // three fill rules, over the checked period.
    let chosen = opt.map_or(p.clone(), |o| o.params.clone());
    let (fa, fz) = (e.index_at(st_from), e.index_at(st_to).max(1));
    let fills: Vec<(String, Stats)> = [("touch", 0.0), ("set", chosen.fill_through), ("through $0.10", 0.10)]
        .iter()
        .map(|(k, ft)| {
            let q = Params {
                fill_through: *ft,
                ..chosen.clone()
            };
            (k.to_string(), stats(&e.trades(&q, fa, fz), st_from, st_to))
        })
        .collect();
    let p_perm = welch(&rs, &permuted_r);
    let base = {
        let bp = Params {
            use_model: false,
            filters: Default::default(),
            max_open: 1,
            ..p.clone()
        };
        let t = e.trades(&bp, from, to);
        stats(&t, e.bars[from.min(e.bars.len() - 1)].time, e.bars[to - 1].time + 300)
    };

    let mut checks = vec![
        Check {
            id: "oos_positive",
            pass: bmr.0 > 0.0,
            value: format!("{:+.3} R [{:+.3}, {:+.3}]", st.avg_r, bmr.0, bmr.1),
        },
        Check {
            id: "psr",
            pass: psr0 >= 0.95,
            value: format!("{psr0:.3}"),
        },
        Check {
            id: "t_daily",
            pass: t_daily > 3.0,
            value: format!("{t_daily:.2}"),
        },
        Check {
            id: "fills",
            pass: fills.last().is_some_and(|f| f.1.avg_r > 0.0),
            value: fills
                .iter()
                .map(|(k, s)| format!("{k}: {:+.3} R, {:.1}%", s.avg_r, 100.0 * s.win_rate))
                .collect::<Vec<_>>()
                .join(" · "),
        },
        Check {
            id: "dsr",
            pass: dsr >= 0.95,
            value: format!("{dsr:.3} (SR {selected:.3} vs SR0 {sr0:.3}, N={n_trials})"),
        },
        Check {
            id: "control",
            pass: p_control < 0.01,
            value: format!(
                "{:+.3} R vs {:+.3} R on random levels (p={:.1e}); win {:.1}% vs {:.1}%",
                st.avg_r,
                control.avg_r,
                p_control,
                100.0 * st.win_rate,
                100.0 * control.win_rate
            ),
        },
        Check {
            id: "permutation",
            pass: permuted.trades < 30 || p_perm < 0.01,
            value: format!(
                "{:+.3} R vs {:+.3} R with shuffled metrics (p={:.1e}), {} trades",
                st.avg_r, permuted.avg_r, p_perm, permuted.trades
            ),
        },
    ];
    if let Some(o) = opt {
        checks.push(Check {
            id: "pbo",
            pass: pbo_v < 0.5,
            value: format!("{pbo_v:.2} ({configs} settings, {combos} splits)"),
        });
        let ro = &o.baseline.random_out_of_sample;
        let t = |s: &Stats| s.sharpe * (s.trades as f64).sqrt();
        checks.push(Check {
            id: "ga_vs_random",
            pass: t(&o.out_of_sample) >= t(ro),
            value: format!(
                "out of sample: GA {:+.3} R × {} (t={:.1}) vs random search {:+.3} R × {} (t={:.1})",
                o.out_of_sample.avg_r,
                o.out_of_sample.trades,
                t(&o.out_of_sample),
                ro.avg_r,
                ro.trades,
                t(ro)
            ),
        });
    }
    Validation {
        stats: st,
        sharpe,
        psr: psr0,
        dsr,
        selected_sharpe: selected,
        sr0,
        trials: n_trials,
        pbo: pbo_v,
        pbo_loss,
        pbo_configs: configs,
        pbo_combinations: combos,
        boot_win_rate: bwr,
        boot_avg_r: bmr,
        boot_per_day: bpd,
        control,
        permuted,
        t_daily,
        fills,
        base,
        baseline: opt.map(|o| o.baseline.clone()),
        checks,
    }
}

/// Engine for the same bars with the metrics permuted: used by the CLI's GA-on-noise check.
pub fn permuted(e: &Engine, seed: u64) -> Engine {
    permuted_engine(e, seed)
}

/// Bars shared between engines.
pub type SharedBars = Arc<Vec<super::bar::Bar>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_functions() {
        assert!((norm_cdf(0.0) - 0.5).abs() < 1e-7);
        assert!((norm_cdf(1.96) - 0.975).abs() < 1e-4);
        assert!((norm_cdf(-1.0) - 0.158_655).abs() < 1e-5);
        for p in [0.001, 0.025, 0.3, 0.5, 0.9, 0.999] {
            assert!((norm_cdf(norm_ppf(p)) - p).abs() < 1e-6, "{p}");
        }
    }

    #[test]
    fn sharpe_statistics() {
        // PSR rises with sample size and falls with fat tails.
        assert!(psr(0.1, 1000, 0.0, 3.0, 0.0) > psr(0.1, 100, 0.0, 3.0, 0.0));
        assert!(psr(0.1, 400, -1.0, 10.0, 0.0) < psr(0.1, 400, 0.0, 3.0, 0.0));
        assert!((psr(0.0, 400, 0.0, 3.0, 0.0) - 0.5).abs() < 1e-6);
        // More trials → higher bar (≈ sqrt(2 ln N) · σ for large N).
        let a = expected_max_sharpe(10, 0.01);
        let b = expected_max_sharpe(1000, 0.01);
        assert!(b > a && a > 0.0);
        assert!((b / 0.1 - 3.2).abs() < 0.2, "{b}");
        let (m, sd, sk, ku) = moments(&[1.0, 2.0, 3.0, 4.0]);
        assert!(
            (m - 2.5).abs() < 1e-12 && (sd - 1.290_994).abs() < 1e-5 && sk.abs() < 1e-12 && (ku - 1.64).abs() < 1e-9
        );
    }

    #[test]
    fn pbo_detects_noise_and_skill() {
        let mut rng = Rng::new(5);
        let mut gauss = || {
            let (u, v) = (rng.u().max(1e-12), rng.u());
            (-2.0 * u.ln()).sqrt() * (2.0 * std::f64::consts::PI * v).cos()
        };
        let block = |mean: f64, g: &mut dyn FnMut() -> f64| {
            let xs: Vec<f64> = (0..50).map(|_| mean + g()).collect();
            (50.0, xs.iter().sum::<f64>(), xs.iter().map(|x| x * x).sum::<f64>())
        };
        // 30 useless configs: the in-sample winner is a coin flip out of sample.
        let noise: Vec<Vec<_>> = (0..30)
            .map(|_| (0..10).map(|_| block(0.0, &mut gauss)).collect())
            .collect();
        let (p_noise, _, combos) = pbo(&noise);
        assert_eq!(combos, 252);
        assert!((0.3..0.7).contains(&p_noise), "{p_noise}");
        // One config with a real edge: it wins in and out of sample.
        let mut skill = noise.clone();
        skill[0] = (0..10).map(|_| block(0.5, &mut gauss)).collect();
        let (p_skill, loss, _) = pbo(&skill);
        assert!(p_skill < 0.05 && loss < 0.05, "{p_skill} {loss}");
    }

    #[test]
    fn proportion_test() {
        assert!(prop_test(700, 1000, 500, 1000) < 1e-6);
        assert!(prop_test(500, 1000, 500, 1000) > 0.4);
    }
}
