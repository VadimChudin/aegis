//! Local history for backtests: Dukascopy bid / ask 1-minute candles downloaded once, kept on disk
//! for a chosen date range, and built into the chosen timeframes.
//!
//! Layout under the data folder: `raw/YYYY/MM/DD_BID.bi5` (the downloaded day files, an empty file
//! for a day without trading), `<tf>.bin` (built bars) and `manifest.json`. A sync downloads only
//! days not on disk, deletes days outside the range, and rebuilds the timeframes. With a rolling
//! window, Refresh keeps the window length and moves it to end yesterday, so the oldest days
//! are deleted as new ones arrive.

pub mod dukascopy;
pub mod time;

use std::{
    collections::BTreeMap,
    fs,
    io::{BufWriter, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

use serde::{Deserialize, Serialize};

use crate::market::Timeframe;
use dukascopy::Side;

/// Dukascopy's gold history starts in 2003.
pub const EARLIEST: &str = "2003-01-01";
/// After this many files in a row fail, the server is treated as down and the sync stops.
const MAX_FAILED_IN_ROW: usize = 3;
const RECORD: usize = 36;

#[derive(Debug, thiserror::Error)]
pub enum DataError {
    #[error("network: {0}")]
    Net(String),
    #[error("data format: {0}")]
    Format(String),
    #[error("disk: {0}")]
    Io(String),
    #[error("{0}")]
    Config(String),
    #[error("cancelled")]
    Cancelled,
}

impl From<std::io::Error> for DataError {
    fn from(e: std::io::Error) -> Self {
        DataError::Io(e.to_string())
    }
}

/// What to keep on disk.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DatasetConfig {
    pub symbol: String,
    /// First day, "YYYY-MM-DD".
    pub from: String,
    /// Last day, inclusive; later than yesterday (UTC) means "up to yesterday".
    pub to: String,
    /// Refresh keeps the window length and moves it to end yesterday.
    pub rolling: bool,
    pub timeframes: Vec<Timeframe>,
    /// Also download ask prices (the spread); doubles the number of files.
    pub ask: bool,
}

impl Default for DatasetConfig {
    fn default() -> Self {
        let y = today() - 1;
        DatasetConfig {
            symbol: "XAUUSD".into(),
            from: time::format_date(y - 364),
            to: time::format_date(y),
            rolling: true,
            timeframes: vec![Timeframe::M1, Timeframe::H1],
            ask: true,
        }
    }
}

/// Today as a UTC day number.
pub fn today() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64 / 86_400)
        .unwrap_or(0)
}

impl DatasetConfig {
    /// (first, last) day numbers; the last day is at most yesterday, since today is incomplete.
    pub fn range(&self, today: i64) -> Result<(i64, i64), DataError> {
        let bad = |s: &str| DataError::Config(format!("not a date: {s}"));
        let from = time::parse_date(&self.from).ok_or_else(|| bad(&self.from))?;
        let to = time::parse_date(&self.to).ok_or_else(|| bad(&self.to))?.min(today - 1);
        let earliest = time::parse_date(EARLIEST).unwrap_or(0);
        if from < earliest {
            return Err(DataError::Config(format!("history starts on {EARLIEST}")));
        }
        if from > to {
            return Err(DataError::Config("the first day is after the last one".into()));
        }
        if self.timeframes.is_empty() {
            return Err(DataError::Config("choose at least one timeframe".into()));
        }
        Ok((from, to))
    }

    /// The window Refresh keeps: same length, ending yesterday (unchanged without rolling).
    pub fn rolled(&self, today: i64) -> DatasetConfig {
        let mut c = self.clone();
        if let (true, Some(from), Some(to)) = (self.rolling, time::parse_date(&self.from), time::parse_date(&self.to)) {
            let len = (to - from).max(0);
            let end = today - 1;
            c.from = time::format_date(end - len);
            c.to = time::format_date(end);
        }
        c
    }

    fn sides(&self) -> Vec<Side> {
        if self.ask {
            vec![Side::Bid, Side::Ask]
        } else {
            vec![Side::Bid]
        }
    }
}

