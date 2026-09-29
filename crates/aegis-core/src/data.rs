//! DATA strategy: smart-money rules that passed the research (`docs/research/smc_models.md`).
//!
//! Power of 3 from the midnight open: at the entry time (New York) trade on the side of the
//! midnight open (long above it, short below), stop behind the day's extreme so far, exit at
//! the exit time. One trade per weekday. On six years of XAUUSD 1m: +0.12 R per trade before
//! costs, positive in both halves; at Binance taker fees it is negative, so costs are settings.
//!
//! *Auto* re-picks the entry and exit time every month from the best of the previous months
//! (walk-forward), so every Auto trade uses settings chosen on earlier data only.

use serde::{Deserialize, Serialize};

use crate::bounce::Minute;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Manual,
    Auto,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Params {
    pub mode: Mode,
    /// Entry, minutes after New York midnight.
    pub entry_min: u32,
    /// Exit, minutes after New York midnight.
    pub exit_min: u32,
    pub long: bool,
    pub short: bool,
    /// Commission per side, % of the price (Binance taker 0.05).
    pub fee_pct: f64,
    /// Spread and slippage per trade, $ per oz.
    pub spread_usd: f64,
    /// Skip days whose stop would be closer than this, $.
    pub min_risk_usd: f64,
    /// Auto: months of history the entry and exit time are picked on.
    pub auto_months: u32,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            mode: Mode::Manual,
            entry_min: 510,
            exit_min: 960,
            long: true,
            short: true,
            fee_pct: 0.05,
            spread_usd: 0.0,
            min_risk_usd: 0.0,
            auto_months: 6,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ParamSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub help: &'static str,
    pub kind: &'static str,
    pub lo: f64,
    pub hi: f64,
    pub step: f64,
}

pub fn param_specs() -> Vec<ParamSpec> {
    let s = |id, label, help, kind, lo, hi, step| ParamSpec {
        id,
        label,
        help,
        kind,
        lo,
        hi,
        step,
    };
    vec![
        s("entry_min", "Entry time (New York)", "When the trade opens. ICT's 8:30 open was the best in the research; every time from 7:00 to 10:00 was positive before costs.", "time", 420.0, 600.0, 15.0),
        s("exit_min", "Exit time (New York)", "When an open trade is closed at market.", "time", 660.0, 990.0, 15.0),
        s("long", "Longs", "Buy when price is above the New York midnight open at the entry time.", "toggle", 0.0, 1.0, 1.0),
        s("short", "Shorts", "Sell when price is below the midnight open at the entry time.", "toggle", 0.0, 1.0, 1.0),
        s("fee_pct", "Commission per side, %", "Binance taker 0.05, Bybit taker 0.055, a CFD account usually 0 (the cost is in the spread).", "slider", 0.0, 0.1, 0.005),
        s("spread_usd", "Spread and slippage, $ per oz", "Paid once per trade. A RoboForex-type ECN account is about $0.2-0.4 with commission.", "slider", 0.0, 1.0, 0.05),
        s("min_risk_usd", "Min stop distance, $", "Skip days when the day's extreme is closer than this: costs eat small stops.", "slider", 0.0, 20.0, 0.5),
        s("auto_months", "Auto: months of history", "Auto picks the entry and exit time with the best result over this many previous months, and changes them every month.", "slider", 2.0, 12.0, 1.0),
    ]
}

#[derive(Clone, Debug, Serialize)]
pub struct Trade {
    /// New York date, days since 1970-01-01.
    pub day: i64,
    pub side: i8,
    pub entry_time: i64,
    pub exit_time: i64,
    pub entry: f64,
    pub stop: f64,
    pub exit: f64,
    /// "stop" or "time".
    pub reason: &'static str,
    pub pnl: f64,
    pub cost: f64,
    pub r_gross: f64,
    pub r: f64,
    pub entry_min: u32,
    pub exit_min: u32,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Stats {
    pub trades: usize,
    pub win_rate: f64,
    pub avg_r: f64,
    pub avg_r_gross: f64,
    pub total_r: f64,
    pub max_dd_r: f64,
    pub avg_usd: f64,
    pub median_risk_usd: f64,
    /// t-statistic of the mean R (above ~2 is unlikely to be luck).
    pub t_stat: f64,
}

pub fn stats(trades: &[Trade]) -> Stats {
    let n = trades.len();
    if n == 0 {
        return Stats::default();
    }
    let r: Vec<f64> = trades.iter().map(|t| t.r).collect();
    let mean = r.iter().sum::<f64>() / n as f64;
    let var = r.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n.max(2) - 1) as f64;
    let (mut eq, mut peak, mut dd) = (0.0f64, 0.0f64, 0.0f64);
    for x in &r {
        eq += x;
        peak = peak.max(eq);
        dd = dd.max(peak - eq);
    }
    let mut risks: Vec<f64> = trades.iter().map(|t| (t.entry - t.stop).abs()).collect();
    risks.sort_by(f64::total_cmp);
    Stats {
        trades: n,
        win_rate: r.iter().filter(|x| **x > 0.0).count() as f64 / n as f64,
        avg_r: mean,
        avg_r_gross: trades.iter().map(|t| t.r_gross).sum::<f64>() / n as f64,
        total_r: eq,
        max_dd_r: dd,
        avg_usd: trades.iter().map(|t| t.pnl - t.cost).sum::<f64>() / n as f64,
        median_risk_usd: risks[n / 2],
        t_stat: if var > 0.0 { mean / (var / n as f64).sqrt() } else { 0.0 },
    }
}

