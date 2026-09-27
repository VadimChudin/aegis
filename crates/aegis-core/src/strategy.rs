//! Strategy slots. Bounce has settings and a backtest (`crate::bounce`); the other slots are
//! stubs. No slot produces live signals or orders yet.

use serde::Serialize;

use crate::market::Candle;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StrategyStatus {
    Stub,
    /// Settings and backtest work; no live orders yet.
    Backtest,
}

#[derive(Clone, Debug, Serialize)]
pub struct StrategyInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub summary: &'static str,
    pub status: StrategyStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Long,
    Short,
}

#[derive(Clone, Debug, Serialize)]
pub struct Signal {
    pub side: Side,
    pub price: f64,
    pub reason: String,
}

pub trait Strategy: Send {
    fn info(&self) -> StrategyInfo;
    /// Called on every closed bar.
    fn on_candle(&mut self, candle: &Candle) -> Option<Signal>;
}

macro_rules! stub {
    ($ty:ident, $id:literal, $name:literal, $summary:literal) => {
        stub!($ty, $id, $name, $summary, Stub);
    };
    ($ty:ident, $id:literal, $name:literal, $summary:literal, $status:ident) => {
        #[derive(Default)]
        pub struct $ty;

        impl Strategy for $ty {
            fn info(&self) -> StrategyInfo {
                StrategyInfo {
                    id: $id,
                    name: $name,
                    summary: $summary,
                    status: StrategyStatus::$status,
                }
            }

            fn on_candle(&mut self, _candle: &Candle) -> Option<Signal> {
                None
            }
        }
    };
}

stub!(
    Breakout,
    "breakout",
    "Breakout",
    "Enters when price breaks a level and holds beyond it."
);
stub!(
    Bounce,
    "bounce",
    "Bounce",
    "Limit order at a level, stop behind it; trades only touches the win-probability model rates highly. Backtest only (no live orders yet).",
    Backtest
);
stub!(
    LiquiditySweep,
    "liquidity_sweep",
    "Liquidity Sweep",
    "Trades the return after price runs the stops beyond a swing high or low."
);
stub!(
    Data,
    "data",
    "DATA",
    "Statistical setups mined from history: ticks, volume and order book."
);

pub fn registry() -> Vec<Box<dyn Strategy>> {
    vec![
        Box::new(Breakout),
        Box::new(Bounce),
        Box::new(LiquiditySweep),
        Box::new(Data),
    ]
}

pub fn catalog() -> Vec<StrategyInfo> {
    registry().iter().map(|s| s.info()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_slots_with_unique_ids() {
        let names: Vec<_> = catalog().iter().map(|s| s.name).collect();
        assert_eq!(names, ["Breakout", "Bounce", "Liquidity Sweep", "DATA"]);
        let mut ids: Vec<_> = catalog().iter().map(|s| s.id).collect();
        ids.dedup();
        assert_eq!(ids.len(), 4);
        for s in catalog() {
            let want = if s.id == "bounce" {
                StrategyStatus::Backtest
            } else {
                StrategyStatus::Stub
            };
            assert_eq!(s.status, want, "{}", s.id);
        }
    }

    #[test]
    fn no_slot_signals_live_yet() {
        let bar = Candle {
            time: 0,
            open: 1.0,
            high: 2.0,
            low: 0.5,
            close: 1.5,
            volume: 10.0,
        };
        for mut s in registry() {
            assert!(s.on_candle(&bar).is_none(), "{}", s.info().name);
        }
    }
}
