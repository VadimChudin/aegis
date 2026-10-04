//! Dukascopy's public datafeed: one LZMA file of 1-minute candles per day and price side.
//!
//! `{base}/{SYMBOL}/{YYYY}/{MM}/{DD}/{BID|ASK}_candles_min_1.bi5`, MM 0-based. Records are 24 bytes
//! big-endian: u32 seconds from the day start (UTC), u32 open, close, low, high (price × point),
//! f32 volume. Minutes without ticks come as flat candles with volume 0 and are dropped. The
//! server answers 429 to requests without a browser User-Agent and 503 to concurrent ones, and
//! often takes 15-35 s to the first byte, so files are fetched one at a time with backoff.

use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use super::DataError;

pub const DEFAULT_URL: &str = "https://datafeed.dukascopy.com/datafeed";
const USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0 Safari/537.36";
const REFERER: &str = "https://www.dukascopy.com/swiss/english/marketwatch/historical/";
const TRIES: u32 = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Bid,
    Ask,
}

impl Side {
    pub fn as_str(self) -> &'static str {
        match self {
            Side::Bid => "BID",
            Side::Ask => "ASK",
        }
    }
}

/// Price scale of an instrument in the datafeed.
pub fn point(symbol: &str) -> f64 {
    match symbol {
        "XAUUSD" | "XAGUSD" => 1000.0,
        s if s.ends_with("JPY") => 1000.0,
        _ => 100_000.0,
    }
}

pub fn day_url(base: &str, symbol: &str, day: i64, side: Side) -> String {
    let (y, m, d) = super::time::civil_from_days(day);
    format!("{base}/{symbol}/{y:04}/{:02}/{d:02}/{}_candles_min_1.bi5", m - 1, side.as_str())
}

/// One decoded minute. `time` is the minute open, UTC seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RawMinute {
    pub time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

/// Decodes one day file. An empty body (404: no trading that day) gives no minutes.
pub fn decode(blob: &[u8], day_start: i64, point: f64) -> Result<Vec<RawMinute>, DataError> {
    if blob.is_empty() {
        return Ok(Vec::new());
    }
    let mut raw = Vec::new();
    lzma_rs::lzma_decompress(&mut std::io::Cursor::new(blob), &mut raw)
        .map_err(|e| DataError::Format(format!("lzma: {e}")))?;
    if raw.len() % 24 != 0 {
        return Err(DataError::Format(format!("{} bytes is not a whole number of records", raw.len())));
    }
    let u = |b: &[u8]| u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
    Ok(raw
        .chunks_exact(24)
        .filter_map(|r| {
            let volume = f32::from_be_bytes([r[20], r[21], r[22], r[23]]) as f64;
            (volume > 0.0).then(|| RawMinute {
                time: day_start + u(&r[0..4]) as i64,
                open: u(&r[4..8]) as f64 / point,
                close: u(&r[8..12]) as f64 / point,
                low: u(&r[12..16]) as f64 / point,
                high: u(&r[16..20]) as f64 / point,
                volume,
            })
        })
        .collect())
}

pub fn client() -> Result<reqwest::Client, DataError> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(150))
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| DataError::Net(e.to_string()))
}

/// Body of `url` (empty for 404), retrying 429 / 503 / network errors with growing pauses.
pub async fn fetch(client: &reqwest::Client, url: &str, cancel: &AtomicBool) -> Result<Vec<u8>, DataError> {
    let mut wait = Duration::from_secs(3);
    let mut last = String::new();
    for _ in 0..TRIES {
        if cancel.load(Ordering::Relaxed) {
            return Err(DataError::Cancelled);
        }
        match client.get(url).header("Referer", REFERER).send().await {
            Ok(r) if r.status() == reqwest::StatusCode::NOT_FOUND => return Ok(Vec::new()),
            Ok(r) if r.status().is_success() => {
                let body = r.bytes().await.map_err(|e| DataError::Net(e.to_string()))?;
                // The server sometimes answers 200 with an HTML error page.
                if body.starts_with(b"<") {
                    last = format!("{url}: HTML instead of data");
                } else {
                    return Ok(body.to_vec());
                }
            }
            Ok(r) => {
                last = format!("{url}: HTTP {}", r.status());
                if r.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    wait *= 2;
                }
            }
            Err(e) => last = format!("{url}: {e}"),
        }
        // Sleep in short steps so Cancel is not blocked by a long backoff.
        let until = std::time::Instant::now() + wait;
        while std::time::Instant::now() < until {
            if cancel.load(Ordering::Relaxed) {
                return Err(DataError::Cancelled);
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        wait = (wait * 2).min(Duration::from_secs(120));
    }
    Err(DataError::Net(last))
}

#[cfg(test)]
pub(crate) fn encode(minutes: &[(u32, [u32; 4], f32)]) -> Vec<u8> {
    let mut raw = Vec::new();
    for (sec, [o, c, l, h], v) in minutes {
        for x in [*sec, *o, *c, *l, *h] {
            raw.extend_from_slice(&x.to_be_bytes());
        }
        raw.extend_from_slice(&v.to_be_bytes());
    }
    let mut out = Vec::new();
    lzma_rs::lzma_compress(&mut std::io::Cursor::new(raw), &mut out).unwrap();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_records_and_drops_empty_minutes() {
        let blob = encode(&[
            (0, [4_000_100, 4_000_300, 4_000_000, 4_000_500], 1.5),
            (60, [4_000_300, 4_000_300, 4_000_300, 4_000_300], 0.0),
            (120, [4_000_300, 3_999_900, 3_999_800, 4_000_400], 2.0),
        ]);
        let m = decode(&blob, 86_400, 1000.0).unwrap();
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].time, 86_400);
        assert_eq!((m[0].open, m[0].high, m[0].low, m[0].close), (4000.1, 4000.5, 4000.0, 4000.3));
        assert_eq!(m[1].time, 86_520);
        assert_eq!(m[1].close, 3999.9);
        assert!(decode(&[], 0, 1000.0).unwrap().is_empty());
        assert!(decode(b"<html>", 0, 1000.0).is_err());
    }

    #[test]
    fn urls_use_zero_based_months() {
        let day = super::super::time::parse_date("2025-01-07").unwrap();
        assert_eq!(
            day_url(DEFAULT_URL, "XAUUSD", day, Side::Ask),
            "https://datafeed.dukascopy.com/datafeed/XAUUSD/2025/00/07/ASK_candles_min_1.bi5"
        );
    }
}
