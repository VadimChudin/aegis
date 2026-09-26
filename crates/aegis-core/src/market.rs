use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Chart timeframes. Every connector supports all of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Timeframe {
    #[serde(rename = "1m")]
    M1,
    #[serde(rename = "5m")]
    M5,
    #[serde(rename = "15m")]
    M15,
    #[serde(rename = "1h")]
    H1,
    #[serde(rename = "4h")]
    H4,
    #[serde(rename = "1d")]
    D1,
}

impl Timeframe {
    pub const ALL: [Timeframe; 6] = [
        Timeframe::M1,
        Timeframe::M5,
        Timeframe::M15,
        Timeframe::H1,
        Timeframe::H4,
        Timeframe::D1,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Timeframe::M1 => "1m",
            Timeframe::M5 => "5m",
            Timeframe::M15 => "15m",
            Timeframe::H1 => "1h",
            Timeframe::H4 => "4h",
            Timeframe::D1 => "1d",
        }
    }

    pub fn seconds(self) -> i64 {
        match self {
            Timeframe::M1 => 60,
            Timeframe::M5 => 300,
            Timeframe::M15 => 900,
            Timeframe::H1 => 3_600,
            Timeframe::H4 => 14_400,
            Timeframe::D1 => 86_400,
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.as_str() == s)
    }
}

/// One OHLCV bar. `time` is the bar open in Unix seconds, UTC.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Candle {
    pub time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

/// Exchanges send prices as strings ("4293.18") or numbers; accept both.
pub(crate) fn num(v: &Value) -> Option<f64> {
    match v {
        Value::String(s) => s.parse().ok(),
        Value::Number(n) => n.as_f64(),
        _ => None,
    }
}

pub(crate) fn int(v: &Value) -> Option<i64> {
    match v {
        Value::String(s) => s.parse().ok(),
        Value::Number(n) => n.as_i64(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeframe_round_trips_through_its_label() {
        for tf in Timeframe::ALL {
            assert_eq!(Timeframe::parse(tf.as_str()), Some(tf));
            let json = serde_json::to_string(&tf).unwrap();
            assert_eq!(json, format!("\"{}\"", tf.as_str()));
            assert_eq!(serde_json::from_str::<Timeframe>(&json).unwrap(), tf);
        }
        assert_eq!(Timeframe::parse("2h"), None);
    }

    #[test]
    fn numbers_parse_from_strings_and_numbers() {
        assert_eq!(num(&Value::from("4293.18")), Some(4293.18));
        assert_eq!(num(&Value::from(1.5)), Some(1.5));
        assert_eq!(num(&Value::Null), None);
        assert_eq!(int(&Value::from("1790208000000")), Some(1_790_208_000_000));
    }
}
