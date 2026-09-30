//! Position engine on 1-second candles: the entry and every exit rule are checked second by
//! second instead of on 5m bars.
//!
//! - Entry: a resting limit at the level (as the bar engine), a market entry after the touch bar
//!   closes, or **absorption**: price reaches the level, aggressive volume hits it for
//!   `abs_window` seconds (at least `abs_vol` × the average of the last hour) and price does not
//!   go more than `abs_hold_atr` through the level, then turns `abs_confirm_atr` off the extreme.
//!   The market entry is at the next second's open; the stop sits `abs_stop_atr` behind the
//!   extreme of the absorption, so the stop is small.
//! - Management: breakeven after `be_r` R, trailing stop `trail_atr` behind the best price after
//!   `trail_from_r` R, partial take profit of `part_frac` at `part_r` R, the target `tp_r`, a time
//!   exit, and a **flow exit**: aggressive volume against the position of `eat_vol` × average
//!   over `abs_window` seconds while price is through the level (the level is being eaten).
//! - **Density exit**: the aggressive volume absorbed at the entry is taken as the size of the
//!   resting orders (the "density") the stop hides behind. When aggressive volume at that price
//!   reaches `dens_eat` of it again, the density is being eaten: exit at market before the stop
//!   (`dens_eat` 0.7 = leave with 30% of it left). The tape shows traded volume, not the book, so
//!   this estimates the density; icebergs and cancelled orders are not seen.
//! - **Limit after absorption** (`abs_limit`): instead of a market entry, rest a limit
//!   `abs_limit_atr` off the absorption extreme and wait for the pull-back (maker fee, no spread).
//! - **Flip**: when the first position ends at its initial stop or by a flow exit, an opposite
//!   position opens at market (the breakout), stop `flip_sl_atr` behind the level, target
//!   `flip_tp_r` R, the same management. Its R is added to the trade's R (each leg risks 1 R).
//!
//! Inside one second the path is open → low → high → close for an up second and open → high →
//! low → close otherwise. Every decision uses only seconds that have closed.

use super::{
    bar::Bar,
    engine::{spread, Trade},
    params::Params,
    scan::Touch,
    secs::Sec,
};

/// Exit-side price: longs sell at the bid (the trade price), shorts buy at the ask.
fn exit_side(d: f64, px: f64, sp: f64) -> f64 {
    if d > 0.0 {
        px
    } else {
        px + sp
    }
}

/// Market entry price before slippage: longs buy at the ask, shorts sell at the bid.
fn entry_side(d: f64, px: f64, sp: f64) -> f64 {
    if d > 0.0 {
        px + sp
    } else {
        px
    }
}

/// Price on the side of a second that goes against a position in direction `d`.
fn adverse(d: f64, s: &Sec) -> f64 {
    f64::from(if d > 0.0 { s.low } else { s.high })
}

/// A resting entry limit can fill in a gap already beyond its stop. In that
/// case the stop executes at the open's exit-side price, not at a stale stop.
fn fill_second_stop(d: f64, x: &Sec, stop: f64, c: &Ctx) -> Option<Exit> {
    let open = exit_side(d, x.open as f64, c.sp);
    let price = if d * (open - stop) <= 0.0 {
        open
    } else if d * (exit_side(d, adverse(d, x), c.sp) - stop) <= 0.0 {
        stop
    } else {
        return None;
    };
    Some(Exit {
        price: price - d * c.p.slippage,
        time: x.time,
        outcome: "sl",
        part: false,
    })
}

/// Aggressive volume against a position in direction `d` (sells hit a long's level).
fn against(d: f64, s: &Sec) -> f64 {
    f64::from(if d > 0.0 { s.sell } else { s.buy })
}

struct Ctx<'a> {
    p: &'a Params,
    sp: f64,
    atr: f64,
    level: f64,
    /// Average volume per second over the hour before the touch bar.
    vbase: f64,
}

