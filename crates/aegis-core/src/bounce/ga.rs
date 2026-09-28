//! Genetic algorithm for the strategy settings, run walk-forward.
//!
//! Operators follow Deb's real-coded GA: tournament selection, simulated binary crossover
//! (SBX, η=15) and polynomial mutation (η=20) on bounded genes, uniform crossover and bit-flip
//! for toggles, elitism and random immigrants against premature convergence.
//!
//! Overfitting controls:
//! - each window is split into train (first 75%) and validation (last 25%); the GA only sees
//!   train, early stopping and the final choice use validation;
//! - the final choice among the best distinct solutions weighs validation fitness and the mean
//!   fitness of perturbed neighbours ("plateau, not peak");
//! - the reported result is the walk-forward test windows the GA never saw;
//! - a random search with the same evaluation budget runs next to the GA as a baseline, and all
//!   trials are kept for the Deflated Sharpe Ratio and PBO (see `validate`).

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use super::{
    engine::{stats, Engine, Quick, Stats, Trade},
    features::{index, FEATURES},
    params::{param_specs, Params},
};

/// One tunable value. Toggles are 0/1 genes.
#[derive(Clone, Debug, Serialize)]
pub struct Gene {
    pub id: String,
    pub lo: f64,
    pub hi: f64,
    pub step: f64,
    pub boolean: bool,
}

