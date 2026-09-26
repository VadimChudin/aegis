//! Strategy slots. v0.1 ships the four slots as stubs: they are listed in the
//! window and wired into the registry, but never produce a signal.

use serde::Serialize;

use crate::market::Candle;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StrategyStatus {
    Stub,
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
        #[derive(Default)]
        pub struct $ty;

        impl Strategy for $ty {
            fn info(&self) -> StrategyInfo {
                StrategyInfo {
                    id: $id,
                    name: $name,
                    summary: $summary,
                    status: StrategyStatus::Stub,
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
    "Fades a level that holds: entry against the approach, stop behind the level."
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
    fn four_stub_slots_with_unique_ids() {
        let names: Vec<_> = catalog().iter().map(|s| s.name).collect();
        assert_eq!(names, ["Breakout", "Bounce", "Liquidity Sweep", "DATA"]);
        let mut ids: Vec<_> = catalog().iter().map(|s| s.id).collect();
        ids.dedup();
        assert_eq!(ids.len(), 4);
        assert!(catalog().iter().all(|s| s.status == StrategyStatus::Stub));
    }

    #[test]
    fn stubs_never_signal() {
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
