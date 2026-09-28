//! Scheduled high-impact US releases (FOMC, CPI, NFP, PCE, PPI, GDP) from `data/calendar.csv`.
//! Release dates are published in advance, so distances to them are known before a touch.

use std::sync::OnceLock;

#[derive(Clone, Debug)]
pub struct Event {
    pub time: i64,
    pub event: String,
    pub importance: u8,
}

const CSV: &str = include_str!("../../data/calendar.csv");
/// Distances are capped at one day.
const CAP_MIN: f64 = 1440.0;

pub fn parse(text: &str) -> Vec<Event> {
    let mut v: Vec<Event> = text
        .lines()
        .skip(1)
        .filter_map(|l| {
            let mut c = l.split(',');
            let time = c.next()?.trim().parse().ok()?;
            let event = c.next()?.trim().to_string();
            let importance = c.next()?.trim().parse().ok()?;
            Some(Event {
                time,
                event,
                importance,
            })
        })
        .collect();
    v.sort_by_key(|e| e.time);
    v
}

pub fn calendar() -> &'static [Event] {
    static CAL: OnceLock<Vec<Event>> = OnceLock::new();
    CAL.get_or_init(|| parse(CSV))
}

/// (minutes until the next event, minutes since the last event) at `t`, each capped at a day.
/// NaN for both when the calendar does not cover `t`.
pub fn distance(cal: &[Event], t: i64) -> (f64, f64) {
    if cal.is_empty() || t < cal[0].time - 86_400 * 40 || t > cal[cal.len() - 1].time + 86_400 * 40 {
        return (f64::NAN, f64::NAN);
    }
    let k = cal.partition_point(|e| e.time <= t);
    let after = k
        .checked_sub(1)
        .map_or(CAP_MIN, |j| ((t - cal[j].time) as f64 / 60.0).min(CAP_MIN));
    let before = cal
        .get(k)
        .map_or(CAP_MIN, |e| ((e.time - t) as f64 / 60.0).min(CAP_MIN));
    (before, after)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distances() {
        let cal = parse("time,event,importance,source\n7200,CPI,3,x\n1000,NFP,3,x\n");
        assert_eq!(cal[0].time, 1000);
        assert_eq!(distance(&cal, 1000), (103.33333333333333, 0.0));
        assert_eq!(distance(&cal, 7260), (CAP_MIN, 1.0));
        let (b, a) = distance(&cal, 400);
        assert_eq!((b, a), (10.0, CAP_MIN));
        assert!(distance(&cal, 10_000_000).0.is_nan());
    }
}
