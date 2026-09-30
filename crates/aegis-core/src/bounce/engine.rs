//! Backtest engine. A level scan and a walk-forward score model are computed once; trade
//! outcomes and calibrated probabilities are cached per trade setting, so re-running with other
//! filters, toggles or a probability threshold takes well under a millisecond. The genetic
//! algorithm relies on this.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::{Arc, Mutex},
};

use serde::Serialize;

use super::{
    bar::Bar,
    model::{row, Model},
    params::{session_id, Params},
    position::simulate_secs,
    scan::{scan_full, ScanConfig, Touch, KINDS},
    secs::Sec,
};

/// Months the score model and the calibration learn from (walk-forward).
const TRAIN_MONTHS: usize = 4;
const MAX_TRAIN_ROWS: usize = 12_000;
const CACHE_ENTRIES: usize = 256;

#[derive(Clone, Debug, Serialize)]
pub struct Trade {
    pub entry_time: i64,
    pub exit_time: i64,
    /// First-leg settlement; distinct from the combined exit when a flip opens.
    pub first_exit_time: i64,
    pub dir: i8,
    pub entry: f64,
    pub exit: f64,
    pub sl: f64,
    pub tp: f64,
    pub level: f64,
    pub kinds: u16,
    /// "tp", "sl" or "time".
    /// The 1-second engine adds "be" (breakeven stop), "trail" (trailing stop) and "flow"
    /// (aggressive volume through the level).
    pub outcome: &'static str,
    /// Net result in R after commission, spread and slippage; with a flip, both legs.
    pub r: f64,
    /// Calibrated win probability known before the trade.
    pub prob: f64,
    /// Partial take profit price (NaN: none).
    pub part: f64,
    /// The opposite position opened when this one failed (NaN / "" when none).
    pub flip_time: i64,
    pub flip_entry: f64,
    pub flip_exit: f64,
    pub flip_sl: f64,
    pub flip_outcome: &'static str,
    pub flip_r: f64,
}

impl Trade {
    pub(crate) fn empty() -> Trade {
        Trade {
            entry_time: 0,
            exit_time: 0,
            first_exit_time: 0,
            dir: 0,
            entry: f64::NAN,
            exit: f64::NAN,
            sl: f64::NAN,
            tp: f64::NAN,
            level: f64::NAN,
            kinds: 0,
            outcome: "",
            r: f64::NAN,
            prob: f64::NAN,
            part: f64::NAN,
            flip_time: 0,
            flip_entry: f64::NAN,
            flip_exit: f64::NAN,
            flip_sl: f64::NAN,
            flip_outcome: "",
            flip_r: f64::NAN,
        }
    }
}

pub(crate) fn spread(b: &Bar, assumed: f64) -> f64 {
    if b.spread.is_finite() {
        b.spread
    } else {
        assumed
    }
}

/// A 1-minute candle, used to resolve the order of events inside a 5m bar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Minute {
    pub time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
}

/// The 1m candles inside the 5m bar that opens at `t0`.
fn minutes_of(mins: &[Minute], t0: i64) -> &[Minute] {
    let a = mins.partition_point(|m| m.time < t0);
    let b = mins.partition_point(|m| m.time < t0 + 300);
    &mins[a..b]
}

/// Simulates one trade on 5m bars only (see `simulate_in`).
pub fn simulate(bars: &[Bar], t: &Touch, p: &Params) -> Option<(Trade, usize)> {
    simulate_in(bars, &[], t, p)
}

