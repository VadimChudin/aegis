//! The bar-by-bar simulation. Positions change at a bar's open: a window is entered at the first
//! bar that opens at or after its start and left at the first bar that opens at or after its end,
//! so on 1-hour bars an 18:05 entry happens at 19:00. P&L is a share of equity on one unit of
//! notional per unit of size, summed (not compounded) like the research reports.

use serde::Serialize;

use super::{fomc, Params};
use crate::dataset::{time, HBar};

/// A New York session runs from 17:00 to 17:00; it is named after the day it ends.
const ROLL_MIN: u16 = 17 * 60;

/// Bars with the New York calendar fields the rules need, computed once per history.
pub struct Prepared {
    pub time: Vec<i64>,
    open: Vec<f32>,
    low: Vec<f32>,
    close: Vec<f32>,
    spread: Vec<f32>,
    ny_min: Vec<u16>,
    /// Weekday of the evening a bar belongs to (0 = Monday); bars before 17:00 belong to the
    /// previous evening.
    evening: Vec<u8>,
    session: Vec<i32>,
    /// Index of the bar's session in `closes`.
    sess_idx: Vec<u32>,
    /// Minutes from the bar's open to the next FOMC statement (i32::MAX when none is known).
    to_fomc: Vec<i32>,
    /// Last close of every session, in order.
    closes: Vec<f64>,
    pub tf_secs: i64,
}

impl Prepared {
    pub fn new(bars: &[HBar], tf_secs: i64) -> Prepared {
        let n = bars.len();
        let mut p = Prepared {
            time: Vec::with_capacity(n),
            open: Vec::with_capacity(n),
            low: Vec::with_capacity(n),
            close: Vec::with_capacity(n),
            spread: Vec::with_capacity(n),
            ny_min: Vec::with_capacity(n),
            evening: Vec::with_capacity(n),
            session: Vec::with_capacity(n),
            sess_idx: Vec::with_capacity(n),
            to_fomc: Vec::with_capacity(n),
            closes: Vec::new(),
            tf_secs,
        };
        // Statement minute (New York minutes since the epoch): 14:15 before 2013, then 14:00.
        let y2013 = time::days_from_civil(2013, 1, 1);
        let statements: Vec<i64> = fomc::decision_days()
            .into_iter()
            .map(|d| d * 1440 + if d < y2013 { 855 } else { 840 })
            .collect();
        let mut clock = time::NyClock::default();
        let mut k = 0;
        for b in bars {
            let ny = b.time + clock.offset(b.time);
            let day = ny.div_euclid(86_400);
            let min = (ny.rem_euclid(86_400) / 60) as u16;
            let after_roll = min >= ROLL_MIN;
            let session = (day + i64::from(after_roll)) as i32;
            if p.session.last() != Some(&session) {
                p.closes.push(f64::from(b.close));
            } else if let Some(c) = p.closes.last_mut() {
                *c = f64::from(b.close);
            }
            let t = day * 1440 + i64::from(min);
            while k < statements.len() && statements[k] < t {
                k += 1;
            }
            p.time.push(b.time);
            p.open.push(b.open);
            p.low.push(b.low);
            p.close.push(b.close);
            p.spread.push(b.spread_open);
            p.ny_min.push(min);
            p.evening.push(time::weekday(if after_roll { day } else { day - 1 }) as u8);
            p.session.push(session);
            p.sess_idx.push((p.closes.len() - 1) as u32);
            p.to_fomc
                .push(statements.get(k).map_or(i32::MAX, |&s| (s - t).min(i32::MAX as i64) as i32));
        }
        p
    }

    pub fn len(&self) -> usize {
        self.time.len()
    }

    pub fn is_empty(&self) -> bool {
        self.time.is_empty()
    }

    /// First bar at or after `share` of the time span (the train / test split).
    pub fn split(&self, share: f64) -> usize {
        if self.is_empty() {
            return 0;
        }
        let (a, b) = (self.time[0], self.time[self.len() - 1]);
        let t = a + ((b - a) as f64 * share.clamp(0.0, 1.0)) as i64;
        self.time.partition_point(|&x| x < t)
    }