struct Leg {
    d: f64,
    entry: f64,
    stop: f64,
    risk: f64,
    tp: f64,
    part_px: f64,
    entry_fee: f64,
    entry_time: i64,
    /// Density price (the absorption extreme) and its absorbed volume; NaN when none.
    zone: f64,
    dens: f64,
}

struct Exit {
    price: f64,
    time: i64,
    /// "tp", "sl", "be", "trail", "flow" or "time".
    outcome: &'static str,
    part: bool,
}

impl Leg {
    fn new(c: &Ctx, d: f64, entry: f64, stop: f64, tp_r: f64, entry_fee: f64, entry_time: i64) -> Option<Leg> {
        let p = c.p;
        let risk = d * (entry - stop);
        if risk.is_nan() || risk <= 0.0 || risk < p.min_risk_atr * c.atr || risk > p.max_risk_atr * c.atr {
            return None;
        }
        Some(Leg {
            d,
            entry,
            stop,
            risk,
            tp: entry + d * tp_r * risk,
            part_px: entry + d * p.part_r * risk,
            entry_fee,
            entry_time,
            zone: f64::NAN,
            dens: f64::NAN,
        })
    }

    /// Net R after commission; slippage and spread are already in the prices.
    fn r(&self, x: &Exit, p: &Params) -> f64 {
        let pf = if x.part { p.part_frac } else { 0.0 };
        let exit_fee = if x.outcome == "tp" { p.maker_bps } else { p.taker_bps };
        let pnl = pf * self.d * (self.part_px - self.entry) + (1.0 - pf) * self.d * (x.price - self.entry);
        let fees =
            (self.entry_fee * self.entry + pf * p.maker_bps * self.part_px + (1.0 - pf) * exit_fee * x.price) / 1e4;
        (pnl - fees) / self.risk
    }
}

/// Runs a position from `s[start]` until `end` (exclusive).
fn manage(s: &[Sec], start: usize, end: i64, leg: &Leg, c: &Ctx) -> Exit {
    let (p, d) = (c.p, leg.d);
    // Breakeven covers the round trip, so a breakeven exit really is ≈ 0 R.
    let be_px = leg.entry + d * ((leg.entry_fee + p.taker_bps) * leg.entry / 1e4 + p.slippage);
    let (mut stop, mut best, mut state) = (leg.stop, leg.entry, "sl");
    let (mut be_done, mut part) = (false, false);
    let w = p.abs_window.max(1.0) as i64;
    let eat = p.eat_vol * c.vbase * w as f64;
    let (mut lo, mut flow) = (start, 0.0f64);
    let dens_on = p.dens_eat > 0.0 && leg.zone.is_finite() && leg.dens > 0.0;
    let mut eaten = 0.0f64;
    let mut last: Option<&Sec> = None;
    for (j, x) in s.iter().enumerate().skip(start) {
        if x.time >= end {
            break;
        }
        last = Some(x);
        let (o, h, l, cl) = (x.open as f64, x.high as f64, x.low as f64, x.close as f64);
        let path = if cl >= o { [o, l, h, cl] } else { [o, h, l, cl] };
        for (k, &px) in path.iter().enumerate() {
            let q = exit_side(d, px, c.sp);
            if d * (q - stop) <= 0.0 {
                // A gap through the stop at the open fills at the open.
                let px = if k == 0 { q } else { stop };
                return Exit {
                    price: px - d * p.slippage,
                    time: x.time,
                    outcome: state,
                    part,
                };
            }
            // A partial above (or at) the full target cannot execute first, even
            // when one OHLC segment jumps over both resting limits.
            if p.part_frac > 0.0 && d * (leg.tp - leg.part_px) > 0.0 && !part && d * (q - leg.part_px) >= 0.0 {
                part = true;
            }
            // The target is a resting limit: it fills at its price.
            if d * (q - leg.tp) >= 0.0 {
                return Exit {
                    price: leg.tp,
                    time: x.time,
                    outcome: "tp",
                    part,
                };
            }
            if d * (q - best) > 0.0 {
                best = q;
            }
            let fav = d * (best - leg.entry);
            if p.be_r > 0.0 && !be_done && fav >= p.be_r * leg.risk {
                be_done = true;
                if d * (be_px - stop) > 0.0 {
                    stop = be_px;
                    state = "be";
                }
            }
            if p.trail_atr > 0.0 && fav >= p.trail_from_r * leg.risk {
                let t = best - d * p.trail_atr * c.atr;
                if d * (t - stop) > 0.0 {
                    stop = t;
                    state = "trail";
                }
            }
        }
        // Volume traded at the density price again: how much of it is gone.
        if dens_on && d * (adverse(d, x) - leg.zone) <= 0.03 * c.atr {
            eaten += against(d, x);
            if eaten >= p.dens_eat * leg.dens {
                return Exit {
                    price: exit_side(d, cl, c.sp) - d * p.slippage,
                    time: x.time,
                    outcome: "dens",
                    part,
                };
            }
        }
        if p.eat_vol > 0.0 {
            flow += against(d, x);
            while s[lo].time <= x.time - w {
                flow -= against(d, &s[lo]);
                lo += 1;
            }
            if lo > j {
                lo = j;
            }
            if flow >= eat && d * (cl - c.level) < 0.0 {
                return Exit {
                    price: exit_side(d, cl, c.sp) - d * p.slippage,
                    time: x.time,
                    outcome: "flow",
                    part,
                };
            }
        }
    }
    let (px, time) = last.map_or((leg.entry, leg.entry_time), |x| (x.close as f64, x.time));
    Exit {
        price: exit_side(d, px, c.sp) - d * p.slippage,
        time,
        outcome: "time",
        part,
    }
}

