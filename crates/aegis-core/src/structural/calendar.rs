//! Civil dates and modern US daylight saving (data starts in 2026).
pub fn days(y: i64, m: u32, d: u32) -> i64 {
    let y = y - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yo = y - era * 400;
    let mp = if m > 2 { m as i64 - 3 } else { m as i64 + 9 };
    era * 146097 + yo * 365 + yo / 4 - yo / 100 + (153 * mp + 2) / 5 + d as i64 - 1 - 719468
}
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yo = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yo + yo / 4 - yo / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (yo + era * 400 + i64::from(m <= 2), m, d)
}
pub fn weekday(d: i64) -> u32 {
    (d + 3).rem_euclid(7) as u32
}
pub fn parse(s: &str) -> Result<i64, String> {
    let p: Vec<_> = s.split('-').collect();
    if p.len() != 3 {
        return Err("invalid date".into());
    }
    let y = p[0].parse::<i64>().map_err(|_| "invalid year")?;
    let m = p[1].parse::<u32>().map_err(|_| "invalid month")?;
    let d = p[2].parse::<u32>().map_err(|_| "invalid day")?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return Err("invalid date".into());
    }
    let x = days(y, m, d);
    if civil_from_days(x) != (y, m, d) {
        return Err("invalid date".into());
    }
    Ok(x)
}
pub fn format(d: i64) -> String {
    let (y, m, d) = civil_from_days(d);
    format!("{y:04}-{m:02}-{d:02}")
}
pub fn ny_offset(t: i64) -> i64 {
    let (y, _, _) = civil_from_days(t.div_euclid(86400));
    let mar = days(y, 3, 1);
    let nov = days(y, 11, 1);
    let start = (mar + ((6 + 7 - weekday(mar)) % 7) as i64 + 7) * 86400 + 7 * 3600;
    let end = (nov + ((6 + 7 - weekday(nov)) % 7) as i64) * 86400 + 6 * 3600;
    if (start..end).contains(&t) {
        -4 * 3600
    } else {
        -5 * 3600
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dates_and_dst() {
        assert!(parse("2026-02-30").is_err());
        assert_eq!(ny_offset(days(2026, 3, 8) * 86400 + 6 * 3600), -5 * 3600);
        assert_eq!(ny_offset(days(2026, 3, 8) * 86400 + 7 * 3600), -4 * 3600);
        assert_eq!(format(parse("2026-09-30").unwrap()), "2026-09-30");
    }
}
