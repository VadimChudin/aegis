//! Bounce backtest: every slider and toggle of the strategy, trade simulation with costs,
//! statistics, per-signal probability and walk-forward tuning.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{
    bar::Bar,
    features::{index, FEATURES},
    scan::{scan, ScanConfig, Touch, KINDS},
};

/// A metric filter: the touch passes when `min <= value <= max`. Missing values pass
/// (a source without aggTrades has no cluster metrics).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Filter {
    pub on: bool,
    pub min: f64,
    pub max: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Params {
    pub long: bool,
    pub short: bool,
    /// Level kind id → enabled.
    pub kinds: BTreeMap<String, bool>,
    /// Session id (asia, london, ny, late) → enabled.
    pub sessions: BTreeMap<String, bool>,
    pub scan: ScanConfig,
    /// "limit": a resting order at the level fills on the touch; only metrics known before the
    /// touch bar are used. "close": enter at the close of the touch bar, using its metrics too.
    pub entry: String,
    /// Stop this many ATR beyond the level ("limit") or beyond the touch bar's extreme ("close").
    pub sl_atr: f64,
    /// Take profit in R.
    pub tp_r: f64,
    /// Close at market after this many bars.
    pub max_bars: usize,
    /// Commission for limit fills (entry in "limit" mode, take profit), basis points of notional.
    pub maker_bps: f64,
    /// Commission for market fills (entry in "close" mode, stop, time exit).
    pub taker_bps: f64,
    /// Price cost per fill in $ (half the spread plus slippage). Market fills only.
    pub slippage: f64,
    pub min_risk_atr: f64,
    pub max_risk_atr: f64,
    /// A limit order fills only when price trades this many $ through it.
    pub fill_through: f64,
    /// Use the win-probability model: trade only touches with probability >= `min_prob`.
    pub use_model: bool,
    pub min_prob: f64,
    /// Metric id → filter.
    pub filters: BTreeMap<String, Filter>,
}

pub const SESSIONS: [(&str, &str); 4] = [
    ("asia", "Asia 00–07 UTC"),
    ("london", "London 07–13 UTC"),
    ("ny", "New York 13–21 UTC"),
    ("late", "Late 21–24 UTC"),
];

impl Default for Params {
    fn default() -> Self {
        Params {
            long: true,
            short: true,
            kinds: KINDS.iter().map(|k| (k.id().to_string(), true)).collect(),
            sessions: SESSIONS.iter().map(|(s, _)| (s.to_string(), true)).collect(),
            scan: ScanConfig::default(),
            entry: "limit".into(),
            sl_atr: 0.5,
            tp_r: 1.0,
            max_bars: 24,
            maker_bps: 0.0,
            taker_bps: 4.0,
            slippage: 0.05,
            min_risk_atr: 0.2,
            max_risk_atr: 3.0,
            fill_through: 0.02,
            use_model: true,
            min_prob: 0.7,
            filters: BTreeMap::new(),
        }
    }
}

impl Params {
    fn kind_mask(&self) -> u8 {
        KINDS
            .iter()
            .filter(|k| self.kinds.get(k.id()).copied().unwrap_or(true))
            .fold(0, |m, k| m | k.bit())
    }

    pub fn on_close(&self) -> bool {
        self.entry == "close"
    }