impl Gene {
    fn snap(&self, v: f64) -> f64 {
        if self.boolean {
            return if v >= 0.5 { 1.0 } else { 0.0 };
        }
        let v = v.clamp(self.lo, self.hi);
        if self.step > 0.0 {
            (self.lo + ((v - self.lo) / self.step).round() * self.step).clamp(self.lo, self.hi)
        } else {
            v
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct GaSpec {
    pub population: usize,
    pub generations: usize,
    /// Stop when validation fitness has not improved for this many generations.
    pub patience: usize,
    pub seed: u64,
    pub target_win_rate: f64,
    pub target_trades_per_day: f64,
    pub min_trades: usize,
    /// Fitness cost per active metric filter.
    pub complexity: f64,
    pub train_days: i64,
    pub test_days: i64,
    /// Metric filters the GA may switch on and set (ids from `FEATURES`).
    pub metrics: Vec<String>,
    /// Other settings the GA may change (spec ids). Empty = every tunable non-filter setting.
    pub tune: Vec<String>,
}

impl Default for GaSpec {
    fn default() -> Self {
        GaSpec {
            population: 48,
            generations: 30,
            patience: 8,
            seed: 7,
            target_win_rate: 0.7,
            target_trades_per_day: 20.0,
            min_trades: 30,
            complexity: 0.05,
            train_days: 60,
            test_days: 30,
            metrics: Vec::new(),
            tune: Vec::new(),
        }
    }
}

/// Metrics the GA may filter on by default: the strongest in the research report plus news.
pub const DEFAULT_METRICS: [&str; 14] = [
    "atr_usd",
    "atr_pct",
    "adr_used",
    "weekday",
    "hour",
    "trend_1h",
    "room_atr",
    "level_age_h",
    "confluence",
    "appr_speed",
    "appr_delta",
    "rsi_stretch",
    "news_before_min",
    "news_after_min",
];

pub fn genes(spec: &GaSpec) -> Vec<Gene> {
    let specs = param_specs();
    let mut g = Vec::new();
    for s in specs.iter().filter(|s| s.tunable && s.kind != "filter") {
        if !spec.tune.is_empty() && !spec.tune.contains(&s.id) {
            continue;
        }
        // Narrower ranges than the sliders where the full range is never sensible.
        let (lo, hi) = match s.id.as_str() {
            "min_prob" => (0.3, 0.9),
            "tp_r" => (0.3, 2.0),
            "sl_atr" => (0.1, 1.5),
            "max_bars" => (6.0, 48.0),
            "max_open" => (1.0, 8.0),
            _ => (s.lo, s.hi),
        };
        g.push(Gene {
            id: s.id.clone(),
            lo,
            hi,
            step: s.step.max(if s.id == "max_bars" { 3.0 } else { 0.0 }),
            boolean: s.kind == "toggle",
        });
    }
    let metrics: Vec<String> = if spec.metrics.is_empty() {
        DEFAULT_METRICS.iter().map(|s| s.to_string()).collect()
    } else {
        spec.metrics.clone()
    };
    for m in &metrics {
        let Some(k) = index(m) else { continue };
        let f = &FEATURES[k];
        g.push(Gene {
            id: format!("filters.{m}.on"),
            lo: 0.0,
            hi: 1.0,
            step: 1.0,
            boolean: true,
        });
        for part in ["min", "max"] {
            g.push(Gene {
                id: format!("filters.{m}.{part}"),
                lo: f.lo,
                hi: f.hi,
                step: f.step,
                boolean: false,
            });
        }
    }
    g
}

pub fn apply(base: &Params, genes: &[Gene], x: &[f64]) -> Params {
    let mut p = base.clone();
    for (g, v) in genes.iter().zip(x) {
        p.set(&g.id, *v);
    }
    for f in p.filters.values_mut() {
        if f.min > f.max {
            std::mem::swap(&mut f.min, &mut f.max);
        }
    }
    p
}

fn encode(base: &Params, genes: &[Gene]) -> Vec<f64> {
    genes.iter().map(|g| g.snap(base.get(&g.id).unwrap_or(g.lo))).collect()
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Objective {
    pub target_win_rate: f64,
    pub target_trades_per_day: f64,
    pub min_trades: usize,
    pub complexity: f64,
}

/// Fitness: the t-statistic of the mean R (rewards edge and sample size together), minus
/// penalties for missing the win-rate and trades-per-day targets and for every active filter.
pub fn fitness(q: &Quick, active_filters: usize, o: &Objective) -> f64 {
    if q.n < o.min_trades {
        return -10.0 + q.n as f64 / o.min_trades.max(1) as f64;
    }
    let sd = q.std().max(0.05);
    let t = q.mean() / sd * (q.n as f64).sqrt();
    let wr_short = (o.target_win_rate - q.win_rate()).max(0.0);
    let tpd_short = if o.target_trades_per_day > 0.0 {
        (1.0 - q.per_day() / o.target_trades_per_day).max(0.0)
    } else {
        0.0
    };
    t - 40.0 * wr_short - 4.0 * tpd_short - o.complexity * active_filters as f64
}

/// xorshift64*: small, fast and deterministic for a seed.
#[derive(Clone)]
pub struct Rng(u64);
impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    pub fn u(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }
    pub fn below(&mut self, n: usize) -> usize {
        ((self.u() * n as f64) as usize).min(n.saturating_sub(1))
    }
}

fn random_genome(genes: &[Gene], base: &[f64], rng: &mut Rng) -> Vec<f64> {
    genes
        .iter()
        .zip(base)
        .map(|(g, &b)| {
            if g.boolean {
                // Keep toggles mostly as the user set them; filters start mostly off.
                if rng.u() < 0.75 {
                    b
                } else {
                    1.0 - b
                }
            } else {
                g.snap(g.lo + rng.u() * (g.hi - g.lo))
            }
        })
        .collect()
}

/// Simulated binary crossover for one bounded variable (Deb & Agrawal 1995).
fn sbx(a: f64, b: f64, lo: f64, hi: f64, eta: f64, rng: &mut Rng) -> (f64, f64) {
    if (a - b).abs() < 1e-12 || hi <= lo {
        return (a, b);
    }
    let u = rng.u();
    let beta = if u <= 0.5 {
        (2.0 * u).powf(1.0 / (eta + 1.0))
    } else {
        (1.0 / (2.0 * (1.0 - u))).powf(1.0 / (eta + 1.0))
    };
    let c1 = 0.5 * ((1.0 + beta) * a + (1.0 - beta) * b);
    let c2 = 0.5 * ((1.0 - beta) * a + (1.0 + beta) * b);
    (c1.clamp(lo, hi), c2.clamp(lo, hi))
}

/// Polynomial mutation for one bounded variable (Deb & Goyal 1996).
fn poly(x: f64, lo: f64, hi: f64, eta: f64, rng: &mut Rng) -> f64 {
    if hi <= lo {
        return x;
    }
    let u = rng.u();
    let delta = if u < 0.5 {
        (2.0 * u).powf(1.0 / (eta + 1.0)) - 1.0
    } else {
        1.0 - (2.0 * (1.0 - u)).powf(1.0 / (eta + 1.0))
    };
    (x + delta * (hi - lo)).clamp(lo, hi)
}

fn mutate(genes: &[Gene], x: &mut [f64], rate: f64, eta: f64, rng: &mut Rng) {
    for (g, v) in genes.iter().zip(x.iter_mut()) {
        if rng.u() < rate {
            *v = if g.boolean {
                1.0 - *v
            } else {
                g.snap(poly(*v, g.lo, g.hi, eta, rng))
            };
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct GenStat {
    pub generation: usize,
    pub best: f64,
    pub mean: f64,
    /// Validation fitness of the generation's best individual.
    pub validation: f64,
    /// Distinct genomes in the population (diversity).
    pub distinct: usize,
}

/// One evaluated setting: kept for the Deflated Sharpe Ratio and PBO.
#[derive(Clone, Debug, Serialize)]
pub struct Trial {
    pub fitness: f64,
    /// Daily Sharpe on the train window (all trials share its days).
    pub sharpe: f64,
    pub trades: usize,
}

pub struct GaRun {
    pub best: Params,
    pub best_genome: Vec<f64>,
    pub train_fitness: f64,
    pub validation_fitness: f64,
    pub history: Vec<GenStat>,
    pub trials: Vec<Trial>,
    /// Distinct genomes of the last population (for PBO).
    pub final_population: Vec<Vec<f64>>,
    pub evaluations: usize,
}

pub struct Window {
    pub train: (usize, usize),
    pub validation: (usize, usize),
}

pub fn eval(
    engine: &Engine,
    base: &Params,
    genes: &[Gene],
    x: &[f64],
    range: (usize, usize),
    o: &Objective,
) -> (f64, Quick) {
    let p = apply(base, genes, x);
    let q = engine.quick(&p, range.0, range.1);
    (fitness(&q, p.active_filters(), o), q)
}

fn tournament(fit: &[f64], rng: &mut Rng) -> usize {
    let mut best = rng.below(fit.len());
    for _ in 0..2 {
        let c = rng.below(fit.len());
        if fit[c] > fit[best] {
            best = c;
        }
    }
    best
}

#[allow(clippy::too_many_arguments)]
pub fn run_ga(
    engine: &Engine,
    base: &Params,
    genes: &[Gene],
    o: &Objective,
    spec: &GaSpec,
    w: &Window,
    rng: &mut Rng,
    progress: &(dyn Fn(usize, f64) + Sync),
) -> GaRun {
    let n = spec.population.max(8);
    let rate = 1.0 / genes.len().max(1) as f64;
    let start = encode(base, genes);
    let mut pop: Vec<Vec<f64>> = std::iter::once(start.clone())
        .chain((1..n).map(|_| random_genome(genes, &start, rng)))
        .collect();
    let mut trials = Vec::new();
    let mut history = Vec::new();
    let mut hall: Vec<(f64, Vec<f64>)> = Vec::new();
    let mut evaluations = 0usize;
    let (mut best_val, mut stale) = (f64::NEG_INFINITY, 0usize);
    for gen in 0..spec.generations.max(1) {
        let scored: Vec<(f64, Quick)> = pop
            .par_iter()
            .map(|x| eval(engine, base, genes, x, w.train, o))
            .collect();
        evaluations += scored.len();
        let fit: Vec<f64> = scored.iter().map(|s| s.0).collect();
        for (x, (f, q)) in pop.iter().zip(&scored) {
            trials.push(Trial {
                fitness: *f,
                sharpe: q.daily_sharpe(),
                trades: q.n,
            });
            if !hall.iter().any(|h| h.1 == *x) {
                hall.push((*f, x.clone()));
            }
        }
        hall.sort_by(|a, b| b.0.total_cmp(&a.0));
        hall.truncate(12);
        let bi = (0..n).max_by(|&a, &b| fit[a].total_cmp(&fit[b])).unwrap_or(0);
        let (vf, _) = eval(engine, base, genes, &pop[bi], w.validation, o);
        evaluations += 1;
        let mut distinct = pop.clone();
        distinct.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        distinct.dedup();
        history.push(GenStat {
            generation: gen,
            best: fit[bi],
            mean: fit.iter().sum::<f64>() / n as f64,
            validation: vf,
            distinct: distinct.len(),
        });
        progress(gen, fit[bi]);
        if vf > best_val + 1e-9 {
            best_val = vf;
            stale = 0;
        } else {
            stale += 1;
            if stale >= spec.patience.max(1) {
                break;
            }
        }
        if gen + 1 == spec.generations.max(1) {
            break;
        }
        // Next generation: 2 elites, children of tournament parents, 10% random immigrants.
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by(|&a, &b| fit[b].total_cmp(&fit[a]));
        let mut next: Vec<Vec<f64>> = order.iter().take(2).map(|&i| pop[i].clone()).collect();
        let immigrants = n / 10;
        while next.len() < n - immigrants {
            let (a, b) = (tournament(&fit, rng), tournament(&fit, rng));
            let (mut c1, mut c2) = (pop[a].clone(), pop[b].clone());
            if rng.u() < 0.9 {
                for (k, g) in genes.iter().enumerate() {
                    if rng.u() < 0.5 {
                        if g.boolean {
                            std::mem::swap(&mut c1[k], &mut c2[k]);
                        } else {
                            let (x, y) = sbx(c1[k], c2[k], g.lo, g.hi, 15.0, rng);
                            (c1[k], c2[k]) = (g.snap(x), g.snap(y));
                        }
                    }
                }
            }
            mutate(genes, &mut c1, rate, 20.0, rng);
            mutate(genes, &mut c2, rate, 20.0, rng);
            next.push(c1);
            if next.len() < n - immigrants {
                next.push(c2);
            }
        }
        while next.len() < n {
            next.push(random_genome(genes, &start, rng));
        }
        pop = next;
    }
    // Final choice: validation fitness and neighbourhood robustness of the best distinct genomes.
    let mut best = (f64::NEG_INFINITY, start.clone(), f64::NEG_INFINITY, f64::NEG_INFINITY);
    for (train_f, x) in &hall {
        let (vf, _) = eval(engine, base, genes, x, w.validation, o);
        let neighbours: Vec<Vec<f64>> = (0..6)
            .map(|_| {
                let mut y = x.clone();
                mutate(genes, &mut y, 0.3, 40.0, rng);
                y
            })
            .collect();
        let robust = neighbours
            .par_iter()
            .map(|y| eval(engine, base, genes, y, w.train, o).0)
            .sum::<f64>()
            / neighbours.len() as f64;
        evaluations += 7;
        let score = 0.5 * vf + 0.5 * robust.min(*train_f);
        if score > best.0 {
            best = (score, x.clone(), *train_f, vf);
        }
    }
    let mut final_population = pop;
    final_population.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    final_population.dedup();
    GaRun {
        best: apply(base, genes, &best.1),
        best_genome: best.1,
        train_fitness: best.2,
        validation_fitness: best.3,
        history,
        trials,
        final_population,
        evaluations,
    }
}

/// Random search with the same budget: the baseline the GA must beat on validation.
pub fn random_search(
    engine: &Engine,
    base: &Params,
    genes: &[Gene],
    o: &Objective,
    w: &Window,
    budget: usize,
    rng: &mut Rng,
) -> (f64, f64, Params) {
    let start = encode(base, genes);
    let pool: Vec<Vec<f64>> = (0..budget.max(1)).map(|_| random_genome(genes, &start, rng)).collect();
    let scored: Vec<f64> = pool
        .par_iter()
        .map(|x| eval(engine, base, genes, x, w.train, o).0)
        .collect();
    let bi = (0..pool.len())
        .max_by(|&a, &b| scored[a].total_cmp(&scored[b]))
        .unwrap_or(0);
    (
        scored[bi],
        eval(engine, base, genes, &pool[bi], w.validation, o).0,
        apply(base, genes, &pool[bi]),
    )
}

#[derive(Clone, Debug, Serialize)]
pub struct WindowResult {
    pub train_from: i64,
    pub test_from: i64,
    pub test_to: i64,
    pub train: Stats,
    pub test: Stats,
    pub generations: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct Baseline {
    pub budget: usize,
    pub ga_train: f64,
    pub ga_validation: f64,
    pub random_train: f64,
    pub random_validation: f64,
    /// Walk-forward test windows traded with the random search's pick (same budget per window).
    pub random_out_of_sample: Stats,
}

#[derive(Clone, Debug, Serialize)]
pub struct OptimizeReport {
    /// Settings from the GA on the most recent window: the ones to trade now.
    pub params: Params,
    /// Every test window traded with settings the GA chose before it: the honest result.
    pub out_of_sample: Stats,
    pub oos_trades: Vec<Trade>,
    pub windows: Vec<WindowResult>,
    /// Convergence of the most recent GA run.
    pub convergence: Vec<GenStat>,
    pub baseline: Baseline,
    pub genes: Vec<Gene>,
    pub evaluations: usize,
    pub objective: Objective,
    /// All trials of the most recent GA run (Sharpe per trade on its train window).
    pub trials: Vec<Trial>,
    /// Distinct settings of the final population (for PBO).
    #[serde(skip)]
    pub final_population: Vec<Params>,
    /// The recent window the final settings were chosen on (bar indices).
    pub recent: (usize, usize),
}

#[derive(Clone, Debug, Serialize)]
pub struct Progress {
    pub stage: String,
    pub window: usize,
    pub windows: usize,
    pub generation: usize,
    pub generations: usize,
    pub best: f64,
}

fn split(a: usize, b: usize) -> Window {
    let m = a + (b - a) * 3 / 4;
    Window {
        train: (a, m),
        validation: (m, b),
    }
}

pub fn optimize(engine: &Engine, base: &Params, spec: &GaSpec, progress: &(dyn Fn(Progress) + Sync)) -> OptimizeReport {
    let genes = genes(spec);
    let o = Objective {
        target_win_rate: spec.target_win_rate,
        target_trades_per_day: spec.target_trades_per_day,
        min_trades: spec.min_trades,
        complexity: spec.complexity,
    };
    let bars = &engine.bars;
    let day = 86_400;
    // Training can only start where the score model has probabilities.
    let first = bars
        .first()
        .map_or(0, |b| b.time)
        .max(if base.use_model { engine.scored_from() } else { 0 });
    let last = bars.last().map_or(0, |b| b.time + 300);
    let mut plan = Vec::new();
    let mut test_from = first + spec.train_days * day;
    while test_from + day < last {
        let test_to = (test_from + spec.test_days * day).min(last);
        plan.push((test_from - spec.train_days * day, test_from, test_to));
        test_from = test_to;
    }
    let total = plan.len() + 1;
    let mut rng = Rng::new(spec.seed);
    let mut windows = Vec::new();
    let mut oos: Vec<Trade> = Vec::new();
    let mut random_oos: Vec<Trade> = Vec::new();
    let mut evaluations = 0;
    for (k, &(train_from, test_from, test_to)) in plan.iter().enumerate() {
        let (a, b, c) = (
            engine.index_at(train_from),
            engine.index_at(test_from),
            engine.index_at(test_to),
        );
        if b <= a + 500 || c <= b {
            continue;
        }
        let run = run_ga(engine, base, &genes, &o, spec, &split(a, b), &mut rng, &|g, best| {
            progress(Progress {
                stage: "walk-forward".into(),
                window: k + 1,
                windows: total,
                generation: g + 1,
                generations: spec.generations,
                best,
            })
        });
        evaluations += run.evaluations;
        let (_, _, rp) = random_search(engine, base, &genes, &o, &split(a, b), run.evaluations, &mut rng);
        random_oos.extend(engine.trades(&rp, b, c));
        let train_trades = engine.trades(&run.best, a, b);
        let test_trades = engine.trades(&run.best, b, c);
        windows.push(WindowResult {
            train_from,
            test_from,
            test_to,
            train: stats(&train_trades, train_from, test_from),
            test: stats(&test_trades, test_from, test_to),
            generations: run.history.len(),
        });
        oos.extend(test_trades);
    }
    // Settings for now: the GA on the most recent train window.
    let (a, z) = (engine.index_at(last - spec.train_days * day), bars.len());
    let w = split(a, z);
    let run = run_ga(engine, base, &genes, &o, spec, &w, &mut rng, &|g, best| {
        progress(Progress {
            stage: "current settings".into(),
            window: total,
            windows: total,
            generation: g + 1,
            generations: spec.generations,
            best,
        })
    });
    progress(Progress {
        stage: "random-search baseline".into(),
        window: total,
        windows: total,
        generation: spec.generations,
        generations: spec.generations,
        best: run.train_fitness,
    });
    let (rt, rv, _) = random_search(engine, base, &genes, &o, &w, run.evaluations, &mut rng);
    let oos_from = windows.first().map_or(first, |w| w.test_from);
    let oos_to = windows.last().map_or(last, |w| w.test_to);
    OptimizeReport {
        out_of_sample: stats(&oos, oos_from, oos_to),
        oos_trades: oos,
        windows,
        convergence: run.history.clone(),
        baseline: Baseline {
            budget: run.evaluations,
            ga_train: run.train_fitness,
            ga_validation: run.validation_fitness,
            random_train: rt,
            random_validation: rv,
            random_out_of_sample: stats(&random_oos, oos_from, oos_to),
        },
        genes: genes.clone(),
        evaluations: evaluations + run.evaluations,
        objective: o,
        trials: run.trials.clone(),
        final_population: run.final_population.iter().map(|x| apply(base, &genes, x)).collect(),
        params: run.best,
        recent: (a, z),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The GA machinery on a known landscape: it must find the planted optimum of a bumpy
    /// function far better than random search with the same budget.
    #[test]
    fn operators_find_a_planted_optimum() {
        let genes: Vec<Gene> = (0..8)
            .map(|i| Gene {
                id: format!("g{i}"),
                lo: 0.0,
                hi: 10.0,
                step: 0.0,
                boolean: false,
            })
            .collect();
        let target = [3.0, 7.0, 1.0, 9.0, 5.0, 2.0, 8.0, 4.0];
        let f = |x: &[f64]| -> f64 {
            -x.iter()
                .zip(target)
                .map(|(a, b)| (a - b).powi(2) - 2.0 * (3.0 * (a - b)).cos())
                .sum::<f64>()
        };
        let mut rng = Rng::new(1);
        let n = 40;
        let mut pop: Vec<Vec<f64>> = (0..n).map(|_| random_genome(&genes, &[0.0; 8], &mut rng)).collect();
        let mut evals = 0;
        for _ in 0..60 {
            let fit: Vec<f64> = pop.iter().map(|x| f(x)).collect();
            evals += n;
            let mut order: Vec<usize> = (0..n).collect();
            order.sort_by(|&a, &b| fit[b].total_cmp(&fit[a]));
            let mut next: Vec<Vec<f64>> = order.iter().take(2).map(|&i| pop[i].clone()).collect();
            while next.len() < n {
                let (a, b) = (tournament(&fit, &mut rng), tournament(&fit, &mut rng));
                let (mut c1, mut c2) = (pop[a].clone(), pop[b].clone());
                for (k, g) in genes.iter().enumerate() {
                    if rng.u() < 0.5 {
                        (c1[k], c2[k]) = sbx(c1[k], c2[k], g.lo, g.hi, 15.0, &mut rng);
                    }
                }
                mutate(&genes, &mut c1, 1.0 / 8.0, 20.0, &mut rng);
                mutate(&genes, &mut c2, 1.0 / 8.0, 20.0, &mut rng);
                next.push(c1);
                next.push(c2);
            }
            next.truncate(n);
            pop = next;
        }
        let ga_best = pop.iter().map(|x| f(x)).fold(f64::NEG_INFINITY, f64::max);
        let rs_best = (0..evals)
            .map(|_| f(&random_genome(&genes, &[0.0; 8], &mut rng)))
            .fold(f64::NEG_INFINITY, f64::max);
        assert!(ga_best > -2.0 * 8.0 + 1.0, "GA did not converge: {ga_best}");
        assert!(ga_best > rs_best + 5.0, "GA {ga_best} vs random {rs_best}");
    }

    #[test]
    fn operators_respect_bounds_and_steps() {
        let g = Gene {
            id: "x".into(),
            lo: 0.2,
            hi: 1.4,
            step: 0.1,
            boolean: false,
        };
        let mut rng = Rng::new(3);
        for _ in 0..1000 {
            let (a, b) = sbx(0.3, 1.3, g.lo, g.hi, 15.0, &mut rng);
            let m = poly(a, g.lo, g.hi, 20.0, &mut rng);
            for v in [a, b, m] {
                assert!((g.lo..=g.hi).contains(&v));
            }
            let s = g.snap(m);
            assert!((((s - g.lo) / g.step).round() * g.step + g.lo - s).abs() < 1e-9);
        }
    }

    #[test]
    fn fitness_prefers_edge_and_targets() {
        let o = Objective {
            target_win_rate: 0.7,
            target_trades_per_day: 20.0,
            min_trades: 30,
            complexity: 0.05,
        };
        let q = |n: usize, wins: usize, days: f64| Quick {
            n,
            wins,
            sum: wins as f64 * 1.0 - (n - wins) as f64 * 1.2,
            sumsq: wins as f64 + (n - wins) as f64 * 1.44,
            days,
            ..Quick::default()
        };
        let good = fitness(&q(400, 300, 20.0), 0, &o);
        let low_wr = fitness(&q(400, 240, 20.0), 0, &o);
        let few = fitness(&q(40, 30, 20.0), 0, &o);
        assert!(good > low_wr && good > few);
        assert!(fitness(&q(400, 300, 20.0), 5, &o) < good);
        assert!(fitness(&q(10, 10, 20.0), 0, &o) < few);
    }
}
