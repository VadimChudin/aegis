//! Public Bybit daily trade archives. Download once; reuse compressed files on subsequent runs.
use super::{calendar, Tick};
use std::{fs, io::Read, path::Path};
pub fn read_file(p: &Path) -> Result<Vec<Tick>, String> {
    let f = fs::File::open(p).map_err(|e| e.to_string())?;
    let mut csv = String::new();
    flate2::read::GzDecoder::new(f)
        .read_to_string(&mut csv)
        .map_err(|e| e.to_string())?;
    let mut lines = csv.lines();
    let head: Vec<_> = lines.next().ok_or("empty archive")?.split(',').collect();
    let index = |s: &str| {
        head.iter()
            .position(|x| *x == s)
            .ok_or_else(|| format!("missing column {s}"))
    };
    let (ti, pi, qi, si, id) = (
        index("timestamp")?,
        index("price")?,
        index("size")?,
        index("side")?,
        index("trdMatchID")?,
    );
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for l in lines {
        let c: Vec<_> = l.split(',').collect();
        if c.len() <= head.len().saturating_sub(1) {
            return Err("truncated trade row".into());
        }
        if !seen.insert(c[id]) {
            continue;
        }
        let n = |i: usize| c[i].parse::<f64>().map_err(|_| "invalid number".to_string());
        let t = Tick {
            time: n(ti)?,
            price: n(pi)?,
            qty: n(qi)?,
            buy: match c[si] {
                "Buy" => true,
                "Sell" => false,
                _ => return Err("invalid aggressor side".into()),
            },
        };
        if !t.time.is_finite() || !t.price.is_finite() || !t.qty.is_finite() || t.price <= 0. || t.qty <= 0. {
            return Err("invalid trade".into());
        }
        out.push(t);
    }
    out.sort_by(|a, b| a.time.total_cmp(&b.time));
    Ok(out)
}
pub async fn load(root: &Path, from: &str, to: &str, progress: impl Fn(usize, usize)) -> Result<Vec<Tick>, String> {
    let a = calendar::parse(from)?;
    let b = calendar::parse(to)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs() as i64
        / 86400;
    if a < calendar::days(2026, 3, 9) || a > b || b >= now || b - a > 183 {
        return Err("choose 1–184 complete days from 2026-03-09 onwards".into());
    }
    fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())?;
    let mut all = Vec::new();
    for (i, day) in (a..=b).enumerate() {
        progress(i, (b - a + 1) as usize);
        let name = format!("XAUUSDT{}.csv.gz", calendar::format(day));
        let path = root.join(&name);
        if !path.exists() {
            let url = format!("https://public.bybit.com/trading/XAUUSDT/{name}");
            let mut error = String::new();
            let mut ok = false;
            for attempt in 0..3 {
                match client.get(&url).send().await {
                    Ok(r) if r.status().is_success() => {
                        let bytes = r.bytes().await.map_err(|e| e.to_string())?;
                        let tmp = path.with_extension("part");
                        fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
                        read_file(&tmp)?;
                        fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
                        ok = true;
                        break;
                    }
                    Ok(r) => error = format!("{name}: HTTP {}", r.status()),
                    Err(e) => error = e.to_string(),
                }
                tokio::time::sleep(std::time::Duration::from_secs(1 << attempt)).await;
            }
            if !ok {
                return Err(error);
            }
        }
        let ticks = read_file(&path)?;
        if ticks
            .iter()
            .any(|t| t.time < (day * 86400) as f64 || t.time >= ((day + 1) * 86400) as f64)
        {
            return Err(format!("timestamps outside {name}"));
        }
        all.extend(ticks);
    }
    progress((b - a + 1) as usize, (b - a + 1) as usize);
    Ok(all)
}