    /// The metrics this entry mode may use.
    pub fn metrics<'a>(&self, t: &'a Touch) -> &'a [f64; super::features::NF] {
        if self.on_close() {
            &t.f
        } else {
            &t.pre
        }
    }

    fn session_ok(&self, hour: f64) -> bool {
        let id = match hour as i64 {
            0..=6 => "asia",
            7..=12 => "london",
            13..=20 => "ny",
            _ => "late",
        };
        self.sessions.get(id).copied().unwrap_or(true)
    }

    fn compiled(&self) -> Vec<(usize, f64, f64)> {
        self.filters
            .iter()
            .filter(|(_, f)| f.on)
            .filter_map(|(id, f)| index(id).map(|k| (k, f.min, f.max)))
            .collect()
    }

    pub fn passes(&self, t: &Touch) -> bool {
        self.passes_with(t, self.kind_mask(), &self.compiled())
    }

    pub(crate) fn passes_with(&self, t: &Touch, mask: u8, filters: &[(usize, f64, f64)]) -> bool {
        if (t.dir > 0 && !self.long) || (t.dir < 0 && !self.short) {
            return false;
        }
        if t.kinds & mask == 0 {
            return false;
        }
        if !self.session_ok(t.f[super::features::F::Hour as usize]) {
            return false;
        }
        let m = self.metrics(t);
        filters.iter().all(|&(k, lo, hi)| {
            let v = m[k];
            v.is_nan() || (v >= lo && v <= hi)
        })
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ParamSpec {
    pub id: String,
    pub group: &'static str,
    pub label: String,
    /// "toggle", "slider" or "filter" (a metric with an on switch and a min–max range).
    pub kind: &'static str,
    pub lo: f64,
    pub hi: f64,
    pub step: f64,
    pub help: String,
}

/// Everything the settings panel shows. Metric filters come from `FEATURES`.
pub fn param_specs() -> Vec<ParamSpec> {
    let mut v = Vec::new();
    let mut push = |id: &str, group, label: &str, kind, lo, hi, step, help: &str| {
        v.push(ParamSpec {
            id: id.into(),
            group,
            label: label.into(),
            kind,
            lo,
            hi,
            step,
            help: help.into(),
        })
    };
    push("long", "Direction", "Longs off support", "toggle", 0., 1., 1., "");
    push("short", "Direction", "Shorts off resistance", "toggle", 0., 1., 1., "");
    for k in KINDS {
        push(
            &format!("kinds.{}", k.id()),
            "Levels",
            k.label(),
            "toggle",
            0.,
            1.,
            1.,
            "",
        );
    }
    for (id, label) in SESSIONS {
        push(&format!("sessions.{id}"), "Sessions", label, "toggle", 0., 1., 1., "");
    }
    push(
        "scan.zone_atr",
        "Touch",
        "Touch zone, ATR",
        "slider",
        0.0,
        0.5,
        0.01,
        "A bar touches a level when it comes this close.",
    );
    push(
        "scan.away_atr",
        "Touch",
        "Re-arm distance, ATR",
        "slider",
        0.25,
        4.0,
        0.25,
        "Price must leave the level this far before the next touch counts.",
    );
    push(
        "scan.swing_n",
        "Touch",
        "Swing bars each side",
        "slider",
        2.0,
        12.0,
        1.0,
        "Bars on each side of a 5m swing high/low.",
    );
    push(
        "scan.round_step",
        "Touch",
        "Round price step, $",
        "slider",
        5.0,
        50.0,
        5.0,
        "",
    );
    push("entry", "Trade", "Enter on touch-bar close", "toggle", 0., 1., 1., "Off: a limit order rests at the level and fills on the touch (maker fee, metrics from before the touch). On: market entry at the close of the touch bar, which may also use the touch bar's candle, delta and clusters.");
    push(
        "sl_atr",
        "Trade",
        "Stop beyond level, ATR",
        "slider",
        0.0,
        2.0,
        0.05,
        "Limit entry: stop this far beyond the level. Close entry: beyond the touch bar's extreme.",
    );
    push(
        "tp_r",
        "Trade",
        "Take profit, R",
        "slider",
        0.2,
        3.0,
        0.05,
        "Target as a multiple of the risk. Lower targets win more often but earn less per win.",
    );
    push(
        "max_bars",
        "Trade",
        "Time exit, bars",
        "slider",
        3.0,
        96.0,
        1.0,
        "Close at market after this many 5m bars.",
    );
    push(
        "min_risk_atr",
        "Trade",
        "Min stop size, ATR",
        "slider",
        0.0,
        2.0,
        0.05,
        "Skip trades whose stop is too tight for costs.",
    );
    push(
        "max_risk_atr",
        "Trade",
        "Max stop size, ATR",
        "slider",
        0.5,
        6.0,
        0.1,
        "",
    );
    push("use_model", "Probability", "Use probability model", "toggle", 0., 1., 1., "Logistic model on the level, approach, market and tape metrics, retrained every month on the previous 4 months (walk-forward): each probability is out of sample.");
    push(
        "min_prob",
        "Probability",
        "Min win probability",
        "slider",
        0.4,
        0.95,
        0.01,
        "Trade only touches the model rates at least this likely to win.",
    );
    push(
        "fill_through",
        "Costs",
        "Limit fill needs trade-through, $",
        "slider",
        0.0,
        0.5,
        0.01,
        "A resting order fills only if price trades this far through it (queue position).",
    );
    push(
        "maker_bps",
        "Costs",
        "Maker commission, bp",
        "slider",
        0.0,
        5.0,
        0.1,
        "Limit fills. Binance TradFi perps: 0 (promotion), standard USDⓈ-M: 2. RoboForex ECN: about 0.2.",
    );
    push(
        "taker_bps",
        "Costs",
        "Taker commission, bp",
        "slider",
        0.0,
        10.0,
        0.1,
        "Market fills (stop, time exit, close entry). Binance TradFi perps: 4. RoboForex ECN: about 0.2.",
    );
    push(
        "slippage",
        "Costs",
        "Spread/slippage per market fill, $",
        "slider",
        0.0,
        1.0,
        0.01,
        "Half the spread plus slippage. Binance XAUUSDT about $0.02–0.05, RoboForex ECN about $0.10–0.20.",
    );
    for f in FEATURES {
        push(
            &format!("filters.{}", f.id),
            f.group,
            f.label,
            "filter",
            f.lo,
            f.hi,
            f.step,
            f.help,
        );
    }
    v
}

#[derive(Clone, Debug, Serialize)]
pub struct Trade {
    pub entry_time: i64,
    pub exit_time: i64,
    pub dir: i8,
    pub entry: f64,
    pub exit: f64,
    pub sl: f64,
    pub tp: f64,
    pub level: f64,
    pub kinds: u8,
    /// "tp", "sl" or "time".
    pub outcome: &'static str,
    /// Net result in R after commission and slippage.
    pub r: f64,
    /// Probability of a win estimated from the backtest (see `Report::signals`).
    pub prob: f64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Stats {
    pub trades: usize,
    pub wins: usize,
    pub win_rate: f64,
    /// Lower bound of the 95% Wilson interval of the win rate.
    pub win_rate_lo: f64,
    pub avg_r: f64,
    pub total_r: f64,
    pub profit_factor: f64,
    pub max_dd_r: f64,
    pub avg_win_r: f64,
    pub avg_loss_r: f64,
    pub max_losing_streak: usize,
    pub trades_per_week: f64,
    pub touches: usize,
}

/// A potential entry point for the chart: the bar, side, level and estimated win probability.
#[derive(Clone, Debug, Serialize)]
pub struct SignalMark {
    pub time: i64,
    pub dir: i8,
    pub price: f64,
    pub level: f64,
    pub prob: f64,
    pub taken: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub from: i64,
    pub to: i64,
    pub bars: usize,
    pub stats: Stats,
    pub trades: Vec<Trade>,
    pub signals: Vec<SignalMark>,
    /// (time, cumulative R) after every trade.
    pub equity: Vec<(i64, f64)>,
    /// Level kind id → stats of trades touching that kind.
    pub by_kind: BTreeMap<String, Stats>,
    pub by_session: BTreeMap<String, Stats>,
    /// "YYYY-MM" → stats.
    pub by_month: BTreeMap<String, Stats>,
    pub params: Params,
}

fn wilson_lo(wins: usize, n: usize) -> f64 {
    if n == 0 {
        return 0.0;
    }
    let (p, n, z) = (wins as f64 / n as f64, n as f64, 1.96);
    (p + z * z / (2.0 * n) - z * ((p * (1.0 - p) + z * z / (4.0 * n)) / n).sqrt()) / (1.0 + z * z / n)
}

pub(crate) fn stats(trades: &[Trade], weeks: f64) -> Stats {
    let n = trades.len();
    let wins = trades.iter().filter(|t| t.r > 0.0).count();
    let total: f64 = trades.iter().map(|t| t.r).sum();
    let gw: f64 = trades.iter().filter(|t| t.r > 0.0).map(|t| t.r).sum();
    let gl: f64 = -trades.iter().filter(|t| t.r <= 0.0).map(|t| t.r).sum::<f64>();
    let (mut eq, mut peak, mut dd, mut streak, mut worst) = (0.0f64, 0.0f64, 0.0f64, 0, 0);
    for t in trades {
        eq += t.r;
        peak = peak.max(eq);
        dd = dd.max(peak - eq);
        if t.r <= 0.0 {
            streak += 1;
            worst = worst.max(streak);
        } else {
            streak = 0;
        }
    }
    Stats {
        trades: n,
        wins,
        win_rate: if n > 0 { wins as f64 / n as f64 } else { 0.0 },
        win_rate_lo: wilson_lo(wins, n),
        avg_r: if n > 0 { total / n as f64 } else { 0.0 },
        total_r: total,
        profit_factor: if gl > 0.0 {
            gw / gl
        } else if gw > 0.0 {
            99.0
        } else {
            0.0
        },
        max_dd_r: dd,
        avg_win_r: if wins > 0 { gw / wins as f64 } else { 0.0 },
        avg_loss_r: if n > wins { -gl / (n - wins) as f64 } else { 0.0 },
        max_losing_streak: worst,
        trades_per_week: if weeks > 0.0 { n as f64 / weeks } else { 0.0 },
        touches: 0,
    }
}

/// Simulates one trade from the close of the touch bar. When the stop and the target are both
/// inside the same bar the stop is assumed first (conservative).
pub fn simulate(bars: &[Bar], t: &Touch, p: &Params) -> Option<(Trade, usize)> {
    let b = &bars[t.i];
    let d = t.dir as f64;
    let on_close = p.on_close();
    let (entry, stop, entry_fee) = if on_close {
        let stop = if d > 0.0 {
            b.low.min(t.level) - p.sl_atr * t.atr
        } else {
            b.high.max(t.level) + p.sl_atr * t.atr
        };
        (b.close + d * p.slippage, stop, p.taker_bps)
    } else {
        let ext = if d > 0.0 { b.low } else { b.high };
        if d * (t.fill - ext) < p.fill_through {
            return None;
        }
        (t.fill, t.level - d * p.sl_atr * t.atr, p.maker_bps)
    };
    let risk = d * (entry - stop);
    if risk <= 0.0 || risk < p.min_risk_atr * t.atr || risk > p.max_risk_atr * t.atr {
        return None;
    }
    let tp = entry + d * p.tp_r * risk;
    let last = (t.i + p.max_bars.max(1)).min(bars.len() - 1);
    if last <= t.i {
        return None;
    }
    let (mut exit, mut j_exit, mut outcome) = (f64::NAN, last, "time");
    // A limit fill can be stopped out inside the touch bar itself; a target hit in that bar is
    // not counted because the order of high and low inside the bar is unknown.
    if !on_close && d * ((if d > 0.0 { b.low } else { b.high }) - stop) <= 0.0 {
        (exit, j_exit, outcome) = (stop - d * p.slippage, t.i, "sl");
    }
    if exit.is_nan() {
        for (j, x) in bars.iter().enumerate().take(last + 1).skip(t.i + 1) {
            let (adverse, favour) = if d > 0.0 { (x.low, x.high) } else { (x.high, x.low) };
            if d * (x.open - stop) <= 0.0 {
                (exit, j_exit, outcome) = (x.open - d * p.slippage, j, "sl");
                break;
            }
            if d * (adverse - stop) <= 0.0 {
                (exit, j_exit, outcome) = (stop - d * p.slippage, j, "sl");
                break;
            }
            // The take profit is a resting limit: it fills at its price even when price gaps past it.
            if d * (favour - tp) >= 0.0 {
                (exit, j_exit, outcome) = (tp, j, "tp");
                break;
            }
        }
    }
    if exit.is_nan() {
        exit = bars[last].close - d * p.slippage;
    }
    let exit_fee = if outcome == "tp" { p.maker_bps } else { p.taker_bps };
    let fees = (entry_fee * entry + exit_fee * exit) / 1e4;
    let r = (d * (exit - entry) - fees) / risk;
    let entry_time = if on_close { b.time + 300 } else { b.time };
    Some((
        Trade {
            entry_time,
            exit_time: bars[j_exit].time + 300,
            dir: t.dir,
            entry,
            exit,
            sl: stop,
            tp,
            level: t.level,
            kinds: t.kinds,
            outcome,
            r,
            prob: f64::NAN,
        },
        j_exit,
    ))
}

/// Trades from touches in `[from_i, to_i)`, one position at a time. `probs[k]` is the model
/// probability of touch `k` (NaN when unknown).
fn run(
    bars: &[Bar],
    touches: &[Touch],
    probs: &[f64],
    p: &Params,
    from_i: usize,
    to_i: usize,
) -> (Vec<Trade>, Vec<usize>) {
    let mask = p.kind_mask();
    let filters = p.compiled();
    let mut trades = Vec::new();
    let mut passed = Vec::new();
    let mut busy_until = 0usize;
    for (k, t) in touches.iter().enumerate() {
        if t.i < from_i || t.i >= to_i || !p.passes_with(t, mask, &filters) {
            continue;
        }
        let pr = probs.get(k).copied().unwrap_or(f64::NAN);
        if p.use_model && (pr.is_nan() || pr < p.min_prob) {
            continue;
        }
        passed.push(k);
        if t.i < busy_until {
            continue;
        }
        if let Some((mut tr, j)) = simulate(bars, t, p) {
            busy_until = j + 1;
            tr.prob = pr;
            trades.push(tr);
        }
    }
    (trades, passed)
}

/// Walk-forward model probabilities: touches of each calendar month are scored by a model
/// trained on the previous (up to) 4 months, labelled with this trade setup's net result.
/// The first 2 months have no probability.
pub(crate) fn model_probs(bars: &[Bar], touches: &[Touch], p: &Params) -> Vec<f64> {
    use super::model::{row, Model};
    let n = touches.len();
    let mut probs = vec![f64::NAN; n];
    if !p.use_model || n == 0 {
        return probs;
    }
    let months: Vec<String> = touches.iter().map(|t| month_id(t.time)).collect();
    let mut order: Vec<String> = months.clone();
    order.dedup();
    let rows: Vec<_> = touches.iter().map(|t| row(p.metrics(t), t.dir)).collect();
    let label: Vec<Option<bool>> = touches
        .iter()
        .map(|t| simulate(bars, t, p).map(|(tr, _)| tr.r > 0.0))
        .collect();
    for k in 2..order.len() {
        let train: Vec<usize> = (0..n)
            .filter(|&i| order[k.saturating_sub(4)..k].contains(&months[i]) && label[i].is_some())
            .collect();
        // Keep training fast: at most the 12 000 most recent touches.
        let train = &train[train.len().saturating_sub(12_000)..];
        let xs: Vec<_> = train.iter().map(|&i| rows[i]).collect();
        let ys: Vec<bool> = train.iter().map(|&i| label[i].unwrap_or(false)).collect();
        if let Some(m) = Model::fit(&xs, &ys) {
            for i in (0..n).filter(|&i| months[i] == order[k]) {
                probs[i] = m.predict(&rows[i]);
            }
        }
    }
    probs
}

fn session_id(time: i64) -> &'static str {
    match (time / 3600).rem_euclid(24) {
        0..=6 => "asia",
        7..=12 => "london",
        13..=20 => "ny",
        _ => "late",
    }
}

pub(crate) fn month_id(time: i64) -> String {
    // Civil date from days since epoch (Howard Hinnant's algorithm).
    let z = time.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}")
}

