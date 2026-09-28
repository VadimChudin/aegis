//! 1-second candles with aggressive buy and sell volume, built from Binance aggTrades. The
//! position engine (`position.rs`) walks them to manage a trade second by second.
//!
//! On disk: an 8-byte magic, then little-endian records (time i64, open, high, low, close,
//! buy, sell as f32), 32 bytes each.

use std::{
    io::{BufRead, BufReader, Read, Write},
    path::Path,
};

const MAGIC: &[u8; 8] = b"AEGSEC1\n";
const REC: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sec {
    pub time: i64,
    pub open: f32,
    pub high: f32,
    pub low: f32,
    pub close: f32,
    /// Aggressive buy volume (taker bought).
    pub buy: f32,
    /// Aggressive sell volume.
    pub sell: f32,
}

impl Sec {
    fn new(time: i64, px: f32) -> Sec {
        Sec {
            time,
            open: px,
            high: px,
            low: px,
            close: px,
            buy: 0.0,
            sell: 0.0,
        }
    }
}

/// Aggregates aggTrades CSV lines (`agg_trade_id,price,quantity,first_trade_id,last_trade_id,
/// transact_time,is_buyer_maker`, with or without a header) into 1-second candles.
pub fn seconds_from_agg(reader: impl BufRead) -> Result<Vec<Sec>, String> {
    let mut out: Vec<Sec> = Vec::new();
    for (n, line) in reader.lines().enumerate() {
        let line = line.map_err(|e| e.to_string())?;
        if !line.starts_with(|c: char| c.is_ascii_digit()) {
            continue;
        }
        let cells: Vec<&str> = line.split(',').collect();
        let parse = |k: usize| cells.get(k).and_then(|s| s.trim().parse::<f64>().ok());
        let (Some(px), Some(qty), Some(ms)) = (parse(1), parse(2), parse(5)) else {
            return Err(format!("aggTrades line {}: bad row", n + 1));
        };
        let t = (ms as i64).div_euclid(1000);
        let px = px as f32;
        // Buyer is maker = the aggressor sold.
        let sell = cells.get(6).is_some_and(|m| m.trim().eq_ignore_ascii_case("true"));
        match out.last_mut() {
            Some(s) if s.time == t => {
                s.high = s.high.max(px);
                s.low = s.low.min(px);
                s.close = px;
            }
            // A rare out-of-order print is folded into the current second.
            Some(s) if s.time > t => {
                s.high = s.high.max(px);
                s.low = s.low.min(px);
            }
            _ => out.push(Sec::new(t, px)),
        }
        let s = out.last_mut().expect("pushed");
        if sell {
            s.sell += qty as f32;
        } else {
            s.buy += qty as f32;
        }
    }
    Ok(out)
}

/// 1-second candles of an aggTrades zip archive.
pub fn seconds_from_zip(bytes: &[u8]) -> Result<Vec<Sec>, String> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let f = z.by_index(0).map_err(|e| e.to_string())?;
    seconds_from_agg(BufReader::with_capacity(1 << 20, f))
}

pub fn encode(secs: &[Sec]) -> Vec<u8> {
    let mut v = Vec::with_capacity(MAGIC.len() + secs.len() * REC);
    v.extend_from_slice(MAGIC);
    for s in secs {
        v.extend_from_slice(&s.time.to_le_bytes());
        for x in [s.open, s.high, s.low, s.close, s.buy, s.sell] {
            v.extend_from_slice(&x.to_le_bytes());
        }
    }
    v
}

pub fn decode(bytes: &[u8]) -> Result<Vec<Sec>, String> {
    let body = bytes
        .strip_prefix(MAGIC.as_slice())
        .ok_or("not an AEGIS seconds file")?;
    if body.len() % REC != 0 {
        return Err("truncated seconds file".into());
    }
    let f = |b: &[u8], k: usize| f32::from_le_bytes(b[8 + 4 * k..12 + 4 * k].try_into().expect("4 bytes"));
    Ok(body
        .chunks_exact(REC)
        .map(|b| Sec {
            time: i64::from_le_bytes(b[..8].try_into().expect("8 bytes")),
            open: f(b, 0),
            high: f(b, 1),
            low: f(b, 2),
            close: f(b, 3),
            buy: f(b, 4),
            sell: f(b, 5),
        })
        .collect())
}

pub fn save_seconds(path: &Path, secs: &[Sec]) -> std::io::Result<()> {
    let tmp = path.with_extension("part");
    std::fs::File::create(&tmp)?.write_all(&encode(secs))?;
    std::fs::rename(tmp, path)
}

/// Loads a seconds file, or every `*.zip` aggTrades archive of a directory (in name order,
/// overlapping seconds dropped).
pub fn load_seconds(path: &Path) -> Result<Vec<Sec>, String> {
    if !path.is_dir() {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        return decode(&bytes);
    }
    use rayon::prelude::*;
    let mut files: Vec<_> = std::fs::read_dir(path)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "zip"))
        .collect();
    files.sort();
    let parts: Vec<Result<Vec<Sec>, String>> = files
        .par_iter()
        .map(|p| {
            let mut b = Vec::new();
            std::fs::File::open(p)
                .and_then(|mut f| f.read_to_end(&mut b))
                .map_err(|e| format!("{}: {e}", p.display()))?;
            seconds_from_zip(&b).map_err(|e| format!("{}: {e}", p.display()))
        })
        .collect();
    let mut all = Vec::new();
    for part in parts {
        append(&mut all, part?);
    }
    Ok(all)
}

/// Appends a later chunk, skipping seconds already covered.
pub fn append(all: &mut Vec<Sec>, part: Vec<Sec>) {
    let last = all.last().map_or(i64::MIN, |s| s.time);
    all.extend(part.into_iter().filter(|s| s.time > last));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregates_trades_into_seconds_and_round_trips() {
        let csv = "agg_trade_id,price,quantity,first_trade_id,last_trade_id,transact_time,is_buyer_maker\n\
                   1,100.0,1.0,1,1,1000,false\n\
                   2,101.0,2.0,2,2,1500,true\n\
                   3,99.5,0.5,3,3,1999,false\n\
                   4,100.5,3.0,4,4,3000,true\n";
        let s = seconds_from_agg(csv.as_bytes()).unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(
            (s[0].time, s[0].open, s[0].high, s[0].low, s[0].close),
            (1, 100.0, 101.0, 99.5, 99.5)
        );
        assert_eq!((s[0].buy, s[0].sell), (1.5, 2.0));
        assert_eq!((s[1].time, s[1].sell), (3, 3.0));
        assert_eq!(decode(&encode(&s)).unwrap(), s);
    }
}