/// Simulates one trade. Prices are trades (Binance) or bids (quote sources with `spread`): with a
/// spread, buys happen at the ask = bid + spread.
///
/// Order of events inside a bar: when a 5m bar reaches both the stop and the target, its 1m
/// candles decide which came first; only inside a single 1m candle the OHLC path is assumed (an
/// up candle goes open → low → high → close). Without 1m data the same assumption is made on the
/// 5m bar, which biases results down: the nearer barrier is really hit first more often.
/// A limit fill is located in the 1m candles of the touch bar; after it, the rest of that bar can
/// stop the trade or reach the target.
pub fn simulate_in(bars: &[Bar], mins: &[Minute], t: &Touch, p: &Params) -> Option<(Trade, usize)> {
    let b = bars.get(t.i)?;
    let d = t.dir as f64;
    let on_close = p.on_close();
    let sp = |x: &Bar| spread(x, p.spread);
    let (entry, stop, entry_fee) = if on_close {
        let stop = if d > 0.0 {
            b.low.min(t.level) - p.sl_atr * t.atr
        } else {
            b.high.max(t.level) + p.sl_atr * t.atr
        };
        let px = b.close + if d > 0.0 { sp(b) } else { 0.0 };
        (px + d * p.slippage, stop, p.taker_bps)
    } else {
        let through = if d > 0.0 {
            t.fill - (b.low + sp(b))
        } else {
            b.high - t.fill
        };
        if through < p.fill_through {
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
    // Exit-side price of a candle: longs sell at the bid, shorts buy at the ask.
    let side = |px: f64, s: f64| if d > 0.0 { px } else { px + s };
    // One candle with an open position: Some((exit price, outcome)) when it closes the trade.
    let check = |o: f64, h: f64, l: f64, c: f64, s: f64, heuristic: bool| -> Option<(f64, &'static str, bool)> {
        let (adverse, favour) = if d > 0.0 { (l, h) } else { (h, l) };
        let (open, adverse, favour) = (side(o, s), side(adverse, s), side(favour, s));
        if d * (open - stop) <= 0.0 {
            return Some((open - d * p.slippage, "sl", false));
        }
        let (hit_sl, hit_tp) = (d * (adverse - stop) <= 0.0, d * (favour - tp) >= 0.0);
        if hit_sl && hit_tp && !heuristic {
            return Some((f64::NAN, "", true));
        }
        if hit_sl && (!hit_tp || (c >= o) == (d > 0.0)) {
            return Some((stop - d * p.slippage, "sl", false));
        }
        // The take profit is a resting limit: it fills at its price even when price gaps past it.
        hit_tp.then_some((tp, "tp", false))
    };
    // A 5m bar, resolved through its 1m candles when both barriers are inside it.
    let bar_step = |x: &Bar| -> Option<(f64, &'static str)> {
        match check(x.open, x.high, x.low, x.close, sp(x), false) {
            Some((_, _, true)) => {
                let m = minutes_of(mins, x.time);
                m.iter()
                    .find_map(|m| check(m.open, m.high, m.low, m.close, sp(x), true).map(|r| (r.0, r.1)))
                    .or_else(|| check(x.open, x.high, x.low, x.close, sp(x), true).map(|r| (r.0, r.1)))
            }
            r => r.map(|r| (r.0, r.1)),
        }
    };
    let (mut exit, mut j_exit, mut outcome) = (f64::NAN, last, "time");
    if !on_close {
        // The touch bar after the fill.
        let m = minutes_of(mins, b.time);
        let fill_at = m.iter().position(|m| {
            if d > 0.0 {
                t.fill - (m.low + sp(b)) >= p.fill_through
            } else {
                m.high - t.fill >= p.fill_through
            }
        });
        let res = match fill_at {
            Some(k) => {
                let fm = &m[k];
                let adverse = side(if d > 0.0 { fm.low } else { fm.high }, sp(b));
                if d * (adverse - stop) <= 0.0 {
                    let open = side(fm.open, sp(b));
                    let fill = if d * (open - stop) <= 0.0 { open } else { stop };
                    Some((fill - d * p.slippage, "sl"))
                } else {
                    m[k + 1..]
                        .iter()
                        .find_map(|m| check(m.open, m.high, m.low, m.close, sp(b), true).map(|r| (r.0, r.1)))
                }
            }
            // No 1m data: the bar's extreme may stop the trade; a target is not counted because
            // the high may have come before the fill.
            None => (d * (side(if d > 0.0 { b.low } else { b.high }, sp(b)) - stop) <= 0.0).then(|| {
                let open = side(b.open, sp(b));
                let fill = if d * (open - stop) <= 0.0 { open } else { stop };
                (fill - d * p.slippage, "sl")
            }),
        };
        if let Some((px, o)) = res {
            (exit, j_exit, outcome) = (px, t.i, o);
        }
    }
    if exit.is_nan() {
        for (j, x) in bars.iter().enumerate().take(last + 1).skip(t.i + 1) {
            if let Some((px, o)) = bar_step(x) {
                (exit, j_exit, outcome) = (px, j, o);
                break;
            }
        }
    }
    if exit.is_nan() {
        exit = side(bars[last].close, sp(&bars[last])) - d * p.slippage;
    }
    let exit_fee = if outcome == "tp" { p.maker_bps } else { p.taker_bps };
    let fees = (entry_fee * entry + exit_fee * exit) / 1e4;
    let r = (d * (exit - entry) - fees) / risk;
    Some((
        Trade {
            entry_time: if on_close { b.time + 300 } else { b.time },
            exit_time: bars[j_exit].time + 300,
            first_exit_time: bars[j_exit].time + 300,
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
            ..Trade::empty()
        },
        j_exit,
    ))
}

/// Civil "YYYY-MM" of a Unix time (Howard Hinnant's days-to-civil algorithm).
pub fn month_id(time: i64) -> String {
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

/// Calendar days touched by `[from, to)`.
pub fn calendar_days(from: i64, to: i64) -> f64 {
    ((to - 1).div_euclid(86_400) - from.div_euclid(86_400) + 1).max(1) as f64
}

/// Daily R series over `[from, to)`: one value per calendar day, 0 on days without trades.
pub fn daily_r(trades: &[Trade], from: i64, to: i64) -> Vec<f64> {
    let d0 = from.div_euclid(86_400);
    let mut v = vec![0.0; calendar_days(from, to) as usize];
    for t in trades {
        let k = t.entry_time.div_euclid(86_400) - d0;
        if k >= 0 && (k as usize) < v.len() {
            v[k as usize] += t.r;
        }
    }
    v
}

/// Trade outcomes and calibrated probabilities for one trade setting, for every touch.
pub struct Sim {
    /// Net R, NaN when the touch gives no trade (no fill, stop size out of range).
    pub r: Vec<f32>,
    /// Bar index of the exit.
    pub exit: Vec<u32>,
    /// Exact label settlement timestamp, including the second-engine exit.
    pub exit_time: Vec<i64>,
    /// Calibrated win probability (NaN when unknown).
    pub prob: Vec<f32>,
}

/// Running totals of a trade list, enough for the GA fitness.
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct Quick {
    pub n: usize,
    pub wins: usize,
    pub sum: f64,
    pub sumsq: f64,
    /// Calendar days in the evaluated range.
    pub days: f64,
    /// Sum and sum of squares of the daily R series (0 on days without trades).
    pub dsum: f64,
    pub dsumsq: f64,
}

impl Quick {
    pub fn win_rate(&self) -> f64 {
        if self.n == 0 {
            0.0
        } else {
            self.wins as f64 / self.n as f64
        }
    }
    pub fn mean(&self) -> f64 {
        if self.n == 0 {
            0.0
        } else {
            self.sum / self.n as f64
        }
    }
    pub fn std(&self) -> f64 {
        if self.n < 2 {
            return 0.0;
        }
        let m = self.mean();
        ((self.sumsq - self.n as f64 * m * m) / (self.n as f64 - 1.0))
            .max(0.0)
            .sqrt()
    }
    /// Per-trade Sharpe ratio (mean / std of R).
    pub fn sharpe(&self) -> f64 {
        let s = self.std();
        if s > 0.0 {
            self.mean() / s
        } else {
            0.0
        }
    }
    pub fn per_day(&self) -> f64 {
        self.n as f64 / self.days.max(1.0)
    }
    /// Sharpe ratio of the daily R series: comparable between settings with different trade
    /// counts, as the Deflated Sharpe Ratio requires.
    pub fn daily_sharpe(&self) -> f64 {
        let t = self.days.max(1.0);
        if t < 2.0 {
            return 0.0;
        }
        let m = self.dsum / t;
        let sd = ((self.dsumsq - t * m * m) / (t - 1.0)).max(0.0).sqrt();
        if sd > 0.0 {
            m / sd
        } else {
            0.0
        }
    }
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
    /// Trades per calendar day of the range.
    pub trades_per_day: f64,
    /// Per-trade Sharpe ratio (mean R / std R).
    pub sharpe: f64,
    /// Sharpe ratio of the daily R series.
    pub daily_sharpe: f64,
    pub touches: usize,
}

/// The trades in money: every trade risks `risk_pct` of the account (compounded), the position
/// is cut when that would need more than `max_leverage`.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Money {
    pub risk_pct: f64,
    pub max_leverage: f64,
    /// Account growth over the range, %.
    pub return_pct: f64,
    pub max_dd_pct: f64,
    /// Leverage the full risk would need: median and maximum over trades.
    pub leverage_median: f64,
    pub leverage_max: f64,
    /// Share of trades cut by the leverage limit.
    pub capped: f64,
}

/// Settlement events, never realizing an entire flip chain on its final day.
pub(crate) fn realized_legs(t: &Trade) -> Vec<(i64, f64)> {
    if t.flip_entry.is_finite() && t.flip_r.is_finite() {
        vec![(t.first_exit_time, t.r - t.flip_r), (t.exit_time, t.flip_r)]
    } else {
        vec![(t.exit_time, t.r)]
    }
}

pub fn money(trades: &[Trade], risk_pct: f64, max_leverage: f64) -> Money {
    // Realized-equity sizing, not a portfolio margin simulator. Fix each leg's
    // dollar risk at entry, then add its net PnL at settlement. In particular,
    // overlapping positions cannot resize retroactively at another trade's exit.
    // Short-sale proceeds are not spendable equity: both directions add only
    // net PnL. Partial cash flows are deferred to the leg's final settlement.
    let f = if risk_pct.is_finite() {
        (risk_pct / 100.0).max(0.0)
    } else {
        0.0
    };
    let cap = if max_leverage.is_finite() {
        max_leverage.max(0.0)
    } else {
        0.0
    };
    let (mut eq, mut peak, mut dd, mut capped) = (1.0f64, 1.0f64, 0.0f64, 0usize);
    let mut lev: Vec<f64> = Vec::with_capacity(trades.len());
    // (entry time, exit time, entry price, price risk, net R)
    let mut legs = Vec::new();
    for t in trades {
        let flipped = t.flip_entry.is_finite() && t.flip_r.is_finite();
        let first_end = if flipped { t.first_exit_time } else { t.exit_time };
        legs.push((
            t.entry_time,
            first_end,
            t.entry,
            (t.entry - t.sl).abs(),
            if flipped { t.r - t.flip_r } else { t.r },
        ));
        if flipped {
            legs.push((
                t.flip_time,
                t.exit_time,
                t.flip_entry,
                (t.flip_entry - t.flip_sl).abs(),
                t.flip_r,
            ));
        }
    }
    let mut events = Vec::new();
    for (i, &(start, end, price, risk, r)) in legs.iter().enumerate() {
        if !price.is_finite() || price <= 0.0 || !risk.is_finite() || risk <= 0.0 || !r.is_finite() || end < start {
            continue;
        }
        // At equal timestamps, existing closes settle before new entries.
        events.push((start, 1u8, i));
        // Same-second positions still enter before their own settlement.
        // Older positions close before new entries at the timestamp.
        events.push((end, if end == start { 2u8 } else { 0u8 }, i));
    }
    events.sort_unstable();
    let mut pnl = vec![0.0; legs.len()];
    for (_, phase, i) in events {
        let (_, _, price, risk, r) = legs[i];
        if phase == 1 {
            let need = f * price / risk;
            lev.push(need);
            let used = if need > cap {
                capped += 1;
                f * cap / need
            } else {
                f
            };
            pnl[i] = eq * used * r;
        } else {
            eq = (eq + pnl[i]).max(0.0);
            peak = peak.max(eq);
            dd = dd.max(1.0 - eq / peak);
        }
    }
    lev.sort_by(f64::total_cmp);
    Money {
        risk_pct,
        max_leverage,
        return_pct: 100.0 * (eq - 1.0),
        max_dd_pct: 100.0 * dd,
        leverage_median: lev.get(lev.len() / 2).copied().unwrap_or(0.0),
        leverage_max: lev.last().copied().unwrap_or(0.0),
        capped: if lev.is_empty() {
            0.0
        } else {
            capped as f64 / lev.len() as f64
        },
    }
}

pub fn wilson_lo(wins: usize, n: usize) -> f64 {
    if n == 0 {
        return 0.0;
    }
    let (p, n, z) = (wins as f64 / n as f64, n as f64, 1.96);
    (p + z * z / (2.0 * n) - z * ((p * (1.0 - p) + z * z / (4.0 * n)) / n).sqrt()) / (1.0 + z * z / n)
}

/// Statistics of trades over a range `[from, to)` (Unix seconds).
pub fn stats(trades: &[Trade], from: i64, to: i64) -> Stats {
    let n = trades.len();
    let wins = trades.iter().filter(|t| t.r > 0.0).count();
    let total: f64 = trades.iter().map(|t| t.r).sum();
    let gw: f64 = trades.iter().filter(|t| t.r > 0.0).map(|t| t.r).sum();
    let gl: f64 = -trades.iter().filter(|t| t.r <= 0.0).map(|t| t.r).sum::<f64>();
    let mut by_exit: Vec<&Trade> = trades.iter().collect();
    by_exit.sort_by_key(|t| t.exit_time);
    let (mut eq, mut peak, mut dd, mut streak, mut worst) = (0.0f64, 0.0f64, 0.0f64, 0, 0);
    for t in by_exit {
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
    let daily = daily_r(trades, from, to);
    let q = Quick {
        n,
        wins,
        sum: total,
        sumsq: trades.iter().map(|t| t.r * t.r).sum(),
        days: calendar_days(from, to),
        dsum: daily.iter().sum(),
        dsumsq: daily.iter().map(|x| x * x).sum(),
    };
    Stats {
        trades: n,
        wins,
        win_rate: q.win_rate(),
        win_rate_lo: wilson_lo(wins, n),
        avg_r: q.mean(),
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
        trades_per_week: n as f64 / ((to - from) as f64 / 604_800.0).max(1e-9),
        trades_per_day: q.per_day(),
        sharpe: q.sharpe(),
        daily_sharpe: q.daily_sharpe(),
        touches: 0,
    }
}

/// A potential entry point for the chart: the bar, side, level and calibrated win probability.
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
    /// (time, cumulative R) after every trade, by exit time.
    pub equity: Vec<(i64, f64)>,
    pub by_kind: BTreeMap<String, Stats>,
    pub by_session: BTreeMap<String, Stats>,
    pub by_month: BTreeMap<String, Stats>,
    pub money: Money,
    /// Share of touches with a probability (the first months only train the model).
    pub scored_from: i64,
    pub params: Params,
}

/// A level price is near: the order the strategy would rest there, with its probability.
#[derive(Clone, Debug, Serialize)]
pub struct ExpectedEntry {
    pub dir: i8,
    pub level: f64,
    pub entry: f64,
    pub sl: f64,
    pub tp: f64,
    pub kinds: u16,
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
    /// Touches the current score model was trained on.
    pub trained_on: usize,
}

/// The fixed trade used to label touches for the score model: a limit at the level (or a close
/// entry), stop 0.5 ATR beyond, target 1 R, 24 bars, no costs. It measures "did the level hold".
fn reference(close_entry: bool) -> Params {
    Params {
        entry: if close_entry { "close" } else { "limit" }.into(),
        sl_atr: if close_entry { 0.2 } else { 0.5 },
        tp_r: 1.0,
        max_bars: 24,
        maker_bps: 0.0,
        taker_bps: 0.0,
        slippage: 0.0,
        spread: 0.0,
        fill_through: 0.0,
        min_risk_atr: 0.0,
        max_risk_atr: 1e9,
        ..Params::default()
    }
}

pub struct Engine {
    pub bars: Arc<Vec<Bar>>,
    /// 1m candles for the order of events inside 5m bars (may be empty).
    pub minutes: Arc<Vec<Minute>>,
    /// 1-second candles for the position engine (may be empty).
    pub seconds: Arc<Vec<Sec>>,
    pub touches: Vec<Touch>,
    /// Expected touches after the last bar.
    pub pending: Vec<Touch>,
    /// Walk-forward score of each touch (probability the reference trade wins); NaN in the
    /// first months, which only train.
    pub scores: Vec<f64>,
    pub close_entry: bool,
    pub scan: ScanConfig,
    month_of: Vec<usize>,
    cache: Mutex<HashMap<String, Arc<Sim>>>,
}

impl Engine {
    pub fn new(bars: Arc<Vec<Bar>>, scan: &ScanConfig, close_entry: bool) -> Engine {
        Engine::build(bars, Arc::new(Vec::new()), scan, close_entry)
    }

    pub fn build(bars: Arc<Vec<Bar>>, minutes: Arc<Vec<Minute>>, scan: &ScanConfig, close_entry: bool) -> Engine {
        let (touches, pending) = scan_full(&bars, scan);
        Engine::with_touches(bars, minutes, touches, pending, scan, close_entry)
    }

    pub fn with_touches(
        bars: Arc<Vec<Bar>>,
        minutes: Arc<Vec<Minute>>,
        touches: Vec<Touch>,
        pending: Vec<Touch>,
        scan: &ScanConfig,
        close_entry: bool,
    ) -> Engine {
        let months: Vec<String> = touches.iter().map(|t| month_id(t.time)).collect();
        let mut month_of = Vec::with_capacity(touches.len());
        let mut k = 0usize;
        for (i, m) in months.iter().enumerate() {
            if i > 0 && *m != months[i - 1] {
                k += 1;
            }
            month_of.push(k);
        }
        let mut e = Engine {
            bars,
            minutes,
            seconds: Arc::new(Vec::new()),
            touches,
            pending,
            scores: Vec::new(),
            close_entry,
            scan: *scan,
            month_of,
            cache: Mutex::new(HashMap::new()),
        };
        e.scores = e.walk_forward_scores();
        e
    }

    /// Reuse the scanned touches, but fit scores on the requested causal entry basis.
    pub(crate) fn for_entry_mode(&self, close_entry: bool) -> Engine {
        Engine::with_touches(
            self.bars.clone(),
            self.minutes.clone(),
            self.touches.clone(),
            self.pending.clone(),
            &self.scan,
            close_entry,
        )
        .with_seconds(self.seconds.clone())
    }

    /// Adds 1-second candles for settings with `sec_engine` on.
    pub fn with_seconds(mut self, seconds: Arc<Vec<Sec>>) -> Engine {
        self.seconds = seconds;
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).clear();
        self
    }

    /// One trade of a touch: the 1-second position engine when `sec_engine` is on, else 5m bars
    /// with 1m candles.
    pub fn simulate(&self, t: &Touch, p: &Params) -> Option<(Trade, usize)> {
        if p.sec_engine {
            simulate_secs(&self.bars, &self.seconds, t, p)
        } else {
            simulate_in(&self.bars, &self.minutes, t, p)
        }
    }

    fn metrics_row(&self, t: &Touch) -> super::model::Row {
        row(if self.close_entry { &t.f } else { &t.pre }, t.dir)
    }

    fn reference_labels(&self) -> Vec<Option<bool>> {
        let rp = reference(self.close_entry);
        self.touches
            .iter()
            .map(|t| simulate_in(&self.bars, &self.minutes, t, &rp).map(|(tr, _)| tr.r > 0.0))
            .collect()
    }

    fn fit_on(&self, idx: &[usize], labels: &[Option<bool>]) -> Option<Model> {
        let idx: Vec<usize> = idx.iter().copied().filter(|&i| labels[i].is_some()).collect();
        let idx = &idx[idx.len().saturating_sub(MAX_TRAIN_ROWS)..];
        let xs: Vec<_> = idx.iter().map(|&i| self.metrics_row(&self.touches[i])).collect();
        let ys: Vec<bool> = idx.iter().map(|&i| labels[i].unwrap_or(false)).collect();
        Model::fit(&xs, &ys)
    }

    /// Month k is scored by a model trained on months k−4 … k−1.
    fn walk_forward_scores(&self) -> Vec<f64> {
        let n = self.touches.len();
        let mut scores = vec![f64::NAN; n];
        let labels = self.reference_labels();
        let months = self.month_of.last().map_or(0, |m| m + 1);
        for k in 2..months {
            let Some(first) = (0..n).find(|&i| self.month_of[i] == k) else {
                continue;
            };
            // Purge: a label must be settled before month k starts (reference trade = 24 bars).
            let cut = self.touches[first].i;
            let train: Vec<usize> = (0..n)
                .filter(|&i| self.month_of[i] < k && self.month_of[i] + TRAIN_MONTHS >= k)
                .filter(|&i| self.touches[i].i + 25 <= cut)
                .collect();
            if let Some(m) = self.fit_on(&train, &labels) {
                for i in (0..n).filter(|&i| self.month_of[i] == k) {
                    scores[i] = m.predict(&self.metrics_row(&self.touches[i]));
                }
            }
        }
        scores
    }

    /// Start of the first month with calibrated probabilities: the month after the first scored
    /// one (calibration learns from scored months only).
    pub fn scored_from(&self) -> i64 {
        let Some(k0) = self.scores.iter().position(|s| s.is_finite()).map(|i| self.month_of[i]) else {
            return i64::MAX;
        };
        self.month_of
            .iter()
            .position(|&m| m > k0)
            .map_or(i64::MAX, |i| self.touches[i].time)
    }

    fn key(p: &Params) -> String {
        // Lossless identity: nearby accepted settings must never share outcomes.
        let values = [
            p.spread,
            p.sl_atr,
            p.tp_r,
            p.maker_bps,
            p.taker_bps,
            p.slippage,
            p.fill_through,
            p.min_risk_atr,
            p.max_risk_atr,
            p.abs_window,
            p.abs_vol,
            p.abs_hold_atr,
            p.abs_confirm_atr,
            p.abs_stop_atr,
            p.abs_wait,
            p.abs_limit_atr,
            p.dens_eat,
            p.be_r,
            p.trail_atr,
            p.trail_from_r,
            p.part_frac,
            p.part_r,
            p.eat_vol,
            p.flip_sl_atr,
            p.flip_tp_r,
        ];
        let mut key = format!(
            "{}|{}|{}|{}|{}|{}|{}",
            p.max_bars,
            p.on_close(),
            p.sec_engine,
            p.absorb,
            p.abs_limit,
            p.flip,
            values.len()
        );
        for v in values {
            key.push_str(&format!("|{:016x}", v.to_bits()));
        }
        key
    }

    /// Outcomes and calibrated probabilities of every touch for `p`'s trade setting (cached).
    pub fn sim(&self, p: &Params) -> Arc<Sim> {
        let key = Engine::key(p);
        if let Some(s) = self.cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
            return s.clone();
        }
        let one = |t: &Touch| match self.simulate(t, p) {
            Some((tr, j)) => (tr.r as f32, (j as u32, tr.exit_time)),
            None => (f32::NAN, (t.i as u32, t.time)),
        };
        // The 1-second engine is ~100× slower per touch; spread it over the cores.
        let (r, settled): (Vec<f32>, Vec<(u32, i64)>) = if p.sec_engine {
            use rayon::prelude::*;
            self.touches.par_iter().map(one).unzip()
        } else {
            self.touches.iter().map(one).unzip()
        };
        let (exit, exit_time): (Vec<u32>, Vec<i64>) = settled.into_iter().unzip();
        // A cache key distinguishes the entry mode, but its model must also do so:
        // close-bar features are unavailable to a resting limit at the bar's start.
        let prob = if p.on_close() == self.close_entry {
            self.calibrate(&r, &exit)
        } else {
            self.for_entry_mode(p.on_close()).calibrate(&r, &exit)
        };
        let sim = Arc::new(Sim {
            r,
            exit,
            exit_time,
            prob,
        });
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if cache.len() >= CACHE_ENTRIES {
            cache.clear();
        }
        cache.insert(key, sim.clone());
        sim
    }

    /// Walk-forward calibration: in month k, a score maps to the win rate (net of costs, for
    /// this trade setting) of touches with similar scores in months k−4 … k−1. Bins of equal
    /// count, a Beta prior toward the window's win rate, then pool-adjacent-violators so a higher
    /// score never means a lower probability. Too little history: the raw score is used.
    fn calibrate(&self, r: &[f32], exit: &[u32]) -> Vec<f32> {
        let n = self.touches.len();
        let mut prob = vec![f32::NAN; n];
        let months = self.month_of.last().map_or(0, |m| m + 1);
        let mut start = 0usize;
        for k in 0..months {
            let end = start + self.month_of[start..].iter().take_while(|&&m| m == k).count();
            let cut = self.touches.get(start).map_or(usize::MAX, |t| t.i) as u32;
            // Purged: only trades that closed before month k starts.
            let mut train: Vec<(f64, bool)> = (0..start)
                .rev()
                .take_while(|&i| self.month_of[i] + TRAIN_MONTHS >= k)
                .filter(|&i| self.scores[i].is_finite() && r[i].is_finite() && exit[i] < cut)
                .map(|i| (self.scores[i], r[i] > 0.0))
                .collect();
            // Without calibration history the probability is unknown: raw scores measure the
            // reference trade, not this setting, and would overstate it.
            if train.len() >= 150 {
                let map = Calibration::fit(&mut train);
                for (slot, &score) in prob[start..end].iter_mut().zip(&self.scores[start..end]) {
                    *slot = map.apply(score) as f32;
                }
            }
            start = end;
        }
        prob
    }

    /// Touches taken in `[from_i, to_i)` (bar indices) and touches that passed all filters.
    pub fn select(&self, p: &Params, sim: &Sim, from_i: usize, to_i: usize) -> (Vec<usize>, Vec<usize>) {
        self.select_labels(p, sim, from_i, to_i, false)
    }

    /// GA windows admit only settled labels. Apply the cutoff before selection:
    /// unavailable outcomes must not reserve slots or enter the daily loss ledger.
    fn select_labels(
        &self,
        p: &Params,
        sim: &Sim,
        from_i: usize,
        to_i: usize,
        settled_only: bool,
    ) -> (Vec<usize>, Vec<usize>) {
        let mask = p.kind_mask();
        let filters = p.compiled();
        let max_open = p.max_open.max(1);
        let mut open: Vec<(u32, f64, i8)> = Vec::new();
        // Daily loss ledger is keyed by realization day, not entry day. Keep
        // pending overnight exits until their day arrives; only closed labels count.
        let mut today: Vec<(i64, i64, f64)> = Vec::new();
        let (mut taken, mut passed) = (Vec::new(), Vec::new());
        let lo = self.touches.partition_point(|t| t.i < from_i);
        for k in lo..self.touches.len() {
            let t = &self.touches[k];
            if t.i >= to_i {
                break;
            }
            if settled_only && sim.exit[k] as usize >= to_i {
                continue;
            }
            if !p.passes_with(t, mask, &filters) {
                continue;
            }
            let pr = sim.prob[k];
            if p.use_model && (pr.is_nan() || (pr as f64) < p.min_prob) {
                continue;
            }
            passed.push(k);
            if sim.r[k].is_nan() {
                continue;
            }
            open.retain(|o| o.0 as usize >= t.i);
            if open.len() >= max_open || open.iter().any(|o| o.2 == t.dir && (o.1 - t.level).abs() < 1e-9) {
                continue;
            }
            if p.day_stop_r > 0.0 {
                let day = t.time.div_euclid(86_400);
                today.retain(|x| x.0 >= day);
                let lost: f64 = today.iter().filter(|x| x.0 == day && x.1 <= t.time).map(|x| x.2).sum();
                if lost <= -p.day_stop_r {
                    continue;
                }
                if p.sec_engine && p.flip {
                    if let Some((tr, _)) = self.simulate(t, p) {
                        for (time, r) in realized_legs(&tr) {
                            today.push((time.div_euclid(86_400), time, r));
                        }
                    }
                } else {
                    today.push((sim.exit_time[k].div_euclid(86_400), sim.exit_time[k], sim.r[k] as f64));
                }
            }
            open.push((sim.exit[k], t.level, t.dir));
            taken.push(k);
        }
        (taken, passed)
    }

    fn range_times(&self, from_i: usize, to_i: usize) -> (i64, i64) {
        let b = &self.bars;
        if b.is_empty() {
            return (0, 0);
        }
        let a = b[from_i.min(b.len() - 1)].time;
        let z = b[to_i.clamp(1, b.len()) - 1].time + 300;
        (a, z)
    }

    /// Totals for the GA: no allocation of trade records.
    pub fn quick(&self, p: &Params, from_i: usize, to_i: usize) -> Quick {
        let sim = self.sim(p);
        let (taken, _) = self.select_labels(p, &sim, from_i, to_i, true);
        let (a, z) = self.range_times(from_i, to_i);
        let mut q = Quick {
            days: calendar_days(a, z),
            ..Quick::default()
        };
        let d0 = a.div_euclid(86_400);
        let mut daily = vec![0.0f64; q.days as usize];
        for k in taken {
            let r = sim.r[k] as f64;
            q.n += 1;
            q.wins += usize::from(r > 0.0);
            q.sum += r;
            q.sumsq += r * r;
            let d = (self.touches[k].time.div_euclid(86_400) - d0).clamp(0, daily.len() as i64 - 1);
            daily[d as usize] += r;
        }
        q.dsum = daily.iter().sum();
        q.dsumsq = daily.iter().map(|x| x * x).sum();
        q
    }

    /// Settled training records for GA drawdown/downside evaluation. Like
    /// `quick`, purge exits at or beyond `to_i` BEFORE selection so unavailable
    /// labels cannot consume open slots or influence daily stops. Simulation
    /// uses the full tape; crossing trades are excluded, never forcibly closed.
    pub fn training_trades(&self, p: &Params, from_i: usize, to_i: usize) -> Vec<Trade> {
        let sim = self.sim(p);
        let (taken, _) = self.select_labels(p, &sim, from_i, to_i, true);
        taken
            .into_iter()
            .filter_map(|k| {
                let (mut tr, exit) = self.simulate(&self.touches[k], p)?;
                if exit >= to_i {
                    return None;
                }
                tr.prob = sim.prob[k] as f64;
                Some(tr)
            })
            .collect()
    }

    /// Filtered, simulated candidates without portfolio admission. Needed when
    /// one chronological admission ledger spans several walk-forward folds.
    pub fn candidate_trades(&self, p: &Params, from_i: usize, to_i: usize) -> Vec<Trade> {
        let sim = self.sim(p);
        let mask = p.kind_mask();
        let filters = p.compiled();
        let lo = self.touches.partition_point(|t| t.i < from_i);
        self.touches
            .iter()
            .enumerate()
            .skip(lo)
            .take_while(|(_, t)| t.i < to_i)
            .filter_map(|(k, t)| {
                if !p.passes_with(t, mask, &filters) {
                    return None;
                }
                let prob = sim.prob[k];
                if p.use_model && (!prob.is_finite() || (prob as f64) < p.min_prob) {
                    return None;
                }
                let (mut tr, _) = self.simulate(t, p)?;
                tr.prob = prob as f64;
                Some(tr)
            })
            .collect()
    }

    /// Full trade records of the touches taken in a range. This entry-window
    /// API intentionally permits exits beyond the range for OOS carry. Training
    /// fitness callers must use `training_trades` instead.
    pub fn trades(&self, p: &Params, from_i: usize, to_i: usize) -> Vec<Trade> {
        let sim = self.sim(p);
        let (taken, _) = self.select(p, &sim, from_i, to_i);
        taken
            .into_iter()
            .filter_map(|k| {
                self.simulate(&self.touches[k], p).map(|(mut tr, _)| {
                    tr.prob = sim.prob[k] as f64;
                    tr
                })
            })
            .collect()
    }

    pub fn index_at(&self, time: i64) -> usize {
        self.bars.partition_point(|b| b.time < time)
    }

    pub fn report(&self, p: &Params) -> Report {
        if p.on_close() != self.close_entry {
            return self.for_entry_mode(p.on_close()).report(p);
        }
        let n = self.bars.len();
        let sim = self.sim(p);
        let (taken, passed) = self.select(p, &sim, 0, n);
        let trades = self.trades(p, 0, n);
        // With the model, statistics cover the months that have probabilities.
        let from_i = if p.use_model {
            self.index_at(self.scored_from()).min(n.saturating_sub(1))
        } else {
            0
        };
        let (from, to) = self.range_times(from_i, n);
        let mut st = stats(&trades, from, to);
        st.touches = self.touches.len();
        let taken: HashSet<usize> = taken.into_iter().collect();
        let signals = passed
            .iter()
            .map(|&k| {
                let t = &self.touches[k];
                SignalMark {
                    time: t.time,
                    dir: t.dir,
                    price: if p.on_close() { self.bars[t.i].close } else { t.fill },
                    level: t.level,
                    prob: sim.prob[k] as f64,
                    taken: taken.contains(&k),
                }
            })
            .collect();
        let mut by_exit: Vec<&Trade> = trades.iter().collect();
        by_exit.sort_by_key(|t| t.exit_time);
        let mut eq = 0.0;
        let equity = by_exit
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
            m.into_iter().map(|(k, v)| (k, stats(&v, from, to))).collect()
        };
        let by_month: BTreeMap<String, Stats> = {
            let mut m: BTreeMap<String, Vec<Trade>> = BTreeMap::new();
            for t in &trades {
                m.entry(month_id(t.entry_time)).or_default().push(t.clone());
            }
            m.into_iter()
                .map(|(k, v)| {
                    let a = v.first().map_or(from, |t| t.entry_time);
                    let z = v.last().map_or(to, |t| t.exit_time);
                    (k, stats(&v, a.max(from), z.min(to).max(a + 1)))
                })
                .collect()
        };
        Report {
            from,
            to,
            bars: n,
            by_kind: group(&|t| {
                KINDS
                    .iter()
                    .filter(|k| t.kinds & k.bit() != 0)
                    .map(|k| k.id().to_string())
                    .collect()
            }),
            by_session: group(&|t| vec![session_id(t.entry_time / 3600).to_string()]),
            by_month,
            money: money(&trades, p.risk_pct, p.max_leverage),
            stats: st,
            trades,
            signals,
            equity,
            scored_from: self.scored_from(),
            params: p.clone(),
        }
    }

    /// Expected entries after the last bar: a score model trained on the last 4 months, and the
    /// calibration of the last 4 months for `p`'s trade setting.
    pub fn live(&self, p: &Params) -> LiveReport {
        if p.on_close() != self.close_entry {
            return self.for_entry_mode(p.on_close()).live(p);
        }
        let last = self.bars.last().copied().unwrap_or(Bar::ohlcv(0, 0., 0., 0., 0., 0.));
        let n = self.touches.len();
        let since = last.time - 122 * 86_400;
        let recent: Vec<usize> = (0..n).filter(|&i| self.touches[i].time >= since).collect();
        let labels = self.reference_labels();
        let model = if p.use_model {
            self.fit_on(&recent, &labels)
        } else {
            None
        };
        let sim = self.sim(p);
        let mut pairs: Vec<(f64, bool)> = recent
            .iter()
            .filter(|&&i| self.scores[i].is_finite() && sim.r[i].is_finite())
            .map(|&i| (self.scores[i], sim.r[i] > 0.0))
            .collect();
        let cal = (pairs.len() >= 150).then(|| Calibration::fit(&mut pairs));
        let mask = p.kind_mask();
        let filters = p.compiled();
        let entries = self
            .pending
            .iter()
            .map(|t| {
                let d = t.dir as f64;
                let sl = t.level - d * p.sl_atr * t.atr;
                let risk = d * (t.fill - sl);
                let score = model.as_ref().map_or(f64::NAN, |m| m.predict(&self.metrics_row(t)));
                let prob = cal.as_ref().map_or(f64::NAN, |c| c.apply(score));
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
            atr: self.pending.first().or(self.touches.last()).map_or(f64::NAN, |t| t.atr),
            entries,
            trained_on: recent.len(),
        }
    }
}