/// Probability of a win for a touch with these level kinds: the backtest win rate of trades
/// sharing a level kind, shrunk toward the overall win rate (20 pseudo-trades).
fn probability(trades: &[Trade], kinds: u8, overall: f64) -> f64 {
    let same: Vec<_> = trades.iter().filter(|t| t.kinds & kinds != 0).collect();
    let wins = same.iter().filter(|t| t.r > 0.0).count() as f64;
    (wins + 20.0 * overall) / (same.len() as f64 + 20.0)
}

pub fn evaluate(bars: &[Bar], p: &Params) -> Report {
    let touches = scan(bars, &p.scan);
    evaluate_touches(bars, &touches, p)
}

pub(crate) fn evaluate_touches(bars: &[Bar], touches: &[Touch], p: &Params) -> Report {
    let probs = model_probs(bars, touches, p);
    evaluate_with(bars, touches, &probs, p)
}

fn evaluate_with(bars: &[Bar], touches: &[Touch], probs: &[f64], p: &Params) -> Report {
    let (mut trades, passed) = run(bars, touches, probs, p, 0, bars.len());
    let span = match (bars.first(), bars.last()) {
        (Some(a), Some(b)) => (a.time, b.time + 300),
        _ => (0, 0),
    };
    let weeks = (span.1 - span.0) as f64 / 604_800.0;
    let mut st = stats(&trades, weeks);
    st.touches = touches.len();
    let overall = st.win_rate;
    let fallback: Vec<f64> = trades.iter().map(|t| probability(&trades, t.kinds, overall)).collect();
    for (t, pr) in trades.iter_mut().zip(fallback) {
        if t.prob.is_nan() {
            t.prob = pr;
        }
    }
    let entry_time = |t: &Touch| if p.on_close() { t.time + 300 } else { t.time };
    let taken: std::collections::HashSet<(i64, i8)> = trades.iter().map(|t| (t.entry_time, t.dir)).collect();
    let signals = passed
        .iter()
        .map(|&k| {
            let t = &touches[k];
            SignalMark {
                time: t.time,
                dir: t.dir,
                price: if p.on_close() { bars[t.i].close } else { t.fill },
                level: t.level,
                prob: if probs[k].is_finite() {
                    probs[k]
                } else {
                    probability(&trades, t.kinds, overall)
                },
                taken: taken.contains(&(entry_time(t), t.dir)),
            }
        })
        .collect();
    let mut eq = 0.0;
    let equity = trades
        .iter()
        .map(|t| {
            eq += t.r;
            (t.exit_time, eq)
        })
        .collect();
    let group = |key: &dyn Fn(&Trade) -> Vec<String>| {
        let mut m: BTreeMap<String, Vec<Trade>> = BTreeMap::new();
        for t in &trades {
            for k in key(t) {
                m.entry(k).or_default().push(t.clone());
            }
        }
        m.into_iter().map(|(k, v)| (k, stats(&v, weeks))).collect()
    };
    Report {
        from: span.0,
        to: span.1,
        bars: bars.len(),
        by_kind: group(&|t| {
            KINDS
                .iter()
                .filter(|k| t.kinds & k.bit() != 0)
                .map(|k| k.id().to_string())
                .collect()
        }),
        by_session: group(&|t| vec![session_id(t.entry_time).to_string()]),
        by_month: group(&|t| vec![month_id(t.entry_time)]),
        stats: st,
        trades,
        signals,
        equity,
        params: p.clone(),
    }
}

