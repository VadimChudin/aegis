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
    binance_klines(symbol, "5m", days, cache, progress).await
}

/// 1m candles of the same archive, for the order of events inside 5m bars.
pub async fn binance_minutes(
    symbol: &str,
    days: i64,
    cache: &Path,
    progress: impl Fn(usize, usize),
) -> Result<Vec<super::engine::Minute>, HistoryError> {
    Ok(binance_klines(symbol, "1m", days, cache, progress)
        .await?
        .into_iter()
        .map(|b| super::engine::Minute {
            time: b.time,
            open: b.open,
            high: b.high,
            low: b.low,
            close: b.close,
        })
        .collect())
}

async fn binance_klines(
    symbol: &str,
    interval: &str,
    days: i64,
    cache: &Path,
    progress: impl Fn(usize, usize),
) -> Result<Vec<Bar>, HistoryError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| HistoryError::Net(e.to_string()))?;
    let monthly = list(&client, &format!("data/futures/um/monthly/klines/{symbol}/{interval}/")).await?;
    let months: std::collections::HashSet<&str> = monthly.iter().map(|k| tag(k)).collect();
    let daily: Vec<String> = list(&client, &format!("data/futures/um/daily/klines/{symbol}/{interval}/"))
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
    let dir: PathBuf = cache.join(symbol).join(interval);
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

/// 1-second candles since `from` (Unix seconds) from the aggTrades archive, for the position
/// engine. Each archive is converted once and kept as a small `.sec` file (the zips, about 1 GB
/// since the listing, are not kept).
pub async fn binance_seconds(
    symbol: &str,
    from: i64,
    cache: &Path,
    progress: impl Fn(usize, usize),
) -> Result<Vec<super::secs::Sec>, HistoryError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|e| HistoryError::Net(e.to_string()))?;
    let monthly = list(&client, &format!("data/futures/um/monthly/aggTrades/{symbol}/")).await?;
    let months: std::collections::HashSet<&str> = monthly.iter().map(|k| tag(k)).collect();
    let daily: Vec<String> = list(&client, &format!("data/futures/um/daily/aggTrades/{symbol}/"))
        .await?
        .into_iter()
        .filter(|k| !months.contains(&tag(k)[..7.min(tag(k).len())]))
        .collect();
    let from_month = super::backtest_month(from);
    let keys: Vec<&String> = monthly
        .iter()
        .filter(|k| tag(k) >= from_month.as_str())
        .chain(daily.iter().filter(|k| tag(k) >= from_month.as_str()))
        .collect();
    if keys.is_empty() {
        return Err(HistoryError::Empty(symbol.into()));
    }
    let dir: PathBuf = cache.join(symbol).join("seconds");
    std::fs::create_dir_all(&dir).map_err(|e| HistoryError::Archive(e.to_string()))?;
    let mut all = Vec::new();
    for (n, key) in keys.iter().enumerate() {
        progress(n, keys.len());
        let path = dir.join(format!("{}.sec", tag(key)));
        let part = match super::secs::load_seconds(&path) {
            Ok(s) => s,
            Err(_) => {
                let bytes = get(&client, &format!("{BASE}/{key}")).await?;
                let secs = tokio::task::spawn_blocking(move || super::secs::seconds_from_zip(&bytes))
                    .await
                    .map_err(|e| HistoryError::Archive(e.to_string()))?
                    .map_err(HistoryError::Archive)?;
                let _ = super::secs::save_seconds(&path, &secs);
                secs
            }
        };
        super::secs::append(&mut all, part);
    }
    progress(keys.len(), keys.len());
    all.retain(|s| s.time >= from);
    if all.is_empty() {
        return Err(HistoryError::Empty(symbol.into()));
    }
    Ok(all)
}

/// Unix seconds of "YYYY-MM-DD HH:MM:SS" (UTC).
fn parse_utc(s: &str) -> Option<i64> {
    let (d, t) = s.trim().split_once(' ')?;
    let mut dp = d.split('-').map(|x| x.parse::<i64>());
    let (y, m, day) = (dp.next()?.ok()?, dp.next()?.ok()?, dp.next()?.ok()?);
    let mut tp = t.split(':').map(|x| x.parse::<i64>());
    let (hh, mm, ss) = (
        tp.next()?.ok()?,
        tp.next()?.ok()?,
        tp.next().and_then(|x| x.ok()).unwrap_or(0),
    );
    // Days from civil (Howard Hinnant).
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some((era * 146_097 + doe - 719_468) * 86_400 + hh * 3600 + mm * 60 + ss)
}

/// Binance derivatives context for the backtest: open interest, top-trader long/short and taker
/// buy/sell ratio (5m, `metrics` archive) and funding rates (`fundingRate` archive).
#[derive(Clone, Debug, Default)]
pub struct Extras {
    /// (time, open interest, top-trader long/short, taker buy/sell)
    pub metrics: Vec<(i64, f64, f64, f64)>,
    /// (time, funding rate)
    pub funding: Vec<(i64, f64)>,
}

async fn cached(client: &reqwest::Client, dir: &Path, key: &str) -> Result<Vec<u8>, HistoryError> {
    let path = dir.join(key.rsplit('/').next().unwrap_or(key));
    if let Ok(b) = std::fs::read(&path) {
        return Ok(b);
    }
    let b = get(client, &format!("{BASE}/{key}")).await?;
    let _ = std::fs::write(&path, &b);
    Ok(b)
}