    /// Size for a trade opened in each session: volatility target ÷ realised volatility of the
    /// previous `vol_days` sessions; 0 until enough history exists.
    fn sizes(&self, p: &Params) -> Vec<f64> {
        let m = self.closes.len();
        if p.vol_target <= 0.0 {
            return vec![1.0; m];
        }
        let n = (p.vol_days.round() as usize).max(2);
        let mut s1 = vec![0.0; m + 1];
        let mut s2 = vec![0.0; m + 1];
        for j in 1..m {
            let r = (self.closes[j] / self.closes[j - 1]).ln();
            let r = if r.is_finite() { r } else { 0.0 };
            s1[j + 1] = s1[j] + r;
            s2[j + 1] = s2[j] + r * r;
        }
        (0..m)
            .map(|k| {
                // returns r_j for j in [max(1, k - n), k): sessions finished before session k
                let lo = k.saturating_sub(n).max(1);
                let cnt = k.saturating_sub(lo);
                if cnt < (n / 2).max(5) {
                    return 0.0;
                }
                let (a, b) = (s1[k] - s1[lo], s2[k] - s2[lo]);
                let mean = a / cnt as f64;
                let var = (b / cnt as f64 - mean * mean).max(0.0) * cnt as f64 / (cnt - 1) as f64;
                let vol = var.sqrt() * 252f64.sqrt();
                if vol > 0.0 {
                    (p.vol_target / 100.0 / vol).min(p.max_size)
                } else {
                    0.0
                }
            })
            .collect()
    }

    /// Summary of bars [a, b) with these settings (fast path for the search).
    pub fn stats(&self, p: &Params, a: usize, b: usize) -> Stats {
        self.sim(p, a, b, false).stats
    }

    /// Full backtest of bars [a, b): statistics, years, parts, equity and trades.
    pub fn report(&self, p: &Params, a: usize, b: usize) -> Report {
        let s = self.sim(p, a, b, true);
        let mut years: Vec<YearRow> = Vec::new();
        for (&(day, r), _) in s.daily.iter().zip(0..) {
            let (y, _, _) = time::civil_from_days(i64::from(day));
            match years.last_mut() {
                Some(row) if row.year == y => row.returns.push(r),
                _ => years.push(YearRow {
                    year: y,
                    returns: vec![r],
                    ..Default::default()
                }),
            }
        }
        for t in &s.trades {
            let (y, _, _) = time::civil_from_days(t.entry.div_euclid(86_400));
            if let Some(row) = years.iter_mut().find(|r| r.year == y) {
                row.trades += 1;
            }
        }
        for row in &mut years {
            row.pct = row.returns.iter().sum::<f64>() * 100.0;
            row.sharpe = sharpe(&row.returns);
        }
        let parts = ["asia", "fomc", "both"]
            .into_iter()
            .map(|kind| {
                let r: Vec<f64> = s.trades.iter().filter(|t| t.kind == kind).map(|t| t.net_bp / 1e4).collect();
                Part {
                    kind,
                    trades: r.len(),
                    win_rate: frac(r.iter().filter(|&&x| x > 0.0).count(), r.len()),
                    avg_bp: mean(&r) * 1e4,
                    total_pct: r.iter().sum::<f64>() * 100.0,
                }
            })
            .collect();
        let mut cum = 0.0;
        let step = (s.daily.len() / 1500).max(1);
        let equity = s
            .daily
            .iter()
            .enumerate()
            .filter_map(|(i, &(day, r))| {
                cum += r * 100.0;
                (i % step == 0 || i + 1 == s.daily.len()).then_some([(i64::from(day) * 86_400) as f64, cum])
            })
            .collect();
        let total = s.trades.len();
        Report {
            stats: s.stats,
            years,
            parts,
            equity,
            trades: s.trades.into_iter().rev().take(500).collect(),
            trades_total: total,
            bars: b.saturating_sub(a),
            spread_from_data: frac(s.data_spread_fills, s.fills),
        }
    }

