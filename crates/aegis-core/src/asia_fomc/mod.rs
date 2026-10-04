//! Asia + FOMC: the two session tiles that survived the gold research (`docs/research/gold_book.md`)
//! as one strategy on the local Dukascopy history.
//!
//! * Asia: long from an evening time to a time after midnight, New York, on chosen evenings.
//! * FOMC: long into a scheduled FOMC statement, from a lead time before 14:00 to shortly before it.
//!
//! Overlapping windows hold one position. Costs: spread (from the data or fixed), commission per
//! million per side, slippage, and the long swap for every 17:00 New York rollover held (triple on
//! Wednesday, none on the weekend). Sizing: one unit, or a volatility target.

mod engine;
mod fomc;
pub mod search;

use serde::{Deserialize, Serialize};

pub use engine::{Prepared, Report, Stats, Trade, YearRow};
pub use fomc::decision_days;
pub use search::{search, SearchProgress, SearchReport, SearchSpec, Step};

/// Every setting of the strategy. Times are New York minutes of the day.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Params {
    pub asia: bool,
    pub asia_enter: f64,
    pub asia_leave: f64,
    pub sun: bool,
    pub mon: bool,
    pub tue: bool,
    pub wed: bool,
    pub thu: bool,
    pub fomc: bool,
    /// Minutes before the 14:00 statement to enter.
    pub fomc_lead: f64,
    /// Minutes before the 14:00 statement to exit.
    pub fomc_exit: f64,
    /// Volatility target, % a year; 0 = one unit.
    pub vol_target: f64,
    pub vol_days: f64,
    pub max_size: f64,
    /// Stop below the entry, % of price; 0 = off.
    pub stop_pct: f64,
    /// Use the spread from the data (ask − bid) where it exists.
    pub data_spread: bool,
    /// Fixed spread, $/oz, when the data has none or `data_spread` is off.
    pub spread: f64,
    /// Commission, USD per million of notional per side.
    pub commission: f64,
    /// Slippage, $/oz per side.
    pub slippage: f64,
    /// Long swap, $/oz for every rollover held.
    pub swap: f64,
}