/// Monotone score → probability map.
pub(crate) struct Calibration {
    /// Upper score edge of each bin and its probability.
    edges: Vec<f64>,
    probs: Vec<f64>,
}

impl Calibration {
    pub(crate) fn fit(pairs: &mut [(f64, bool)]) -> Calibration {
        pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
        let n = pairs.len();
        let base = pairs.iter().filter(|p| p.1).count() as f64 / n as f64;
        let bins = (n / 60).clamp(3, 12);
        let mut edges = Vec::new();
        let mut stats: Vec<(f64, f64)> = Vec::new(); // (wins + prior, count + prior)
        for b in 0..bins {
            let (a, z) = (b * n / bins, (b + 1) * n / bins);
            let w = pairs[a..z].iter().filter(|p| p.1).count() as f64;
            stats.push((w + 4.0 * base, (z - a) as f64 + 4.0));
            edges.push(pairs[z - 1].0);
        }
        // Pool adjacent violators: merge bins until probabilities are non-decreasing.
        let mut blocks: Vec<(f64, f64, usize)> = Vec::new(); // (wins, count, bins merged)
        for s in stats {
            blocks.push((s.0, s.1, 1));
            while blocks.len() > 1 {
                let k = blocks.len();
                if blocks[k - 2].0 / blocks[k - 2].1 > blocks[k - 1].0 / blocks[k - 1].1 {
                    let top = blocks.pop().unwrap_or((0.0, 0.0, 0));
                    let prev = blocks.last_mut().expect("two blocks");
                    prev.0 += top.0;
                    prev.1 += top.1;
                    prev.2 += top.2;
                } else {
                    break;
                }
            }
        }
        let mut probs = Vec::with_capacity(bins);
        for (w, c, m) in blocks {
            probs.extend(std::iter::repeat_n(w / c, m));
        }
        Calibration { edges, probs }
    }

