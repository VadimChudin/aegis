//! Every slider and toggle of the Bounce strategy. Each setting has a string id
//! ("tp_r", "kinds.swing5", "filters.wick.min", ...) used by the window, the genetic algorithm
//! and the liveness check, so all three change exactly the same thing.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{
    features::{index, F, FEATURES, NF},
    scan::{ScanConfig, Touch, KINDS},
};

/// A metric filter: the touch passes when `min <= value <= max`. Missing values pass.
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
    /// Price cost per market fill in $ (slippage; half the quoted spread is added when bars have it).
    pub slippage: f64,
    /// Quoted spread in $ assumed when the data has no quotes (trade prints such as Binance).
    /// Buy limits fill only when the ask reaches them; exits pay the spread.
    pub spread: f64,
    pub min_risk_atr: f64,
    pub max_risk_atr: f64,
    /// A limit order fills only when price trades this many $ through it.
    pub fill_through: f64,
    /// Trade only touches whose calibrated win probability is at least `min_prob`.
    pub use_model: bool,
    pub min_prob: f64,
    /// Positions open at the same time (never two on the same level).
    pub max_open: usize,
    /// Metric id → filter.
    pub filters: BTreeMap<String, Filter>,
}

pub const SESSIONS: [(&str, &str); 4] = [
    ("asia", "Asia 00–07 UTC"),
    ("london", "London 07–13 UTC"),
    ("ny", "New York 13–21 UTC"),
    ("late", "Late 21–24 UTC"),
];

/// Neutral defaults: every level kind, a limit at the level, stop 0.5 ATR beyond it, target 1 R,
/// one position, RoboForex ECN costs with a $0.20 spread. With the corrected engine no setting
/// found so far is profitable out of sample (docs/research/bounce.md), so none is suggested here.
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
            maker_bps: 0.2,
            taker_bps: 0.2,
            slippage: 0.15,
            spread: 0.2,
            min_risk_atr: 0.2,
            max_risk_atr: 3.0,
            fill_through: 0.02,
            use_model: true,
            min_prob: 0.4,
            max_open: 1,
            filters: BTreeMap::new(),
        }
    }
}

pub fn session_id(hour: i64) -> &'static str {
    match hour.rem_euclid(24) {
        0..=6 => "asia",
        7..=12 => "london",
        13..=20 => "ny",
        _ => "late",
    }
}

/// Filters compiled to (metric index, min, max).
pub(crate) type Compiled = Vec<(usize, f64, f64)>;

impl Params {
    pub fn kind_mask(&self) -> u16 {
        KINDS
            .iter()
            .filter(|k| self.kinds.get(k.id()).copied().unwrap_or(true))
            .fold(0, |m, k| m | k.bit())
    }

    pub fn on_close(&self) -> bool {
        self.entry == "close"
    }

