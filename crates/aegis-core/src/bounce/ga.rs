//! Mixed-variable multiobjective AMALGAM-style ensemble (not an exact or validated
//! reproduction of Vrugt's AMALGAM). The legacy `run_ga` name remains for callers.
//! NSGA-II SBX/mutation, differential evolution, persistent particle swarm state,
//! and adaptive diagonal Gaussian/Metropolis search compete for offspring slots.
//! Evolution and survivor-contribution adaptation use TRAIN ONLY, for a fixed
//! candidate budget. A bounded nondominated train archive supplies finalists;
//! their final ordering uses validation fitness ONLY. Test is reported afterwards.
//! Objectives are in net R units, not financial/account returns. Random search
//! uses the identical train objectives, archive/finalist policy and engine-call
//! budget. Duplicate evaluations count towards budget (finite spaces can exhaust).

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
    /// Legacy compatibility field; ignored: evolution always exhausts its fixed budget.
    pub patience: usize,
    pub seed: u64,
    pub target_win_rate: f64,
    pub target_trades_per_day: f64,
    pub min_trades: usize,
    /// Fitness cost per active metric filter.
    pub complexity: f64,
    pub train_days: i64,
    pub test_days: i64,
    /// Metric filters to tune (ids from `FEATURES`). Empty = legacy default 14.
    /// `["none"]` explicitly disables filter genes; non-filter `tune` still applies.
    pub metrics: Vec<String>,
    /// Other settings the GA may change (spec ids). Empty = every tunable non-filter setting.
    pub tune: Vec<String>,
}

// Bound externally supplied work before allocating a population or multiplying budgets.
// Defaults are unchanged; excessive requests are normalized, not silently allowed to hang.
const MAX_POPULATION: usize = 256;
const MAX_GENERATIONS: usize = 256;
const MAX_DAYS: i64 = 36_500;

fn bounded_spec(spec: &GaSpec) -> GaSpec {
    let mut s = spec.clone();
    s.population = s.population.clamp(8, MAX_POPULATION);
    s.generations = s.generations.clamp(1, MAX_GENERATIONS);
    s.train_days = s.train_days.clamp(2, MAX_DAYS);
    s.test_days = s.test_days.clamp(1, MAX_DAYS);
    s
}