/// Days of a range that can have trading (gold is closed all Saturday UTC).
pub fn trading_days(from: i64, to: i64) -> Vec<i64> {
    (from..=to).filter(|&d| time::weekday(d) != 5).collect()
}

/// What is on disk after the last sync.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Manifest {
    pub config: DatasetConfig,
    pub source: String,
    /// First and last minute on disk, UTC seconds.
    pub first: Option<i64>,
    pub last: Option<i64>,
    /// Days with data.
    pub days: usize,
    /// Weekdays the server has no data for (holidays).
    pub empty_days: usize,
    /// Days that failed to download; Refresh retries them.
    pub missing: Vec<String>,
    pub bars: BTreeMap<String, usize>,
    pub bytes: u64,
    /// When the sync finished, UTC seconds.
    pub updated: i64,
    pub cancelled: bool,
    /// Why the download stopped early, if it did.
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Progress {
    pub stage: &'static str,
    pub done: usize,
    pub total: usize,
    pub day: String,
}

/// Files a sync still has to download: (trading days in range, files missing on disk).
pub fn pending(root: &Path, config: &DatasetConfig, today: i64) -> Result<(usize, usize), DataError> {
    let (from, to) = config.range(today)?;
    let days = trading_days(from, to);
    let missing = days
        .iter()
        .flat_map(|&d| config.sides().into_iter().map(move |s| (d, s)))
        .filter(|&(d, s)| !raw_path(root, d, s).exists())
        .count();
    Ok((days.len(), missing))
}

fn raw_path(root: &Path, day: i64, side: Side) -> PathBuf {
    let (y, m, d) = time::civil_from_days(day);
    root.join("raw")
        .join(format!("{y:04}"))
        .join(format!("{m:02}"))
        .join(format!("{d:02}_{}.bi5", side.as_str()))
}

fn raw_day(path: &Path) -> Option<(i64, Side)> {
    let name = path.file_stem()?.to_str()?;
    let (d, side) = name.split_once('_')?;
    let m = path.parent()?.file_name()?.to_str()?;
    let y = path.parent()?.parent()?.file_name()?.to_str()?;
    let day = time::parse_date(&format!("{y}-{m}-{d}"))?;
    let side = match side {
        "BID" => Side::Bid,
        "ASK" => Side::Ask,
        _ => return None,
    };
    Some((day, side))
}

pub fn bars_path(root: &Path, tf: Timeframe) -> PathBuf {
    root.join(format!("{}.bin", tf.as_str()))
}

pub fn read_manifest(root: &Path) -> Option<Manifest> {
    serde_json::from_slice(&fs::read(root.join("manifest.json")).ok()?).ok()
}

fn write_atomic(path: &Path, body: &[u8]) -> Result<(), DataError> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("part");
    fs::write(&tmp, body)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Downloads what is missing in the configured range, deletes days outside it and rebuilds the
/// timeframes. `root` is the folder of one symbol. A download that is cancelled or stopped by a
/// failing server still builds what is on disk.
pub async fn sync(
    root: &Path,
    base_url: &str,
    config: &DatasetConfig,
    today: i64,
    progress: impl Fn(Progress),
    cancel: &AtomicBool,
) -> Result<Manifest, DataError> {
    let (from, to) = config.range(today)?;
    let jobs: Vec<(i64, Side)> = trading_days(from, to)
        .into_iter()
        .flat_map(|d| config.sides().into_iter().map(move |s| (d, s)))
        .filter(|&(d, s)| !raw_path(root, d, s).exists())
        .collect();
    let client = dukascopy::client()?;
    let (mut missing, mut failed_in_row) = (std::collections::BTreeSet::new(), 0usize);
    let (mut cancelled, mut error) = (false, None);
    for (k, &(day, side)) in jobs.iter().enumerate() {
        progress(Progress {
            stage: "Downloading",
            done: k,
            total: jobs.len(),
            day: time::format_date(day),
        });
        let url = dukascopy::day_url(base_url, &config.symbol, day, side);
        match dukascopy::fetch(&client, &url, cancel).await {
            Ok(body) => {
                write_atomic(&raw_path(root, day, side), &body)?;
                failed_in_row = 0;
            }
            Err(DataError::Cancelled) => {
                cancelled = true;
                break;
            }
            Err(e) => {
                missing.insert(day);
                failed_in_row += 1;
                if failed_in_row >= MAX_FAILED_IN_ROW {
                    error = Some(format!("the server is not answering ({e}); press Refresh later to continue"));
                    break;
                }
            }
        }
    }
    // Days after a stop are still missing.
    for &(day, side) in &jobs {
        if !raw_path(root, day, side).exists() && (cancelled || error.is_some()) {
            missing.insert(day);
        }
    }
    prune(root, from, to, config.ask)?;
    let mut m = build(root, config, from, to, &progress)?;
    m.missing = missing.into_iter().map(time::format_date).collect();
    m.cancelled = cancelled;
    m.error = error;
    m.bytes = dir_size(root);
    write_atomic(
        &root.join("manifest.json"),
        &serde_json::to_vec_pretty(&m).map_err(|e| DataError::Format(e.to_string()))?,
    )?;
    Ok(m)
}

