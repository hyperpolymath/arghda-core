//! Dependency-free UTC timestamps.
//!
//! Emits RFC 3339 / ISO-8601 strings (e.g. `2026-06-18T11:45:32Z`) without
//! pulling in `chrono`/`time`, so the build stays hermetic against the
//! committed lockfile. The calendar conversion is Howard Hinnant's
//! `civil_from_days` algorithm (proleptic Gregorian, exact for all i64 days).

use std::time::{SystemTime, UNIX_EPOCH};

/// Current UTC time as an RFC 3339 string. Falls back to the epoch if the
/// system clock is set before 1970 (it never is in practice).
pub fn now_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    rfc3339_from_unix(secs)
}

/// Format a Unix timestamp (seconds since 1970-01-01T00:00:00Z) as RFC 3339.
pub fn rfc3339_from_unix(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (hh, mm, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

/// Convert days-since-epoch to a `(year, month, day)` civil date.
/// `month` is 1..=12, `day` is 1..=31.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_is_unix_zero() {
        assert_eq!(rfc3339_from_unix(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn known_timestamp_round_trips() {
        // 1_700_000_000 = 2023-11-14T22:13:20Z (a well-known reference point).
        assert_eq!(rfc3339_from_unix(1_700_000_000), "2023-11-14T22:13:20Z");
    }

    #[test]
    fn leap_day_2024() {
        // 2024-02-29T00:00:00Z = 1_709_164_800.
        assert_eq!(rfc3339_from_unix(1_709_164_800), "2024-02-29T00:00:00Z");
    }

    #[test]
    fn now_has_expected_shape() {
        let s = now_rfc3339();
        assert_eq!(s.len(), 20, "RFC3339 Z form is 20 chars: {s}");
        assert!(s.ends_with('Z'));
    }
}