// ---- New York time -------------------------------------------------------------------------

/// Days since 1970-01-01 of a civil date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn year_of(days: i64) -> i64 {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    yoe + era * 400 + i64::from(m <= 2)
}

/// The `n`-th Sunday (1-based) of a month, as days since 1970.
fn nth_sunday(y: i64, m: i64, n: i64) -> i64 {
    let first = days_from_civil(y, m, 1);
    let dow = (first + 4).rem_euclid(7); // 0 = Sunday
    first + (7 - dow) % 7 + 7 * (n - 1)
}

/// UTC offset of New York in seconds at a UTC time (US rules since 2007).
pub fn ny_offset(utc: i64) -> i64 {
    let y = year_of(utc.div_euclid(86_400));
    // DST from 02:00 EST on the second Sunday of March to 02:00 EDT on the first Sunday of November.
    let start = nth_sunday(y, 3, 2) * 86_400 + 7 * 3600;
    let end = nth_sunday(y, 11, 1) * 86_400 + 6 * 3600;
    if utc >= start && utc < end {
        -4 * 3600
    } else {
        -5 * 3600
    }
}

/// (New York day since 1970, minute of the day, weekday 0 = Monday).
pub fn ny_clock(utc: i64) -> (i64, u32, u32) {
    let local = utc + ny_offset(utc);
    let day = local.div_euclid(86_400);
    (
        day,
        (local.rem_euclid(86_400) / 60) as u32,
        (day + 3).rem_euclid(7) as u32,
    )
}

// ---- rule ----------------------------------------------------------------------------------

/// One New York weekday of 1m candles (up to 17:00, the CFD daily break).
struct Day<'a> {
    day: i64,
    mins: &'a [Minute],
    clock: Vec<u32>,
}

fn days(mins: &[Minute]) -> Vec<Day<'_>> {
    let mut out = Vec::new();
    let mut a = 0;
    while a < mins.len() {
        let (d, _, dow) = ny_clock(mins[a].time);
        let mut b = a;
        while b < mins.len() && ny_clock(mins[b].time).0 == d {
            b += 1;
        }
        if dow < 5 {
            let slice = &mins[a..b];
            let clock = slice.iter().map(|m| ny_clock(m.time).1).collect();
            out.push(Day {
                day: d,
                mins: slice,
                clock,
            });
        }
        a = b;
    }
    out
}

fn cost(p: &Params, price: f64) -> f64 {
    2.0 * p.fee_pct / 100.0 * price + p.spread_usd
}

