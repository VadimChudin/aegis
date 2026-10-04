//! Calendar dates as day numbers (days since 1970-01-01, UTC) and New York local time, without a
//! date library. US daylight saving: 2007+ from the second Sunday of March to the first Sunday of
//! November; 1987-2006 from the first Sunday of April to the last Sunday of October; 02:00 local.

/// Day number of a civil date (Howard Hinnant's algorithm).
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Civil date of a day number.
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// 0 = Monday … 6 = Sunday.
pub fn weekday(day: i64) -> u32 {
    (day + 3).rem_euclid(7) as u32
}

/// "YYYY-MM-DD" → day number.
pub fn parse_date(s: &str) -> Option<i64> {
    let mut it = s.trim().splitn(3, '-');
    let y: i64 = it.next()?.parse().ok()?;
    let m: u32 = it.next()?.parse().ok()?;
    let d: u32 = it.next()?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let day = days_from_civil(y, m, d);
    (civil_from_days(day) == (y, m, d)).then_some(day)
}

pub fn format_date(day: i64) -> String {
    let (y, m, d) = civil_from_days(day);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Day number of the n-th (1-based) `wd` weekday of a month; n = 0 means the last one.
fn nth_weekday(y: i64, m: u32, wd: u32, n: u32) -> i64 {
    if n == 0 {
        let next = if m == 12 { days_from_civil(y + 1, 1, 1) } else { days_from_civil(y, m + 1, 1) };
        let last = next - 1;
        return last - ((weekday(last) + 7 - wd) % 7) as i64;
    }
    let first = days_from_civil(y, m, 1);
    first + ((wd + 7 - weekday(first)) % 7) as i64 + 7 * (n as i64 - 1)
}

/// UTC seconds when New York daylight time starts and ends in year `y`.
fn dst_bounds(y: i64) -> (i64, i64) {
    let (start, end) = if y >= 2007 {
        (nth_weekday(y, 3, 6, 2), nth_weekday(y, 11, 6, 1))
    } else {
        (nth_weekday(y, 4, 6, 1), nth_weekday(y, 10, 6, 0))
    };
    // 02:00 EST = 07:00 UTC; 02:00 EDT = 06:00 UTC.
    (start * 86_400 + 7 * 3600, end * 86_400 + 6 * 3600)
}

/// New York offset from UTC in seconds (−5 h or −4 h) at UTC time `t`.
pub fn ny_offset(t: i64) -> i64 {
    let (y, _, _) = civil_from_days(t.div_euclid(86_400));
    let (a, b) = dst_bounds(y);
    if (a..b).contains(&t) {
        -4 * 3600
    } else {
        -5 * 3600
    }
}

/// Fast New York offsets for a time-ordered series: caches the bounds of the current year.
#[derive(Default)]
pub struct NyClock {
    year_lo: i64,
    year_hi: i64,
    dst: (i64, i64),
}

impl NyClock {
    pub fn offset(&mut self, t: i64) -> i64 {
        if !(self.year_lo..self.year_hi).contains(&t) {
            let (y, _, _) = civil_from_days(t.div_euclid(86_400));
            self.year_lo = days_from_civil(y, 1, 1) * 86_400;
            self.year_hi = days_from_civil(y + 1, 1, 1) * 86_400;
            self.dst = dst_bounds(y);
        }
        if (self.dst.0..self.dst.1).contains(&t) {
            -4 * 3600
        } else {
            -5 * 3600
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_round_trip() {
        for day in [-1000, 0, 11_016, 20_000, 20_724, 30_000] {
            let (y, m, d) = civil_from_days(day);
            assert_eq!(days_from_civil(y, m, d), day);
        }
        assert_eq!(parse_date("2026-09-30"), Some(days_from_civil(2026, 9, 30)));
        assert_eq!(format_date(parse_date("2008-02-29").unwrap()), "2008-02-29");
        assert_eq!(parse_date("2025-02-29"), None);
        assert_eq!(parse_date("junk"), None);
        assert_eq!(weekday(0), 3, "1970-01-01 was a Thursday");
        assert_eq!(weekday(parse_date("2026-09-30").unwrap()), 2, "a Wednesday");
    }

    #[test]
    fn new_york_daylight_saving() {
        let at = |s: &str, h: i64| parse_date(s).unwrap() * 86_400 + h * 3600;
        // 2026: 8 March 07:00 UTC … 1 November 06:00 UTC.
        assert_eq!(ny_offset(at("2026-03-08", 6)), -5 * 3600);
        assert_eq!(ny_offset(at("2026-03-08", 7)), -4 * 3600);
        assert_eq!(ny_offset(at("2026-11-01", 5)), -4 * 3600);
        assert_eq!(ny_offset(at("2026-11-01", 6)), -5 * 3600);
        // 2006 rules: 2 April … 29 October.
        assert_eq!(ny_offset(at("2006-04-01", 12)), -5 * 3600);
        assert_eq!(ny_offset(at("2006-07-01", 12)), -4 * 3600);
        assert_eq!(ny_offset(at("2006-10-29", 7)), -5 * 3600);
        let mut c = NyClock::default();
        for t in (at("2005-01-01", 0)..at("2027-01-01", 0)).step_by(3 * 3600 + 17) {
            assert_eq!(c.offset(t), ny_offset(t), "{t}");
        }
    }
}
