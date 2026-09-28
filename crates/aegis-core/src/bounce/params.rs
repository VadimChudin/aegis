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
    /// Stop taking trades for the day once closed trades of the day lost this many R (0 = off).
    pub day_stop_r: f64,
    /// Money view of the backtest: risk per trade, % of the account, and the leverage limit.
    pub risk_pct: f64,
    pub max_leverage: f64,
    /// Simulate on 1-second candles (needs aggTrades history); enables everything below.
    pub sec_engine: bool,
    /// Enter on absorption at the level instead of the resting limit (see `position.rs`).
    pub absorb: bool,
    /// Seconds of the rolling volume window (absorption and flow exit).
    pub abs_window: f64,
    /// Aggressive volume into the level, × the average of the last hour for the window.
    pub abs_vol: f64,
    /// How far price may go through the level while it is absorbed, ATR.
    pub abs_hold_atr: f64,
    /// Turn off the extreme that confirms the absorption, ATR.
    pub abs_confirm_atr: f64,
    /// Stop behind the absorption extreme, ATR.
    pub abs_stop_atr: f64,
    /// Give up waiting for absorption (or for the limit fill after it) after this many seconds.
    pub abs_wait: f64,
    /// After absorption, rest a limit `abs_limit_atr` off the extreme instead of a market entry.
    pub abs_limit: bool,
    pub abs_limit_atr: f64,
    /// Exit when this share of the absorbed volume has traded at the density price again (0 = off).
    pub dens_eat: f64,
    /// Move the stop to breakeven after this many R (0 = off).
    pub be_r: f64,
    /// Trailing stop distance, ATR (0 = off), active after `trail_from_r` R.
    pub trail_atr: f64,
    pub trail_from_r: f64,
    /// Close this share at `part_r` R (0 = off).
    pub part_frac: f64,
    pub part_r: f64,
    /// Exit when aggressive volume against the position reaches this × average while price is
    /// through the level (0 = off).
    pub eat_vol: f64,
    /// After the initial stop or a flow exit, open the opposite position (breakout).
    pub flip: bool,
    pub flip_sl_atr: f64,
    pub flip_tp_r: f64,
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
            day_stop_r: 0.0,
            risk_pct: 1.0,
            max_leverage: 20.0,
            sec_engine: false,
            absorb: false,
            abs_window: 30.0,
            abs_vol: 3.0,
            abs_hold_atr: 0.2,
            abs_confirm_atr: 0.05,
            abs_stop_atr: 0.05,
            abs_wait: 300.0,
            abs_limit: false,
            abs_limit_atr: 0.05,
            dens_eat: 0.0,
            be_r: 0.0,
            trail_atr: 0.0,
            trail_from_r: 0.5,
            part_frac: 0.0,
            part_r: 0.5,
            eat_vol: 0.0,
            flip: false,
            flip_sl_atr: 0.5,
            flip_tp_r: 1.0,
            filters: BTreeMap::new(),
        }
    }
}