struct Absorbed {
    entry: f64,
    stop: f64,
    /// Second of the entry (the fill second for a limit).
    k: usize,
    /// A limit filled at the extreme of second `k`: that extreme may already stop it.
    limit: bool,
    ext: f64,
    dens: f64,
}

/// Absorption entry.
fn absorb(s: &[Sec], t: &Touch, bar_end: i64, end: i64, c: &Ctx) -> Option<Absorbed> {
    let (p, d) = (c.p, t.dir as f64);
    let reach = s
        .iter()
        .take_while(|x| x.time < bar_end)
        .position(|x| d * (t.fill - adverse(d, x)) >= 0.0)?;
    let t_reach = s[reach].time;
    let w = p.abs_window.max(1.0) as i64;
    let need = p.abs_vol * c.vbase * w as f64;
    let (mut ext, mut into, mut lo) = (adverse(d, &s[reach]), 0.0f64, reach);
    for j in reach..s.len() {
        let x = &s[j];
        if x.time > t_reach + p.abs_wait as i64 || x.time >= end {
            return None;
        }
        let a = adverse(d, x);
        if d * (a - ext) < 0.0 {
            ext = a;
        }
        // Pushed through the level: nothing held it.
        if d * (t.level - ext) > p.abs_hold_atr * c.atr {
            return None;
        }
        into += against(d, x);
        while s[lo].time <= x.time - w {
            into -= against(d, &s[lo]);
            lo += 1;
        }
        if into >= need && d * (x.close as f64 - ext) >= p.abs_confirm_atr * c.atr {
            let stop = exit_side(d, ext, c.sp) - d * p.abs_stop_atr * c.atr;
            if !p.abs_limit {
                let nx = s.get(j + 1).filter(|n| n.time < end)?;
                let entry = entry_side(d, nx.open as f64, c.sp) + d * p.slippage;
                return Some(Absorbed {
                    entry,
                    stop,
                    k: j + 1,
                    limit: false,
                    ext,
                    dens: into,
                });
            }
            // Rest a limit off the extreme and wait for the pull-back.
            let px = ext + d * p.abs_limit_atr * c.atr;
            let deadline = x.time + p.abs_wait as i64;
            let k = (j + 1..s.len())
                .take_while(|&k| s[k].time < end && s[k].time <= deadline)
                .find(|&k| {
                    let y = &s[k];
                    if d > 0.0 {
                        px - (y.low as f64 + c.sp) >= p.fill_through
                    } else {
                        y.high as f64 - px >= p.fill_through
                    }
                })?;
            return Some(Absorbed {
                entry: px,
                stop,
                k,
                limit: true,
                ext,
                dens: into,
            });
        }
    }
    None
}