    fn sim(&self, p: &Params, a: usize, b: usize, detail: bool) -> Sim {
        let b = b.min(self.len());
        let mut out = Sim::default();
        if a >= b {
            return out;
        }
        let sizes = self.sizes(p);
        let evenings = p.evenings();
        let (enter, leave) = (p.asia_enter.round() as u16, p.asia_leave.round() as u16);
        let (lead, exit) = (p.fomc_lead.round() as i32, p.fomc_exit.round() as i32);
        let comm = p.commission * 1e-6;
        let stop = p.stop_pct / 100.0;
        let mut pos = 0.0f64;
        let (mut entry_px, mut entry_t, mut trade_ret, mut kinds) = (0.0f64, 0i64, 0.0f64, 0u8);
        let mut stopped = false;
        let mut day_ret = 0.0;
        let mut session = self.session[a];
        let mut prev_close = f64::from(self.open[a]);
        let mut trade_rets: Vec<f64> = Vec::new();
        let mut in_pos = 0usize;
        let half_spread = |i: usize, out: &mut Sim| -> f64 {
            out.fills += 1;
            let s = self.spread[i];
            if p.data_spread && s.is_finite() && s >= 0.0 {
                out.data_spread_fills += 1;
                f64::from(s) / 2.0
            } else {
                p.spread / 2.0
            }
        };
        for i in a..b {
            let o = f64::from(self.open[i]);
            if self.session[i] != session {
                if pos > 0.0 {
                    let nights: f64 = (session..self.session[i])
                        .map(|d| match time::weekday(i64::from(d)) {
                            2 => 3.0,
                            5 | 6 => 0.0,
                            _ => 1.0,
                        })
                        .sum();
                    let c = p.swap * nights * pos / prev_close;
                    day_ret -= c;
                    trade_ret -= c;
                    out.stats.swaps_pct += c * 100.0;
                }
                out.daily.push((session, day_ret));
                day_ret = 0.0;
                session = self.session[i];
            }
            if pos > 0.0 {
                let g = pos * (o / prev_close - 1.0);
                day_ret += g;
                trade_ret += g;
            }
            let m = self.ny_min[i];
            let asia_on = p.asia
                && evenings[self.evening[i] as usize]
                && if m >= ROLL_MIN { m >= enter } else { m < leave };
            let tf = self.to_fomc[i];
            let fomc_on = p.fomc && tf > exit && tf <= lead;
            let want = asia_on || fomc_on;
            if !want {
                stopped = false;
            }
            let target = if !want || stopped {
                0.0
            } else if pos > 0.0 {
                pos
            } else {
                sizes[self.sess_idx[i] as usize]
            };
            if target != pos {
                let c = (target - pos).abs() * (half_spread(i, &mut out) + comm * o + p.slippage) / o;
                day_ret -= c;
                out.stats.costs_pct += c * 100.0;
                if pos == 0.0 {
                    (entry_px, entry_t, trade_ret, kinds) = (o, self.time[i], -c, 0);
                } else {
                    trade_ret -= c;
                    close_trade(&mut out, &mut trade_rets, detail, (entry_t, self.time[i]), (entry_px, o), pos, trade_ret, kinds, "time");
                }
                pos = target;
            }
            let c1 = f64::from(self.close[i]);
            if pos > 0.0 {
                in_pos += 1;
                kinds |= u8::from(asia_on) | (u8::from(fomc_on) << 1);
                let stop_px = entry_px * (1.0 - stop);
                if stop > 0.0 && f64::from(self.low[i]) <= stop_px {
                    let px = stop_px.min(o);
                    let g = pos * (px / o - 1.0);
                    let c = pos * (half_spread(i, &mut out) + comm * px + p.slippage) / px;
                    day_ret += g - c;
                    trade_ret += g - c;
                    out.stats.costs_pct += c * 100.0;
                    close_trade(&mut out, &mut trade_rets, detail, (entry_t, self.time[i]), (entry_px, px), pos, trade_ret, kinds, "stop");
                    pos = 0.0;
                    stopped = true;
                } else {
                    let g = pos * (c1 / o - 1.0);
                    day_ret += g;
                    trade_ret += g;
                }
            }
            prev_close = c1;
        }
        if pos > 0.0 {
            let i = b - 1;
            let c = pos * (half_spread(i, &mut out) + comm * prev_close + p.slippage) / prev_close;
            day_ret -= c;
            trade_ret -= c;
            out.stats.costs_pct += c * 100.0;
            close_trade(&mut out, &mut trade_rets, detail, (entry_t, self.time[i]), (entry_px, prev_close), pos, trade_ret, kinds, "end");
        }
        out.daily.push((session, day_ret));

        let st = &mut out.stats;
        let daily: Vec<f64> = out.daily.iter().map(|d| d.1).collect();
        st.trades = trade_rets.len();
        st.win_rate = frac(trade_rets.iter().filter(|&&x| x > 0.0).count(), trade_rets.len());
        st.avg_bp = mean(&trade_rets) * 1e4;
        let (gain, loss): (f64, f64) = trade_rets
            .iter()
            .fold((0.0, 0.0), |(g, l), &x| if x > 0.0 { (g + x, l) } else { (g, l - x) });
        st.profit_factor = if loss > 0.0 { gain / loss } else if gain > 0.0 { 99.0 } else { 0.0 };
        st.total_pct = daily.iter().sum::<f64>() * 100.0;
        let years = ((self.time[b - 1] - self.time[a]) as f64 / (365.25 * 86_400.0)).max(1.0 / 365.25);
        st.pct_per_year = st.total_pct / years;
        st.sharpe = sharpe(&daily);
        let (mut cum, mut peak, mut dd) = (0.0f64, 0.0f64, 0.0f64);
        for r in &daily {
            cum += r;
            peak = peak.max(cum);
            dd = dd.min(cum - peak);
        }
        st.max_dd_pct = dd * 100.0;
        let mut by_year: Vec<(i64, f64)> = Vec::new();
        for &(day, r) in &out.daily {
            let (y, _, _) = time::civil_from_days(i64::from(day));
            match by_year.last_mut() {
                Some((yy, s)) if *yy == y => *s += r,
                _ => by_year.push((y, r)),
            }
        }
        st.years = by_year.len();
        st.years_up = by_year.iter().filter(|(_, r)| *r > 0.0).count();
        st.exposure = frac(in_pos, b - a);
        st.first = self.time[a];
        st.last = self.time[b - 1];
        out
    }
}