/// The trade of one day, if the rule trades it. `complete` = the exit time has passed.
fn trade_day(d: &Day, p: &Params, entry_min: u32, exit_min: u32) -> Option<(Trade, bool)> {
    let k = d.clock.partition_point(|&m| m < entry_min);
    // The midnight open must be in the data, and a nearly complete morning (holidays and
    // feed gaps would set a false day extreme).
    if k < 30 || k >= d.mins.len() || d.clock[0] > 5 || (k as f64) < 0.8 * f64::from(entry_min) {
        return None;
    }
    let m = d.mins;
    let px = m[k].open;
    let side: i8 = if px > m[0].open { 1 } else { -1 };
    if (side > 0 && !p.long) || (side < 0 && !p.short) {
        return None;
    }
    let (hi, lo) = m[..k]
        .iter()
        .fold((f64::MIN, f64::MAX), |(h, l), c| (h.max(c.high), l.min(c.low)));
    let stop = if side > 0 { lo } else { hi };
    let risk = (px - stop).abs();
    if risk <= 0.0 || risk < p.min_risk_usd {
        return None;
    }
    let k2 = d.clock.partition_point(|&c| c < exit_min);
    let s = f64::from(side);
    let mut exit = None;
    for (i, c) in m.iter().enumerate().take(k2).skip(k) {
        let adverse = if side > 0 { c.low } else { c.high };
        if (adverse - stop) * s <= 0.0 {
            // A gap through the stop fills at the open.
            let fill = if i > k && (c.open - stop) * s < 0.0 {
                c.open
            } else {
                stop
            };
            exit = Some((i, fill, "stop"));
            break;
        }
    }
    let complete = k2 < m.len();
    let (i, fill, reason) = match exit {
        Some(e) => e,
        None if k2 > k => (k2 - 1, m[k2 - 1].close, "time"),
        None => return None,
    };
    let pnl = (fill - px) * s;
    let c = cost(p, px);
    Some((
        Trade {
            day: d.day,
            side,
            entry_time: m[k].time,
            exit_time: m[i].time + 60,
            entry: px,
            stop,
            exit: fill,
            reason,
            pnl,
            cost: c,
            r_gross: pnl / risk,
            r: (pnl - c) / risk,
            entry_min,
            exit_min,
        },
        complete || reason == "stop",
    ))
}

/// Entry and exit times Auto chooses from.
pub const AUTO_ENTRIES: [u32; 7] = [420, 450, 480, 510, 540, 570, 600];
pub const AUTO_EXITS: [u32; 3] = [720, 840, 960];

fn month_of(day: i64) -> i64 {
    let y = year_of(day);
    let mut m = 1;
    while m < 12 && days_from_civil(y, m + 1, 1) <= day {
        m += 1;
    }
    y * 12 + m - 1
}

#[derive(Clone, Debug, Serialize)]
pub struct AutoPick {
    /// "YYYY-MM".
    pub month: String,
    pub entry_min: u32,
    pub exit_min: u32,
    /// Average R of the pick over the months it was chosen on.
    pub train_r: f64,
    pub train_trades: usize,
}

fn month_label(m: i64) -> String {
    format!("{:04}-{:02}", m.div_euclid(12), m.rem_euclid(12) + 1)
}

#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub params: Params,
    pub stats: Stats,
    /// First half and second half of the trades by time.
    pub halves: [Stats; 2],
    pub by_year: Vec<(i64, Stats)>,
    pub by_month: Vec<(String, Stats)>,
    pub trades: Vec<Trade>,
    /// Auto: the entry/exit time used in every month.
    pub picks: Vec<AutoPick>,
    /// Equity in R after each trade.
    pub equity: Vec<(i64, f64)>,
    pub from: i64,
    pub to: i64,
}

/// All completed trades of fixed entry/exit times.
fn fixed(days: &[Day], p: &Params, e: u32, x: u32) -> Vec<Trade> {
    days.iter()
        .filter_map(|d| trade_day(d, p, e, x))
        .filter(|(_, done)| *done)
        .map(|(t, _)| t)
        .collect()
}

/// Auto's pick for `month` from the months before it (at least 40 trades).
fn pick(grid: &[(u32, u32, Vec<Trade>)], month: i64, back: u32) -> Option<AutoPick> {
    let lo = month - i64::from(back);
    let mut best: Option<AutoPick> = None;
    for (e, x, trades) in grid {
        let tr: Vec<&Trade> = trades
            .iter()
            .filter(|t| (lo..month).contains(&month_of(t.day)))
            .collect();
        if tr.len() < 40 {
            continue;
        }
        let r = tr.iter().map(|t| t.r).sum::<f64>() / tr.len() as f64;
        if best.as_ref().is_none_or(|b| r > b.train_r) {
            best = Some(AutoPick {
                month: month_label(month),
                entry_min: *e,
                exit_min: *x,
                train_r: r,
                train_trades: tr.len(),
            });
        }
    }
    best
}

fn auto_grid(days: &[Day], p: &Params) -> Vec<(u32, u32, Vec<Trade>)> {
    let mut grid = Vec::new();
    for &e in &AUTO_ENTRIES {
        for &x in &AUTO_EXITS {
            grid.push((e, x, fixed(days, p, e, x)));
        }
    }
    grid
}

