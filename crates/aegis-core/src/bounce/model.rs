//! Win-probability model for touches: L2-regularised logistic regression on standardised
//! metrics and their squares (squares capture "too little / too much" shapes such as the 1h
//! trend). Research (python/aegis_lab/research) found it as accurate out of sample as gradient
//! boosting on this data, and it is small enough to train inside the app.

use super::features::{index, NF};

/// Metrics the model reads. All of them come from 5m klines, so the model works on the
/// archive the app downloads (no aggTrades needed).
pub const MODEL_FEATURES: [&str; 29] = [
    "atr_usd",
    "atr_pct",
    "adr_used",
    "trend_1h",
    "trend_5m",
    "room_atr",
    "level_age_h",
    "level_crosses",
    "level_touches",
    "confluence",
    "appr_speed",
    "appr_from",
    "appr_delta",
    "appr_range",
    "appr_vol",
    "rsi_stretch",
    "vwap_dev",
    "day_pos",
    "weekday",
    "hour",
    "spread_proxy",
    "pen_atr",
    "wick",
    "close_pos",
    "body",
    "delta",
    "vol_ratio",
    "tape_speed",
    "dir",
];

const C: f64 = 0.1;

pub struct Model {
    cols: Vec<usize>,
    center: Vec<f64>,
    scale: Vec<f64>,
    w: Vec<f64>,
}

fn quantile(v: &mut [f64], q: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    v[((v.len() - 1) as f64 * q).round() as usize]
}

/// Raw inputs of one touch: the metric vector plus the direction (index `NF`).
pub type Row = [f64; NF + 1];

pub fn row(metrics: &[f64; NF], dir: i8) -> Row {
    let mut r = [0.0; NF + 1];
    r[..NF].copy_from_slice(metrics);
    r[NF] = dir as f64;
    r
}

fn cols() -> Vec<usize> {
    MODEL_FEATURES
        .iter()
        .map(|id| if *id == "dir" { Some(NF) } else { index(id) })
        .collect::<Option<Vec<_>>>()
        .expect("model features exist")
}

impl Model {
    fn design(&self, r: &Row, out: &mut Vec<f64>) {
        out.clear();
        out.push(1.0);
        let k = self.cols.len();
        for j in 0..k {
            let v = r[self.cols[j]];
            let z = if v.is_finite() {
                ((v - self.center[j]) / self.scale[j]).clamp(-3.0, 3.0)
            } else {
                0.0
            };
            out.push(z);
        }
        for j in 0..k {
            let z = out[1 + j];
            out.push(z * z);
        }
    }

    /// Fits on rows with labels (1 = win). Returns `None` with too little data or one class only.
    pub fn fit(rows: &[Row], y: &[bool]) -> Option<Model> {
        let n = rows.len();
        let pos = y.iter().filter(|b| **b).count();
        if n < 200 || pos < 20 || n - pos < 20 {
            return None;
        }
        let cols = cols();
        let (mut center, mut scale) = (Vec::new(), Vec::new());
        for &c in &cols {
            let mut v: Vec<f64> = rows.iter().map(|r| r[c]).filter(|x| x.is_finite()).collect();
            let med = quantile(&mut v, 0.5);
            let spread = quantile(&mut v, 0.9) - quantile(&mut v, 0.1);
            center.push(med);
            scale.push(if spread > 1e-12 { spread } else { 1.0 });
        }
        let mut m = Model {
            cols,
            center,
            scale,
            w: Vec::new(),
        };
        let dim = 1 + 2 * m.cols.len();
        let mut xs = Vec::with_capacity(n * dim);
        let mut buf = Vec::with_capacity(dim);
        for r in rows {
            m.design(r, &mut buf);
            xs.extend_from_slice(&buf);
        }
        let mut w = vec![0.0; dim];
        // Newton / IRLS with ridge 1/C on all weights except the intercept.
        for _ in 0..25 {
            let mut h = vec![0.0; dim * dim];
            let mut g = vec![0.0; dim];
            for i in 0..n {
                let x = &xs[i * dim..(i + 1) * dim];
                let s: f64 = x.iter().zip(&w).map(|(a, b)| a * b).sum();
                let p = 1.0 / (1.0 + (-s).exp());
                let e = p - f64::from(u8::from(y[i]));
                let wt = (p * (1.0 - p)).max(1e-9);
                for a in 0..dim {
                    g[a] += e * x[a];
                    let xa = wt * x[a];
                    for b in a..dim {
                        h[a * dim + b] += xa * x[b];
                    }
                }
            }
            for a in 1..dim {
                g[a] += w[a] / C;
                h[a * dim + a] += 1.0 / C;
            }
            for a in 0..dim {
                for b in 0..a {
                    h[a * dim + b] = h[b * dim + a];
                }
            }
            let step = solve(&mut h, &g, dim)?;
            let mut change = 0.0f64;
            for a in 0..dim {
                w[a] -= step[a];
                change = change.max(step[a].abs());
            }
            if change < 1e-6 {
                break;
            }
        }
        m.w = w;
        Some(m)
    }

    pub fn predict(&self, r: &Row) -> f64 {
        let mut buf = Vec::with_capacity(self.w.len());
        self.design(r, &mut buf);
        let s: f64 = buf.iter().zip(&self.w).map(|(a, b)| a * b).sum();
        1.0 / (1.0 + (-s).exp())
    }
}

/// Solves `h · x = g` for a symmetric positive-definite `h` (Cholesky, in place).
fn solve(h: &mut [f64], g: &[f64], n: usize) -> Option<Vec<f64>> {
    for j in 0..n {
        let mut d = h[j * n + j];
        for k in 0..j {
            d -= h[j * n + k] * h[j * n + k];
        }
        if d <= 0.0 {
            return None;
        }
        let d = d.sqrt();
        h[j * n + j] = d;
        for i in j + 1..n {
            let mut s = h[i * n + j];
            for k in 0..j {
                s -= h[i * n + k] * h[j * n + k];
            }
            h[i * n + j] = s / d;
        }
    }
    let mut y = vec![0.0; n];
    for i in 0..n {
        let mut s = g[i];
        for k in 0..i {
            s -= h[i * n + k] * y[k];
        }
        y[i] = s / h[i * n + i];
    }
    let mut x = vec![0.0; n];
    for i in (0..n).rev() {
        let mut s = y[i];
        for k in i + 1..n {
            s -= h[k * n + i] * x[k];
        }
        x[i] = s / h[i * n + i];
    }
    Some(x)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learns_a_simple_rule() {
        let wick = index("wick").unwrap();
        let mut rows = Vec::new();
        let mut y = Vec::new();
        let mut s = 1u64;
        for _ in 0..3000 {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let u = (s >> 11) as f64 / (1u64 << 53) as f64;
            let mut r = [f64::NAN; NF + 1];
            r[wick] = u;
            r[NF] = 1.0;
            rows.push(r);
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let noise = (s >> 11) as f64 / (1u64 << 53) as f64;
            y.push(noise < 0.2 + 0.6 * u);
        }
        let m = Model::fit(&rows, &y).unwrap();
        let mut lo = [f64::NAN; NF + 1];
        lo[wick] = 0.05;
        let mut hi = lo;
        hi[wick] = 0.95;
        let (a, b) = (m.predict(&lo), m.predict(&hi));
        assert!(a < 0.35 && b > 0.65, "{a} {b}");
    }
}