// ---- expected entries now ------------------------------------------------------------------

/// A level price is near: the order the strategy would rest there, with its probability.
#[derive(Clone, Debug, Serialize)]
pub struct ExpectedEntry {
    pub dir: i8,
    pub level: f64,
    pub entry: f64,
    pub sl: f64,
    pub tp: f64,
    pub kinds: u8,
    pub prob: f64,
    /// Passes every toggle, filter and the probability threshold.
    pub ok: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct LiveReport {
    pub time: i64,
    pub price: f64,
    pub atr: f64,
    pub entries: Vec<ExpectedEntry>,
    /// Touches the current model was trained on (the last 4 months).
    pub trained_on: usize,
}

/// Expected entries after the last bar, scored by a model trained on the last 4 months.
pub fn live(bars: &[Bar], p: &Params) -> LiveReport {
    use super::model::{row, Model};
    let (touches, pending) = super::scan::scan_full(bars, &p.scan);
    let last = bars.last().copied().unwrap_or(Bar::ohlcv(0, 0., 0., 0., 0., 0.));
    let since = last.time - 122 * 86_400;
    let train: Vec<&Touch> = touches.iter().filter(|t| t.time >= since).collect();
    let train = &train[train.len().saturating_sub(12_000)..];
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    for t in train {
        if let Some((tr, _)) = simulate(bars, t, p) {
            xs.push(row(p.metrics(t), t.dir));
            ys.push(tr.r > 0.0);
        }
    }
    let model = if p.use_model { Model::fit(&xs, &ys) } else { None };
    let mask = p.kind_mask();
    let filters = p.compiled();
    let entries = pending
        .iter()
        .map(|t| {
            let d = t.dir as f64;
            let sl = t.level - d * p.sl_atr * t.atr;
            let risk = d * (t.fill - sl);
            let prob = model.as_ref().map_or(f64::NAN, |m| m.predict(&row(&t.pre, t.dir)));
            let sized = risk >= p.min_risk_atr * t.atr && risk <= p.max_risk_atr * t.atr;
            let prob_ok = !p.use_model || prob >= p.min_prob;
            ExpectedEntry {
                dir: t.dir,
                level: t.level,
                entry: t.fill,
                sl,
                tp: t.fill + d * p.tp_r * risk,
                kinds: t.kinds,
                prob,
                ok: sized && prob_ok && p.passes_with(t, mask, &filters),
            }
        })
        .collect();
    LiveReport {
        time: last.time + 300,
        price: last.close,
        atr: touches.last().map_or(f64::NAN, |t| t.atr),
        entries,
        trained_on: xs.len(),
    }
}

// ---- walk-forward tuning ------------------------------------------------------------------

#[derive(Clone, Debug, Serialize)]
pub struct OptimizeReport {
    /// Settings tuned on the most recent training window: the ones to trade now.
    pub params: Params,
    /// Trades of every test window, each traded with settings tuned only on the data before it.
    pub out_of_sample: Stats,
    pub oos_trades: Vec<Trade>,
    pub windows: Vec<WindowResult>,
    /// Full-history backtest of `params` (in-sample, for reference only).
    pub in_sample: Stats,
}

#[derive(Clone, Debug, Serialize)]
pub struct WindowResult {
    pub train_from: i64,
    pub test_from: i64,
    pub test_to: i64,
    pub train: Stats,
    pub test: Stats,
}

/// Small deterministic PRNG (xorshift) so results repeat for the same seed.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, lo: f64, hi: f64, step: f64) -> f64 {
        let v = lo + self.next() * (hi - lo);
        if step > 0.0 {
            (lo + ((v - lo) / step).round() * step).clamp(lo, hi)
        } else {
            v
        }
    }
}