#[allow(clippy::too_many_arguments)]
fn close_trade(
    out: &mut Sim,
    rets: &mut Vec<f64>,
    detail: bool,
    (entry, exit): (i64, i64),
    (entry_px, exit_px): (f64, f64),
    size: f64,
    ret: f64,
    kinds: u8,
    reason: &'static str,
) {
    rets.push(ret);
    if detail {
        out.trades.push(Trade {
            entry,
            exit,
            entry_px,
            exit_px,
            size,
            net_bp: ret * 1e4,
            kind: match kinds {
                1 => "asia",
                2 => "fomc",
                _ => "both",
            },
            reason,
        });
    }
}

fn frac(a: usize, b: usize) -> f64 {
    if b == 0 {
        0.0
    } else {
        a as f64 / b as f64
    }
}

fn mean(x: &[f64]) -> f64 {
    if x.is_empty() {
        0.0
    } else {
        x.iter().sum::<f64>() / x.len() as f64
    }
}

/// Annualised Sharpe of daily (per session) returns.
fn sharpe(x: &[f64]) -> f64 {
    if x.len() < 2 {
        return 0.0;
    }
    let m = mean(x);
    let var = x.iter().map(|r| (r - m) * (r - m)).sum::<f64>() / (x.len() - 1) as f64;
    if var > 0.0 {
        m / var.sqrt() * 252f64.sqrt()
    } else {
        0.0
    }
}