/// Settings that only the 1-second engine uses.
pub fn sec_only(id: &str) -> bool {
    matches!(
        id,
        "absorb"
            | "abs_window"
            | "abs_vol"
            | "abs_hold_atr"
            | "abs_confirm_atr"
            | "abs_stop_atr"
            | "abs_wait"
            | "abs_limit"
            | "abs_limit_atr"
            | "dens_eat"
            | "be_r"
            | "trail_atr"
            | "trail_from_r"
            | "part_frac"
            | "part_r"
            | "eat_vol"
            | "flip"
            | "flip_sl_atr"
            | "flip_tp_r"
    )
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
            "day_stop_r" => self.day_stop_r,
            "risk_pct" => self.risk_pct,
            "max_leverage" => self.max_leverage,
            "sec_engine" => b(self.sec_engine),
            "absorb" => b(self.absorb),
            "abs_window" => self.abs_window,
            "abs_vol" => self.abs_vol,
            "abs_hold_atr" => self.abs_hold_atr,
            "abs_confirm_atr" => self.abs_confirm_atr,
            "abs_stop_atr" => self.abs_stop_atr,
            "abs_wait" => self.abs_wait,
            "abs_limit" => b(self.abs_limit),
            "abs_limit_atr" => self.abs_limit_atr,
            "dens_eat" => self.dens_eat,
            "be_r" => self.be_r,
            "trail_atr" => self.trail_atr,
            "trail_from_r" => self.trail_from_r,
            "part_frac" => self.part_frac,
            "part_r" => self.part_r,
            "eat_vol" => self.eat_vol,
            "flip" => b(self.flip),
            "flip_sl_atr" => self.flip_sl_atr,
            "flip_tp_r" => self.flip_tp_r,
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
            "day_stop_r" => self.day_stop_r = v,
            "risk_pct" => self.risk_pct = v,
            "max_leverage" => self.max_leverage = v,
            "sec_engine" => self.sec_engine = on,
            "absorb" => self.absorb = on,
            "abs_window" => self.abs_window = v,
            "abs_vol" => self.abs_vol = v,
            "abs_hold_atr" => self.abs_hold_atr = v,
            "abs_confirm_atr" => self.abs_confirm_atr = v,
            "abs_stop_atr" => self.abs_stop_atr = v,
            "abs_wait" => self.abs_wait = v,
            "abs_limit" => self.abs_limit = on,
            "abs_limit_atr" => self.abs_limit_atr = v,
            "dens_eat" => self.dens_eat = v,
            "be_r" => self.be_r = v,
            "trail_atr" => self.trail_atr = v,
            "trail_from_r" => self.trail_from_r = v,
            "part_frac" => self.part_frac = v,
            "part_r" => self.part_r = v,
            "eat_vol" => self.eat_vol = v,
            "flip" => self.flip = on,
            "flip_sl_atr" => self.flip_sl_atr = v,
            "flip_tp_r" => self.flip_tp_r = v,
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
                || id == "sec_engine"
                || group == "Risk"
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
    push("sec_engine", "Position", "1-second engine", "toggle", 0., 1., 1., "Simulate entries and exits second by second on Binance aggTrades (downloads about 1 GB once). Needed for absorption, breakeven, trailing, partial exits, the flow exit and the flip.");
    push(
        "be_r",
        "Position",
        "Breakeven after, R",
        "slider",
        0.0,
        2.0,
        0.05,
        "Move the stop to entry plus costs once price has gone this many R in favour. 0 = off.",
    );
    push(
        "trail_atr",
        "Position",
        "Trailing stop, ATR",
        "slider",
        0.0,
        2.0,
        0.05,
        "Keep the stop this far behind the best price. 0 = off.",
    );
    push(
        "trail_from_r",
        "Position",
        "Trailing starts after, R",
        "slider",
        0.0,
        2.0,
        0.05,
        "",
    );
    push(
        "part_frac",
        "Position",
        "Partial exit, share",
        "slider",
        0.0,
        0.9,
        0.05,
        "Close this share of the position at the partial target. 0 = off.",
    );
    push("part_r", "Position", "Partial target, R", "slider", 0.2, 3.0, 0.05, "");
    push("eat_vol", "Position", "Flow exit, × average volume", "slider", 0.0, 20.0, 0.5, "Exit at market when aggressive volume against the position over the volume window reaches this multiple of the hour's average while price is through the level: the level is being eaten. 0 = off.");
    push("flip", "Position", "Flip on failure", "toggle", 0., 1., 1., "When the position ends at its first stop or by the flow exit, open the opposite position at market (the breakout). Its R is added to the trade.");
    push(
        "flip_sl_atr",
        "Position",
        "Flip stop behind level, ATR",
        "slider",
        0.0,
        2.0,
        0.05,
        "",
    );
    push("flip_tp_r", "Position", "Flip target, R", "slider", 0.2, 3.0, 0.05, "");
    push("absorb", "Absorption", "Enter on absorption", "toggle", 0., 1., 1., "Instead of a resting limit: wait at the level until aggressive volume hits it and price holds, then enter at market with the stop just behind the absorption extreme (a small stop).");
    push(
        "abs_window",
        "Absorption",
        "Volume window, s",
        "slider",
        5.0,
        120.0,
        5.0,
        "Rolling window for the absorption volume and the flow exit.",
    );
    push("abs_vol", "Absorption", "Absorbed volume, × average", "slider", 0.5, 20.0, 0.5, "Aggressive volume into the level over the window, as a multiple of the average for the window over the last hour.");
    push(
        "abs_hold_atr",
        "Absorption",
        "Max push through level, ATR",
        "slider",
        0.0,
        1.0,
        0.02,
        "If price goes further through the level, nothing is holding it: no entry.",
    );
    push(
        "abs_confirm_atr",
        "Absorption",
        "Turn off the extreme, ATR",
        "slider",
        0.0,
        0.5,
        0.01,
        "Enter once price has moved this far back from the absorption extreme.",
    );
    push(
        "abs_stop_atr",
        "Absorption",
        "Stop behind the extreme, ATR",
        "slider",
        0.0,
        0.5,
        0.01,
        "",
    );
    push(
        "abs_wait",
        "Absorption",
        "Max wait, s",
        "slider",
        30.0,
        1800.0,
        30.0,
        "Give up when no absorption comes this long after price reached the level.",
    );
    push("abs_limit", "Absorption", "Limit after absorption", "toggle", 0., 1., 1., "After the absorption, rest a limit order near its extreme and wait for the pull-back instead of entering at market: no spread or slippage on the entry, maker fee, but some trades never fill.");
    push(
        "abs_limit_atr",
        "Absorption",
        "Limit off the extreme, ATR",
        "slider",
        0.0,
        0.3,
        0.01,
        "",
    );
    push("dens_eat", "Absorption", "Exit when density eaten, share", "slider", 0.0, 1.0, 0.05, "The volume absorbed at entry is the density the stop hides behind. Exit at market when aggressive volume at that price reaches this share of it again, before the stop. 0.7 = leave with 30% left. 0 = off. Estimated from trades, not from the order book.");
    push(
        "day_stop_r",
        "Risk",
        "Daily loss limit, R",
        "slider",
        0.0,
        10.0,
        0.5,
        "No new trades on a day once its closed trades lost this many R. 0 = off.",
    );
    push(
        "risk_pct",
        "Risk",
        "Risk per trade, % of account",
        "slider",
        0.1,
        5.0,
        0.1,
        "Money view only: position size = this share of the account / stop distance. It does not change R.",
    );
    push(
        "max_leverage",
        "Risk",
        "Max leverage",
        "slider",
        1.0,
        100.0,
        1.0,
        "A trade whose stop is so small that the risk needs more leverage is cut to this leverage.",
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