/// Simulates one touch on 1-second candles. `None`: no seconds for the touch bar, no fill, or
/// a stop size out of range. The returned index is the bar of the final exit.
pub fn simulate_secs(bars: &[Bar], secs: &[Sec], t: &Touch, p: &Params) -> Option<(Trade, usize)> {
    let b = bars.get(t.i)?;
    let d = t.dir as f64;
    let (t0, bar_end) = (b.time, b.time + 300);
    let last = (t.i + p.max_bars.max(1)).min(bars.len() - 1);
    if last <= t.i {
        return None;
    }
    let end = bars[last].time + 300;
    let hold = p.max_bars.max(1) as i64 * 300;
    let a = secs.partition_point(|x| x.time < t0);
    let z = secs.partition_point(|x| x.time < end + hold);
    let s = &secs[a..z];
    if s.first().is_none_or(|x| x.time >= bar_end) {
        return None;
    }
    let k0 = t.i.saturating_sub(12);
    let vol: f64 = bars[k0..t.i].iter().map(|b| b.volume).filter(|v| v.is_finite()).sum();
    let c = Ctx {
        p,
        sp: spread(b, p.spread),
        atr: t.atr,
        level: t.level,
        vbase: vol / ((t.i - k0).max(1) as f64 * 300.0),
    };
    let (leg, start, first) = if p.on_close() {
        let k = s.partition_point(|x| x.time < bar_end);
        let x = s.get(k).filter(|x| x.time < end)?;
        let stop = if d > 0.0 {
            b.low.min(t.level) - p.sl_atr * t.atr
        } else {
            b.high.max(t.level) + p.sl_atr * t.atr
        };
        let entry = entry_side(d, x.open as f64, c.sp) + d * p.slippage;
        (Leg::new(&c, d, entry, stop, p.tp_r, p.taker_bps, x.time)?, k, None)
    } else if p.absorb {
        let a = absorb(s, t, bar_end, end, &c)?;
        let fee = if a.limit { p.maker_bps } else { p.taker_bps };
        let mut leg = Leg::new(&c, d, a.entry, a.stop, p.tp_r, fee, s[a.k].time)?;
        leg.zone = a.ext;
        leg.dens = a.dens;
        if a.limit {
            let first = fill_second_stop(d, &s[a.k], leg.stop, &c);
            (leg, a.k + 1, first)
        } else {
            (leg, a.k, None)
        }
    } else {
        let k = s.iter().take_while(|x| x.time < bar_end).position(|x| {
            if d > 0.0 {
                t.fill - (x.low as f64 + c.sp) >= p.fill_through
            } else {
                x.high as f64 - t.fill >= p.fill_through
            }
        })?;
        let leg = Leg::new(
            &c,
            d,
            t.fill,
            t.level - d * p.sl_atr * t.atr,
            p.tp_r,
            p.maker_bps,
            s[k].time,
        )?;
        // The fill happened at the second's extreme: the same extreme may already be the stop.
        let first = fill_second_stop(d, &s[k], leg.stop, &c);
        (leg, k + 1, first)
    };
    let x1 = first.unwrap_or_else(|| manage(s, start, end, &leg, &c));
    let r1 = leg.r(&x1, p);
    let mut tr = Trade {
        entry_time: leg.entry_time,
        exit_time: x1.time + 1,
        first_exit_time: x1.time + 1,
        dir: t.dir,
        entry: leg.entry,
        exit: x1.price,
        sl: leg.stop,
        tp: leg.tp,
        level: t.level,
        kinds: t.kinds,
        outcome: x1.outcome,
        r: r1,
        prob: f64::NAN,
        part: if x1.part { leg.part_px } else { f64::NAN },
        ..Trade::empty()
    };
    if p.flip && matches!(x1.outcome, "sl" | "flow" | "dens") {
        let k = s.partition_point(|x| x.time <= x1.time);
        if let Some(nx) = s.get(k) {
            let d2 = -d;
            let entry = entry_side(d2, nx.open as f64, c.sp) + d2 * p.slippage;
            let stop = t.level - d2 * p.flip_sl_atr * t.atr;
            if let Some(leg2) = Leg::new(&c, d2, entry, stop, p.flip_tp_r, p.taker_bps, nx.time) {
                let x2 = manage(s, k, nx.time + hold, &leg2, &c);
                let r2 = leg2.r(&x2, p);
                tr.flip_entry = leg2.entry;
                tr.flip_exit = x2.price;
                tr.flip_sl = leg2.stop;
                tr.flip_time = leg2.entry_time;
                tr.flip_outcome = x2.outcome;
                tr.flip_r = r2;
                tr.r += r2;
                tr.exit_time = x2.time + 1;
            }
        }
    }
    let j = bars
        .partition_point(|b| b.time < tr.exit_time)
        .saturating_sub(1)
        .max(t.i);
    Some((tr, j))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(level: f64, fill: f64, dir: i8) -> Touch {
        Touch {
            i: 12,
            time: 12 * 300,
            dir,
            level,
            kinds: 1,
            atr: 1.0,
            fill,
            f: [f64::NAN; super::super::NF],
            pre: [f64::NAN; super::super::NF],
        }
    }

    /// 5m bars with 300 volume each (1 per second on average) and flat prices.
    fn bars(n: usize) -> Vec<Bar> {
        (0..n)
            .map(|i| Bar::ohlcv(i as i64 * 300, 100.0, 100.5, 99.5, 100.0, 300.0))
            .collect()
    }

    fn sec(time: i64, o: f32, h: f32, l: f32, c: f32, buy: f32, sell: f32) -> Sec {
        Sec {
            time,
            open: o,
            high: h,
            low: l,
            close: c,
            buy,
            sell,
        }
    }

    /// A path of closes, one per second from `t0`, with small volume.
    fn path(t0: i64, closes: &[f32]) -> Vec<Sec> {
        let mut prev = closes[0];
        closes
            .iter()
            .enumerate()
            .map(|(k, &c)| {
                let s = sec(t0 + k as i64, prev, prev.max(c), prev.min(c), c, 0.1, 0.1);
                prev = c;
                s
            })
            .collect()
    }

    fn costless() -> Params {
        Params {
            sec_engine: true,
            maker_bps: 0.0,
            taker_bps: 0.0,
            slippage: 0.0,
            spread: 0.0,
            fill_through: 0.0,
            min_risk_atr: 0.0,
            sl_atr: 0.5,
            tp_r: 2.0,
            ..Params::default()
        }
    }

    #[test]
    fn partial_at_or_above_full_target_never_executes_first() {
        for d in [1.0, -1.0] {
            for part_r in [1.0, 2.0] {
                let p = Params {
                    part_r,
                    part_frac: 0.5,
                    ..costless()
                };
                let c = Ctx {
                    p: &p,
                    sp: 0.0,
                    atr: 1.0,
                    level: 100.0,
                    vbase: 1.0,
                };
                // Also covers a flip leg with a smaller full target than the
                // shared partial setting: an OHLC jump reaches both prices.
                let leg = Leg::new(&c, d, 100.0, 100.0 - d, 1.0, 0.0, 0).unwrap();
                let s = if d > 0.0 {
                    vec![sec(0, 100.0, 103.0, 99.9, 102.0, 0.1, 0.1)]
                } else {
                    vec![sec(0, 100.0, 100.1, 97.0, 98.0, 0.1, 0.1)]
                };
                let x = manage(&s, 0, 10, &leg, &c);
                assert_eq!(x.outcome, "tp");
                assert!(!x.part, "a partial cannot fill beyond the full exit");
                assert!((leg.r(&x, &p) - 1.0).abs() < 1e-9);
            }
        }
    }

    #[test]
    fn fill_second_gap_stops_use_adverse_open_for_long_and_short() {
        let b = bars(40);
        let t0 = 12 * 300;
        for dir in [1, -1] {
            let p = Params {
                slippage: 0.1,
                spread: 0.25,
                ..costless()
            };
            let open = if dir > 0 { 98.0 } else { 102.0 };
            let s = vec![sec(t0, open, open, open, open, 0.1, 0.1)];
            let (tr, _) = simulate_secs(&b, &s, &touch(100.0, 100.0, dir), &p).unwrap();
            let expected = if dir > 0 { 97.9 } else { 102.35 };
            assert_eq!(tr.outcome, "sl");
            assert!((tr.exit - expected).abs() < 1e-9);
            assert!(tr.r < -4.0);
            assert_eq!(tr.first_exit_time, t0 + 1);
        }
    }

    #[test]
    fn fill_second_intrasecond_stop_still_fills_at_stop() {
        let p = costless();
        let c = Ctx {
            p: &p,
            sp: 0.0,
            atr: 1.0,
            level: 100.0,
            vbase: 1.0,
        };
        for d in [1.0, -1.0] {
            let x = sec(0, 100.0, 102.0, 98.0, 100.0, 0.1, 0.1);
            let stop = 100.0 - d * 0.5;
            let exit = fill_second_stop(d, &x, stop, &c).unwrap();
            assert_eq!(exit.price, stop);
        }
    }

    #[test]
    fn limit_fill_then_target() {
        let b = bars(40);
        let t0 = 12 * 300;
        let s = path(t0, &[101.0, 100.5, 100.0, 100.2, 100.6, 101.1]);
        let (tr, _) = simulate_secs(&b, &s, &touch(100.0, 100.0, 1), &costless()).unwrap();
        assert_eq!((tr.outcome, tr.entry, tr.sl, tr.tp), ("tp", 100.0, 99.5, 101.0));
        assert!((tr.r - 2.0).abs() < 1e-9);
    }

    #[test]
    fn breakeven_and_trailing_move_the_stop() {
        let b = bars(40);
        let t0 = 12 * 300;
        let prices = [100.5, 100.0, 100.3, 100.6, 99.9];
        let p = Params {
            be_r: 0.5,
            ..costless()
        };
        let (tr, _) = simulate_secs(&b, &path(t0, &prices), &touch(100.0, 100.0, 1), &p).unwrap();
        assert_eq!(tr.outcome, "be");
        assert!(tr.r.abs() < 1e-9);
        let p = Params {
            trail_atr: 0.3,
            trail_from_r: 0.5,
            ..costless()
        };
        let (tr, _) = simulate_secs(&b, &path(t0, &prices), &touch(100.0, 100.0, 1), &p).unwrap();
        assert_eq!(tr.outcome, "trail");
        assert!((tr.exit - 100.3).abs() < 1e-4, "{}", tr.exit);
    }

    #[test]
    fn partial_take_profit_is_weighted() {
        let b = bars(40);
        let p = Params {
            part_frac: 0.5,
            part_r: 1.0,
            ..costless()
        };
        let s = path(12 * 300, &[100.5, 100.0, 100.5, 99.4]);
        let (tr, _) = simulate_secs(&b, &s, &touch(100.0, 100.0, 1), &p).unwrap();
        assert_eq!(tr.outcome, "sl");
        // Half at +1 R, half at −1 R.
        assert!(tr.r.abs() < 1e-9, "{}", tr.r);
        assert!((tr.part - 100.5).abs() < 1e-9);
    }

    #[test]
    fn absorption_enters_with_a_stop_behind_the_extreme() {
        let b = bars(40);
        let t0 = 12 * 300;
        let mut s = path(t0, &[100.5, 100.1, 99.95, 99.9, 99.92, 99.95, 100.0, 100.05]);
        // Heavy selling into the level while price holds, then the bounce.
        for x in &mut s[2..6] {
            x.sell = 20.0;
        }
        s.extend(path(t0 + 8, &[100.05, 100.5, 101.0, 101.5, 102.0]));
        let p = Params {
            absorb: true,
            abs_window: 10.0,
            abs_vol: 5.0,
            abs_hold_atr: 0.2,
            abs_confirm_atr: 0.08,
            abs_stop_atr: 0.05,
            tp_r: 3.0,
            ..costless()
        };
        let (tr, _) = simulate_secs(&b, &s, &touch(100.0, 100.0, 1), &p).unwrap();
        assert!((tr.sl - 99.85).abs() < 1e-4, "stop {}", tr.sl);
        assert!((tr.entry - 100.0).abs() < 1e-4, "entry {}", tr.entry);
        assert_eq!(tr.outcome, "tp");
        assert!((tr.r - 3.0).abs() < 1e-6);
        // Without the volume there is no entry.
        let mut quiet = s.clone();
        quiet.iter_mut().for_each(|x| x.sell = 0.1);
        assert!(simulate_secs(&b, &quiet, &touch(100.0, 100.0, 1), &p).is_none());
    }

    #[test]
    fn density_exit_leaves_before_the_stop_and_limit_waits_for_the_pull_back() {
        let b = bars(40);
        let t0 = 12 * 300;
        let mut s = path(t0, &[100.5, 100.1, 99.95, 99.9, 99.92, 99.95, 100.0, 100.05]);
        for x in &mut s[2..6] {
            x.sell = 20.0;
        }
        // Back to the density: sellers eat it (3 × 20 = 75% of the 80 absorbed), then it breaks.
        s.extend(path(t0 + 8, &[100.05, 99.95, 99.91, 99.9, 99.9, 99.88, 99.7]));
        for x in &mut s[10..13] {
            x.sell = 20.0;
        }
        let p = Params {
            absorb: true,
            abs_window: 10.0,
            abs_vol: 5.0,
            abs_hold_atr: 0.2,
            abs_confirm_atr: 0.08,
            abs_stop_atr: 0.05,
            dens_eat: 0.7,
            tp_r: 3.0,
            ..costless()
        };
        let (tr, _) = simulate_secs(&b, &s, &touch(100.0, 100.0, 1), &p).unwrap();
        assert_eq!(tr.outcome, "dens");
        assert!(tr.exit > tr.sl, "left at {} before the stop {}", tr.exit, tr.sl);
        // A limit 0.05 ATR off the extreme (99.95) fills on the pull-back, not at the confirmation.
        let p = Params {
            abs_limit: true,
            abs_limit_atr: 0.05,
            dens_eat: 0.0,
            ..p
        };
        let (tr, _) = simulate_secs(&b, &s, &touch(100.0, 100.0, 1), &p).unwrap();
        assert!((tr.entry - 99.95).abs() < 1e-4, "entry {}", tr.entry);
        assert_eq!(tr.entry_time, t0 + 9);
    }

    #[test]
    fn flow_exit_flips_into_the_breakout() {
        let b = bars(40);
        let t0 = 12 * 300;
        let mut s = path(t0, &[100.5, 100.0, 99.9, 99.8, 99.7]);
        for x in &mut s[3..5] {
            x.sell = 50.0;
        }
        s.extend(path(t0 + 5, &[99.7, 99.4, 99.0, 98.5]));
        let p = Params {
            sl_atr: 1.0,
            eat_vol: 10.0,
            abs_window: 5.0,
            flip: true,
            flip_sl_atr: 0.5,
            flip_tp_r: 1.0,
            ..costless()
        };
        let (tr, _) = simulate_secs(&b, &s, &touch(100.0, 100.0, 1), &p).unwrap();
        assert_eq!((tr.outcome, tr.flip_outcome), ("flow", "tp"));
        // Out at 99.8 (−0.2 R), short at 99.7 with the stop at 100.5: +1 R.
        assert!((tr.r - 0.8).abs() < 1e-4, "{}", tr.r);
    }
}