#[derive(Default)]
struct Sim {
    stats: Stats,
    /// (session day, return) for every session with bars.
    daily: Vec<(i32, f64)>,
    trades: Vec<Trade>,
    fills: usize,
    data_spread_fills: usize,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Stats {
    pub trades: usize,
    pub win_rate: f64,
    /// Average net result per trade, basis points of equity.
    pub avg_bp: f64,
    pub total_pct: f64,
    pub pct_per_year: f64,
    /// Annualised, from session returns including flat sessions.
    pub sharpe: f64,
    pub max_dd_pct: f64,
    pub profit_factor: f64,
    pub years: usize,
    pub years_up: usize,
    /// Share of bars in a position.
    pub exposure: f64,
    pub costs_pct: f64,
    pub swaps_pct: f64,
    pub first: i64,
    pub last: i64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct YearRow {
    pub year: i64,
    pub pct: f64,
    pub trades: usize,
    pub sharpe: f64,
    #[serde(skip)]
    returns: Vec<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Trade {
    pub entry: i64,
    pub exit: i64,
    pub entry_px: f64,
    pub exit_px: f64,
    pub size: f64,
    pub net_bp: f64,
    /// "asia", "fomc" or "both" (the windows the position was held for).
    pub kind: &'static str,
    /// "time", "stop" or "end" (the history ended).
    pub reason: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub struct Part {
    pub kind: &'static str,
    pub trades: usize,
    pub win_rate: f64,
    pub avg_bp: f64,
    pub total_pct: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub stats: Stats,
    pub years: Vec<YearRow>,
    pub parts: Vec<Part>,
    /// [time, cumulative %] per session, at most ~1500 points.
    pub equity: Vec<[f64; 2]>,
    /// The latest 500 trades, newest first.
    pub trades: Vec<Trade>,
    pub trades_total: usize,
    pub bars: usize,
    /// Share of entries and exits costed with the spread from the data.
    pub spread_from_data: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::time::parse_date;

    /// Hourly bars (UTC) for [from, to]: flat at 100, except `f(t)` may move the close.
    fn bars(from: &str, to: &str, step: i64, mut f: impl FnMut(i64, i64) -> f32) -> Vec<HBar> {
        let (a, b) = (parse_date(from).unwrap() * 86_400, (parse_date(to).unwrap() + 1) * 86_400);
        let mut v = Vec::new();
        let mut px = 100.0f32;
        for t in (a..b).step_by(step as usize) {
            let ny = t + time::ny_offset(t);
            let wd = time::weekday(ny.div_euclid(86_400));
            let min = ny.rem_euclid(86_400) / 60;
            // gold: closed Friday 17:00 → Sunday 18:00 and 17:00-18:00 daily (New York)
            if wd == 5 || (wd == 4 && min >= 1020) || (wd == 6 && min < 1080) || (1020..1080).contains(&min) {
                continue;
            }
            let c = f(t, min);
            v.push(HBar {
                time: t,
                open: px,
                high: px.max(c),
                low: px.min(c),
                close: c,
                volume: 1.0,
                spread_open: 0.02,
                spread_close: 0.02,
            });
            px = c;
        }
        v
    }

    fn zero_cost() -> Params {
        Params {
            vol_target: 0.0,
            data_spread: false,
            spread: 0.0,
            commission: 0.0,
            slippage: 0.0,
            swap: 0.0,
            fomc: false,
            ..Params::default()
        }
    }

    #[test]
    fn asia_window_trades_monday_to_thursday_evenings() {
        // Price rises 1 each hour between 19:00 and 01:00 New York, flat otherwise.
        let mut px = 100.0f32;
        let b = bars("2026-09-07", "2026-09-20", 3600, |_, min| {
            if min >= 19 * 60 || min < 60 {
                px += 1.0;
            }
            px
        });
        let pr = Prepared::new(&b, 3600);
        let p = Params { asia_enter: 18.0 * 60.0, asia_leave: 120.0, ..zero_cost() };
        let r = pr.report(&p, 0, pr.len());
        // Two weeks × Mon-Thu evenings = 8 trades, each held 18:00 → 02:00 and earning 6 hourly rises.
        assert_eq!(r.stats.trades, 8);
        assert!(r.trades.iter().all(|t| t.kind == "asia" && t.reason == "time"));
        let first = r.trades.last().unwrap();
        let ny = |t: i64| (t + time::ny_offset(t)).rem_euclid(86_400) / 3600;
        assert_eq!((ny(first.entry), ny(first.exit)), (18, 2));
        assert!((first.exit_px - first.entry_px - 6.0).abs() < 1e-3, "{first:?}");
        assert!(r.stats.win_rate > 0.99 && r.stats.costs_pct == 0.0 && r.stats.swaps_pct == 0.0);
        // Costs are charged on both sides: 2 × (0.05 / 2 + 0.01) per trade.
        let c = Params { spread: 0.05, slippage: 0.01, ..p.clone() };
        let rc = pr.stats(&c, 0, pr.len());
        assert!((r.stats.total_pct - rc.total_pct - rc.costs_pct).abs() < 1e-9);
        assert!(rc.costs_pct > 0.0);
        // Sunday evenings add three: the history starts on Sunday 6 September at 20:00 New York
        // (Monday 00:00 UTC) and ends on Sunday 20 September at 19:00. Asia off leaves none.
        assert_eq!(pr.stats(&Params { sun: true, ..p.clone() }, 0, pr.len()).trades, 11);
        assert_eq!(pr.stats(&Params { asia: false, ..p }, 0, pr.len()).trades, 0);
    }

    #[test]
    fn fomc_window_swap_and_stop() {
        // 2026-09-16 was an FOMC decision day (Wednesday).
        let b = bars("2026-09-14", "2026-09-18", 300, |_, _| 100.0);
        let pr = Prepared::new(&b, 300);
        let p = Params { asia: false, fomc: true, fomc_lead: 25.0 * 60.0, fomc_exit: 5.0, swap: 0.6, ..zero_cost() };
        let r = pr.report(&p, 0, pr.len());
        assert_eq!(r.stats.trades, 1);
        let t = &r.trades[0];
        let ny = |x: i64| x + time::ny_offset(x);
        // 25 h before 14:00 = 13:00 the day before; exit at 13:55 on the day.
        assert_eq!(ny(t.entry).rem_euclid(86_400) / 60, 13 * 60);
        assert_eq!(ny(t.exit).rem_euclid(86_400) / 60, 13 * 60 + 55);
        assert_eq!(t.kind, "fomc");
        // Held through Tuesday's 17:00 rollover: one night of swap at $0.60 on $100.
        assert!((r.stats.swaps_pct - 0.6).abs() < 1e-9, "{}", r.stats.swaps_pct);
        // A 1% drop inside the window triggers a 0.5% stop.
        let b2 = bars("2026-09-14", "2026-09-18", 300, |t, min| {
            let d = time::weekday((t + time::ny_offset(t)).div_euclid(86_400));
            if d == 1 && min >= 20 * 60 { 99.0 } else { 100.0 }
        });
        let pr2 = Prepared::new(&b2, 300);
        let r2 = pr2.report(&Params { stop_pct: 0.5, swap: 0.0, ..p }, 0, pr2.len());
        assert_eq!(r2.trades[0].reason, "stop");
        assert!((r2.trades[0].exit_px - 99.5).abs() < 1e-3);
    }

    #[test]
    fn volatility_target_waits_for_history_and_scales_size() {
        let mut k = 0u32;
        let b = bars("2026-01-05", "2026-06-30", 3600, |_, _| {
            k += 1;
            if k % 2 == 0 { 101.0 } else { 100.0 }
        });
        let pr = Prepared::new(&b, 3600);
        let p = Params { vol_target: 10.0, vol_days: 20.0, max_size: 3.0, ..zero_cost() };
        let r = pr.report(&p, 0, pr.len());
        let sizes: Vec<f64> = r.trades.iter().map(|t| t.size).collect();
        assert!(sizes.iter().all(|&s| s > 0.0 && s <= 3.0));
        // No trade in the first sessions (not enough closes for the estimate).
        let first = r.trades.last().unwrap().entry;
        assert!(first > b[0].time + 10 * 86_400);
    }

    #[test]
    fn split_divides_the_time_span() {
        let b = bars("2026-01-05", "2026-01-30", 3600, |_, _| 100.0);
        let pr = Prepared::new(&b, 3600);
        let s = pr.split(0.5);
        assert!(s > 0 && s < pr.len());
        let mid = (pr.time[0] + pr.time[pr.len() - 1]) / 2;
        assert!(pr.time[s] >= mid && pr.time[s - 1] < mid);
    }
}