/// Which settings the tuner may change: metric ids for filters, and trade settings.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct TuneSpec {
    pub target_win_rate: f64,
    pub min_trades: usize,
    pub train_days: i64,
    pub test_days: i64,
    pub iterations: usize,
    pub seed: u64,
    pub filters: Vec<String>,
    pub tune_trade: bool,
}

impl Default for TuneSpec {
    fn default() -> Self {
        TuneSpec {
            target_win_rate: 0.7,
            min_trades: 30,
            train_days: 90,
            test_days: 30,
            iterations: 300,
            seed: 7,
            filters: Vec::new(),
            tune_trade: true,
        }
    }
}

fn score(s: &Stats, spec: &TuneSpec) -> f64 {
    if s.trades < spec.min_trades {
        return f64::NEG_INFINITY;
    }
    // Net R first; a shortfall against the target win rate costs heavily.
    let short = (spec.target_win_rate - s.win_rate_lo.max(s.win_rate - 0.05)).max(0.0);
    s.total_r / (1.0 + s.max_dd_r).sqrt() - 200.0 * short
}

fn mutate(base: &Params, spec: &TuneSpec, rng: &mut Rng) -> Params {
    let mut p = base.clone();
    for id in &spec.filters {
        let Some(k) = index(id) else { continue };
        let f = &FEATURES[k];
        let roll = rng.next();
        let filter = if roll < 0.4 {
            Filter {
                on: false,
                min: f.lo,
                max: f.hi,
            }
        } else if roll < 0.7 {
            Filter {
                on: true,
                min: rng.range(f.lo, f.hi, f.step),
                max: f.hi,
            }
        } else {
            Filter {
                on: true,
                min: f.lo,
                max: rng.range(f.lo, f.hi, f.step),
            }
        };
        p.filters.insert(id.clone(), filter);
    }
    if base.use_model {
        p.min_prob = rng.range(0.55, 0.85, 0.01);
    }
    if spec.tune_trade {
        p.tp_r = rng.range(0.3, 1.5, 0.05);
        p.sl_atr = if p.on_close() {
            rng.range(0.0, 0.6, 0.05)
        } else {
            rng.range(0.2, 1.5, 0.05)
        };
        p.max_bars = rng.range(6.0, 48.0, 1.0) as usize;
    }
    p
}

