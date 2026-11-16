//! The time, as the record writes it: UTC, to the second. No time-zone database is read here.

use std::time::{SystemTime, UNIX_EPOCH};

/// Now, in seconds since 1970.
pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// The date of `secs` in UTC: `2026-11-16`.
pub fn day(secs: u64) -> String {
    // Days since 1970-01-01 to a date of the calendar (Howard Hinnant's `civil_from_days`).
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// `secs` as the record writes a moment: `2026-11-16T09:00:00Z`.
pub fn rfc3339(secs: u64) -> String {
    format!("{}T{:02}:{:02}:{:02}Z", day(secs), secs / 3600 % 24, secs / 60 % 60, secs % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moments_are_written_as_the_calendar_has_them() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z", "a leap day");
        assert_eq!(rfc3339(1_794_819_599), "2026-11-16T08:59:59Z");
        assert_eq!(day(1_794_873_600 - 1), "2026-11-16", "the last second of a day");
        assert_eq!(day(1_794_873_600), "2026-11-17");
        assert_eq!(rfc3339(4_102_444_800), "2100-01-01T00:00:00Z", "2100 is not a leap year");
    }
}