/// Deletes day files outside [from, to] (and ask files when ask prices are off).
fn prune(root: &Path, from: i64, to: i64, ask: bool) -> Result<(), DataError> {
    let raw = root.join("raw");
    let Ok(years) = fs::read_dir(&raw) else {
        return Ok(());
    };
    for y in years.flatten() {
        for m in fs::read_dir(y.path()).into_iter().flatten().flatten() {
            for f in fs::read_dir(m.path()).into_iter().flatten().flatten() {
                let p = f.path();
                let keep = raw_day(&p).is_some_and(|(d, s)| (from..=to).contains(&d) && (ask || s == Side::Bid));
                if !keep {
                    fs::remove_file(&p)?;
                }
            }
            let _ = fs::remove_dir(m.path()); // only succeeds when empty
        }
        let _ = fs::remove_dir(y.path());
    }
    Ok(())
}

fn dir_size(p: &Path) -> u64 {
    fs::read_dir(p)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_size(&e.path()),
            _ => e.metadata().map(|m| m.len()).unwrap_or(0),
        })
        .sum()
}

/// One bar on disk and in memory. Prices are the bid; `spread_open` / `spread_close` are ask −
/// bid at the bar's first and last minute (NaN without ask prices). `time` is the open, UTC.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct HBar {
    pub time: i64,
    pub open: f32,
    pub high: f32,
    pub low: f32,
    pub close: f32,
    pub volume: f32,
    pub spread_open: f32,
    pub spread_close: f32,
}

impl HBar {
    fn write(&self, w: &mut impl Write) -> std::io::Result<()> {
        w.write_all(&self.time.to_le_bytes())?;
        for x in [self.open, self.high, self.low, self.close, self.volume, self.spread_open, self.spread_close] {
            w.write_all(&x.to_le_bytes())?;
        }
        Ok(())
    }

    fn read(b: &[u8]) -> HBar {
        let f = |i: usize| f32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        HBar {
            time: i64::from_le_bytes(b[0..8].try_into().unwrap_or_default()),
            open: f(8),
            high: f(12),
            low: f(16),
            close: f(20),
            volume: f(24),
            spread_open: f(28),
            spread_close: f(32),
        }
    }
}

/// Built bars of one timeframe.
pub fn load(root: &Path, tf: Timeframe) -> Result<Vec<HBar>, DataError> {
    let mut body = Vec::new();
    fs::File::open(bars_path(root, tf))
        .map_err(|e| DataError::Io(format!("{} not built yet ({e})", tf.as_str())))?
        .read_to_end(&mut body)?;
    Ok(body.chunks_exact(RECORD).map(HBar::read).collect())
}

/// Groups minutes into bars of one timeframe (bars start on UTC multiples of its length).
struct Aggregator {
    secs: i64,
    cur: Option<HBar>,
    out: BufWriter<fs::File>,
    n: usize,
}