fn index_at(bars: &[Bar], time: i64) -> usize {
    bars.partition_point(|b| b.time < time)
}

pub fn optimize(bars: &[Bar], base: &Params, spec: &TuneSpec) -> OptimizeReport {
    let touches = scan(bars, &base.scan);
    let probs = model_probs(bars, &touches, base);
    let spec = &TuneSpec {
        tune_trade: spec.tune_trade && !base.use_model,
        ..spec.clone()
    };
    let (Some(first), Some(last)) = (bars.first(), bars.last()) else {
        return OptimizeReport {
            params: base.clone(),
            out_of_sample: Stats::default(),
            oos_trades: vec![],
            windows: vec![],
            in_sample: Stats::default(),
        };
    };
    let day = 86_400;
    let mut rng = Rng(spec.seed.max(1));
    let candidates: Vec<Params> = std::iter::once(base.clone())
        .chain((0..spec.iterations).map(|_| mutate(base, spec, &mut rng)))
        .collect();
    let best_on = |from: usize, to: usize| -> (Params, Stats) {
        let weeks = (bars[to - 1].time - bars[from].time) as f64 / 604_800.0;
        candidates
            .iter()
            .map(|c| {
                let s = stats(&run(bars, &touches, &probs, c, from, to).0, weeks);
                (score(&s, spec), c, s)
            })
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, c, s)| (c.clone(), s))
            .unwrap_or((base.clone(), Stats::default()))
    };

    let mut windows = Vec::new();
    let mut oos = Vec::new();
    let mut test_from = first.time + spec.train_days * day;
    while test_from < last.time {
        let test_to = (test_from + spec.test_days * day).min(last.time + 300);
        let (a, b, c) = (
            index_at(bars, test_from - spec.train_days * day),
            index_at(bars, test_from),
            index_at(bars, test_to),
        );
        if b > a + 100 && c > b {
            let (p, train) = best_on(a, b);
            let weeks = (test_to - test_from) as f64 / 604_800.0;
            let test_trades = run(bars, &touches, &probs, &p, b, c).0;
            windows.push(WindowResult {
                train_from: test_from - spec.train_days * day,
                test_from,
                test_to,
                train,
                test: stats(&test_trades, weeks),
            });
            oos.extend(test_trades);
        }
        test_from = test_to;
    }
    let from_now = index_at(bars, last.time - spec.train_days * day);
    let (params, _) = best_on(from_now, bars.len());
    let oos_weeks = windows.iter().map(|w| (w.test_to - w.test_from) as f64).sum::<f64>() / 604_800.0;
    let in_sample = evaluate_with(bars, &touches, &probs, &params).stats;
    OptimizeReport {
        params,
        out_of_sample: stats(&oos, oos_weeks),
        oos_trades: oos,
        windows,
        in_sample,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bounce::NF;

    fn bars() -> Vec<Bar> {
        (0..3000)
            .map(|i| {
                let x = 2000.0 + 15.0 * ((i as f64) / 20.0).sin() + 5.0 * ((i as f64) / 3.3).cos();
                let mut b = Bar::ohlcv(i as i64 * 300, x, x + 1.5, x - 1.5, x + 0.3, 10.0);
                b.buy_volume = 5.0 + ((i % 7) as f64 - 3.0);
                b.trades = 5.0;
                b
            })
            .collect()
    }

    fn plain() -> Params {
        Params {
            use_model: false,
            ..Params::default()
        }
    }

    #[test]
    fn model_probabilities_are_walk_forward() {
        // 100 days of bars: months 1–2 have no probability, later months do.
        let b: Vec<Bar> = (0..28_800)
            .map(|i| {
                let x = 2000.0 + 15.0 * ((i as f64) / 20.0).sin() + 5.0 * ((i as f64) / 3.3).cos();
                let mut bar = Bar::ohlcv(i as i64 * 300, x, x + 1.5, x - 1.5, x + 0.3, 10.0);
                bar.buy_volume = 5.0 + ((i % 7) as f64 - 3.0);
                bar.trades = 5.0;
                bar
            })
            .collect();
        let p = Params::default();
        let touches = scan(&b, &p.scan);
        let probs = model_probs(&b, &touches, &p);
        for (t, pr) in touches.iter().zip(&probs) {
            if month_id(t.time).as_str() < "1970-03" {
                assert!(pr.is_nan());
            }
        }
        assert!(probs.iter().any(|x| x.is_finite() && (0.0..=1.0).contains(x)));
    }

    #[test]
    fn backtest_produces_consistent_trades() {
        let b = bars();
        let r = evaluate(&b, &plain());
        assert!(r.stats.trades > 10);
        assert_eq!(r.stats.trades, r.trades.len());
        let mut last_exit = 0;
        for t in &r.trades {
            assert!(t.entry_time >= last_exit, "positions overlap");
            last_exit = t.exit_time;
            assert!(t.r.is_finite());
            assert!((0.0..=1.0).contains(&t.prob));
            if t.outcome == "sl" {
                assert!(t.r < 0.0);
            }
        }
        let total: f64 = r.trades.iter().map(|t| t.r).sum();
        assert!((total - r.stats.total_r).abs() < 1e-9);
    }

    #[test]
    fn filters_and_toggles_reduce_trades() {
        let b = bars();
        let all = evaluate(&b, &plain()).signals.len();
        let mut p = plain();
        p.short = false;
        let longs = evaluate(&b, &p).signals.len();
        assert!(longs < all && longs > 0);
        p.filters.insert(
            "wick".into(),
            Filter {
                on: true,
                min: 0.6,
                max: 1.0,
            },
        );
        assert!(evaluate(&b, &p).signals.len() < longs);
    }

    #[test]
    fn costs_lower_the_result() {
        let b = bars();
        let mut p = plain();
        p.maker_bps = 0.0;
        p.taker_bps = 0.0;
        p.slippage = 0.0;
        let free = evaluate(&b, &p).stats.total_r;
        p.taker_bps = 5.0;
        assert!(evaluate(&b, &p).stats.total_r < free);
    }

    #[test]
    fn month_ids() {
        assert_eq!(month_id(0), "1970-01");
        assert_eq!(month_id(1_767_225_600), "2026-01");
        assert_eq!(month_id(1_790_466_900), "2026-09");
    }

    #[test]
    fn specs_cover_every_metric() {
        let s = param_specs();
        assert_eq!(s.iter().filter(|x| x.kind == "filter").count(), NF);
    }
}
