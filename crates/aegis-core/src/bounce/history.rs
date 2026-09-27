//! Backtest history from Binance's public archive (data.binance.vision): 5m klines of the
//! USDⓈ-M perpetual, with taker-buy volume and trade counts. No key is needed and the archive
//! is reachable from regions where the trading API is not. Files are cached on disk.

use std::{
    io::Read,
    path::{Path, PathBuf},
};

use super::bar::{parse_csv, Bar};

const BASE: &str = "https://data.binance.vision";
const LIST: &str = "https://s3-ap-northeast-1.amazonaws.com/data.binance.vision";
/// XAUUSDT (TradFi perpetual) opened for trading on 2026-01-05; earlier archive bars are pre-launch.
pub const BINANCE_LISTING: i64 = 1_767_571_200;
const HEADER: &str =
    "open_time,open,high,low,close,volume,close_time,quote_volume,count,taker_buy_volume,taker_buy_quote_volume,ignore";

#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    #[error("network: {0}")]
    Net(String),
    #[error("archive: {0}")]
    Archive(String),
    #[error("no history for {0}")]
    Empty(String),
}

async fn get(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, HistoryError> {
    let mut last = String::new();
    for attempt in 0..4 {
        match client.get(url).send().await {
            Ok(r) if r.status().is_success() => {
                return r
                    .bytes()
                    .await
                    .map(|b| b.to_vec())
                    .map_err(|e| HistoryError::Net(e.to_string()))
            }
            Ok(r) => last = format!("{url}: HTTP {}", r.status()),
            Err(e) => last = format!("{url}: {e}"),
        }
        tokio::time::sleep(std::time::Duration::from_millis(500 << attempt)).await;
    }
    Err(HistoryError::Net(last))
}

async fn list(client: &reqwest::Client, prefix: &str) -> Result<Vec<String>, HistoryError> {
    let mut keys = Vec::new();
    let mut marker = String::new();
    loop {
        let url = format!("{LIST}?prefix={prefix}&delimiter=/&marker={marker}");
        let body = String::from_utf8_lossy(&get(client, &url).await?).into_owned();
        let page: Vec<String> = body
            .split("<Key>")
            .skip(1)
            .filter_map(|s| s.split("</Key>").next())
            .filter(|k| k.ends_with(".zip"))
            .map(str::to_string)
            .collect();
        let more = body.contains("<IsTruncated>true</IsTruncated>");
        marker = page.last().cloned().unwrap_or_default();
        keys.extend(page);
        if !more || marker.is_empty() {
            return Ok(keys);
        }
    }
}

fn unzip_csv(bytes: &[u8]) -> Result<String, HistoryError> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| HistoryError::Archive(e.to_string()))?;
    let mut f = z.by_index(0).map_err(|e| HistoryError::Archive(e.to_string()))?;
    let mut s = String::new();
    f.read_to_string(&mut s)
        .map_err(|e| HistoryError::Archive(e.to_string()))?;
    // Older archives have no header row.
    if s.starts_with(|c: char| c.is_ascii_digit()) {
        s = format!("{HEADER}\n{s}");
    }
    Ok(s)
}

/// Period tag of an archive name: "2026-01" (monthly) or "2026-09-26" (daily).
fn tag(key: &str) -> &str {
    let name = key.rsplit('/').next().unwrap_or(key).trim_end_matches(".zip");
    let parts: Vec<&str> = name.split('-').collect();
    let n = if parts.len() >= 3 && parts[parts.len() - 3].len() == 4 {
        3
    } else {
        2
    };
    let start = name.len() - parts[parts.len() - n..].join("-").len();
    &name[start..]
}