impl Aggregator {
    fn push(&mut self, m: &HBar) -> std::io::Result<()> {
        let start = m.time - m.time.rem_euclid(self.secs);
        match &mut self.cur {
            Some(b) if b.time == start => {
                b.high = b.high.max(m.high);
                b.low = b.low.min(m.low);
                b.close = m.close;
                b.volume += m.volume;
                b.spread_close = m.spread_close;
            }
            _ => {
                self.flush()?;
                self.cur = Some(HBar { time: start, ..*m });
            }
        }
        Ok(())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if let Some(b) = self.cur.take() {
            b.write(&mut self.out)?;
            self.n += 1;
        }
        Ok(())
    }
}

/// Decodes the day files of [from, to] and writes every configured timeframe.
fn build(
    root: &Path,
    config: &DatasetConfig,
    from: i64,
    to: i64,
    progress: &impl Fn(Progress),
) -> Result<Manifest, DataError> {
    let point = dukascopy::point(&config.symbol);
    for tf in Timeframe::ALL {
        if !config.timeframes.contains(&tf) {
            let _ = fs::remove_file(bars_path(root, tf));
        }
    }
    let mut aggs: Vec<(Timeframe, Aggregator, PathBuf)> = Vec::new();
    for &tf in &config.timeframes {
        let tmp = bars_path(root, tf).with_extension("part");
        fs::create_dir_all(root)?;
        aggs.push((
            tf,
            Aggregator {
                secs: tf.seconds(),
                cur: None,
                out: BufWriter::new(fs::File::create(&tmp)?),
                n: 0,
            },
            tmp,
        ));
    }
    let mut m = Manifest {
        config: config.clone(),
        source: "Dukascopy (datafeed.dukascopy.com), bid / ask 1-minute candles, UTC".into(),
        updated: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
        ..Default::default()
    };
    let days = trading_days(from, to);
    for (k, &day) in days.iter().enumerate() {
        if k % 20 == 0 {
            progress(Progress {
                stage: "Building timeframes",
                done: k,
                total: days.len(),
                day: time::format_date(day),
            });
        }
        let Ok(bid) = fs::read(raw_path(root, day, Side::Bid)) else {
            continue;
        };
        let start = day * 86_400;
        let bid = dukascopy::decode(&bid, start, point)?;
        if bid.is_empty() {
            m.empty_days += 1;
            continue;
        }
        let ask = if config.ask {
            fs::read(raw_path(root, day, Side::Ask))
                .ok()
                .map(|b| dukascopy::decode(&b, start, point))
                .transpose()?
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        m.days += 1;
        let mut j = 0;
        for b in &bid {
            while j < ask.len() && ask[j].time < b.time {
                j += 1;
            }
            let (so, sc) = match ask.get(j) {
                Some(a) if a.time == b.time => ((a.open - b.open) as f32, (a.close - b.close) as f32),
                _ => (f32::NAN, f32::NAN),
            };
            let bar = HBar {
                time: b.time,
                open: b.open as f32,
                high: b.high as f32,
                low: b.low as f32,
                close: b.close as f32,
                volume: b.volume as f32,
                spread_open: so,
                spread_close: sc,
            };
            for (_, a, _) in aggs.iter_mut() {
                a.push(&bar)?;
            }
        }
        m.first.get_or_insert(bid[0].time);
        m.last = Some(bid[bid.len() - 1].time);
    }
    for (tf, mut a, tmp) in aggs {
        a.flush()?;
        a.out.flush()?;
        drop(a.out);
        fs::rename(&tmp, bars_path(root, tf))?;
        m.bars.insert(tf.as_str().to_string(), a.n);
    }
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(s: &str) -> i64 {
        time::parse_date(s).unwrap()
    }

    fn config(from: &str, to: &str) -> DatasetConfig {
        DatasetConfig {
            symbol: "XAUUSD".into(),
            from: from.into(),
            to: to.into(),
            rolling: true,
            timeframes: vec![Timeframe::M1, Timeframe::H1, Timeframe::D1],
            ask: true,
        }
    }

    #[test]
    fn range_is_capped_at_yesterday_and_validated() {
        let c = config("2026-09-01", "2030-01-01");
        assert_eq!(c.range(day("2026-09-30")).unwrap(), (day("2026-09-01"), day("2026-09-29")));
        assert!(config("2026-09-10", "2026-09-01").range(day("2026-09-30")).is_err());
        assert!(config("1999-01-01", "2026-09-01").range(day("2026-09-30")).is_err());
        let mut none = config("2026-09-01", "2026-09-10");
        none.timeframes.clear();
        assert!(none.range(day("2026-09-30")).is_err());
    }

    #[test]
    fn rolling_keeps_the_window_length() {
        let c = config("2026-09-01", "2026-09-22");
        let r = c.rolled(day("2026-09-30"));
        assert_eq!((r.from.as_str(), r.to.as_str()), ("2026-09-08", "2026-09-29"));
        let fixed = DatasetConfig { rolling: false, ..c.clone() };
        assert_eq!(fixed.rolled(day("2026-09-30")), fixed);
    }

    #[test]
    fn saturdays_are_skipped_and_paths_round_trip() {
        let days = trading_days(day("2026-09-25"), day("2026-09-28"));
        assert_eq!(days, vec![day("2026-09-25"), day("2026-09-27"), day("2026-09-28")]);
        let p = raw_path(Path::new("/x"), day("2026-01-07"), Side::Ask);
        assert!(p.ends_with("raw/2026/01/07_ASK.bi5"));
        assert_eq!(raw_day(&p), Some((day("2026-01-07"), Side::Ask)));
    }

    /// Builds from files already on disk (no network): 1m, 1h and 1d bars, spreads, pruning.
    #[test]
    fn builds_timeframes_and_prunes_days_outside_the_range() {
        let root = std::env::temp_dir().join(format!("aegis-dataset-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let d0 = day("2026-09-28");
        // Two minutes in the 10:00 hour, one at 11:00; ask is 0.05 above bid.
        let bid = dukascopy::encode(&[
            (36_000, [4_000_000, 4_001_000, 3_999_000, 4_002_000], 1.0),
            (36_060, [4_001_000, 4_003_000, 4_000_500, 4_004_000], 2.0),
            (39_600, [4_003_000, 4_002_000, 4_001_000, 4_003_500], 1.0),
        ]);
        let ask = dukascopy::encode(&[
            (36_000, [4_000_050, 4_001_050, 3_999_050, 4_002_050], 1.0),
            (36_060, [4_001_050, 4_003_050, 4_000_550, 4_004_050], 2.0),
        ]);
        write_atomic(&raw_path(&root, d0, Side::Bid), &bid).unwrap();
        write_atomic(&raw_path(&root, d0, Side::Ask), &ask).unwrap();
        write_atomic(&raw_path(&root, d0 - 30, Side::Bid), &bid).unwrap(); // outside the range
        write_atomic(&raw_path(&root, d0 + 1, Side::Bid), &[]).unwrap(); // holiday
        let c = config("2026-09-28", "2026-09-29");
        prune(&root, d0, d0 + 1, true).unwrap();
        assert!(!raw_path(&root, d0 - 30, Side::Bid).exists());
        let m = build(&root, &c, d0, d0 + 1, &|_| {}).unwrap();
        assert_eq!((m.days, m.empty_days), (1, 1));
        assert_eq!(m.bars["1m"], 3);
        assert_eq!(m.bars["1h"], 2);
        assert_eq!(m.bars["1d"], 1);
        let h = load(&root, Timeframe::H1).unwrap();
        assert_eq!(h[0].time, d0 * 86_400 + 36_000);
        assert_eq!((h[0].open, h[0].high, h[0].low, h[0].close), (4000.0, 4004.0, 3999.0, 4003.0));
        assert_eq!(h[0].volume, 3.0);
        assert!((h[0].spread_open - 0.05).abs() < 1e-3 && (h[0].spread_close - 0.05).abs() < 1e-3);
        assert!(h[1].spread_open.is_nan(), "no ask for the 11:00 minute");
        let d = load(&root, Timeframe::D1).unwrap();
        assert_eq!((d[0].open, d[0].close), (4000.0, 4002.0));
        let _ = fs::remove_dir_all(&root);
    }
}