pub fn backtest(mins: &[Minute], p: &Params) -> Report {
    let ds = days(mins);
    let mut picks = Vec::new();
    let trades = match p.mode {
        Mode::Manual => fixed(&ds, p, p.entry_min, p.exit_min),
        Mode::Auto => {
            let grid = auto_grid(&ds, p);
            let mut months: Vec<i64> = ds.iter().map(|d| month_of(d.day)).collect();
            months.dedup();
            let mut out = Vec::new();
            for m in months {
                let Some(pk) = pick(&grid, m, p.auto_months) else {
                    continue;
                };
                let (_, _, tr) = grid
                    .iter()
                    .find(|(e, x, _)| *e == pk.entry_min && *x == pk.exit_min)
                    .expect("pick comes from the grid");
                out.extend(tr.iter().filter(|t| month_of(t.day) == m).cloned());
                picks.push(pk);
            }
            out
        }
    };
    report(p, trades, picks, mins)
}

fn report(p: &Params, trades: Vec<Trade>, picks: Vec<AutoPick>, mins: &[Minute]) -> Report {
    let half = trades.len() / 2;
    let mut by_year: Vec<(i64, Stats)> = Vec::new();
    let mut by_month: Vec<(String, Stats)> = Vec::new();
    let mut a = 0;
    while a < trades.len() {
        let y = year_of(trades[a].day);
        let b = a + trades[a..].iter().take_while(|t| year_of(t.day) == y).count();
        by_year.push((y, stats(&trades[a..b])));
        a = b;
    }
    let mut a = 0;
    while a < trades.len() {
        let m = month_of(trades[a].day);
        let b = a + trades[a..].iter().take_while(|t| month_of(t.day) == m).count();
        by_month.push((month_label(m), stats(&trades[a..b])));
        a = b;
    }
    let mut eq = 0.0;
    let equity = trades
        .iter()
        .map(|t| {
            eq += t.r;
            (t.exit_time, eq)
        })
        .collect();
    Report {
        params: p.clone(),
        stats: stats(&trades),
        halves: [stats(&trades[..half]), stats(&trades[half..])],
        by_year,
        by_month,
        equity,
        picks,
        from: mins.first().map_or(0, |m| m.time),
        to: mins.last().map_or(0, |m| m.time + 60),
        trades,
    }
}

// ---- live and paper --------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize)]
pub struct Live {
    /// "weekend", "waiting", "open", "closed", "skipped".
    pub state: &'static str,
    pub entry_min: u32,
    pub exit_min: u32,
    pub midnight_open: Option<f64>,
    pub price: Option<f64>,
    /// The trade of today (open or closed), with R marked to the last price when open.
    pub trade: Option<Trade>,
    /// Completed paper trades found in the loaded candles (for the journal).
    pub closed: Vec<Trade>,
    pub now: i64,
}

/// Today's state from recent 1m candles; `pick` overrides the entry/exit time (Auto).
pub fn live(mins: &[Minute], p: &Params, pick: Option<(u32, u32)>, now: i64) -> Live {
    let (e, x) = pick.unwrap_or((p.entry_min, p.exit_min));
    let ds = days(mins);
    let closed = ds
        .iter()
        .filter_map(|d| trade_day(d, p, e, x))
        .filter(|(_, done)| *done)
        .map(|(t, _)| t)
        .collect();
    let (today, minute, dow) = ny_clock(now);
    let mut out = Live {
        state: "waiting",
        entry_min: e,
        exit_min: x,
        midnight_open: None,
        price: mins.last().map(|m| m.close),
        trade: None,
        closed,
        now,
    };
    if dow >= 5 {
        out.state = "weekend";
        return out;
    }
    let Some(d) = ds.iter().find(|d| d.day == today) else {
        return out;
    };
    if d.clock[0] <= 5 {
        out.midnight_open = Some(d.mins[0].open);
    }
    if minute < e {
        return out;
    }
    match trade_day(d, p, e, x) {
        Some((mut t, done)) => {
            if !done {
                // Still open: mark to the last price.
                let last = d.mins.last().map_or(t.entry, |m| m.close);
                t.exit = last;
                t.exit_time = now;
                t.reason = "open";
                t.pnl = (last - t.entry) * f64::from(t.side);
                let risk = (t.entry - t.stop).abs();
                t.r_gross = t.pnl / risk;
                t.r = (t.pnl - t.cost) / risk;
            }
            out.state = if done { "closed" } else { "open" };
            out.trade = Some(t);
        }
        None => out.state = "skipped",
    }
    out
}