    /// The metrics this entry mode may use.
    pub fn metrics<'a>(&self, t: &'a Touch) -> &'a [f64; NF] {
        if self.on_close() {
            &t.f
        } else {
            &t.pre
        }
    }

    pub(crate) fn compiled(&self) -> Compiled {
        self.filters
            .iter()
            .filter(|(_, f)| f.on)
            .filter_map(|(id, f)| index(id).map(|k| (k, f.min, f.max)))
            .collect()
    }

    pub fn active_filters(&self) -> usize {
        self.filters.values().filter(|f| f.on).count()
    }

    pub fn passes(&self, t: &Touch) -> bool {
        self.passes_with(t, self.kind_mask(), &self.compiled())
    }

    pub(crate) fn passes_with(&self, t: &Touch, mask: u16, filters: &[(usize, f64, f64)]) -> bool {
        if (t.dir > 0 && !self.long) || (t.dir < 0 && !self.short) || t.kinds & mask == 0 {
            return false;
        }
        let hour = t.f[F::Hour as usize];
        if hour.is_finite() && !self.sessions.get(session_id(hour as i64)).copied().unwrap_or(true) {
            return false;
        }
        let m = self.metrics(t);
        filters.iter().all(|&(k, lo, hi)| {
            let v = m[k];
            v.is_nan() || (v >= lo && v <= hi)
        })
    }

    /// Numeric value of a setting by id; toggles are 0/1. `None` for an unknown id.
    pub fn get(&self, id: &str) -> Option<f64> {
        let b = |x: bool| f64::from(u8::from(x));
        Some(match id {
            "long" => b(self.long),
            "short" => b(self.short),
            "entry" => b(self.on_close()),
            "sl_atr" => self.sl_atr,
            "tp_r" => self.tp_r,
            "max_bars" => self.max_bars as f64,
            "maker_bps" => self.maker_bps,
            "taker_bps" => self.taker_bps,
            "slippage" => self.slippage,
            "spread" => self.spread,
            "min_risk_atr" => self.min_risk_atr,
            "max_risk_atr" => self.max_risk_atr,
            "fill_through" => self.fill_through,
            "use_model" => b(self.use_model),
            "min_prob" => self.min_prob,
            "max_open" => self.max_open as f64,
            "scan.zone_atr" => self.scan.zone_atr,
            "scan.away_atr" => self.scan.away_atr,
            "scan.swing_n" => self.scan.swing_n as f64,
            "scan.round_step" => self.scan.round_step,
            _ => {
                let (head, rest) = id.split_once('.')?;
                match head {
                    "kinds" => b(self.kinds.get(rest).copied().unwrap_or(true)),
                    "sessions" => b(self.sessions.get(rest).copied().unwrap_or(true)),
                    "filters" => {
                        let (metric, part) = rest.rsplit_once('.')?;
                        let k = index(metric)?;
                        let f = self.filters.get(metric).copied().unwrap_or(Filter {
                            on: false,
                            min: FEATURES[k].lo,
                            max: FEATURES[k].hi,
                        });
                        match part {
                            "on" => b(f.on),
                            "min" => f.min,
                            "max" => f.max,
                            _ => return None,
                        }
                    }
                    _ => return None,
                }
            }
        })
    }

    /// Sets a setting by id; toggles take v >= 0.5 as on. Returns false for an unknown id.
    pub fn set(&mut self, id: &str, v: f64) -> bool {
        let on = v >= 0.5;
        match id {
            "long" => self.long = on,
            "short" => self.short = on,
            "entry" => self.entry = if on { "close" } else { "limit" }.into(),
            "sl_atr" => self.sl_atr = v,
            "tp_r" => self.tp_r = v,
            "max_bars" => self.max_bars = v.round().max(1.0) as usize,
            "maker_bps" => self.maker_bps = v,
            "taker_bps" => self.taker_bps = v,
            "slippage" => self.slippage = v,
            "spread" => self.spread = v,
            "min_risk_atr" => self.min_risk_atr = v,
            "max_risk_atr" => self.max_risk_atr = v,
            "fill_through" => self.fill_through = v,
            "use_model" => self.use_model = on,
            "min_prob" => self.min_prob = v,
            "max_open" => self.max_open = v.round().max(1.0) as usize,
            "scan.zone_atr" => self.scan.zone_atr = v,
            "scan.away_atr" => self.scan.away_atr = v,
            "scan.swing_n" => self.scan.swing_n = v.round().max(1.0) as usize,
            "scan.round_step" => self.scan.round_step = v,
            _ => {
                let Some((head, rest)) = id.split_once('.') else {
                    return false;
                };
                match head {
                    "kinds" if KINDS.iter().any(|k| k.id() == rest) => {
                        self.kinds.insert(rest.into(), on);
                    }
                    "sessions" if SESSIONS.iter().any(|(s, _)| *s == rest) => {
                        self.sessions.insert(rest.into(), on);
                    }
                    "filters" => {
                        let Some((metric, part)) = rest.rsplit_once('.') else {
                            return false;
                        };
                        let Some(k) = index(metric) else { return false };
                        let f = self.filters.entry(metric.into()).or_insert(Filter {
                            on: false,
                            min: FEATURES[k].lo,
                            max: FEATURES[k].hi,
                        });
                        match part {
                            "on" => f.on = on,
                            "min" => f.min = v,
                            "max" => f.max = v,
                            _ => return false,
                        }
                    }
                    _ => return false,
                }
            }
        }
        true
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
    /// Changing it needs a new level scan (slower: the model is retrained).
    pub rescan: bool,
    /// The genetic algorithm may tune it. Costs and scan settings describe reality, not a choice.
    pub tunable: bool,
}

/// Everything the settings panel shows. Metric filters come from `FEATURES`.
pub fn param_specs() -> Vec<ParamSpec> {
    let mut v = Vec::new();
    #[allow(clippy::too_many_arguments)]
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
            rescan: id.starts_with("scan.") || id == "entry",
            tunable: !(id.starts_with("scan.")
                || id == "entry"
                || id == "use_model"
                || group == "Costs"
                || id == "min_risk_atr"
                || id == "max_risk_atr"),
        })
    };
    push("use_model", "Probability", "Use probability model", "toggle", 0., 1., 1., "A logistic model scores every touch from its metrics; the score is turned into a win probability for the current trade settings using only earlier months (walk-forward calibration). Every probability in the backtest is out of sample.");
    push(
        "min_prob",
        "Probability",
        "Min win probability",
        "slider",
        0.4,
        0.95,
        0.01,
        "Trade only touches whose calibrated probability is at least this.",
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
        "max_open",
        "Trade",
        "Positions at once",
        "slider",
        1.0,
        10.0,
        1.0,
        "How many positions may be open together (never two on the same level). More positions = more trades per day.",
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
        "Slippage per market fill, $",
        "slider",
        0.0,
        1.0,
        0.01,
        "Added to every market fill. Half the quoted spread is added on top when the data has quotes.",
    );
    push("spread", "Costs", "Spread when data has no quotes, $", "slider", 0.0, 1.0, 0.01, "Binance history has trades, not quotes. RoboForex ECN gold: about $0.10–0.30. Buy limits fill only when the ask reaches them, and every exit pays the spread.");
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
        "A bar touches a level when it comes this close; the limit order rests at this distance.",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_spec_round_trips_through_get_and_set() {
        for s in param_specs() {
            let ids: Vec<String> = if s.kind == "filter" {
                ["on", "min", "max"].iter().map(|p| format!("{}.{p}", s.id)).collect()
            } else {
                vec![s.id.clone()]
            };
            for id in ids {
                let mut p = Params::default();
                let want = if id.ends_with(".on") || s.kind == "toggle" {
                    1.0 - p.get(&id).unwrap()
                } else {
                    let mid = s.lo + ((s.hi - s.lo) / 2.0 / s.step).round() * s.step;
                    if (mid - p.get(&id).unwrap()).abs() < 1e-9 {
                        s.hi
                    } else {
                        mid
                    }
                };
                assert!(p.set(&id, want), "set {id}");
                assert!(
                    (p.get(&id).unwrap() - want).abs() < 1e-9,
                    "{id}: {} != {want}",
                    p.get(&id).unwrap()
                );
            }
        }
        assert!(!Params::default().set("nope", 1.0));
    }
}