impl Default for Params {
    /// The setting the walk-forward settled on in `gold_book.md`, with RoboForex ECN costs.
    fn default() -> Self {
        Params {
            asia: true,
            asia_enter: (18 * 60 + 5) as f64,
            asia_leave: 120.0,
            sun: false,
            mon: true,
            tue: true,
            wed: true,
            thu: true,
            fomc: true,
            fomc_lead: (19 * 60 + 55) as f64,
            fomc_exit: 5.0,
            vol_target: 10.0,
            vol_days: 60.0,
            max_size: 2.0,
            stop_pct: 0.0,
            data_spread: true,
            spread: 0.05,
            commission: 20.0,
            slippage: 0.02,
            swap: 0.60,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ParamSpec {
    pub id: &'static str,
    pub group: &'static str,
    pub label: &'static str,
    /// "toggle" or "slider".
    pub kind: &'static str,
    pub lo: f64,
    pub hi: f64,
    pub step: f64,
    /// How the window shows the value: "clock" (minute of day), "minutes", "usd", "pct", "x", "days", "".
    pub unit: &'static str,
    pub help: &'static str,
    /// The search may tune it; costs describe the broker, not a choice.
    pub tunable: bool,
}

pub fn param_specs() -> Vec<ParamSpec> {
    #[allow(clippy::too_many_arguments)]
    fn s(
        id: &'static str,
        group: &'static str,
        label: &'static str,
        kind: &'static str,
        (lo, hi, step): (f64, f64, f64),
        unit: &'static str,
        tunable: bool,
        help: &'static str,
    ) -> ParamSpec {
        ParamSpec {
            id,
            group,
            label,
            kind,
            lo,
            hi,
            step,
            unit,
            help,
            tunable,
        }
    }
    let t = (0.0, 1.0, 1.0);
    vec![
        s("asia", "Asia", "Asian session trade", "toggle", t, "", true, "Long through the Asian session: most of gold's rise since 2008 happened between the evening and the night in New York."),
        s("asia_enter", "Asia", "Enter at (New York)", "slider", (1020.0, 1435.0, 5.0), "clock", true, "Evening entry time, New York. The market reopens at 18:00 after the daily break."),
        s("asia_leave", "Asia", "Exit at (New York, after midnight)", "slider", (0.0, 600.0, 5.0), "clock", true, "Exit time after midnight, New York. Before 17:00 the position pays no swap."),
        s("sun", "Asia", "Sunday evening", "toggle", t, "", true, "Trade the evening that opens the week."),
        s("mon", "Asia", "Monday evening", "toggle", t, "", true, "Trade Monday evening into Tuesday."),
        s("tue", "Asia", "Tuesday evening", "toggle", t, "", true, "Trade Tuesday evening into Wednesday."),
        s("wed", "Asia", "Wednesday evening", "toggle", t, "", true, "Trade Wednesday evening into Thursday."),
        s("thu", "Asia", "Thursday evening", "toggle", t, "", true, "Trade Thursday evening into Friday."),
        s("fomc", "FOMC", "Pre-FOMC trade", "toggle", t, "", true, "Long into scheduled FOMC statements (dates 2008-2026 are built in)."),
        s("fomc_lead", "FOMC", "Enter before the statement", "slider", (30.0, 2880.0, 5.0), "minutes", true, "How long before the 14:00 statement to enter. 19 h 55 min = 18:05 the evening before."),
        s("fomc_exit", "FOMC", "Exit before the statement", "slider", (0.0, 120.0, 5.0), "minutes", true, "How long before the 14:00 statement to be flat."),
        s("vol_target", "Size and risk", "Volatility target", "slider", (0.0, 30.0, 1.0), "pct", true, "Position size = target ÷ gold's recent volatility (a year's worth). 0 = always one unit."),
        s("vol_days", "Size and risk", "Volatility lookback", "slider", (10.0, 250.0, 5.0), "days", true, "Days of closes behind the volatility estimate."),
        s("max_size", "Size and risk", "Largest size", "slider", (0.5, 5.0, 0.5), "x", true, "Cap on the volatility-targeted size, in units of notional per unit of equity."),
        s("stop_pct", "Size and risk", "Stop loss", "slider", (0.0, 3.0, 0.05), "pct", true, "Exit when price falls this far below the entry. 0 = no stop, exit by time only."),
        s("data_spread", "Costs", "Spread from the data", "toggle", t, "", false, "Use the ask − bid of the loaded history at each entry and exit; the fixed spread fills gaps."),
        s("spread", "Costs", "Fixed spread", "slider", (0.0, 0.5, 0.01), "usd", false, "$/oz. RoboForex ECN publishes 5 pips = $0.05."),
        s("commission", "Costs", "Commission", "slider", (0.0, 60.0, 1.0), "", false, "USD per million of notional per side. RoboForex ECN: 20."),
        s("slippage", "Costs", "Slippage", "slider", (0.0, 0.5, 0.01), "usd", false, "$/oz per side."),
        s("swap", "Costs", "Long swap per night", "slider", (0.0, 2.0, 0.05), "usd", false, "$/oz for every 17:00 New York rollover held; triple on Wednesday. RoboForex XAUUSD: −60 pips = $0.60."),
    ]
}

impl Params {
    /// The value of a setting by its id (toggles as 0 / 1).
    pub fn get(&self, id: &str) -> Option<f64> {
        let b = |x: bool| f64::from(u8::from(x));
        Some(match id {
            "asia" => b(self.asia),
            "asia_enter" => self.asia_enter,
            "asia_leave" => self.asia_leave,
            "sun" => b(self.sun),
            "mon" => b(self.mon),
            "tue" => b(self.tue),
            "wed" => b(self.wed),
            "thu" => b(self.thu),
            "fomc" => b(self.fomc),
            "fomc_lead" => self.fomc_lead,
            "fomc_exit" => self.fomc_exit,
            "vol_target" => self.vol_target,
            "vol_days" => self.vol_days,
            "max_size" => self.max_size,
            "stop_pct" => self.stop_pct,
            "data_spread" => b(self.data_spread),
            "spread" => self.spread,
            "commission" => self.commission,
            "slippage" => self.slippage,
            "swap" => self.swap,
            _ => return None,
        })
    }

    pub fn set(&mut self, id: &str, v: f64) {
        let on = v >= 0.5;
        match id {
            "asia" => self.asia = on,
            "asia_enter" => self.asia_enter = v,
            "asia_leave" => self.asia_leave = v,
            "sun" => self.sun = on,
            "mon" => self.mon = on,
            "tue" => self.tue = on,
            "wed" => self.wed = on,
            "thu" => self.thu = on,
            "fomc" => self.fomc = on,
            "fomc_lead" => self.fomc_lead = v,
            "fomc_exit" => self.fomc_exit = v,
            "vol_target" => self.vol_target = v,
            "vol_days" => self.vol_days = v,
            "max_size" => self.max_size = v,
            "stop_pct" => self.stop_pct = v,
            "data_spread" => self.data_spread = on,
            "spread" => self.spread = v,
            "commission" => self.commission = v,
            "slippage" => self.slippage = v,
            "swap" => self.swap = v,
            _ => {}
        }
    }

    /// Evenings traded, indexed by weekday (0 = Monday … 6 = Sunday). Friday evening is closed.
    fn evenings(&self) -> [bool; 7] {
        [self.mon, self.tue, self.wed, self.thu, false, false, self.sun]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_spec_reads_and_writes_its_setting() {
        let specs = param_specs();
        let mut ids: Vec<_> = specs.iter().map(|s| s.id).collect();
        ids.dedup();
        assert_eq!(ids.len(), specs.len());
        let d = Params::default();
        for s in &specs {
            let v = d.get(s.id).unwrap_or_else(|| panic!("{}", s.id));
            assert!((s.lo..=s.hi).contains(&v), "{} default {v} outside {}..{}", s.id, s.lo, s.hi);
            let mut p = d.clone();
            let w = if s.kind == "toggle" { 1.0 - v } else { s.hi };
            p.set(s.id, w);
            assert_eq!(p.get(s.id), Some(w), "{}", s.id);
        }
        assert!(d.get("nope").is_none());
    }
}