/// Auto's current pick from history (for the live signal).
pub fn current_pick(mins: &[Minute], p: &Params, now: i64) -> Option<AutoPick> {
    let ds = days(mins);
    pick(&auto_grid(&ds, p), month_of(ny_clock(now).0), p.auto_months)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_york_clock_follows_dst() {
        // 2026-03-08 (second Sunday of March) 06:59 UTC = 01:59 EST; 07:00 UTC = 03:00 EDT.
        let t = days_from_civil(2026, 3, 8) * 86_400;
        assert_eq!(ny_offset(t + 6 * 3600 + 59 * 60), -5 * 3600);
        assert_eq!(ny_offset(t + 7 * 3600), -4 * 3600);
        // 2026-11-01 (first Sunday of November) 05:59 UTC = 01:59 EDT; 06:00 UTC = 01:00 EST.
        let t = days_from_civil(2026, 11, 1) * 86_400;
        assert_eq!(ny_offset(t + 5 * 3600 + 59 * 60), -4 * 3600);
        assert_eq!(ny_offset(t + 6 * 3600), -5 * 3600);
        // 2026-07-15 12:30 UTC = 08:30 New York, a Wednesday.
        let t = days_from_civil(2026, 7, 15) * 86_400 + 12 * 3600 + 30 * 60;
        assert_eq!(ny_clock(t), (days_from_civil(2026, 7, 15), 510, 2));
        assert_eq!(year_of(days_from_civil(2024, 12, 31)), 2024);
        assert_eq!(month_label(month_of(days_from_civil(2025, 2, 28))), "2025-02");
    }

    /// A New York day in 1m candles: flat at `open` until 08:30, then `path(minute)`.
    fn day(date: (i64, i64, i64), f: impl Fn(u32) -> (f64, f64, f64, f64)) -> Vec<Minute> {
        let d = days_from_civil(date.0, date.1, date.2);
        let t0 = d * 86_400 + 4 * 3600; // 00:00 EDT
        (0..17 * 60)
            .map(|m| {
                let (open, high, low, close) = f(m);
                Minute {
                    time: t0 + i64::from(m) * 60,
                    open,
                    high,
                    low,
                    close,
                }
            })
            .collect()
    }

    #[test]
    fn long_above_the_midnight_open_exits_at_time_or_stop() {
        let p = Params {
            fee_pct: 0.0,
            ..Params::default()
        };
        // Midnight open 100, morning low 99, price 102 at 08:30, 104 at the close: long, +2 / 3 R.
        let up = day((2026, 7, 15), |m| match m {
            0 => (100.0, 100.0, 99.0, 100.0),
            _ if m < 600 => (102.0, 102.0, 102.0, 102.0),
            _ => (104.0, 104.0, 104.0, 104.0),
        });
        let r = backtest(&up, &p);
        assert_eq!(r.trades.len(), 1);
        let t = &r.trades[0];
        assert_eq!((t.side, t.entry, t.stop, t.reason), (1, 102.0, 99.0, "time"));
        assert!((t.r - 2.0 / 3.0).abs() < 1e-9);
        // Same morning, then a gap to 97 at 09:00: stopped at the open of the gap, -5/3 R.
        let gap = day((2026, 7, 16), |m| match m {
            0 => (100.0, 100.0, 99.0, 100.0),
            _ if m < 540 => (102.0, 102.0, 102.0, 102.0),
            _ => (97.0, 97.0, 97.0, 97.0),
        });
        let t = &backtest(&gap, &p).trades[0];
        assert_eq!((t.reason, t.exit), ("stop", 97.0));
        assert!((t.r + 5.0 / 3.0).abs() < 1e-9);
        // Costs: 0.05% per side of 102 twice + $0.3 spread.
        let q = Params {
            spread_usd: 0.3,
            ..Params::default()
        };
        let t = &backtest(&up, &q).trades[0];
        assert!((t.cost - (2.0 * 0.0005 * 102.0 + 0.3)).abs() < 1e-9);
        // Shorts only: the long day is skipped.
        let s = Params {
            long: false,
            ..p.clone()
        };
        assert!(backtest(&up, &s).trades.is_empty());
    }

    #[test]
    fn live_marks_an_open_trade_and_waits_before_the_entry() {
        let p = Params::default();
        let mins = day((2026, 7, 15), |m| match m {
            0 => (100.0, 100.0, 99.0, 100.0),
            _ => (98.0, 98.0, 98.0, 98.0),
        });
        let t0 = mins[0].time;
        let before = live(&mins[..400], &p, None, t0 + 400 * 60);
        assert_eq!(before.state, "waiting");
        let open = live(&mins[..600], &p, None, t0 + 600 * 60);
        assert_eq!(open.state, "open");
        let t = open.trade.unwrap();
        assert_eq!((t.side, t.stop), (-1, 100.0));
        assert_eq!(live(&mins, &p, None, t0 + 17 * 3600).state, "closed");
    }
}