/// Loads the last `days` of 5m bars for `symbol`, downloading only archives not yet in `cache`.
/// `progress` receives (done, total) archive counts.
pub async fn binance_history(
    symbol: &str,
    days: i64,
    cache: &Path,
    progress: impl Fn(usize, usize),
) -> Result<Vec<Bar>, HistoryError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| HistoryError::Net(e.to_string()))?;
    let monthly = list(&client, &format!("data/futures/um/monthly/klines/{symbol}/5m/")).await?;
    let months: std::collections::HashSet<&str> = monthly.iter().map(|k| tag(k)).collect();
    let daily: Vec<String> = list(&client, &format!("data/futures/um/daily/klines/{symbol}/5m/"))
        .await?
        .into_iter()
        .filter(|k| !months.contains(&tag(k)[..7.min(tag(k).len())]))
        .collect();
    let since = crate::sign::now_ms() / 1000 - days * 86_400;
    let since_month = super::backtest_month(since);
    let keys: Vec<&String> = monthly
        .iter()
        .filter(|k| tag(k) >= since_month.as_str())
        .chain(daily.iter())
        .collect();
    if keys.is_empty() {
        return Err(HistoryError::Empty(symbol.into()));
    }
    let dir: PathBuf = cache.join(symbol).join("5m");
    std::fs::create_dir_all(&dir).map_err(|e| HistoryError::Archive(e.to_string()))?;
    let mut text = String::new();
    for (n, key) in keys.iter().enumerate() {
        progress(n, keys.len());
        let path = dir.join(key.rsplit('/').next().unwrap_or(key));
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(_) => {
                let b = get(&client, &format!("{BASE}/{key}")).await?;
                let _ = std::fs::write(&path, &b);
                b
            }
        };
        let csv = unzip_csv(&bytes)?;
        let mut lines = csv.lines();
        let header = lines.next().unwrap_or_default();
        if text.is_empty() {
            text.push_str(header);
            text.push('\n');
        }
        for l in lines {
            text.push_str(l);
            text.push('\n');
        }
    }
    progress(keys.len(), keys.len());
    let start = if symbol == "XAUUSDT" {
        since.max(BINANCE_LISTING)
    } else {
        since
    };
    let bars: Vec<Bar> = parse_csv(&text)
        .map_err(HistoryError::Archive)?
        .into_iter()
        .filter(|b| b.time >= start)
        .collect();
    if bars.is_empty() {
        return Err(HistoryError::Empty(symbol.into()));
    }
    Ok(bars)
}

/// The latest 5m klines (up to 1500) from the public futures API, with taker-buy volume and
/// trade counts. Fills the day or two the archive has not published yet.
pub async fn recent_klines(base: &str, symbol: &str, limit: usize) -> Result<Vec<Bar>, HistoryError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| HistoryError::Net(e.to_string()))?;
    let url = format!(
        "{base}/fapi/v1/klines?symbol={symbol}&interval=5m&limit={}",
        limit.clamp(1, 1500)
    );
    let body: serde_json::Value =
        serde_json::from_slice(&get(&client, &url).await?).map_err(|e| HistoryError::Archive(e.to_string()))?;
    let rows = body
        .as_array()
        .ok_or_else(|| HistoryError::Archive("klines: not an array".into()))?;
    let num = |v: &serde_json::Value| match v {
        serde_json::Value::String(s) => s.parse::<f64>().ok(),
        serde_json::Value::Number(n) => n.as_f64(),
        _ => None,
    };
    let now = crate::sign::now_ms();
    let mut bars = Vec::new();
    for r in rows {
        let cell = |k: usize| r.get(k).and_then(num).unwrap_or(f64::NAN);
        // Only closed bars: close time (index 6) in the past.
        if cell(6) >= now as f64 {
            continue;
        }
        let mut b = Bar::ohlcv((cell(0) / 1000.0) as i64, cell(1), cell(2), cell(3), cell(4), cell(5));
        b.trades = cell(8);
        b.buy_volume = cell(9);
        if b.close.is_finite() {
            bars.push(b);
        }
    }
    Ok(bars)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_tags() {
        assert_eq!(
            tag("data/futures/um/monthly/klines/XAUUSDT/5m/XAUUSDT-5m-2026-01.zip"),
            "2026-01"
        );
        assert_eq!(
            tag("data/futures/um/daily/klines/XAUUSDT/5m/XAUUSDT-5m-2026-09-26.zip"),
            "2026-09-26"
        );
    }
}