fn unzip_text(bytes: &[u8]) -> Result<String, HistoryError> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| HistoryError::Archive(e.to_string()))?;
    let mut f = z.by_index(0).map_err(|e| HistoryError::Archive(e.to_string()))?;
    let mut s = String::new();
    f.read_to_string(&mut s)
        .map_err(|e| HistoryError::Archive(e.to_string()))?;
    Ok(s)
}

/// Downloads (and caches) the metrics and funding archives since `since`.
pub async fn binance_extras(
    symbol: &str,
    since: i64,
    cache: &Path,
    progress: impl Fn(usize, usize),
) -> Result<Extras, HistoryError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| HistoryError::Net(e.to_string()))?;
    let day_since = super::backtest_month(since);
    let metrics: Vec<String> = list(&client, &format!("data/futures/um/daily/metrics/{symbol}/"))
        .await?
        .into_iter()
        .filter(|k| tag(k) >= day_since.as_str())
        .collect();
    let funding: Vec<String> = list(&client, &format!("data/futures/um/monthly/fundingRate/{symbol}/"))
        .await?
        .into_iter()
        .filter(|k| tag(k) >= day_since.as_str())
        .collect();
    let dir = cache.join(symbol).join("extras");
    std::fs::create_dir_all(&dir).map_err(|e| HistoryError::Archive(e.to_string()))?;
    let total = metrics.len() + funding.len();
    let mut out = Extras::default();
    // Small files: 8 downloads at a time.
    let mut done = 0;
    for chunk in metrics.chunks(8) {
        let mut set = tokio::task::JoinSet::new();
        for key in chunk {
            let (client, dir, key) = (client.clone(), dir.clone(), key.clone());
            set.spawn(async move { cached(&client, &dir, &key).await });
        }
        while let Some(r) = set.join_next().await {
            let bytes = r.map_err(|e| HistoryError::Net(e.to_string()))??;
            for line in unzip_text(&bytes)?.lines().skip(1) {
                let c: Vec<&str> = line.split(',').collect();
                let num = |k: usize| c.get(k).and_then(|s| s.trim().parse::<f64>().ok()).unwrap_or(f64::NAN);
                if let Some(t) = c.first().and_then(|s| parse_utc(s)) {
                    out.metrics.push((t, num(2), num(5), num(7)));
                }
            }
            done += 1;
            progress(done, total);
        }
    }
    for key in &funding {
        let bytes = cached(&client, &dir, key).await?;
        for line in unzip_text(&bytes)?.lines().skip(1) {
            let mut c = line.split(',');
            if let (Some(t), Some(_), Some(r)) = (c.next(), c.next(), c.next()) {
                if let (Ok(t), Ok(r)) = (t.trim().parse::<i64>(), r.trim().parse::<f64>()) {
                    out.funding.push((t / 1000, r));
                }
            }
        }
        done += 1;
        progress(done, total);
    }
    out.metrics.sort_by_key(|m| m.0);
    out.funding.sort_by_key(|f| f.0);
    Ok(out)
}

/// Adds the extras to bars: each bar gets the latest value stamped at or before its open.
pub fn merge_extras(bars: &mut [Bar], ex: &Extras) {
    let (mut i, mut j) = (0usize, 0usize);
    let (mut m, mut f) = (None, None);
    for b in bars.iter_mut() {
        while i < ex.metrics.len() && ex.metrics[i].0 <= b.time {
            m = Some(ex.metrics[i]);
            i += 1;
        }
        while j < ex.funding.len() && ex.funding[j].0 <= b.time {
            f = Some(ex.funding[j].1);
            j += 1;
        }
        if let Some((_, oi, ls, tr)) = m {
            b.oi = oi;
            b.ls_top = ls;
            b.taker_ratio = tr;
        }
        if let Some(r) = f {
            b.funding = r;
        }
    }
}

/// The latest 5m klines (up to 1500) from the public futures API, with taker-buy volume and
/// trade counts. Fills the day or two the archive has not published yet.
pub async fn recent_klines(base: &str, symbol: &str, limit: usize) -> Result<Vec<Bar>, HistoryError> {
    recent(base, symbol, "5m", limit).await
}

/// The latest closed 1m candles (up to 1500, about 25 hours) from the public futures API.
pub async fn recent_minutes(
    base: &str,
    symbol: &str,
    limit: usize,
) -> Result<Vec<super::engine::Minute>, HistoryError> {
    Ok(recent(base, symbol, "1m", limit)
        .await?
        .into_iter()
        .map(|b| super::engine::Minute {
            time: b.time,
            open: b.open,
            high: b.high,
            low: b.low,
            close: b.close,
        })
        .collect())
}

async fn recent(base: &str, symbol: &str, interval: &str, limit: usize) -> Result<Vec<Bar>, HistoryError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| HistoryError::Net(e.to_string()))?;
    let url = format!(
        "{base}/fapi/v1/klines?symbol={symbol}&interval={interval}&limit={}",
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
    fn utc_and_merge() {
        assert_eq!(parse_utc("1970-01-02 00:00:10"), Some(86_410));
        assert_eq!(parse_utc("2026-01-05 00:00:00"), Some(BINANCE_LISTING));
        let mut bars: Vec<Bar> = (0..4).map(|i| Bar::ohlcv(i * 300, 1., 1., 1., 1., 1.)).collect();
        let ex = Extras {
            metrics: vec![(300, 10.0, 1.5, 0.9)],
            funding: vec![(0, 0.0001)],
        };
        merge_extras(&mut bars, &ex);
        assert!(bars[0].oi.is_nan());
        assert_eq!((bars[1].oi, bars[3].ls_top, bars[2].taker_ratio), (10.0, 1.5, 0.9));
        assert_eq!(bars[0].funding, 0.0001);
    }

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