    pub(crate) fn apply(&self, score: f64) -> f64 {
        if !score.is_finite() {
            return f64::NAN;
        }
        let k = self.edges.partition_point(|&e| e < score).min(self.probs.len() - 1);
        self.probs[k]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn wave_bars(n: usize) -> Vec<Bar> {
        (0..n)
            .map(|i| {
                let x = 2000.0 + 15.0 * ((i as f64) / 20.0).sin() + 5.0 * ((i as f64) / 3.3).cos();
                let mut b = Bar::ohlcv(i as i64 * 300, x, x + 1.5, x - 1.5, x + 0.3, 10.0);
                b.buy_volume = 5.0 + ((i % 7) as f64 - 3.0);
                b.trades = 5.0;
                b
            })
            .collect()
    }

    /// Neutral settings for synthetic data: every level kind, no filters, no model.
    fn plain() -> Params {
        Params {
            use_model: false,
            max_open: 1,
            kinds: KINDS.iter().map(|k| (k.id().to_string(), true)).collect(),
            sessions: Default::default(),
            filters: Default::default(),
            scan: ScanConfig::default(),
            spread: 0.0,
            sl_atr: 0.5,
            tp_r: 1.0,
            ..Params::default()
        }
    }

    /// Every setting the simulation reads must be in the cache key, or changing it reuses the
    /// outcomes of the previous value (the GA and the trade selection would not see it).
    #[test]
    fn every_simulated_setting_is_in_the_cache_key() {
        // Applied after the simulation (selection, money view) or by a new level scan.
        let after = |id: &str| {
            id.starts_with("kinds.")
                || id.starts_with("sessions.")
                || id.starts_with("scan.")
                || matches!(
                    id,
                    "long"
                        | "short"
                        | "use_model"
                        | "min_prob"
                        | "max_open"
                        | "day_stop_r"
                        | "risk_pct"
                        | "max_leverage"
                )
        };
        let base = Params {
            sec_engine: true,
            ..Params::default()
        };
        for s in super::super::param_specs() {
            if s.kind == "filter" || after(&s.id) {
                continue;
            }
            let mut p = base.clone();
            let v = p.get(&s.id).unwrap();
            p.set(&s.id, if (v - s.hi).abs() > 1e-9 { s.hi } else { s.lo });
            assert_ne!(Engine::key(&p), Engine::key(&base), "{} is not in the cache key", s.id);
        }
    }

    /// Stop and target inside one 5m bar: the 1m candles decide the order.
    #[test]
    fn minutes_decide_the_order_inside_a_bar() {
        let touch = Touch {
            i: 0,
            time: 0,
            dir: 1,
            level: 100.0,
            kinds: 1,
            atr: 2.0,
            fill: 100.0,
            f: [f64::NAN; super::super::features::NF],
            pre: [f64::NAN; super::super::features::NF],
        };
        let p = Params {
            sl_atr: 0.5,
            tp_r: 1.0,
            spread: 0.0,
            slippage: 0.0,
            maker_bps: 0.0,
            taker_bps: 0.0,
            fill_through: 0.0,
            ..Params::default()
        };
        // Touch bar fills at 100 (stop 99, target 101); the next bar spans both and closes up.
        let bars = vec![
            Bar::ohlcv(0, 100.5, 100.6, 99.8, 100.2, 1.0),
            Bar::ohlcv(300, 100.2, 101.5, 98.5, 101.2, 1.0),
            Bar::ohlcv(600, 101.2, 101.3, 101.0, 101.1, 1.0),
        ];
        // Without minutes an up bar is assumed to reach its low first.
        assert_eq!(simulate(&bars, &touch, &p).unwrap().0.outcome, "sl");
        let m = |time, open, high, low, close| Minute {
            time,
            open,
            high,
            low,
            close,
        };
        let mins = vec![
            m(300, 100.2, 101.5, 100.1, 101.0),
            m(360, 101.0, 101.1, 98.5, 99.0),
            m(420, 99.0, 101.2, 99.0, 101.2),
        ];
        assert_eq!(simulate_in(&bars, &mins, &touch, &p).unwrap().0.outcome, "tp");
    }

    #[test]
    fn calibration_is_monotone_and_bounded() {
        let mut pairs: Vec<(f64, bool)> = (0..1000)
            .map(|i| (i as f64 / 1000.0, (i * 7919) % 100 < i / 10))
            .collect();
        let c = Calibration::fit(&mut pairs);
        let mut last = 0.0;
        for s in 0..=20 {
            let p = c.apply(s as f64 / 20.0);
            assert!((0.0..=1.0).contains(&p));
            assert!(p >= last - 1e-12, "not monotone at {s}");
            last = p;
        }
        assert!(c.apply(0.95) > c.apply(0.05) + 0.3);
    }

    #[test]
    fn trades_are_consistent_and_positions_respect_the_limit() {
        let e = Engine::new(Arc::new(wave_bars(6000)), &ScanConfig::default(), false);
        for max_open in [1usize, 3] {
            let p = Params { max_open, ..plain() };
            let r = e.report(&p);
            assert!(r.stats.trades > 10);
            let q = e.quick(&p, 0, e.bars.len());
            assert_eq!(q.n, r.stats.trades);
            assert!((q.sum - r.stats.total_r).abs() < 1e-3);
            // Never more than max_open positions at a time.
            for t in &r.trades {
                let open = r
                    .trades
                    .iter()
                    .filter(|o| o.entry_time <= t.entry_time && o.exit_time > t.entry_time)
                    .count();
                assert!(open <= max_open, "{open} > {max_open}");
            }
        }
        let one = e.quick(&plain(), 0, e.bars.len()).n;
        let three = e.quick(&Params { max_open: 3, ..plain() }, 0, e.bars.len()).n;
        assert!(three > one, "{three} <= {one}");
    }

    #[test]
    fn costs_and_spread_lower_the_result() {
        let mut bars = wave_bars(4000);
        let e = Engine::new(Arc::new(bars.clone()), &ScanConfig::default(), false);
        let free = Params {
            maker_bps: 0.0,
            taker_bps: 0.0,
            slippage: 0.0,
            ..plain()
        };
        let a = e.quick(&free, 0, bars.len()).sum;
        let b = e
            .quick(
                &Params {
                    taker_bps: 5.0,
                    ..free.clone()
                },
                0,
                bars.len(),
            )
            .sum;
        assert!(b < a);
        for x in &mut bars {
            x.spread = 0.3;
        }
        let e2 = Engine::new(Arc::new(bars.clone()), &ScanConfig::default(), false);
        assert!(e2.quick(&free, 0, bars.len()).sum < a);
    }

    fn test_touch(i: usize, dir: i8, level: f64, atr: f64) -> Touch {
        Touch {
            i,
            time: i as i64 * 300,
            dir,
            level,
            kinds: 1,
            atr,
            fill: level,
            f: [f64::NAN; super::super::NF],
            pre: [f64::NAN; super::super::NF],
        }
    }

    fn test_engine(bars: Vec<Bar>, touches: Vec<Touch>) -> Engine {
        let mut e = Engine::new(Arc::new(bars), &ScanConfig::default(), false);
        e.scores = vec![f64::NAN; touches.len()];
        e.month_of = vec![0; touches.len()];
        e.touches = touches;
        e
    }

    #[test]
    fn quick_cutoff_is_invariant_to_mutated_post_cutoff_bars() {
        let mut bars: Vec<_> = (0..10)
            .map(|i| Bar::ohlcv(i * 300, 100.0, 100.1, 99.9, 100.0, 1.0))
            .collect();
        bars[2] = Bar::ohlcv(600, 100.0, 101.2, 100.0, 101.0, 1.0);
        let p = Params {
            tp_r: 2.0,
            max_bars: 9,
            max_open: 1,
            day_stop_r: 1.0,
            maker_bps: 0.0,
            taker_bps: 0.0,
            slippage: 0.0,
            fill_through: 0.0,
            ..plain()
        };
        // Earlier short remains open across the cutoff; the later long settles
        // inside it. The unavailable short must not consume its selection slot.
        let touches = vec![test_touch(0, -1, 100.0, 10.0), test_touch(1, 1, 100.0, 1.0)];
        let mut loss_bars = bars.clone();
        loss_bars[7] = Bar::ohlcv(2100, 106.0, 106.1, 105.9, 106.0, 1.0);
        let mut win_bars = bars;
        win_bars[7] = Bar::ohlcv(2100, 89.0, 89.1, 88.9, 89.0, 1.0);
        let a = test_engine(loss_bars, touches.clone());
        let b = test_engine(win_bars, touches);
        let sa = a.sim(&p);
        let sb = b.sim(&p);
        assert!(sa.exit[0] >= 6 && sb.exit[0] >= 6);
        assert!(
            sa.r[0] < 0.0 && sb.r[0] > 0.0,
            "mutation must change the unavailable label"
        );
        let qa = a.quick(&p, 0, 6);
        let qb = b.quick(&p, 0, 6);
        assert_eq!(qa.n, 1);
        assert!((qa.sum - 2.0).abs() < 1e-6);
        assert_eq!(
            (qa.n, qa.wins, qa.sum, qa.sumsq, qa.dsum, qa.dsumsq),
            (qb.n, qb.wins, qb.sum, qb.sumsq, qb.dsum, qb.dsumsq)
        );
        assert_eq!(a.select_labels(&p, &sa, 0, 6, true).0, vec![1]);
        assert_eq!(sa.exit[1], 2);
        assert!(a.training_trades(&p, 0, 2).is_empty());
        assert_eq!(a.quick(&p, 0, 2).n, 0);
        let ta = a.training_trades(&p, 0, 6);
        let tb = b.training_trades(&p, 0, 6);
        assert_eq!(ta.len(), qa.n);
        assert_eq!(ta.len(), tb.len());
        for (left, right) in ta.iter().zip(&tb) {
            assert_eq!(
                (left.entry_time, left.exit_time, left.r, left.outcome),
                (right.entry_time, right.exit_time, right.r, right.outcome)
            );
            assert!(left.exit_time <= a.bars[5].time + 300);
        }
        // Preserve the explicit OOS carry behavior of the old entry-window API.
        let carry_a = a.trades(&p, 0, 6);
        let carry_b = b.trades(&p, 0, 6);
        assert_eq!(carry_a.len(), 1);
        assert!(carry_a[0].exit_time > a.bars[5].time + 300);
        assert!(carry_a[0].r < 0.0 && carry_b[0].r > 0.0);
        // Boundary is strict, not <=: an exit on to_i is unavailable.
        let mut boundary = Sim {
            r: sa.r.clone(),
            exit: sa.exit.clone(),
            exit_time: sa.exit_time.clone(),
            prob: sa.prob.clone(),
        };
        boundary.exit[1] = 6;
        assert!(a.select_labels(&p, &boundary, 0, 6, true).0.is_empty());
    }

    #[test]
    fn overnight_loss_stops_realization_day_and_resets_next_day() {
        let e = test_engine(
            wave_bars(600),
            vec![
                test_touch(287, 1, 100.0, 1.0),
                test_touch(289, 1, 101.0, 1.0),
                test_touch(578, 1, 102.0, 1.0),
            ],
        );
        let p = Params {
            day_stop_r: 0.5,
            ..plain()
        };
        let sim = Sim {
            r: vec![-1.0, 1.0, 1.0],
            exit: vec![288, 290, 579],
            exit_time: vec![86_401, 87_300, 174_000],
            prob: vec![f32::NAN; 3],
        };
        assert_eq!(e.select(&p, &sim, 0, 600).0, vec![0, 2]);
        assert_eq!(e.select_labels(&p, &sim, 0, 600, true).0, vec![0, 2]);
    }

    fn money_trade(start: i64, end: i64, dir: i8, r: f64) -> Trade {
        Trade {
            entry_time: start,
            exit_time: end,
            first_exit_time: end,
            dir,
            entry: 100.0,
            sl: 100.0 - dir as f64,
            r,
            ..Trade::empty()
        }
    }

    #[test]
    fn cache_keys_do_not_round_distinct_simulation_parameters() {
        let a = Params::default();
        for id in ["sl_atr", "tp_r", "spread", "be_r", "part_r", "abs_vol"] {
            let mut b = a.clone();
            let value = b.get(id).unwrap();
            b.set(id, value + 0.000001);
            assert_ne!(Engine::key(&a), Engine::key(&b), "{id}");
        }
    }

    #[test]
    fn first_flip_loss_is_realized_before_profitable_flip_exit() {
        let mut t = money_trade(0, 100000, 1, 1.0);
        t.first_exit_time = 80000;
        t.flip_entry = 100.0;
        t.flip_r = 2.0;
        let events = realized_legs(&t);
        assert_eq!(events, vec![(80000, -1.0), (100000, 2.0)]);
        let before_flip_close: f64 = events.iter().filter(|(time, _)| *time <= 85000).map(|(_, r)| r).sum();
        assert!(before_flip_close <= -0.5);
        assert_ne!(events[0].0.div_euclid(86400), events[1].0.div_euclid(86400));
    }

    #[test]
    fn money_same_second_outcomes_are_not_silently_dropped() {
        for dir in [1, -1] {
            let win = money(&[money_trade(5, 5, dir, 1.0)], 10.0, 100.0);
            let loss = money(&[money_trade(5, 5, dir, -1.0)], 10.0, 100.0);
            assert!((win.return_pct - 10.0).abs() < 1e-9);
            assert!((loss.return_pct + 10.0).abs() < 1e-9);
        }
    }

    #[test]
    fn overlapping_money_risk_is_fixed_at_entry_for_both_directions() {
        for dir in [1, -1] {
            let ts = vec![money_trade(0, 3, dir, 1.0), money_trade(1, 2, dir, 1.0)];
            let m = money(&ts, 10.0, 100.0);
            assert!((m.return_pct - 20.0).abs() < 1e-9); // not 21% exit compounding
            let serial = money(&[money_trade(0, 1, dir, 1.0), money_trade(1, 2, dir, 1.0)], 10.0, 100.0);
            assert!((serial.return_pct - 21.0).abs() < 1e-9);
        }
    }

    #[test]
    fn money_flip_resizes_after_first_loss_and_caps_its_own_stop() {
        let mut t = money_trade(0, 3, 1, 0.0);
        t.first_exit_time = 1;
        t.flip_time = 2;
        t.flip_entry = 100.0;
        t.flip_sl = 100.1; // flip needs 100x at 10% risk, unlike initial 10x
        t.flip_r = 1.0;
        let m = money(&[t], 10.0, 10.0);
        // Initial -10%, then flip +1% of the remaining 0.9 = 0.009.
        assert!((m.return_pct + 9.1).abs() < 1e-9);
        assert_eq!(m.capped, 0.5); // one of two legs capped
        assert!((m.leverage_max - 100.0).abs() < 1e-8);
    }

    #[test]
    fn money_first_exit_settles_before_delayed_flip_and_other_entry() {
        let mut t = money_trade(0, 4, 1, 0.0);
        t.first_exit_time = 1;
        t.flip_time = 3;
        t.flip_entry = 100.0;
        t.flip_sl = 101.0;
        t.flip_r = 1.0;
        let m = money(&[t, money_trade(2, 5, -1, 1.0)], 10.0, 100.0);
        // Both entries after the initial loss size on 0.9, not the later exit equity.
        assert!((m.return_pct - 8.0).abs() < 1e-9);
    }

    #[test]
    fn bar_and_minute_fill_gaps_do_not_receive_stale_stop_price() {
        for dir in [1, -1] {
            let open = if dir > 0 { 98.0 } else { 102.0 };
            let bars = vec![
                Bar::ohlcv(0, open, open, open, open, 1.0),
                Bar::ohlcv(300, open, open, open, open, 1.0),
            ];
            let t = test_touch(0, dir, 100.0, 1.0);
            let p = Params {
                maker_bps: 0.0,
                taker_bps: 0.0,
                slippage: 0.1,
                fill_through: 0.0,
                ..plain()
            };
            let mins = [Minute {
                time: 0,
                open,
                high: open,
                low: open,
                close: open,
            }];
            for ms in [&[][..], &mins[..]] {
                let (tr, _) = simulate_in(&bars, ms, &t, &p).unwrap();
                assert!((tr.exit - (open - dir as f64 * 0.1)).abs() < 1e-9);
                assert!(tr.r < -4.0);
            }
        }
    }

    #[test]
    fn month_ids_and_weekdays() {
        assert_eq!(month_id(0), "1970-01");
        assert_eq!(month_id(1_767_225_600), "2026-01");
        assert_eq!(calendar_days(0, 4 * 86_400), 4.0);
        assert_eq!(calendar_days(100, 200), 1.0);
    }
    #[test]
    fn cached_limit_probabilities_ignore_close_features_and_construction_mode() {
        let mut bars: Vec<Bar> = (0..4800)
            .map(|i| {
                let time = (i / 800) as i64 * 31 * 86_400 + (i % 800) as i64 * 300;
                Bar::ohlcv(time, 100.0, 100.1, 99.9, 100.0, 1.0)
            })
            .collect();
        for i in (1..bars.len() - 25).step_by(4) {
            let time = bars[i + 1].time;
            bars[i + 1] = if (i / 4) % 2 == 0 {
                Bar::ohlcv(time, 100.0, 102.0, 99.9, 101.5, 1.0)
            } else {
                Bar::ohlcv(time, 100.0, 100.1, 98.0, 99.0, 1.0)
            };
        }
        let touches: Vec<Touch> = (1..bars.len() - 25)
            .step_by(4)
            .map(|i| {
                let mut t = test_touch(i, 1, 100.0, 1.0);
                t.time = bars[i].time;
                t.pre.fill(0.0);
                t.f.fill(if (i / 4) % 2 == 0 { 1.0 } else { -1.0 });
                t
            })
            .collect();
        let bars = Arc::new(bars);
        let make = |ts: Vec<Touch>, close| {
            Engine::with_touches(
                bars.clone(),
                Arc::new(Vec::new()),
                ts,
                Vec::new(),
                &ScanConfig::default(),
                close,
            )
        };
        let p = Params {
            entry: "limit".into(),
            use_model: true,
            ..plain()
        };
        let correct = make(touches.clone(), false).sim(&p);
        assert!(
            correct.prob.iter().any(|x| x.is_finite()),
            "fixture must exercise calibration"
        );
        let mut altered = touches.clone();
        for (i, t) in altered.iter_mut().enumerate() {
            t.f.fill((i % 7) as f64 * 1000.0);
        }
        for ts in [touches, altered] {
            let e = make(ts, true);
            let first = e.sim(&p);
            let cached = e.sim(&p);
            assert!(Arc::ptr_eq(&first, &cached));
            for (got, want) in first.prob.iter().zip(&correct.prob) {
                assert_eq!(got.to_bits(), want.to_bits());
            }
            for (got, want) in first.r.iter().zip(&correct.r) {
                assert_eq!(got.to_bits(), want.to_bits());
            }
        }
    }
}
