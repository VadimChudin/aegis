//! Finds levels and every touch of them, and records the touch metrics. Only data up to the
//! close of the touch bar is used: a level is known from the bar after it is confirmed.

use serde::{Deserialize, Serialize};

use super::{
    bar::Bar,
    features::{F, NF},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LevelKind {
    /// 5m swing high / low.
    Swing5,
    /// 1h swing high / low.
    Swing1h,
    /// Previous day high / low (UTC).
    Day,
    /// Previous week high / low.
    Week,
    /// High / low of the previous session (Asia 00–07, London 07–13, New York 13–21 UTC).
    Session,
    /// Round price (multiple of `round_step`).
    Round,
    /// Previous day's point of control (price with the most volume).
    Poc,
    Swing15,
    Swing4h,
    /// Daily swing high / low (from daily bars).
    Swing1d,
    /// Previous month high / low.
    Month,
    /// Equal highs / lows: a second 5m swing at the price of an earlier one (resting stops).
    Equal,
    /// Edge of a fair value gap (3-bar imbalance) of at least 0.3 ATR.
    Fvg,
}

pub const KINDS: [LevelKind; 13] = [
    LevelKind::Swing5,
    LevelKind::Swing1h,
    LevelKind::Day,
    LevelKind::Week,
    LevelKind::Session,
    LevelKind::Round,
    LevelKind::Poc,
    LevelKind::Swing15,
    LevelKind::Swing4h,
    LevelKind::Swing1d,
    LevelKind::Month,
    LevelKind::Equal,
    LevelKind::Fvg,
];

/// Level kinds a swing aggregator produces, with their bar length in seconds.
const SWING_TFS: [(i64, LevelKind); 4] = [
    (900, LevelKind::Swing15),
    (3_600, LevelKind::Swing1h),
    (14_400, LevelKind::Swing4h),
    (86_400, LevelKind::Swing1d),
];

impl LevelKind {
    pub fn bit(self) -> u16 {
        1 << KINDS.iter().position(|k| *k == self).unwrap_or(0)
    }

    pub fn id(self) -> &'static str {
        match self {
            LevelKind::Swing5 => "swing5",
            LevelKind::Swing1h => "swing1h",
            LevelKind::Day => "day",
            LevelKind::Week => "week",
            LevelKind::Session => "session",
            LevelKind::Round => "round",
            LevelKind::Poc => "poc",
            LevelKind::Swing15 => "swing15",
            LevelKind::Swing4h => "swing4h",
            LevelKind::Swing1d => "swing1d",
            LevelKind::Month => "month",
            LevelKind::Equal => "equal",
            LevelKind::Fvg => "fvg",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            LevelKind::Swing5 => "Swing 5m",
            LevelKind::Swing1h => "Swing 1h",
            LevelKind::Day => "Previous day high/low",
            LevelKind::Week => "Previous week high/low",
            LevelKind::Session => "Session high/low",
            LevelKind::Round => "Round price",
            LevelKind::Poc => "Previous day POC",
            LevelKind::Swing15 => "Swing 15m",
            LevelKind::Swing4h => "Swing 4h",
            LevelKind::Swing1d => "Swing 1d",
            LevelKind::Month => "Previous month high/low",
            LevelKind::Equal => "Equal highs/lows",
            LevelKind::Fvg => "Fair value gap edge",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScanConfig {
    /// A bar touches a level when it comes within `zone_atr`·ATR of it.
    pub zone_atr: f64,
    /// After a touch the level re-arms once price has moved `away_atr`·ATR away from it.
    pub away_atr: f64,
    /// Bars on each side of a 5m swing.
    pub swing_n: usize,
    pub round_step: f64,
    /// Levels older than this (bars) are dropped. Round levels never expire.
    pub max_age: usize,
    /// Control experiment: every level is moved by ±(0.5…1.5)·`control_shift`·ATR to a price
    /// with no meaning. 0 = off. If the edge is real, it must disappear with shifted levels.
    #[serde(default)]
    pub control_shift: f64,
}

impl Default for ScanConfig {
    fn default() -> Self {
        ScanConfig {
            zone_atr: 0.1,
            away_atr: 1.0,
            swing_n: 5,
            round_step: 10.0,
            max_age: 7 * 288,
            control_shift: 0.0,
        }
    }
}

/// One touch of a level at the close of bar `i`. `dir` is +1 for a long off support
/// (price came from above), −1 for a short off resistance.
#[derive(Clone, Debug, Serialize)]
pub struct Touch {
    pub i: usize,
    pub time: i64,
    pub dir: i8,
    pub level: f64,
    /// Bitmask of `LevelKind::bit()` for all levels in the zone.
    pub kinds: u16,
    pub atr: f64,
    /// Fill price of a limit order resting at the edge of the touch zone.
    pub fill: f64,
    /// Metrics at the close of the touch bar (entry on close).
    #[serde(skip)]
    pub f: [f64; NF],
    /// The same metrics as of the close of the bar before the touch (limit entry at the level).
    #[serde(skip)]
    pub pre: [f64; NF],
}

struct Ctx<'a> {
    level: &'a Level,
    calendar: &'a [super::news::Event],
    kinds: u16,
    d: f64,
    p: f64,
    atr: f64,
    adr: f64,
}

struct Level {
    price: f64,
    kinds: u16,
    born: usize,
    touches: u32,
    crosses: u32,
    armed: bool,
}

#[derive(Default)]
struct Period {
    high: f64,
    low: f64,
    started: bool,
}

impl Period {
    fn add(&mut self, b: &Bar) {
        if !self.started {
            *self = Period {
                high: b.high,
                low: b.low,
                started: true,
            };
        } else {
            self.high = self.high.max(b.high);
            self.low = self.low.min(b.low);
        }
    }
}

fn session_of(hour: i64) -> u8 {
    match hour {
        0..=6 => 0,
        7..=12 => 1,
        13..=20 => 2,
        _ => 3,
    }
}

fn ratio(a: f64, b: f64) -> f64 {
    if b.abs() > 1e-12 && a.is_finite() {
        a / b
    } else {
        f64::NAN
    }
}

/// Per-bar indicators, each computed from bars up to and including that bar.
struct Ind {
    atr: Vec<f64>,
    ema50: Vec<f64>,
    ema600: Vec<f64>,
    rsi: Vec<f64>,
    vol_avg: Vec<f64>,
    trades_avg: Vec<f64>,
    maxtrade_avg: Vec<f64>,
    atr_pct: Vec<f64>,
    cvd: Vec<f64>,
}

fn indicators(bars: &[Bar]) -> Ind {
    let n = bars.len();
    let mut atr = vec![f64::NAN; n];
    let mut ema50 = vec![f64::NAN; n];
    let mut ema600 = vec![f64::NAN; n];
    let mut rsi = vec![f64::NAN; n];
    let mut cvd = vec![0.0; n];
    let (mut a, mut e1, mut e2, mut up, mut dn, mut c) = (f64::NAN, f64::NAN, f64::NAN, 0.0, 0.0, 0.0);
    for (i, b) in bars.iter().enumerate() {
        let prev = if i > 0 { bars[i - 1].close } else { b.open };
        let tr = (b.high - b.low).max((b.high - prev).abs()).max((b.low - prev).abs());
        a = if a.is_nan() { tr } else { a + (tr - a) / 14.0 };
        e1 = if e1.is_nan() {
            b.close
        } else {
            e1 + (b.close - e1) * 2.0 / 51.0
        };
        e2 = if e2.is_nan() {
            b.close
        } else {
            e2 + (b.close - e2) * 2.0 / 601.0
        };
        let ch = b.close - prev;
        up += (ch.max(0.0) - up) / 14.0;
        dn += ((-ch).max(0.0) - dn) / 14.0;
        let d = b.delta();
        if d.is_finite() {
            c += d;
        }
        atr[i] = a.max(1e-9);
        ema50[i] = e1;
        ema600[i] = e2;
        rsi[i] = if up + dn > 0.0 { 100.0 * up / (up + dn) } else { 50.0 };
        cvd[i] = c;
    }
    // Averages of the 50 bars before `i` (the touch bar itself is excluded).
    let avg_before = |get: &dyn Fn(&Bar) -> f64| {
        let mut out = vec![f64::NAN; n];
        let (mut sum, mut cnt) = (0.0, 0usize);
        for i in 0..n {
            if cnt > 0 {
                out[i] = sum / cnt as f64;
            }
            let v = get(&bars[i]);
            if v.is_finite() {
                sum += v;
                cnt += 1;
            }
            if i >= 50 {
                let old = get(&bars[i - 50]);
                if old.is_finite() {
                    sum -= old;
                    cnt -= 1;
                }
            }
        }
        out
    };
    let vol_avg = avg_before(&|b| b.volume);
    let trades_avg = avg_before(&|b| b.trades);
    let maxtrade_avg = avg_before(&|b| b.max_trade);
    // ATR percentile over the last week, sampled every 12 bars to keep it cheap.
    let mut atr_pct = vec![f64::NAN; n];
    for i in 0..n {
        let from = i.saturating_sub(2016);
        let (mut below, mut total) = (0usize, 0usize);
        let mut j = i;
        while j >= from && j <= i {
            total += 1;
            if atr[j] < atr[i] {
                below += 1;
            }
            if j < from + 12 {
                break;
            }
            j -= 12;
        }
        atr_pct[i] = below as f64 / total.max(1) as f64;
    }
    Ind {
        atr,
        ema50,
        ema600,
        rsi,
        vol_avg,
        trades_avg,
        maxtrade_avg,
        atr_pct,
        cvd,
    }
}

struct Levels {
    list: Vec<Level>,
    /// Control shift in $ (0 = off), see `ScanConfig::control_shift`.
    shift: f64,
}

/// Deterministic pseudo-random number in [0, 1) from a price and a salt.
fn hash01(x: f64, salt: u64) -> f64 {
    let mut h = x.to_bits() ^ salt.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h ^= h >> 33;
    h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    h ^= h >> 33;
    h = h.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    h ^= h >> 33;
    (h >> 11) as f64 / (1u64 << 53) as f64
}

impl Levels {
    /// Where a level at `price` really goes (moved in control mode).
    fn place(&self, price: f64, kind: LevelKind) -> f64 {
        if self.shift <= 0.0 {
            return price;
        }
        let u = hash01(price, kind as u64 + 1);
        let mag = 0.5 + hash01(price, kind as u64 + 101);
        price + if u < 0.5 { -mag } else { mag } * self.shift
    }

    fn add(&mut self, price: f64, kind: LevelKind, born: usize, atr: f64, close: f64, away: f64) {
        if !price.is_finite() {
            return;
        }
        let price = self.place(price, kind);
        if let Some(l) = self.list.iter_mut().find(|l| (l.price - price).abs() <= 0.15 * atr) {
            // A new 5m swing at an older 5m swing's price: equal highs/lows.
            if kind == LevelKind::Swing5 && l.kinds & LevelKind::Swing5.bit() != 0 && born > l.born + 12 {
                l.kinds |= LevelKind::Equal.bit();
            }
            l.kinds |= kind.bit();
            return;
        }
        self.list.push(Level {
            price,
            kinds: kind.bit(),
            born,
            touches: 0,
            crosses: 0,
            armed: (close - price).abs() >= away * atr,
        });
    }
}

pub fn scan(bars: &[Bar], cfg: &ScanConfig) -> Vec<Touch> {
    scan_full(bars, cfg).0
}

/// All touches, plus the "expected entries" after the last bar: every armed level within
/// 3 ATR of the last close, as a touch of the next bar would see it (metrics from the last bar).
pub fn scan_full(bars: &[Bar], cfg: &ScanConfig) -> (Vec<Touch>, Vec<Touch>) {
    let n = bars.len();
    if n < 60 {
        return (Vec::new(), Vec::new());
    }
    let ind = indicators(bars);
    let shift = if cfg.control_shift > 0.0 {
        let mut a: Vec<f64> = ind.atr.iter().copied().filter(|x| x.is_finite()).collect();
        a.sort_by(f64::total_cmp);
        cfg.control_shift * a.get(a.len() / 2).copied().unwrap_or(1.0)
    } else {
        0.0
    };
    let mut lv = Levels {
        list: Vec::new(),
        shift,
    };
    let calendar = super::news::calendar();
    let mut touches = Vec::new();

    let mut day = Period::default();
    let mut week = Period::default();
    let mut sess = Period::default();
    let mut month = Period::default();
    // Swing aggregators: (current bar, finished bars) per timeframe in SWING_TFS.
    let mut agg: Vec<(Period, Vec<Period>)> = SWING_TFS.iter().map(|_| (Period::default(), Vec::new())).collect();
    let mut day_ranges: Vec<f64> = Vec::new();
    let mut day_profile: std::collections::HashMap<i64, f64> = Default::default();
    let (mut vwap_pv, mut vwap_v) = (0.0, 0.0);
    let sn = cfg.swing_n.max(1);

    for i in 0..n {
        let b = bars[i];
        let atr = if i > 0 { ind.atr[i - 1] } else { ind.atr[0] };
        let close_prev = if i > 0 { bars[i - 1].close } else { b.open };

        // Period boundaries: finalise the previous period before this bar is used.
        if i > 0 {
            let pt = bars[i - 1].time;
            let (pd, cd) = (pt.div_euclid(86_400), b.time.div_euclid(86_400));
            if pd != cd && day.started {
                lv.add(day.high, LevelKind::Day, i, atr, close_prev, cfg.away_atr);
                lv.add(day.low, LevelKind::Day, i, atr, close_prev, cfg.away_atr);
                day_ranges.push(day.high - day.low);
                if let Some((&k, _)) = day_profile.iter().max_by(|a, b| a.1.total_cmp(b.1)) {
                    lv.add(k as f64, LevelKind::Poc, i, atr, close_prev, cfg.away_atr);
                }
                day = Period::default();
                day_profile.clear();
                vwap_pv = 0.0;
                vwap_v = 0.0;
            }
            // Weeks start Monday 00:00 UTC (1970-01-01 was a Thursday).
            let (pw, cw) = ((pt.div_euclid(86_400) + 3).div_euclid(7), (cd + 3).div_euclid(7));
            if pw != cw && week.started {
                lv.add(week.high, LevelKind::Week, i, atr, close_prev, cfg.away_atr);
                lv.add(week.low, LevelKind::Week, i, atr, close_prev, cfg.away_atr);
                week = Period::default();
            }
            let ph = pt.div_euclid(3600);
            let chh = b.time.div_euclid(3600);
            if session_of(ph.rem_euclid(24)) != session_of(chh.rem_euclid(24)) || pd != cd {
                if sess.started {
                    lv.add(sess.high, LevelKind::Session, i, atr, close_prev, cfg.away_atr);
                    lv.add(sess.low, LevelKind::Session, i, atr, close_prev, cfg.away_atr);
                }
                sess = Period::default();
            }
            if super::backtest_month(pt) != super::backtest_month(b.time) && month.started {
                lv.add(month.high, LevelKind::Month, i, atr, close_prev, cfg.away_atr);
                lv.add(month.low, LevelKind::Month, i, atr, close_prev, cfg.away_atr);
                month = Period::default();
            }
            // Swings of higher timeframes: the middle of the last 5 finished bars.
            for (k, &(secs, kind)) in SWING_TFS.iter().enumerate() {
                let (cur, done) = &mut agg[k];
                if pt.div_euclid(secs) != b.time.div_euclid(secs) && cur.started {
                    done.push(std::mem::take(cur));
                    if done.len() > 5 {
                        done.remove(0);
                    }
                    if done.len() == 5 {
                        let mid = &done[2];
                        let (hi, lo) = (mid.high, mid.low);
                        if done.iter().all(|h| h.high <= hi) {
                            lv.add(hi, kind, i, atr, close_prev, cfg.away_atr);
                        }
                        if done.iter().all(|h| h.low >= lo) {
                            lv.add(lo, kind, i, atr, close_prev, cfg.away_atr);
                        }
                    }
                }
            }
        }
        // Round levels near price.
        if cfg.round_step > 0.0 {
            let lo = ((b.low - 5.0 * atr) / cfg.round_step).floor() as i64;
            let hi = ((b.high + 5.0 * atr) / cfg.round_step).ceil() as i64;
            if hi - lo < 200 {
                for k in lo..=hi {
                    let p = lv.place(k as f64 * cfg.round_step, LevelKind::Round);
                    let exists = lv
                        .list
                        .iter_mut()
                        .find(|l| (l.price - p).abs() < 1e-9 || (l.price - p).abs() <= 0.15 * atr);
                    match exists {
                        Some(l) => l.kinds |= LevelKind::Round.bit(),
                        None => lv.add(
                            k as f64 * cfg.round_step,
                            LevelKind::Round,
                            0,
                            atr,
                            close_prev,
                            cfg.away_atr,
                        ),
                    }
                }
            }
        }

        // Touches of levels known before this bar.
        if i >= 50 {
            let zone = cfg.zone_atr * atr;
            for dir in [1i8, -1] {
                let d = dir as f64;
                let mut best: Option<usize> = None;
                let mut kinds = 0u16;
                for (k, l) in lv.list.iter().enumerate() {
                    if !l.armed {
                        continue;
                    }
                    let hit = if dir > 0 {
                        close_prev > l.price && b.low <= l.price + zone
                    } else {
                        close_prev < l.price && b.high >= l.price - zone
                    };
                    if !hit {
                        continue;
                    }
                    // The first level price reaches from the previous close: that is where a
                    // resting limit fills. Picking the level nearest the bar's extreme would use
                    // the bar's low/high, i.e. know in advance where price turned (look-ahead).
                    if best.is_none_or(|j| d * (l.price - lv.list[j].price) > 0.0) {
                        best = Some(k);
                    }
                }
                if let Some(k) = best {
                    let p = lv.list[k].price;
                    for l in &lv.list {
                        if (l.price - p).abs() <= 2.0 * zone.max(0.05 * atr) {
                            kinds |= l.kinds;
                        }
                    }
                    let adr = if day_ranges.is_empty() {
                        f64::NAN
                    } else {
                        let t = &day_ranges[day_ranges.len().saturating_sub(20)..];
                        t.iter().sum::<f64>() / t.len() as f64
                    };
                    let room_from = |px: f64| {
                        lv.list
                            .iter()
                            .filter(|l| d * (l.price - px) > zone)
                            .map(|l| d * (l.price - px))
                            .fold(f64::INFINITY, f64::min)
                    };
                    // At the close of the touch bar (entry on close).
                    let day_hi = day.high.max(b.high);
                    let day_lo = if day.started { day.low.min(b.low) } else { b.low };
                    let vwap = if vwap_v + b.volume > 0.0 {
                        (vwap_pv + (b.high + b.low + b.close) / 3.0 * b.volume) / (vwap_v + b.volume)
                    } else {
                        b.close
                    };
                    let ctx = Ctx {
                        level: &lv.list[k],
                        calendar,
                        kinds,
                        d,
                        p,
                        atr,
                        adr,
                    };
                    let f = features(bars, &ind, i, &ctx, room_from(b.close), day_hi, day_lo, vwap);
                    // Before the touch bar (resting limit order at the level): bar i is not used.
                    let pb = &bars[i - 1];
                    let (pre_hi, pre_lo) = if day.started {
                        (day.high, day.low)
                    } else {
                        (pb.high, pb.low)
                    };
                    let pre_vwap = if vwap_v > 0.0 { vwap_pv / vwap_v } else { pb.close };
                    let pre = features(bars, &ind, i - 1, &ctx, room_from(pb.close), pre_hi, pre_lo, pre_vwap);
                    let limit = p + d * zone;
                    let fill = if d * (b.open - limit) <= 0.0 { b.open } else { limit };
                    touches.push(Touch {
                        i,
                        time: b.time,
                        dir,
                        level: p,
                        kinds,
                        atr,
                        fill,
                        f,
                        pre,
                    });
                    lv.list[k].touches += 1;
                    lv.list[k].armed = false;
                }
            }
        }

        // Update level state with this bar.
        let away = cfg.away_atr * atr;
        for l in lv.list.iter_mut() {
            if (close_prev - l.price).signum() != (b.close - l.price).signum() {
                l.crosses += 1;
            }
            let dist = if l.price < b.low {
                b.low - l.price
            } else if l.price > b.high {
                l.price - b.high
            } else {
                0.0
            };
            if dist >= away {
                l.armed = true;
            }
        }
        let round = LevelKind::Round.bit();
        lv.list.retain(|l| {
            if l.kinds == round {
                (l.price - b.close).abs() <= 8.0 * atr
            } else {
                i.saturating_sub(l.born) <= cfg.max_age
            }
        });

        // Levels confirmed by this bar become usable from the next bar.
        if i >= 2 * sn {
            let m = i - sn;
            let w = &bars[m - sn..=i];
            let atr_now = ind.atr[i];
            if w.iter().all(|x| x.high <= bars[m].high) {
                lv.add(bars[m].high, LevelKind::Swing5, i + 1, atr_now, b.close, cfg.away_atr);
            }
            if w.iter().all(|x| x.low >= bars[m].low) {
                lv.add(bars[m].low, LevelKind::Swing5, i + 1, atr_now, b.close, cfg.away_atr);
            }
        }
        if i >= 2 {
            let (a, atr_now) = (&bars[i - 2], ind.atr[i]);
            if b.low - a.high >= 0.3 * atr_now {
                lv.add(a.high, LevelKind::Fvg, i + 1, atr_now, b.close, cfg.away_atr);
            }
            if a.low - b.high >= 0.3 * atr_now {
                lv.add(a.low, LevelKind::Fvg, i + 1, atr_now, b.close, cfg.away_atr);
            }
        }
        day.add(&b);
        week.add(&b);
        sess.add(&b);
        month.add(&b);
        for (cur, _) in agg.iter_mut() {
            cur.add(&b);
        }
        let px = if b.poc.is_finite() { b.poc } else { b.close };
        *day_profile.entry(px.round() as i64).or_insert(0.0) += b.volume;
        vwap_pv += (b.high + b.low + b.close) / 3.0 * b.volume;
        vwap_v += b.volume;
    }

    let last = &bars[n - 1];
    let atr = ind.atr[n - 1];
    let zone = cfg.zone_atr * atr;
    let adr = if day_ranges.is_empty() {
        f64::NAN
    } else {
        let t = &day_ranges[day_ranges.len().saturating_sub(20)..];
        t.iter().sum::<f64>() / t.len() as f64
    };
    let vwap = if vwap_v > 0.0 { vwap_pv / vwap_v } else { last.close };
    let mut pending = Vec::new();
    for l in lv.list.iter().filter(|l| l.armed) {
        let dir: i8 = if l.price < last.close { 1 } else { -1 };
        let d = dir as f64;
        if d * (last.close - l.price) > 3.0 * atr {
            continue;
        }
        let kinds = lv
            .list
            .iter()
            .filter(|o| (o.price - l.price).abs() <= 2.0 * zone.max(0.05 * atr))
            .fold(0u16, |m, o| m | o.kinds);
        let room = lv
            .list
            .iter()
            .filter(|o| d * (o.price - last.close) > zone)
            .map(|o| d * (o.price - last.close))
            .fold(f64::INFINITY, f64::min);
        let ctx = Ctx {
            level: l,
            calendar,
            kinds,
            d,
            p: l.price,
            atr,
            adr,
        };
        let pre = features(bars, &ind, n - 1, &ctx, room, day.high, day.low, vwap);
        pending.push(Touch {
            i: n,
            time: last.time + 300,
            dir,
            level: l.price,
            kinds,
            atr,
            fill: l.price + d * zone,
            f: pre,
            pre,
        });
    }
    (touches, pending)
}

#[allow(clippy::too_many_arguments)]
fn features(bars: &[Bar], ind: &Ind, i: usize, ctx: &Ctx, room: f64, day_hi: f64, day_lo: f64, vwap: f64) -> [f64; NF] {
    let Ctx {
        level,
        calendar,
        kinds,
        d,
        p,
        atr,
        adr,
    } = *ctx;
    let b = &bars[i];
    let mut f = [f64::NAN; NF];
    let mut set = |k: F, v: f64| f[k as usize] = v;
    let range = (b.high - b.low).max(1e-9);
    let long = d > 0.0;
    let appr = &bars[i - 6..i];

    set(F::Confluence, kinds.count_ones() as f64);
    set(
        F::LevelAge,
        if level.born == 0 {
            f64::NAN
        } else {
            i.saturating_sub(level.born) as f64 / 12.0
        },
    );
    set(F::LevelTouches, level.touches as f64);
    set(F::LevelCrosses, level.crosses as f64);
    set(F::Room, if room.is_finite() { (room / atr).min(10.0) } else { 10.0 });

    set(F::ApprSpeed, d * (bars[i - 6].close - p) / atr);
    let toward = |x: &Bar| if long { x.close < x.open } else { x.close > x.open };
    set(
        F::ApprBars,
        bars[..i].iter().rev().take_while(|x| toward(x)).count().min(20) as f64,
    );
    set(
        F::ApprRange,
        appr.iter().map(|x| x.high - x.low).sum::<f64>() / 6.0 / atr,
    );
    let w24 = &bars[i.saturating_sub(24)..i];
    set(
        F::ApprFrom,
        if long {
            (w24.iter().map(|x| x.high).fold(f64::MIN, f64::max) - p) / atr
        } else {
            (p - w24.iter().map(|x| x.low).fold(f64::MAX, f64::min)) / atr
        },
    );

    set(F::Trend5, d * (ind.ema50[i] - ind.ema50[i - 10]) / atr);
    set(F::Trend1h, d * (b.close - ind.ema600[i]) / atr);
    set(F::AtrUsd, atr);
    set(F::AtrPct, ind.atr_pct[i]);
    let dr = (day_hi - day_lo).max(1e-9);
    set(
        F::DayPos,
        if long {
            (b.close - day_lo) / dr
        } else {
            (day_hi - b.close) / dr
        },
    );
    set(F::AdrUsed, ratio(day_hi - day_lo, adr));
    set(F::Rsi, d * (50.0 - ind.rsi[i]));
    set(F::Vwap, d * (vwap - b.close) / atr);

    let (wick, close_pos) = if long {
        (b.open.min(b.close) - b.low, b.close - b.low)
    } else {
        (b.high - b.open.max(b.close), b.high - b.close)
    };
    set(F::Wick, wick / range);
    set(F::Body, (b.close - b.open).abs() / range);
    set(F::ClosePos, close_pos / range);
    set(F::Pen, if long { p - b.low } else { b.high - p } / atr);
    set(F::RangeAtr, range / atr);
    set(F::CloseBack, if d * (b.close - p) > 0.0 { 1.0 } else { 0.0 });

    set(F::VolZ, ratio(b.volume, ind.vol_avg[i]));
    set(
        F::ApprVol,
        ratio(appr.iter().map(|x| x.volume).sum::<f64>() / 6.0, ind.vol_avg[i]),
    );

    set(F::TradesZ, ratio(b.trades, ind.trades_avg[i]));
    set(F::Delta, d * ratio(b.delta(), b.volume));
    set(
        F::ApprDelta,
        d * ratio(
            appr.iter().map(|x| x.delta()).sum::<f64>(),
            appr.iter().map(|x| x.volume).sum::<f64>(),
        ),
    );
    if b.buy_volume.is_finite() {
        let w12 = i.saturating_sub(12)..i;
        let div = if long {
            let lo = bars[w12.clone()].iter().map(|x| x.low).fold(f64::MAX, f64::min);
            let cmin = ind.cvd[w12].iter().copied().fold(f64::MAX, f64::min);
            b.low <= lo && ind.cvd[i] > cmin
        } else {
            let hi = bars[w12.clone()].iter().map(|x| x.high).fold(f64::MIN, f64::max);
            let cmax = ind.cvd[w12].iter().copied().fold(f64::MIN, f64::max);
            b.high >= hi && ind.cvd[i] < cmax
        };
        set(F::CvdDiv, if div { 1.0 } else { 0.0 });
    }

    set(F::BigDelta, d * ratio(b.big_delta, b.volume));
    set(F::BigShare, ratio(b.big_volume, b.volume));
    set(F::MaxTrade, ratio(b.max_trade, ind.maxtrade_avg[i]));
    let (ev, ed) = if long {
        (b.vol_bottom, b.delta_bottom)
    } else {
        (b.vol_top, b.delta_top)
    };
    set(F::ExtVol, ratio(ev, b.volume));
    set(F::ExtDelta, d * ratio(ed, ev));
    let poc_pos = if long { b.poc - b.low } else { b.high - b.poc } / range;
    set(
        F::PocPos,
        if range > 1e-6 && poc_pos.is_finite() {
            poc_pos.clamp(0.0, 1.0)
        } else {
            f64::NAN
        },
    );
    set(F::Imbalance, d * (b.imb_buy - b.imb_sell));

    set(F::OiChg, 100.0 * ratio(b.oi - bars[i - 6].oi, bars[i - 6].oi));
    set(F::LsTop, b.ls_top);
    set(F::TakerRatio, b.taker_ratio);

    let close_t = b.time + 300;
    set(F::Hour, close_t.div_euclid(3600).rem_euclid(24) as f64);
    set(F::Weekday, (close_t.div_euclid(86_400) + 3).rem_euclid(7) as f64);
    set(F::Spread, ratio(range / b.trades.max(1.0), atr));
    let (before, after) = super::news::distance(calendar, close_t);
    set(F::NewsBefore, before);
    set(F::NewsAfter, after);
    set(F::SpreadUsd, b.spread);
    set(F::Funding, b.funding * 1e4);
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wave(n: usize) -> Vec<Bar> {
        (0..n)
            .map(|i| {
                let x = 2000.0 + 20.0 * ((i as f64) / 15.0).sin();
                let mut b = Bar::ohlcv(i as i64 * 300, x, x + 1.0, x - 1.0, x + 0.2, 10.0);
                b.buy_volume = 5.0;
                b.trades = 3.0;
                b
            })
            .collect()
    }

    /// A bar falling through two support levels fills the upper one first. Choosing the level
    /// nearest the bar's low would use where price turned, which is not known in advance.
    #[test]
    fn a_falling_bar_touches_the_first_level_it_reaches() {
        let mut bars: Vec<Bar> = (0..200)
            .map(|i| Bar::ohlcv(i as i64 * 300, 2030.0, 2031.0, 2029.0, 2030.0, 1.0))
            .collect();
        // Round levels every $10: 2020 and 2010 below price. Bar 150 drops to 2009.9.
        bars[150] = Bar::ohlcv(150 * 300, 2030.0, 2030.0, 2009.9, 2012.0, 1.0);
        let cfg = ScanConfig {
            away_atr: 0.5,
            ..ScanConfig::default()
        };
        let t = scan(&bars, &cfg);
        let hit: Vec<_> = t.iter().filter(|x| x.i == 150 && x.dir == 1).collect();
        assert_eq!(hit.len(), 1);
        assert!((hit[0].level - 2020.0).abs() < 1e-9, "level {}", hit[0].level);
    }

    #[test]
    fn finds_touches_in_a_wave_and_never_looks_ahead() {
        let bars = wave(2000);
        let t = scan(&bars, &ScanConfig::default());
        assert!(t.len() > 20, "{} touches", t.len());
        assert!(t.iter().any(|x| x.dir == 1) && t.iter().any(|x| x.dir == -1));
        // Features of a touch must not change when later bars change.
        let k = t.len() / 2;
        let cut = t[k].i + 1;
        let mut other = bars.clone();
        for b in &mut other[cut..] {
            b.high += 50.0;
            b.close += 30.0;
        }
        let t2 = scan(&other, &ScanConfig::default());
        let a = t.iter().find(|x| x.i == t[k].i && x.dir == t[k].dir).unwrap();
        let b = t2.iter().find(|x| x.i == t[k].i && x.dir == t[k].dir).unwrap();
        for (x, y) in a.f.iter().zip(b.f.iter()) {
            assert!(x == y || (x.is_nan() && y.is_nan()));
        }
    }
}
