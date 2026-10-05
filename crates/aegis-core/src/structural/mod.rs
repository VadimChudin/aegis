//! Experimental prior-day raid → reclaim → structure confirmation. No live orders.
mod calendar;
pub mod history;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize)]
pub struct Spec {
    pub id: &'static str,
    pub group: &'static str,
    pub label: &'static str,
    pub lo: f64,
    pub hi: f64,
    pub step: f64,
    pub default: f64,
    pub toggle: bool,
}
pub fn specs() -> Vec<Spec> {
    let rows = [
        ("long", "Direction", "Long", 0., 1., 1., 1.),
        ("short", "Direction", "Short", 0., 1., 1., 1.),
        ("approach", "Signal", "Approach (%)", 0., 2., 0.05, 0.2),
        ("lookback", "Signal", "Approach lookback (minutes)", 10., 240., 5., 60.),
        ("structure", "Signal", "Structure lookback (minutes)", 1., 30., 1., 5.),
        ("confirm", "Signal", "Confirmation timeout (minutes)", 1., 60., 1., 15.),
        ("flow", "Signal", "Aggressor volume confirmation", 0., 1., 1., 0.),
        ("flow_ratio", "Signal", "Aggressor volume ratio", 1., 5., 0.1, 1.),
        ("sweep", "Signal", "Minimum raid ($/oz)", 0., 5., 0.05, 0.),
        ("reclaim", "Signal", "Reclaim buffer ($/oz)", 0., 5., 0.05, 0.),
        ("once", "Signal", "One attempt per level per day", 0., 1., 1., 1.),
        ("open_hour", "Session", "Start hour (New York)", 0., 23., 1., 7.),
        ("close_hour", "Session", "End hour (New York)", 1., 24., 1., 16.),
        ("weekends", "Session", "Allow weekends", 0., 1., 1., 0.),
        ("limit", "Entry", "Limit retest (off = market)", 0., 1., 1., 0.),
        ("expiry", "Entry", "Limit expiry (minutes)", 1., 60., 1., 15.),
        ("delay", "Entry", "Decision latency (ms)", 0., 2000., 50., 250.),
        ("through", "Entry", "Limit trade-through ($/oz)", 0., 1., 0.01, 0.01),
        ("stop_buffer", "Exit", "Stop buffer ($/oz)", 0., 5., 0.05, 0.5),
        ("rr", "Exit", "Target R from original level", 0.5, 5., 0.1, 2.),
        ("space", "Exit", "Require space to approach extreme", 0., 1., 1., 1.),
        ("hold", "Exit", "Maximum holding (minutes)", 1., 240., 1., 120.),
        ("min_risk", "Risk", "Minimum stop distance ($/oz)", 0.1, 30., 0.1, 1.),
        ("max_risk", "Risk", "Maximum stop distance ($/oz)", 1., 100., 1., 100.),
        ("cost_gate", "Risk", "Cost/risk filter", 0., 1., 1., 0.),
        ("max_cost", "Risk", "Maximum cost/risk (%)", 1., 100., 1., 15.),
        ("min_rr", "Risk", "Minimum remaining gross R/R", 0., 3., 0.05, 0.),
        ("maker", "Costs", "Maker fee (bp per side)", 0., 10., 0.05, 0.),
        ("taker", "Costs", "Taker fee (bp per side)", 0., 10., 0.05, 2.75),
        ("slip", "Costs", "Market slippage ($/oz per side)", 0., 1., 0.01, 0.05),
        ("reserve", "Costs", "Cost reserve ($/oz)", 0., 1., 0.01, 0.2),
    ];
    rows.into_iter()
        .map(|(id, group, label, lo, hi, step, default)| Spec {
            id,
            group,
            label,
            lo,
            hi,
            step,
            default,
            toggle: hi == 1. && lo == 0. && step == 1.,
        })
        .collect()
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Params {
    pub values: BTreeMap<String, f64>,
}
impl Default for Params {
    fn default() -> Self {
        Self {
            values: specs().into_iter().map(|s| (s.id.into(), s.default)).collect(),
        }
    }
}
impl Params {
    pub fn get(&self, id: &str) -> f64 {
        self.values[id]
    }
    pub fn on(&self, id: &str) -> bool {
        self.get(id) >= 0.5
    }
    pub fn validate(&self) -> Result<(), String> {
        for s in specs() {
            let v = self
                .values
                .get(s.id)
                .ok_or_else(|| format!("missing parameter {}", s.id))?;
            if !v.is_finite() || *v < s.lo || *v > s.hi {
                return Err(format!("invalid {}", s.id));
            }
            if s.toggle && *v != 0. && *v != 1. {
                return Err(format!("invalid toggle {}", s.id));
            }
        }
        if self.get("open_hour") >= self.get("close_hour") {
            return Err("start hour must precede end hour".into());
        }
        if self.get("min_risk") > self.get("max_risk") {
            return Err("minimum risk exceeds maximum risk".into());
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Tick {
    pub time: f64,
    pub price: f64,
    pub qty: f64,
    pub buy: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct Minute {
    pub time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub buy: f64,
    pub sell: f64,
}
pub fn minutes(ticks: &[Tick]) -> Vec<Minute> {
    let mut out: Vec<Minute> = Vec::new();
    for t in ticks {
        let time = (t.time as i64 / 60) * 60;
        if out.last().is_none_or(|m| m.time != time) {
            out.push(Minute {
                time,
                open: t.price,
                high: t.price,
                low: t.price,
                close: t.price,
                buy: 0.,
                sell: 0.,
            });
        }
        let m = out.last_mut().expect("minute inserted");
        m.high = m.high.max(t.price);
        m.low = m.low.min(t.price);
        m.close = t.price;
        if t.buy {
            m.buy += t.qty;
        } else {
            m.sell += t.qty;
        }
    }
    out
}
#[derive(Clone, Debug, Serialize)]
pub struct Trade {
    pub entry: f64,
    pub exit: f64,
    pub side: i32,
    pub entry_px: f64,
    pub stop: f64,
    pub target: f64,
    pub exit_px: f64,
    pub gross_r: f64,
    pub net_r: f64,
    pub reason: &'static str,
}
#[derive(Default, Debug, Serialize)]
pub struct Report {
    pub signals: usize,
    pub expired: usize,
    pub invalidated: usize,
    pub cost_rejected: usize,
    pub risk_rejected: usize,
    pub target_passed: usize,
    pub overlapping: usize,
    pub trades: Vec<Trade>,
    pub total_r: f64,
    pub avg_r: f64,
    pub win_rate: f64,
    pub max_dd_r: f64,
    pub equity: Vec<[f64; 2]>,
    pub months: BTreeMap<String, [f64; 2]>,
    pub censored: usize,
}
pub fn backtest(ticks: &[Tick], p: &Params) -> Result<Report, String> {
    p.validate()?;
    let m = minutes(ticks);
    let mut out = Report::default();
    let mut ranges: BTreeMap<i64, (f64, f64)> = BTreeMap::new();
    for x in &m {
        let r = ranges.entry(x.time / 86400).or_insert((x.high, x.low));
        r.0 = r.0.max(x.high);
        r.1 = r.1.min(x.low);
    }
    let look = p.get("lookback") as usize;
    let structure = p.get("structure") as usize;
    let mut seen = BTreeSet::new();
    let mut busy = 0.;
    for i in look.max(structure)..m.len() {
        let x = &m[i];
        let day = x.time / 86400;
        let ny = x.time + calendar::ny_offset(x.time);
        let hr = ny.rem_euclid(86400) / 3600;
        if hr < p.get("open_hour") as i64
            || hr >= p.get("close_hour") as i64
            || (!p.on("weekends") && calendar::weekday(ny.div_euclid(86400)) > 4)
        {
            continue;
        }
        if x.time - m[i - look].time != look as i64 * 60 {
            continue;
        }
        let Some(&(pdh, pdl)) = ranges.get(&(day - 1)) else {
            continue;
        };
        let long = x.low < pdl - p.get("sweep");
        let short = x.high > pdh + p.get("sweep");
        if long == short {
            continue;
        }
        let d = if long { 1. } else { -1. };
        if (long && !p.on("long")) || (short && !p.on("short")) || (p.on("once") && seen.contains(&(day, long))) {
            continue;
        }
        let ah = m[i - look..i].iter().map(|x| x.high).fold(f64::NEG_INFINITY, f64::max);
        let al = m[i - look..i].iter().map(|x| x.low).fold(f64::INFINITY, f64::min);
        let approach = if long { ah - x.close } else { x.close - al };
        if approach / x.close < p.get("approach") / 100. {
            continue;
        }
        seen.insert((day, long));
        let level = if long { pdl } else { -pdh };
        let sh = if long {
            m[i - structure..i]
                .iter()
                .map(|x| x.high)
                .fold(f64::NEG_INFINITY, f64::max)
        } else {
            -m[i - structure..i].iter().map(|x| x.low).fold(f64::INFINITY, f64::min)
        };
        let mut extreme = if long { x.low } else { -x.high };
        for (j, b) in m
            .iter()
            .enumerate()
            .take((i + p.get("confirm") as usize + 1).min(m.len()))
            .skip(i)
        {
            if b.time - x.time != (j - i) as i64 * 60 {
                break;
            }
            extreme = extreme.min(if long { b.low } else { -b.high });
            if d * b.close <= (level + p.get("reclaim")).max(sh) {
                continue;
            }
            if p.on("flow")
                && (if long {
                    b.buy <= b.sell * p.get("flow_ratio")
                } else {
                    b.sell <= b.buy * p.get("flow_ratio")
                })
            {
                continue;
            }
            let stop = extreme - p.get("stop_buffer");
            let original_risk = level - stop;
            if original_risk < p.get("min_risk") {
                break;
            }
            let target = level + p.get("rr") * original_risk;
            if p.on("space") && (if long { ah } else { -al }) < target {
                break;
            }
            out.signals += 1;
            let decision = (b.time + 60) as f64 + p.get("delay") / 1000.;
            if decision < busy {
                out.overlapping += 1;
                break;
            }
            let mut k = ticks.partition_point(|t| t.time < decision);
            if k == ticks.len() {
                out.expired += 1;
                break;
            }
            let mut entry = d * ticks[k].price + p.get("slip");
            if p.on("limit") {
                entry = level;
                while k < ticks.len() && ticks[k].time <= decision + p.get("expiry") * 60. {
                    let px = d * ticks[k].price;
                    if px <= stop || px >= target || px < level - p.get("through") {
                        break;
                    }
                    k += 1;
                }
                if k == ticks.len() || ticks[k].time > decision + p.get("expiry") * 60. {
                    out.expired += 1;
                    break;
                }
                if d * ticks[k].price <= stop {
                    out.invalidated += 1;
                    break;
                }
                if d * ticks[k].price >= target {
                    out.target_passed += 1;
                    break;
                }
            }
            let risk = entry - stop;
            if risk < p.get("min_risk") || risk > p.get("max_risk") {
                out.risk_rejected += 1;
                break;
            }
            if target <= entry || (target - entry) / risk < p.get("min_rr") {
                out.target_passed += 1;
                break;
            }
            if p.on("cost_gate")
                && (2. * entry.abs() * p.get("taker") / 1e4 + p.get("reserve")) / risk > p.get("max_cost") / 100.
            {
                out.cost_rejected += 1;
                break;
            }
            let deadline =
                (ticks[k].time + p.get("hold") * 60.).min(((ticks[k].time / 86400.).floor() + 1.) * 86400. - 1.);
            let mut e = ticks.len() - 1;
            let mut px = d * ticks[e].price - p.get("slip");
            let mut reason = "end";
            for (n, t) in ticks.iter().enumerate().skip(k + 1) {
                let price = d * t.price;
                if price <= stop {
                    e = n;
                    px = price - p.get("slip");
                    reason = "stop";
                    break;
                }
                if price > target + p.get("through") {
                    e = n;
                    px = target;
                    reason = "target";
                    break;
                }
                if t.time >= deadline {
                    e = n;
                    px = price - p.get("slip");
                    reason = "time";
                    break;
                }
            }
            let fee = entry.abs() * p.get(if p.on("limit") { "maker" } else { "taker" }) / 1e4
                + px.abs() * p.get(if reason == "target" { "maker" } else { "taker" }) / 1e4;
            out.trades.push(Trade {
                entry: ticks[k].time,
                exit: ticks[e].time,
                side: d as i32,
                entry_px: d * entry,
                stop: d * stop,
                target: d * target,
                exit_px: d * px,
                gross_r: (px - entry) / risk,
                net_r: (px - entry - fee) / risk,
                reason,
            });
            busy = ticks[e].time;
            break;
        }
    }
    let mut peak = 0f64;
    for t in &out.trades {
        out.total_r += t.net_r;
        peak = peak.max(out.total_r);
        out.max_dd_r = out.max_dd_r.min(out.total_r - peak);
        out.equity.push([t.exit, out.total_r]);
        let (y, mo, _) = calendar::civil_from_days(t.entry as i64 / 86400);
        let a = out.months.entry(format!("{y:04}-{mo:02}")).or_insert([0., 0.]);
        a[0] += 1.;
        a[1] += t.net_r;
        if t.reason == "end" {
            out.censored += 1;
        }
    }
    if !out.trades.is_empty() {
        out.avg_r = out.total_r / out.trades.len() as f64;
        out.win_rate = out.trades.iter().filter(|t| t.net_r > 0.).count() as f64 / out.trades.len() as f64;
    }
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settings_are_bounded() {
        let mut p = Params::default();
        assert!(p.validate().is_ok());
        assert_eq!(p.values.len(), specs().len());
        p.values.insert("taker".into(), f64::NAN);
        assert!(p.validate().is_err());
    }
    #[test]
    fn aggregates_trades() {
        let a = minutes(&[
            Tick {
                time: 60.,
                price: 100.,
                qty: 2.,
                buy: true,
            },
            Tick {
                time: 61.,
                price: 99.,
                qty: 3.,
                buy: false,
            },
        ]);
        assert_eq!((a[0].open, a[0].close, a[0].buy, a[0].sell), (100., 99., 2., 3.));
    }
    #[test]
    fn flat_history_has_no_signals() {
        let t: Vec<_> = (0..3000)
            .map(|i| Tick {
                time: (i * 60) as f64,
                price: 100.,
                qty: 1.,
                buy: true,
            })
            .collect();
        assert!(backtest(&t, &Params::default()).unwrap().trades.is_empty());
        assert_eq!(backtest(&[], &Params::default()).unwrap().total_r, 0.);
    }

    fn episode(gap: bool) -> Vec<Tick> {
        let day = calendar::days(2026, 8, 10) * 86400;
        let mut t = vec![
            Tick {
                time: (day - 86400 + 36000) as f64,
                price: 100.,
                qty: 1.,
                buy: true,
            },
            Tick {
                time: (day - 86400 + 36060) as f64,
                price: 110.,
                qty: 1.,
                buy: true,
            },
        ];
        for i in 0..70 {
            t.push(Tick {
                time: (day + 11 * 3600 + i * 60) as f64,
                price: if i == 20 { 110. } else { 105. },
                qty: 1.,
                buy: true,
            });
        }
        let a = day + 11 * 3600 + 70 * 60;
        for (offset, price) in [
            (0., 99.),
            (10., 98.),
            (40., 106.),
            (60.3, 101.),
            (61., if gap { 94. } else { 106. }),
        ] {
            t.push(Tick {
                time: a as f64 + offset,
                price,
                qty: 1.,
                buy: true,
            });
        }
        t
    }

    #[test]
    fn confirmations_use_completed_bars_and_actual_exit_ticks() {
        let mut p = Params::default();
        p.values.insert("slip".into(), 0.);
        p.values.insert("taker".into(), 0.);
        p.values.insert("space".into(), 0.);
        let r = backtest(&episode(false), &p).unwrap();
        assert_eq!(r.trades.len(), 1);
        let t = &r.trades[0];
        assert_eq!(t.entry_px, 101.);
        assert_eq!(t.target, 105.);
        assert_eq!(t.reason, "target");
        assert!(t.entry.fract() > 0. && t.entry > t.exit - 1.);
        let gap = backtest(&episode(true), &p).unwrap();
        assert_eq!(gap.trades[0].reason, "stop");
        assert!(gap.trades[0].gross_r < -1.);
        p.values.insert("limit".into(), 1.);
        assert!(backtest(&episode(false), &p).unwrap().trades.is_empty());
    }

    #[test]
    fn changing_costs_direction_and_remaining_reward_changes_results() {
        let ticks = episode(false);
        let r = backtest(&ticks, &Params::default()).unwrap();
        assert_eq!(r.trades.len(), 1);
        let mut p = Params::default();
        p.values.insert("taker".into(), 10.);
        assert!(backtest(&ticks, &p).unwrap().total_r < r.total_r);
        p = Params::default();
        p.values.insert("long".into(), 0.);
        assert!(backtest(&ticks, &p).unwrap().trades.is_empty());
        p = Params::default();
        p.values.insert("min_rr".into(), 2.);
        assert!(backtest(&ticks, &p).unwrap().trades.is_empty());
    }

    #[test]
    fn short_slippage_reduces_sale_price_and_worsens_buyback() {
        let mut p = Params::default();
        p.values.insert("space".into(), 0.);
        p.values.insert("taker".into(), 0.);
        let ticks: Vec<_> = episode(true)
            .into_iter()
            .map(|t| Tick {
                price: 200. - t.price,
                buy: !t.buy,
                ..t
            })
            .collect();
        p.values.insert("slip".into(), 0.);
        let base = backtest(&ticks, &p).unwrap();
        assert_eq!(base.trades.len(), 1);
        assert_eq!(base.trades[0].side, -1);
        p.values.insert("slip".into(), 0.1);
        let slipped = backtest(&ticks, &p).unwrap();
        assert!(slipped.trades[0].entry_px < base.trades[0].entry_px);
        assert!(slipped.trades[0].exit_px > base.trades[0].exit_px);
        // R uses each execution's own entry-to-stop risk, which also changes with slippage.
        // Compare dollars per unit, not differently normalised R values.
        assert!(
            slipped.trades[0].entry_px - slipped.trades[0].exit_px < base.trades[0].entry_px - base.trades[0].exit_px
        );
    }
}