impl Default for GaSpec {
    fn default() -> Self {
        GaSpec {
            population: 48,
            generations: 30,
            patience: 8,
            seed: 7,
            target_win_rate: 0.0,
            target_trades_per_day: 0.0,
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
        // Never outside the slider: the window could not show or keep the value.
        let (lo, hi) = (lo.max(s.lo), hi.min(s.hi));
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
    pub pareto: Vec<ParetoPoint>,
    pub operator_stats: Vec<OperatorStat>,
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

/// Scale the final validation trade minimum to the held-out slice length.
/// This is never consulted during generation evolution.
fn validation_objective(o: &Objective, w: &Window) -> Objective {
    let train = (w.train.1 - w.train.0).max(1) as f64;
    let val = (w.validation.1 - w.validation.0) as f64;
    let scaled = (o.min_trades as f64 * val / train).round() as usize;
    Objective {
        min_trades: scaled.max(5).min(o.min_trades),
        ..*o
    }
}

#[cfg(test)]
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

const OPERATORS: usize = 4;
const FINALISTS: usize = 12;
// Full quick + three disjoint train subwindows + full train trades for downside/DD.
const TRAIN_CALLS: usize = 5;

#[derive(Clone, Debug, Default, Serialize)]
pub struct OperatorStat {
    pub name: String,
    pub proposed: usize,
    pub survived: usize,
    pub allocation: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct ParetoPoint {
    pub genome: Vec<f64>,
    /// Maximize: net R/day, negative downside RMS R, negative max drawdown R,
    /// and worst net R/day of three disjoint train subwindows, less complexity.
    pub objectives: [f64; 4],
    pub constraint_violation: f64,
}

#[derive(Clone)]
struct Individual {
    x: Vec<f64>,
    objectives: [f64; 4],
    violation: f64,
    fit: f64,
    sharpe: f64,
    trades: usize,
    velocity: Vec<f64>,
    personal: Vec<f64>,
    personal_objectives: [f64; 4],
    personal_violation: f64,
    origin: Option<usize>,
}

/// Canonical phenotype: bounds/snapping, inactive filter dimensions erased,
/// and swapped filter endpoints normalized before deduplication.
fn canonical(genes: &[Gene], x: &mut [f64]) {
    for (g, v) in genes.iter().zip(x.iter_mut()) {
        *v = g.snap(if v.is_finite() { *v } else { g.lo });
    }
    for (i, g) in genes.iter().enumerate() {
        if let Some(prefix) = g.id.strip_suffix(".on") {
            if let (Some(a), Some(b)) = (
                genes.iter().position(|h| h.id == format!("{prefix}.min")),
                genes.iter().position(|h| h.id == format!("{prefix}.max")),
            ) {
                if x[i] < 0.5 {
                    x[a] = genes[a].snap(genes[a].lo);
                    x[b] = genes[b].snap(genes[b].hi);
                } else if x[a] > x[b] {
                    x.swap(a, b);
                }
            }
        }
    }
}

fn dominates(a: &[f64; 4], av: f64, b: &[f64; 4], bv: f64) -> bool {
    if av != bv {
        return av < bv;
    }
    a.iter().zip(b).all(|(x, y)| x >= y) && a.iter().zip(b).any(|(x, y)| x > y)
}

fn ranks_crowding(pop: &[Individual]) -> (Vec<usize>, Vec<f64>) {
    let n = pop.len();
    let mut rank = vec![usize::MAX; n];
    let mut crowd = vec![0.0; n];
    let mut counts = vec![0usize; n];
    let mut beaten = vec![Vec::new(); n];
    for i in 0..n {
        for j in 0..n {
            if dominates(
                &pop[i].objectives,
                pop[i].violation,
                &pop[j].objectives,
                pop[j].violation,
            ) {
                beaten[i].push(j);
            } else if dominates(
                &pop[j].objectives,
                pop[j].violation,
                &pop[i].objectives,
                pop[i].violation,
            ) {
                counts[i] += 1;
            }
        }
    }
    let mut front: Vec<usize> = (0..n).filter(|&i| counts[i] == 0).collect();
    let mut r = 0;
    while !front.is_empty() {
        for &i in &front {
            rank[i] = r;
        }
        if front.len() <= 2 {
            for &i in &front {
                crowd[i] = f64::INFINITY;
            }
        } else {
            for k in 0..4 {
                front.sort_by(|&i, &j| pop[i].objectives[k].total_cmp(&pop[j].objectives[k]).then(i.cmp(&j)));
                let lo = pop[front[0]].objectives[k];
                let hi = pop[*front.last().unwrap()].objectives[k];
                if hi > lo {
                    crowd[front[0]] = f64::INFINITY;
                    crowd[*front.last().unwrap()] = f64::INFINITY;
                    for t in 1..front.len() - 1 {
                        crowd[front[t]] +=
                            (pop[front[t + 1]].objectives[k] - pop[front[t - 1]].objectives[k]) / (hi - lo);
                    }
                }
            }
        }
        let mut next = Vec::new();
        for &i in &front {
            for &j in &beaten[i] {
                counts[j] -= 1;
                if counts[j] == 0 {
                    next.push(j);
                }
            }
        }
        front = next;
        r += 1;
    }
    (rank, crowd)
}

fn ordered(pop: &[Individual]) -> Vec<usize> {
    let (r, c) = ranks_crowding(pop);
    let mut ix: Vec<_> = (0..pop.len()).collect();
    ix.sort_by(|&a, &b| r[a].cmp(&r[b]).then(c[b].total_cmp(&c[a])).then(a.cmp(&b)));
    ix
}

fn survivors(pop: &[Individual], n: usize) -> Vec<Individual> {
    let mut out: Vec<Individual> = Vec::new();
    let order = ordered(pop);
    for &i in &order {
        if !out.iter().any(|p| p.x == pop[i].x) {
            out.push(pop[i].clone());
        }
        if out.len() == n {
            break;
        }
    }
    // Keep population size even in exhausted discrete/empty spaces.
    let distinct = out.len();
    while out.len() < n && distinct > 0 {
        let mut p = out[out.len() % distinct].clone();
        p.origin = None;
        out.push(p);
    }
    out
}

fn archive_update(archive: &mut Vec<Individual>, candidates: &[Individual], cap: usize) {
    let mut pool = archive.clone();
    pool.extend_from_slice(candidates);
    let (rank, _) = ranks_crowding(&pool);
    let front: Vec<_> = pool
        .into_iter()
        .enumerate()
        .filter(|(i, _)| rank[*i] == 0)
        .map(|(_, p)| p)
        .collect();
    *archive = survivors(&front, cap.min(front.len()));
    archive.dedup_by(|a, b| a.x == b.x);
    // survivors' padding is consecutive only for some orderings: retain unique phenotypes.
    let mut unique: Vec<Individual> = Vec::new();
    for p in archive.drain(..) {
        if !unique.iter().any(|q| q.x == p.x) {
            unique.push(p);
        }
    }
    *archive = unique;
}

fn allocations(n: usize, rates: &[f64; OPERATORS]) -> [usize; OPERATORS] {
    let mut out = [1; OPERATORS];
    let total: f64 = rates.iter().sum();
    for _ in OPERATORS..n {
        let k = (0..OPERATORS)
            .max_by(|&a, &b| {
                let da = rates[a] / total * n as f64 - out[a] as f64;
                let db = rates[b] / total * n as f64 - out[b] as f64;
                da.total_cmp(&db).then(b.cmp(&a))
            })
            .unwrap();
        out[k] += 1;
    }
    out
}

fn normal(rng: &mut Rng) -> f64 {
    (-2.0 * rng.u().max(1e-12).ln()).sqrt() * (std::f64::consts::TAU * rng.u()).cos()
}

fn multi_eval(engine: &Engine, base: &Params, genes: &[Gene], x: Vec<f64>, w: &Window, o: &Objective) -> Individual {
    let (fit, q) = eval(engine, base, genes, &x, w.train, o);
    let p = apply(base, genes, &x);
    let length = w.train.1.saturating_sub(w.train.0);
    let mut worst = f64::INFINITY;
    for k in 0..3 {
        let a = w.train.0 + length * k / 3;
        let b = w.train.0 + length * (k + 1) / 3;
        let sub = engine.quick(&p, a, b);
        worst = worst.min(sub.sum / sub.days.max(1.0));
    }
    let mut trades = engine.training_trades(&p, w.train.0, w.train.1);
    trades.sort_by_key(|t| t.exit_time);
    let (mut equity, mut peak, mut dd, mut downside) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for t in &trades {
        equity += t.r;
        peak = peak.max(equity);
        dd = dd.max(peak - equity);
        downside += t.r.min(0.0).powi(2);
    }
    let penalty = o.complexity * p.active_filters() as f64;
    let objectives = [
        q.sum / q.days.max(1.0) - penalty,
        -(downside / q.n.max(1) as f64).sqrt(),
        -dd,
        worst - penalty,
    ]
    .map(|v| if v.is_finite() { v } else { -1e100 });
    // Feasibility first, not a noisy scalar fitness ordering of the Pareto front.
    let violation = (o.min_trades.saturating_sub(q.n) as f64 / o.min_trades.max(1) as f64)
        + (o.target_win_rate - q.win_rate()).max(0.0)
        + if o.target_trades_per_day > 0.0 {
            (1.0 - q.per_day() / o.target_trades_per_day).max(0.0)
        } else {
            0.0
        };
    let finite = [q.sum, q.sumsq, q.days, q.dsum, q.dsumsq, fit, violation]
        .iter()
        .all(|v| v.is_finite())
        && trades.iter().all(|t| t.r.is_finite());
    let objectives = if finite { objectives } else { [-1e100; 4] };
    let violation = if finite { violation } else { 1e100 };
    let fit = if finite { fit } else { -1e100 };
    Individual {
        personal: x.clone(),
        velocity: vec![0.0; x.len()],
        x,
        objectives,
        violation,
        personal_objectives: objectives,
        personal_violation: violation,
        fit,
        sharpe: if finite { q.daily_sharpe() } else { 0.0 },
        trades: q.n,
        origin: None,
    }
}

fn tournament_multi(rank: &[usize], crowd: &[f64], rng: &mut Rng) -> usize {
    let a = rng.below(rank.len());
    let b = rng.below(rank.len());
    if rank[a] < rank[b] || (rank[a] == rank[b] && crowd[a] >= crowd[b]) {
        a
    } else {
        b
    }
}

/// All operators act on continuous latent positions followed by mixed-variable
/// projection; toggles use uniform crossover/bit flips in NSGA-II and threshold
/// projection in DE/PSO/Gaussian. Gaussian covariance adapts to the surviving
/// population (diagonal approximation), with a nonzero exploration floor.
fn proposal(
    pop: &[Individual],
    archive: &[Individual],
    genes: &[Gene],
    op: usize,
    rng: &mut Rng,
    rank: &[usize],
    crowd: &[f64],
) -> (Vec<f64>, Vec<f64>, usize) {
    let a = tournament_multi(rank, crowd, rng);
    let mut x = pop[a].x.clone();
    let mut velocity = pop[a].velocity.clone();
    match op {
        0 => {
            let b = tournament_multi(rank, crowd, rng);
            for (j, g) in genes.iter().enumerate() {
                if g.boolean {
                    if rng.u() < 0.5 {
                        x[j] = pop[b].x[j];
                    }
                } else {
                    let (c, d) = sbx(x[j], pop[b].x[j], g.lo, g.hi, 15.0, rng);
                    x[j] = if rng.u() < 0.5 { c } else { d };
                }
            }
            mutate(genes, &mut x, 1.0 / genes.len().max(1) as f64, 20.0, rng);
            velocity.fill(0.0);
        }
        1 => {
            let mut donors: Vec<_> = (0..pop.len()).filter(|&i| i != a).collect();
            for k in 0..3 {
                let j = k + rng.below(donors.len() - k);
                donors.swap(k, j);
            }
            let forced = rng.below(genes.len().max(1));
            let f = 0.5 + 0.4 * rng.u();
            for (j, value) in x.iter_mut().enumerate() {
                if j == forced || rng.u() < 0.8 {
                    *value = pop[donors[0]].x[j] + f * (pop[donors[1]].x[j] - pop[donors[2]].x[j]);
                }
            }
            velocity.fill(0.0);
        }
        2 => {
            let leader = &archive[rng.below(archive.len())];
            for (j, g) in genes.iter().enumerate() {
                let range = g.hi - g.lo;
                velocity[j] = (0.7 * velocity[j]
                    + 1.5 * rng.u() * (pop[a].personal[j] - x[j])
                    + 1.5 * rng.u() * (leader.x[j] - x[j]))
                    .clamp(-0.5 * range, 0.5 * range);
                x[j] += velocity[j];
            }
        }
        _ => {
            for (j, g) in genes.iter().enumerate() {
                let mean = pop.iter().map(|p| p.x[j]).sum::<f64>() / pop.len() as f64;
                let var = pop.iter().map(|p| (p.x[j] - mean).powi(2)).sum::<f64>() / pop.len() as f64;
                let sigma = (0.5 * var.sqrt()).max(0.02 * (g.hi - g.lo));
                x[j] += sigma * normal(rng);
                if g.boolean && rng.u() < 0.1 {
                    x[j] = 1.0 - pop[a].x[j];
                }
            }
            velocity.fill(0.0);
        }
    }
    canonical(genes, &mut x);
    (x, velocity, a)
}

fn metropolis_accept(
    child: &Individual,
    parent: &Individual,
    pop: &[Individual],
    temperature: f64,
    rng: &mut Rng,
) -> bool {
    if child.violation != parent.violation {
        return child.violation < parent.violation
            || rng.u() < ((parent.violation - child.violation) / temperature).exp();
    }
    if dominates(&child.objectives, child.violation, &parent.objectives, parent.violation) {
        return true;
    }
    let mut delta = 0.0;
    for k in 0..4 {
        let lo = pop.iter().map(|p| p.objectives[k]).fold(f64::INFINITY, f64::min);
        let hi = pop.iter().map(|p| p.objectives[k]).fold(f64::NEG_INFINITY, f64::max);
        delta += (child.objectives[k] - parent.objectives[k]) / (hi - lo).max(1e-9);
    }
    delta >= 0.0 || rng.u() < (delta / (4.0 * temperature)).exp()
}

fn finalists(archive: &[Individual]) -> Vec<Individual> {
    ordered(archive)
        .into_iter()
        .take(FINALISTS)
        .map(|i| archive[i].clone())
        .collect()
}

/// Validation alone breaks ties among train-Pareto finalists. Always exactly
/// FINALISTS validation calls, cycling the pool when fewer distinct candidates
/// exist, so the random baseline has exactly the same total engine-call budget.
fn select_validation<F: FnMut(&[f64]) -> f64>(archive: &[Individual], mut evaluate: F) -> (Individual, f64) {
    let pool = finalists(archive);
    let mut best = pool[0].clone();
    let mut best_val = f64::NEG_INFINITY;
    for k in 0..FINALISTS {
        let p = &pool[k % pool.len()];
        let v = evaluate(&p.x);
        let v = if v.is_finite() { v } else { -1e100 };
        if v > best_val {
            best = p.clone();
            best_val = v;
        }
    }
    (best, best_val)
}

struct EnsembleRun {
    pop: Vec<Individual>,
    archive: Vec<Individual>,
    history: Vec<GenStat>,
    trials: Vec<Trial>,
    operator_stats: Vec<OperatorStat>,
}

// This is the production evolution loop, also exercised by the planted Pareto
// benchmark. The only evaluator available here is train; there is no validation
// or test range/callback and no early-stopping channel.
fn evolve<F: Fn(Vec<f64>) -> Individual + Sync>(
    genes: &[Gene],
    start: Vec<f64>,
    spec: &GaSpec,
    rng: &mut Rng,
    progress: &(dyn Fn(usize, f64) + Sync),
    evaluate: F,
) -> EnsembleRun {
    let n = spec.population.clamp(8, MAX_POPULATION);
    let generations = spec.generations.clamp(1, MAX_GENERATIONS);
    let mut initial: Vec<_> = std::iter::once(start.clone())
        .chain((1..n).map(|_| random_genome(genes, &start, rng)))
        .collect();
    for x in &mut initial {
        canonical(genes, x);
    }
    let mut pop: Vec<_> = initial.into_par_iter().map(&evaluate).collect();
    let mut trials = Vec::new();
    let mut archive = Vec::new();
    let mut history = Vec::new();
    let mut rates = [1.0; OPERATORS];
    let names = [
        "nsga2_sbx_mutation",
        "differential_evolution",
        "particle_swarm",
        "adaptive_gaussian_metropolis",
    ];
    let mut operator_stats: Vec<_> = names
        .iter()
        .map(|name| OperatorStat {
            name: (*name).into(),
            ..OperatorStat::default()
        })
        .collect();
    let mut evaluated = pop.clone();
    let mut moves = pop.clone();
    for generation in 0..generations {
        for p in &evaluated {
            trials.push(Trial {
                fitness: p.fit,
                sharpe: p.sharpe,
                trades: p.trades,
            });
        }
        archive_update(&mut archive, &evaluated, 48.max(FINALISTS));
        if generation > 0 {
            let mut pool = pop.clone();
            for p in &mut pool {
                p.origin = None;
            }
            pool.extend(moves.clone());
            pop = survivors(&pool, n);
            for op in 0..OPERATORS {
                let successes = pop.iter().filter(|p| p.origin == Some(op)).count();
                operator_stats[op].survived += successes;
                let count = operator_stats[op].allocation.max(1);
                rates[op] = 0.7 * rates[op] + 0.3 * (successes as f64 / count as f64 + 0.05);
            }
        }
        let distinct = pop
            .iter()
            .enumerate()
            .filter(|(i, p)| !pop[..*i].iter().any(|q| q.x == p.x))
            .count();
        let best = pop.iter().map(|p| p.fit).fold(f64::NEG_INFINITY, f64::max);
        history.push(GenStat {
            generation,
            best,
            mean: pop.iter().map(|p| p.fit).sum::<f64>() / n as f64,
            validation: 0.0,
            distinct,
        }); // Compatibility slot: validation unavailable during evolution.
        progress(generation, best);
        if generation + 1 == generations {
            break;
        }
        let alloc = allocations(n, &rates);
        let (rank, crowd) = ranks_crowding(&pop);
        let mut proposals = Vec::with_capacity(n);
        for op in 0..OPERATORS {
            operator_stats[op].allocation = alloc[op];
            operator_stats[op].proposed += alloc[op];
            for _ in 0..alloc[op] {
                let (x, v, a) = proposal(&pop, &archive, genes, op, rng, &rank, &crowd);
                proposals.push((x, v, a, op));
            }
        }
        evaluated = proposals.par_iter().map(|(x, _, _, _)| evaluate(x.clone())).collect();
        moves = evaluated.clone();
        for (child, (_, v, a, op)) in moves.iter_mut().zip(proposals) {
            child.origin = Some(op);
            child.velocity = v;
            if op == 2 {
                child.personal = pop[a].personal.clone();
                child.personal_objectives = pop[a].personal_objectives;
                child.personal_violation = pop[a].personal_violation;
                if dominates(
                    &child.objectives,
                    child.violation,
                    &child.personal_objectives,
                    child.personal_violation,
                ) {
                    child.personal = child.x.clone();
                    child.personal_objectives = child.objectives;
                    child.personal_violation = child.violation;
                }
            }
            if op == 3
                && !metropolis_accept(
                    child,
                    &pop[a],
                    &pop,
                    (1.0 - (generation as f64 / generations as f64)).max(0.05),
                    rng,
                )
            {
                // Evaluated trials still enter the train archive, but a rejected
                // Metropolis move does not replace its parent in the population.
                *child = pop[a].clone();
                child.origin = None;
            }
        }
    }
    EnsembleRun {
        pop,
        archive,
        history,
        trials,
        operator_stats,
    }
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
    let EnsembleRun {
        pop,
        archive,
        history,
        trials,
        operator_stats,
    } = evolve(genes, encode(base, genes), spec, rng, progress, |x| {
        multi_eval(engine, base, genes, x, w, o)
    });
    let n = spec.population.clamp(8, MAX_POPULATION);
    let generations = spec.generations.clamp(1, MAX_GENERATIONS);
    let ov = validation_objective(o, w);
    let (best, validation_fitness) = select_validation(&archive, |x| eval(engine, base, genes, x, w.validation, &ov).0);
    let pareto = archive
        .iter()
        .map(|p| ParetoPoint {
            genome: p.x.clone(),
            objectives: p.objectives,
            constraint_violation: p.violation,
        })
        .collect();
    let mut final_population: Vec<Vec<f64>> = Vec::new();
    for p in &pop {
        if !final_population.contains(&p.x) {
            final_population.push(p.x.clone());
        }
    }
    GaRun {
        best: apply(base, genes, &best.x),
        best_genome: best.x,
        train_fitness: best.fit,
        validation_fitness,
        history,
        trials,
        final_population,
        evaluations: TRAIN_CALLS * n * generations + FINALISTS,
        pareto,
        operator_stats,
    }
}

/// Same train objectives, bounded Pareto archive, finalist validation-only policy
/// and total engine calls as `run_ga`. Budgets emitted by `run_ga` are exact;
/// arbitrary external budgets are rounded down to complete five-call candidates
/// plus twelve validation calls (minimum one candidate). No train-only winner.
#[allow(clippy::too_many_arguments)]
pub fn random_search(
    engine: &Engine,
    base: &Params,
    genes: &[Gene],
    o: &Objective,
    w: &Window,
    budget: usize,
    rng: &mut Rng,
) -> (f64, f64, Params) {
    let count = budget
        .saturating_sub(FINALISTS)
        .min(TRAIN_CALLS * MAX_POPULATION * MAX_GENERATIONS)
        / TRAIN_CALLS;
    let start = encode(base, genes);
    let cap = 48.max(FINALISTS);
    let mut archive = Vec::new();
    // Streaming chunks avoid unbounded Pareto quadratic work/memory.
    let mut remaining = count.max(1);
    while remaining > 0 {
        let chunk = remaining.min(cap);
        let mut xs: Vec<_> = (0..chunk).map(|_| random_genome(genes, &start, rng)).collect();
        for x in &mut xs {
            canonical(genes, x);
        }
        let candidates: Vec<_> = xs
            .into_par_iter()
            .map(|x| multi_eval(engine, base, genes, x, w, o))
            .collect();
        archive_update(&mut archive, &candidates, cap);
        remaining -= chunk;
    }
    let ov = validation_objective(o, w);
    let (best, v) = select_validation(&archive, |x| eval(engine, base, genes, x, w.validation, &ov).0);
    (best.fit, v, apply(base, genes, &best.x))
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
    /// Informational backend; legacy API names retained.
    pub backend: String,
    pub pareto: Vec<ParetoPoint>,
    pub operator_stats: Vec<OperatorStat>,
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

/// Keep each fold's execution settings with its entry-window candidates. Position
/// and daily admission are applied once, below, rather than reset at fold edges.
fn oos_candidates(engine: &Engine, p: &Params, from: usize, to: usize) -> Vec<(Trade, Params)> {
    engine
        .candidate_trades(p, from, to)
        .into_iter()
        .map(|t| (t, p.clone()))
        .collect()
}

/// A chronological admission pass, with no forced exits or future-exit purge.
/// Outcomes enter the UTC daily ledger only at their settlement timestamp. A
/// flipped chain occupies one slot until its final exit, using its original
/// candidate settings, with first and flip legs realized independently.
fn admit_oos(mut candidates: Vec<(Trade, Params)>) -> Vec<Trade> {
    // Stable sorting preserves engine touch order for simultaneous entries.
    candidates.sort_by_key(|(t, _)| t.entry_time);
    let mut admitted: Vec<Trade> = Vec::new();
    let mut open: Vec<usize> = Vec::new();
    let mut pending: Vec<(i64, f64)> = Vec::new();
    let mut ledger = std::collections::BTreeMap::<i64, f64>::new();
    for (t, p) in candidates {
        let now = t.entry_time;
        // Do not use a pending outcome (including a flip's eventual result)
        // before its exit. Same-timestamp prior settlements are now known.
        pending.retain(|&(exit, r)| {
            if exit <= now {
                *ledger.entry(exit.div_euclid(86_400)).or_default() += r;
                false
            } else {
                true
            }
        });
        open.retain(|&i| admitted[i].exit_time > now);
        let duplicate = open.iter().any(|&i| {
            let old = &admitted[i];
            let dir = if old.flip_r.is_finite() && old.flip_time <= now {
                -old.dir
            } else {
                old.dir
            };
            dir == t.dir && (old.level - t.level).abs() < 1e-9
        });
        let realized = ledger.get(&now.div_euclid(86_400)).copied().unwrap_or(0.0);
        if open.len() >= p.max_open.max(1) || duplicate || (p.day_stop_r > 0.0 && realized <= -p.day_stop_r) {
            continue;
        }
        pending.extend(super::engine::realized_legs(&t));
        open.push(admitted.len());
        admitted.push(t);
    }
    admitted
}

pub fn optimize(engine: &Engine, base: &Params, spec: &GaSpec, progress: &(dyn Fn(Progress) + Sync)) -> OptimizeReport {
    if base.on_close() != engine.close_entry {
        return optimize(&engine.for_entry_mode(base.on_close()), base, spec, progress);
    }
    let bounded = bounded_spec(spec);
    let spec = &bounded;
    // Position settings do nothing on 5m bars; tuning them would only add noise.
    let genes: Vec<Gene> = genes(spec)
        .into_iter()
        .filter(|g| base.sec_engine || !super::params::sec_only(&g.id))
        .filter(|g| base.use_model || g.id != "min_prob")
        .collect();
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
    let mut test_from = first.saturating_add(spec.train_days * day);
    while test_from.saturating_add(day) < last {
        let test_to = test_from.saturating_add(spec.test_days * day).min(last);
        plan.push((test_from - spec.train_days * day, test_from, test_to));
        test_from = test_to;
    }
    let total = plan.len() + 1;
    let mut rng = Rng::new(spec.seed);
    let mut windows = Vec::new();
    let mut candidates = Vec::new();
    let mut random_candidates = Vec::new();
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
        random_candidates.extend(oos_candidates(engine, &rp, b, c));
        let training = split(a, b).train;
        let train_trades = engine.training_trades(&run.best, training.0, training.1);
        candidates.extend(oos_candidates(engine, &run.best, b, c));
        windows.push(WindowResult {
            train_from,
            test_from,
            test_to,
            train: stats(&train_trades, bars[training.0].time, bars[training.1 - 1].time + 300),
            test: stats(&[], test_from, test_to),
            generations: run.history.len(),
        });
    }
    let oos = admit_oos(candidates);
    let random_oos = admit_oos(random_candidates);
    for window in &mut windows {
        let entered: Vec<Trade> = oos
            .iter()
            .filter(|t| t.entry_time >= window.test_from && t.entry_time < window.test_to)
            .cloned()
            .collect();
        window.test = stats(&entered, window.test_from, window.test_to);
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
        backend: "amalgam_style_ensemble".into(),
        pareto: run.pareto,
        operator_stats: run.operator_stats,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oos_trade(entry: i64, exit: i64, level: f64, r: f64) -> Trade {
        Trade {
            entry_time: entry,
            exit_time: exit,
            first_exit_time: exit,
            dir: 1,
            level,
            r,
            ..Trade::empty()
        }
    }

    fn admitted_entries(candidates: Vec<(Trade, Params)>) -> Vec<i64> {
        admit_oos(candidates).iter().map(|t| t.entry_time).collect()
    }

    #[test]
    fn fixed_params_oos_admission_is_invariant_to_every_fold_split() {
        let p = Params {
            max_open: 2,
            day_stop_r: 1.0,
            ..Params::default()
        };
        let candidates: Vec<_> = (0..120)
            .map(|i| {
                let entry = 86_350 + i * 7;
                (
                    oos_trade(entry, entry + 31, (i % 5) as f64, if i % 3 == 0 { -0.7 } else { 0.2 }),
                    p.clone(),
                )
            })
            .collect();
        let expected = admitted_entries(candidates.clone());
        assert!(!expected.is_empty());
        assert!(expected.len() < candidates.len());
        for cut in 0..=candidates.len() {
            let mut folded = candidates[..cut].to_vec();
            folded.extend_from_slice(&candidates[cut..]);
            assert_eq!(admitted_entries(folded), expected, "split {cut}");
        }
        // Collection order is irrelevant when entry timestamps are distinct.
        let mut reversed = candidates;
        reversed.reverse();
        assert_eq!(admitted_entries(reversed), expected);
    }

    #[test]
    fn oos_capacity_and_duplicate_level_carry_across_folds() {
        let p = Params {
            max_open: 1,
            day_stop_r: 0.0,
            ..Params::default()
        };
        let wider = Params {
            max_open: 2,
            ..p.clone()
        };
        let candidates = vec![
            (oos_trade(10, 100, 1.0, 1.0), p.clone()),
            (oos_trade(20, 90, 2.0, 1.0), p.clone()), // next fold: still full
            (oos_trade(30, 80, 1.0, 1.0), wider.clone()), // duplicate despite larger cap
            (oos_trade(40, 90, 2.0, 1.0), wider),     // its own cap permits entry
            (oos_trade(50, 70, 3.0, 1.0), p.clone()), // its own cap rejects
            (oos_trade(100, 110, 3.0, 1.0), p),
        ];
        assert_eq!(admitted_entries(candidates), vec![10, 40, 100]);
    }

    #[test]
    fn oos_overnight_daystop_waits_for_first_and_flip_settlements() {
        let p = Params {
            max_open: 8,
            day_stop_r: 1.0,
            ..Params::default()
        };
        let mut flip = oos_trade(86_390, 86_500, 1.0, 2.0);
        flip.first_exit_time = 86_420;
        flip.flip_time = 86_420;
        flip.flip_entry = 100.0;
        flip.flip_r = 3.5; // first leg -1.5, future winning flip must not offset it
        let candidates = vec![
            (flip, p.clone()),
            (oos_trade(86_410, 86_600, 2.0, 0.0), p.clone()),
            (oos_trade(86_430, 86_600, 3.0, 0.0), p.clone()),
            (oos_trade(86_500, 86_600, 4.0, 0.0), p.clone()),
            (oos_trade(172_810, 172_900, 5.0, 0.0), p),
        ];
        assert_eq!(admitted_entries(candidates), vec![86_390, 86_410, 86_500, 172_810]);
    }

    #[test]
    fn oos_does_not_use_unknown_future_losses_or_purge_crossing_exits() {
        let p = Params {
            max_open: 3,
            day_stop_r: 1.0,
            ..Params::default()
        };
        assert_eq!(
            admitted_entries(vec![
                (oos_trade(10, 100, 1.0, -2.0), p.clone()),
                (oos_trade(20, 200, 2.0, 0.0), p.clone()),
                (oos_trade(100, 300, 3.0, 0.0), p),
            ]),
            vec![10, 20]
        );
    }

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

    /// Every gene is a real setting, every tunable setting has a gene, and no gene can leave the
    /// slider range.
    #[test]
    fn genes_cover_tunable_settings_within_slider_bounds() {
        let specs = param_specs();
        let gs = genes(&GaSpec::default());
        for s in specs.iter().filter(|s| s.tunable && s.kind != "filter") {
            assert!(gs.iter().any(|g| g.id == s.id), "no gene for {}", s.id);
        }
        for g in &gs {
            let (lo, hi) = match specs.iter().find(|s| s.id == g.id) {
                Some(s) => (s.lo, s.hi),
                None => {
                    let (m, part) = g.id.trim_start_matches("filters.").rsplit_once('.').unwrap();
                    let f = &FEATURES[index(m).unwrap_or_else(|| panic!("unknown gene {}", g.id))];
                    if part == "on" {
                        (0.0, 1.0)
                    } else {
                        (f.lo, f.hi)
                    }
                }
            };
            assert!(
                g.lo >= lo - 1e-9 && g.hi <= hi + 1e-9 && g.lo < g.hi,
                "{}: {}..{} outside {lo}..{hi}",
                g.id,
                g.lo,
                g.hi
            );
            for v in [g.lo - 1.0, g.hi + 1.0] {
                let p = apply(&Params::default(), std::slice::from_ref(g), &[g.snap(v)]);
                let got = p.get(&g.id).unwrap();
                assert!(got >= lo - 1e-9 && got <= hi + 1e-9, "{} = {got}", g.id);
            }
        }
    }

    #[test]
    fn fitness_prefers_edge_and_targets() {
        let o = Objective {
            target_win_rate: 0.0,
            target_trades_per_day: 0.0,
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

    #[test]
    fn validation_minimum_scales_with_its_length() {
        let o = Objective {
            target_win_rate: 0.0,
            target_trades_per_day: 0.0,
            min_trades: 30,
            complexity: 0.0,
        };
        let w = split(0, 4000);
        assert_eq!(validation_objective(&o, &w).min_trades, 10);
        let small = Objective { min_trades: 10, ..o };
        assert_eq!(validation_objective(&small, &w).min_trades, 5);
    }

    fn synthetic(x: Vec<f64>) -> Individual {
        // Conflicting continuous objectives plus a discrete penalty.
        let a = -((x[0] - 0.2).powi(2) + (x[1] - 0.3).powi(2));
        let b = -((x[0] - 0.8).powi(2) + (x[1] - 0.3).powi(2));
        let obj = [a, b, -(x[1] - 0.3).abs(), -x[2]];
        let violation = x[2];
        Individual {
            personal: x.clone(),
            velocity: vec![0.0; x.len()],
            x,
            objectives: obj,
            violation,
            fit: a + b,
            sharpe: 0.0,
            trades: 100,
            personal_objectives: obj,
            personal_violation: violation,
            origin: None,
        }
    }

    #[test]
    fn pareto_domination_and_front_crowding() {
        assert!(dominates(&[2.0; 4], 0.0, &[1.0; 4], 0.0));
        assert!(!dominates(&[2.0, 0.0, 1.0, 1.0], 0.0, &[1.0; 4], 0.0));
        assert!(!dominates(&[1.0; 4], 0.0, &[1.0; 4], 0.0));
        assert!(dominates(&[-100.0; 4], 0.0, &[100.0; 4], 1.0));
        let mut pop: Vec<_> = (0..5).map(|i| synthetic(vec![i as f64 / 4.0, 0.3, 0.0])).collect();
        for (i, p) in pop.iter_mut().enumerate() {
            p.objectives = [i as f64, 4.0 - i as f64, 0.0, 0.0];
        }
        let (rank, crowd) = ranks_crowding(&pop);
        assert!(rank.iter().all(|r| *r == 0));
        assert!(crowd[0].is_infinite() && crowd[4].is_infinite());
        assert!(crowd[2].is_finite() && crowd[2] > 0.0);
        pop[2].objectives = [-1.0; 4];
        assert_eq!(ranks_crowding(&pop).0[2], 1);
    }

    #[test]
    fn adaptive_allocation_never_starves_an_operator() {
        for n in [8, 12, 48, 101] {
            for rates in [[1.0; 4], [100.0, 0.05, 0.05, 0.05], [0.05, 0.05, 100.0, 0.05]] {
                let slots = allocations(n, &rates);
                assert_eq!(slots.iter().sum::<usize>(), n);
                assert!(slots.iter().all(|s| *s >= 1));
            }
        }
        assert!(allocations(48, &[100.0, 0.05, 0.05, 0.05])[0] > 40);
    }

    #[test]
    fn mixed_projection_erases_inactive_dimensions_and_snaps() {
        let g = vec![
            Gene {
                id: "filters.x.on".into(),
                lo: 0.0,
                hi: 1.0,
                step: 1.0,
                boolean: true,
            },
            Gene {
                id: "filters.x.min".into(),
                lo: 0.0,
                hi: 10.0,
                step: 0.5,
                boolean: false,
            },
            Gene {
                id: "filters.x.max".into(),
                lo: 0.0,
                hi: 10.0,
                step: 0.5,
                boolean: false,
            },
        ];
        let mut a = vec![-10.0, 7.2, 1.1];
        let mut b = vec![0.0, -200.0, 200.0];
        canonical(&g, &mut a);
        canonical(&g, &mut b);
        assert_eq!(a, b);
        let mut c = vec![2.0, 9.2, 2.2];
        canonical(&g, &mut c);
        assert_eq!(c, vec![1.0, 2.0, 9.0]);
        let pop = vec![synthetic(a), synthetic(b)];
        let mut archive = Vec::new();
        archive_update(&mut archive, &pop, 48);
        assert_eq!(archive.len(), 1);
    }

    #[test]
    fn final_selection_reads_only_validation_not_train_scalar_or_test() {
        let mut a = synthetic(vec![0.2, 0.3, 0.0]);
        let mut b = synthetic(vec![0.8, 0.3, 0.0]);
        a.fit = 10000.0;
        b.fit = -10000.0;
        let mut validation_calls = 0;
        // No test evaluator or test range is passed to this function at all.
        // Prefer b on validation despite its deliberately worse scalar train fit.
        let (winner, value) = select_validation(&[a, b], |x| {
            validation_calls += 1;
            x[0]
        });
        assert_eq!(winner.x[0], 0.8);
        assert_eq!(value, 0.8);
        assert_eq!(validation_calls, FINALISTS);
    }

    fn ensemble_benchmark(seed: u64) -> Vec<Vec<f64>> {
        let genes = vec![
            Gene {
                id: "x".into(),
                lo: 0.0,
                hi: 1.0,
                step: 0.0,
                boolean: false,
            },
            Gene {
                id: "y".into(),
                lo: 0.0,
                hi: 1.0,
                step: 0.01,
                boolean: false,
            },
            Gene {
                id: "toggle".into(),
                lo: 0.0,
                hi: 1.0,
                step: 1.0,
                boolean: true,
            },
        ];
        use std::sync::atomic::{AtomicUsize, Ordering};
        let calls = AtomicUsize::new(0);
        let spec = GaSpec {
            population: 32,
            generations: 60,
            patience: 1,
            ..GaSpec::default()
        };
        let mut rng = Rng::new(seed);
        let run = evolve(&genes, vec![0.5, 0.5, 1.0], &spec, &mut rng, &|_, _| {}, |x| {
            calls.fetch_add(1, Ordering::Relaxed);
            synthetic(x)
        });
        assert_eq!(calls.load(Ordering::Relaxed), spec.population * spec.generations);
        assert_eq!(run.history.len(), spec.generations); // patience=1 must not stop evolution.
        assert!(run.history.iter().all(|g| g.validation == 0.0));
        assert!(run.operator_stats.iter().all(|s| s.proposed > 0 && s.survived > 0));
        let archive = run.archive;
        assert!(archive.iter().all(|p| p.x[2] == 0.0));
        // Known Pareto set: toggle=0, y=0.3, x in [0.2,0.8].
        // A finite archive approximates this continuum rather than claiming exact recovery.
        let mean_distance = archive
            .iter()
            .map(|p| (p.x[1] - 0.3).abs() + (0.2 - p.x[0]).max(0.0) + (p.x[0] - 0.8).max(0.0))
            .sum::<f64>()
            / archive.len() as f64;
        assert!(mean_distance < 0.03, "mean Pareto-set distance: {mean_distance}");
        assert!(archive.iter().any(|p| (p.x[1] - 0.3).abs() < 1e-12));
        assert!(archive.iter().any(|p| p.x[0] < 0.3));
        assert!(archive.iter().any(|p| p.x[0] > 0.7));
        archive.into_iter().map(|p| p.x).collect()
    }

    #[test]
    fn seeded_multiobjective_ensemble_is_deterministic_and_spans_front() {
        for seed in [1, 7, 42] {
            assert_eq!(ensemble_benchmark(seed), ensemble_benchmark(seed));
        }
    }

    #[test]
    fn every_amalgam_objective_ignores_post_training_prices() {
        use crate::bounce::bar::Bar;
        use std::sync::Arc;
        let mut bars: Vec<_> = (0..450)
            .map(|i| {
                let open = 1000.0 + 12.0 * (i as f64 / 7.0).sin();
                let close = 1000.0 + 12.0 * ((i + 1) as f64 / 7.0).sin();
                Bar::ohlcv(
                    i * 300,
                    open,
                    open.max(close) + 1.0,
                    open.min(close) - 1.0,
                    close,
                    100.0,
                )
            })
            .collect();
        let mut p = Params {
            use_model: false,
            maker_bps: 0.0,
            taker_bps: 0.0,
            spread: 0.0,
            slippage: 0.0,
            ..Params::default()
        };
        p.scan.round_step = 5.0;
        let first = Engine::new(Arc::new(bars.clone()), &p.scan, false);
        for b in &mut bars[300..] {
            b.open += 500.0;
            b.high += 1000.0;
            b.low -= 500.0;
            b.close -= 400.0;
        }
        let changed = Engine::new(Arc::new(bars), &p.scan, false);
        let w = Window {
            train: (0, 300),
            validation: (300, 450),
        };
        let o = Objective {
            min_trades: 1,
            target_win_rate: 0.0,
            target_trades_per_day: 0.0,
            complexity: 0.0,
        };
        let a = multi_eval(&first, &p, &[], vec![], &w, &o);
        let b = multi_eval(&changed, &p, &[], vec![], &w, &o);
        assert!(a.trades > 0, "causal fixture must contain real trades");
        assert_eq!(a.objectives, b.objectives);
        assert_eq!(a.fit, b.fit);
        assert_eq!(a.trades, b.trades);
        assert_eq!(a.violation, b.violation);
    }

    #[test]
    fn oos_candidate_generation_and_admission_are_split_invariant() {
        use crate::bounce::bar::Bar;
        use std::sync::Arc;
        let bars: Vec<_> = (0..450)
            .map(|i| {
                let open = 1000.0 + 12.0 * (i as f64 / 7.0).sin();
                let close = 1000.0 + 12.0 * ((i + 1) as f64 / 7.0).sin();
                Bar::ohlcv(
                    i * 300,
                    open,
                    open.max(close) + 1.0,
                    open.min(close) - 1.0,
                    close,
                    100.0,
                )
            })
            .collect();
        let mut p = Params {
            use_model: false,
            maker_bps: 0.0,
            taker_bps: 0.0,
            spread: 0.0,
            slippage: 0.0,
            ..Params::default()
        };
        p.scan.round_step = 5.0;
        let e = Engine::new(Arc::new(bars), &p.scan, false);
        let raw = e.candidate_trades(&p, 0, 450);
        assert!(!raw.is_empty());
        let all = admit_oos(raw.into_iter().map(|t| (t, p.clone())).collect());
        let mut parts = e.candidate_trades(&p, 0, 225);
        parts.extend(e.candidate_trades(&p, 225, 450));
        let split = admit_oos(parts.into_iter().map(|t| (t, p.clone())).collect());
        assert_eq!(
            serde_json::to_string(&all).unwrap(),
            serde_json::to_string(&split).unwrap()
        );
        let a = crate::bounce::engine::money(&all, 1.0, 10.0);
        let b = crate::bounce::engine::money(&split, 1.0, 10.0);
        assert_eq!(a.return_pct, b.return_pct);
    }

    #[test]
    fn backward_compatible_spec_has_unconstrained_defaults() {
        let spec: GaSpec = serde_json::from_str("{\"population\":8,\"patience\":1}").unwrap();
        assert_eq!(spec.target_win_rate, 0.0);
        assert_eq!(spec.target_trades_per_day, 0.0);
        assert_eq!(spec.patience, 1); // Deserializable legacy field, never used in evolution.
    }

    #[test]
    fn swarm_proposal_carries_velocity_instead_of_resetting() {
        let g = vec![
            Gene {
                id: "x".into(),
                lo: 0.0,
                hi: 1.0,
                step: 0.0,
                boolean: false,
            },
            Gene {
                id: "y".into(),
                lo: 0.0,
                hi: 1.0,
                step: 0.01,
                boolean: false,
            },
            Gene {
                id: "toggle".into(),
                lo: 0.0,
                hi: 1.0,
                step: 1.0,
                boolean: true,
            },
        ];
        let mut pop = vec![synthetic(vec![0.5, 0.3, 0.0]); 8];
        let archive = vec![pop[0].clone()];
        for p in &mut pop {
            p.velocity[0] = 0.2;
        }
        let (rank, crowd) = ranks_crowding(&pop);
        let (x, v, _) = proposal(&pop, &archive, &g, 2, &mut Rng::new(7), &rank, &crowd);
        assert!((v[0] - 0.14).abs() < 1e-12);
        assert!((x[0] - 0.64).abs() < 1e-12);
        for p in &mut pop {
            p.velocity.fill(0.0);
        }
        let (reset, _, _) = proposal(&pop, &archive, &g, 2, &mut Rng::new(7), &rank, &crowd);
        assert_eq!(reset[0], 0.5);
    }

    #[test]
    fn metropolis_rejects_invalid_move() {
        let parent = synthetic(vec![0.5, 0.3, 0.0]);
        let mut child = parent.clone();
        child.violation = 1e100;
        assert!(!metropolis_accept(
            &child,
            &parent,
            std::slice::from_ref(&parent),
            0.1,
            &mut Rng::new(7)
        ));
    }

    #[test]
    fn explicit_none_metrics_keeps_only_requested_tunable_settings() {
        let spec = GaSpec {
            metrics: vec!["none".into()],
            tune: vec!["sl_atr".into(), "tp_r".into(), "max_bars".into()],
            ..GaSpec::default()
        };
        let g = genes(&spec);
        assert_eq!(g.len(), 3);
        assert!(g.iter().all(|g| !g.id.starts_with("filters.")));
        assert!(g.iter().all(|g| spec.tune.contains(&g.id)));
    }
    #[test]
    fn external_optimizer_work_is_bounded_and_days_advance() {
        let extreme = GaSpec {
            population: usize::MAX,
            generations: usize::MAX,
            train_days: i64::MAX,
            test_days: i64::MIN,
            ..GaSpec::default()
        };
        let b = bounded_spec(&extreme);
        assert_eq!(b.population, MAX_POPULATION);
        assert_eq!(b.generations, MAX_GENERATIONS);
        assert_eq!(b.train_days, MAX_DAYS);
        assert_eq!(b.test_days, 1);
        let small = bounded_spec(&GaSpec {
            train_days: 0,
            test_days: 0,
            ..GaSpec::default()
        });
        assert_eq!((small.train_days, small.test_days), (2, 1));
        let d = GaSpec::default();
        let b = bounded_spec(&d);
        assert_eq!(
            (b.population, b.generations, b.train_days, b.test_days),
            (d.population, d.generations, d.train_days, d.test_days)
        );
    }

    #[test]
    fn empty_history_and_zero_test_days_finish_with_no_oos_trades() {
        let e = Engine::new(
            std::sync::Arc::new(Vec::new()),
            &super::super::scan::ScanConfig::default(),
            false,
        );
        let p = Params {
            use_model: false,
            ..Params::default()
        };
        let spec = GaSpec {
            population: 8,
            generations: 1,
            test_days: 0,
            metrics: vec!["none".into()],
            tune: vec!["tp_r".into()],
            ..GaSpec::default()
        };
        let result = optimize(&e, &p, &spec, &|_| {});
        assert!(result.oos_trades.is_empty() && result.windows.is_empty());
        assert_eq!(result.recent, (0, 0));
        let scored = Params { use_model: true, ..p };
        let result = optimize(&e, &scored, &spec, &|_| {});
        assert!(result.oos_trades.is_empty() && result.windows.is_empty());
    }
}
