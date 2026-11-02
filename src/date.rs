//! Calendar dates without a date crate: days since the Unix epoch and back,
//! after Howard Hinnant's `days_from_civil`/`civil_from_days`.

const SECS_PER_DAY: u64 = 86_400;
const DAYS_PER_ERA: i64 = 146_097;
/// Days from 0000-03-01 (the algorithm's origin) to 1970-01-01.
const EPOCH_OFFSET: i64 = 719_468;

/// Whole days since 1970-01-01 today (UTC); 0 if the clock is before the epoch.
fn today_days() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs() / SECS_PER_DAY).ok())
        .unwrap_or(0)
}

/// Days since 1970-01-01 of the given proleptic Gregorian date.
pub fn days_from_ymd(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * DAYS_PER_ERA + doe - EPOCH_OFFSET
}

/// The (year, month, day) that is `days` after 1970-01-01.
pub fn ymd_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + EPOCH_OFFSET;
    let era = z.div_euclid(DAYS_PER_ERA);
    let doe = z - era * DAYS_PER_ERA;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// Today as `YYYY-MM-DD` (UTC).
pub fn today_iso() -> String {
    let (y, m, d) = ymd_from_days(today_days());
    format!("{y:04}-{m:02}-{d:02}")
}

/// How long ago the ISO timestamp `iso` was ("today", "1 day ago", "N days
/// ago"); `iso` itself when it doesn't start with a `YYYY-MM-DD` date.
pub fn days_ago(iso: &str) -> String {
    days_ago_from(iso, today_days())
}

fn days_ago_from(iso: &str, today: i64) -> String {
    let date_part = iso.split('T').next().unwrap_or(iso);
    let parts: Vec<i64> = date_part
        .split('-')
        .filter_map(|s| s.parse().ok())
        .collect();
    let &[y, m, d] = parts.as_slice() else {
        return iso.to_string();
    };
    match today - days_from_ymd(y, m, d) {
        0 => "today".to_string(),
        1 => "1 day ago".to_string(),
        n => format!("{n} days ago"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::epoch((1970, 1, 1), 0)]
    #[case::next_day((1970, 1, 2), 1)]
    #[case::end_of_year((1970, 12, 31), 364)]
    #[case::y2k((2000, 1, 1), 10957)]
    #[case::leap_day((2000, 2, 29), 11016)]
    #[case::before_epoch((1969, 12, 31), -1)]
    fn days_and_dates_convert_both_ways(#[case] ymd: (i64, i64, i64), #[case] days: i64) {
        assert_eq!(days_from_ymd(ymd.0, ymd.1, ymd.2), days);
        assert_eq!(ymd_from_days(days), ymd);
    }

    #[test]
    fn every_day_of_a_leap_cycle_round_trips() {
        let start = days_from_ymd(1999, 12, 1);
        for days in start..start + 4 * 366 {
            let (y, m, d) = ymd_from_days(days);
            assert_eq!(days_from_ymd(y, m, d), days, "{y}-{m}-{d}");
        }
    }

    #[rstest]
    #[case::same_day("2026-10-29T10:00:00Z", "today")]
    #[case::yesterday("2026-10-28", "1 day ago")]
    #[case::earlier("2026-10-01T00:00:00Z", "28 days ago")]
    #[case::malformed("not-a-date", "not-a-date")]
    fn days_ago_counts_from_today(#[case] iso: &str, #[case] want: &str) {
        assert_eq!(days_ago_from(iso, days_from_ymd(2026, 10, 29)), want);
    }
}
