use std::{collections::HashMap, path::Path};

use serde::{Deserialize, Serialize};

/// A 5m bar with order-flow columns. Columns a source does not have are `NaN`
/// (klines give `buy_volume` and `trades`; aggTrades give the cluster columns).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bar {
    pub time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub buy_volume: f64,
    pub trades: f64,
    pub big_volume: f64,
    pub big_delta: f64,
    pub max_trade: f64,
    pub vol_top: f64,
    pub vol_bottom: f64,
    pub delta_top: f64,
    pub delta_bottom: f64,
    pub poc: f64,
    pub imb_buy: f64,
    pub imb_sell: f64,
    pub oi: f64,
    pub ls_top: f64,
    pub taker_ratio: f64,
}

impl Bar {
    pub fn ohlcv(time: i64, open: f64, high: f64, low: f64, close: f64, volume: f64) -> Self {
        Bar {
            time,
            open,
            high,
            low,
            close,
            volume,
            buy_volume: f64::NAN,
            trades: f64::NAN,
            big_volume: f64::NAN,
            big_delta: f64::NAN,
            max_trade: f64::NAN,
            vol_top: f64::NAN,
            vol_bottom: f64::NAN,
            delta_top: f64::NAN,
            delta_bottom: f64::NAN,
            poc: f64::NAN,
            imb_buy: f64::NAN,
            imb_sell: f64::NAN,
            oi: f64::NAN,
            ls_top: f64::NAN,
            taker_ratio: f64::NAN,
        }
    }

    /// Aggressive buy minus aggressive sell volume.
    pub fn delta(&self) -> f64 {
        2.0 * self.buy_volume - self.volume
    }
}

/// Reads a CSV with a header row; `time` is Unix seconds (or ms, detected). Unknown columns are
/// ignored, missing ones are `NaN`. Binance kline CSVs (`open_time`, `count`, `taker_buy_volume`) work too.
pub fn parse_csv(text: &str) -> Result<Vec<Bar>, String> {
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let header: Vec<&str> = lines.next().ok_or("empty CSV")?.split(',').map(str::trim).collect();
    let col: HashMap<&str, usize> = header.iter().enumerate().map(|(i, h)| (*h, i)).collect();
    let time_col = ["time", "open_time"]
        .iter()
        .find_map(|k| col.get(k))
        .copied()
        .ok_or("CSV has no time column")?;
    let mut bars = Vec::new();
    for (n, line) in lines.enumerate() {
        let cells: Vec<&str> = line.split(',').collect();
        let get = |names: &[&str]| -> f64 {
            names
                .iter()
                .find_map(|name| col.get(name).and_then(|&i| cells.get(i)))
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(f64::NAN)
        };
        let t: f64 = cells
            .get(time_col)
            .and_then(|s| s.trim().parse().ok())
            .ok_or_else(|| format!("line {}: bad time", n + 2))?;
        let time = if t > 1e11 { (t / 1000.0) as i64 } else { t as i64 };
        let mut b = Bar::ohlcv(
            time,
            get(&["open"]),
            get(&["high"]),
            get(&["low"]),
            get(&["close"]),
            get(&["volume"]),
        );
        b.buy_volume = get(&["buy_volume", "taker_buy_volume"]);
        b.trades = get(&["trades", "count"]);
        b.big_volume = get(&["big_volume"]);
        b.big_delta = get(&["big_delta"]);
        b.max_trade = get(&["max_trade"]);
        b.vol_top = get(&["vol_top"]);
        b.vol_bottom = get(&["vol_bottom"]);
        b.delta_top = get(&["delta_top"]);
        b.delta_bottom = get(&["delta_bottom"]);
        b.poc = get(&["poc"]);
        b.imb_buy = get(&["imb_buy"]);
        b.imb_sell = get(&["imb_sell"]);
        b.oi = get(&["oi"]);
        b.ls_top = get(&["ls_top"]);
        b.taker_ratio = get(&["taker_ratio"]);
        if [b.open, b.high, b.low, b.close].iter().any(|v| !v.is_finite()) {
            return Err(format!("line {}: bad OHLC", n + 2));
        }
        bars.push(b);
    }
    bars.sort_by_key(|b| b.time);
    bars.dedup_by_key(|b| b.time);
    Ok(bars)
}

pub fn load_csv(path: &Path) -> Result<Vec<Bar>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse_csv(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_research_and_kline_csv() {
        let bars =
            parse_csv("time,open,high,low,close,volume,buy_volume,trades\n300,1,2,0.5,1.5,10,6,4\n0,1,1,1,1,1,1,1\n")
                .unwrap();
        assert_eq!(bars.len(), 2);
        assert_eq!(bars[0].time, 0);
        assert_eq!(bars[1].delta(), 2.0);
        assert!(bars[1].poc.is_nan());

        let k = parse_csv(
            "open_time,open,high,low,close,volume,close_time,quote_volume,count,taker_buy_volume\n1767225600000,1,2,0.5,1.5,10,0,0,7,3\n",
        )
        .unwrap();
        assert_eq!(k[0].time, 1_767_225_600);
        assert_eq!(k[0].trades, 7.0);
        assert_eq!(k[0].buy_volume, 3.0);
    }
}
