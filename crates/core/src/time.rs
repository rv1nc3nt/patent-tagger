//! Dependency-free UTC timestamp formatting. Every timestamp column in
//! section 4.2 is ISO 8601 text; callers read the actual clock (so this
//! stays a pure, testable function of a Unix timestamp rather than reaching
//! into `SystemTime` itself).

/// Formats a Unix timestamp (seconds since epoch, UTC) as
/// `YYYY-MM-DDTHH:MM:SSZ`.
pub fn format_unix_timestamp(secs_since_epoch: u64) -> String {
    let days = secs_since_epoch / 86_400;
    let time_of_day = secs_since_epoch % 86_400;
    let (year, month, day) = civil_from_days(days as i64);
    let (hour, minute, second) = (
        time_of_day / 3600,
        (time_of_day / 60) % 60,
        time_of_day % 60,
    );
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Howard Hinnant's `civil_from_days`: days since the Unix epoch -> (year,
/// month, day), proleptic Gregorian calendar. Avoids pulling in a date/time
/// crate for something this self-contained.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_is_1970_01_01() {
        assert_eq!(format_unix_timestamp(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn a_known_recent_timestamp() {
        // 2026-01-01T00:00:00Z
        assert_eq!(format_unix_timestamp(1_767_225_600), "2026-01-01T00:00:00Z");
    }

    #[test]
    fn time_of_day_is_formatted_with_leading_zeros() {
        assert_eq!(format_unix_timestamp(3661), "1970-01-01T01:01:01Z");
    }

    #[test]
    fn a_leap_day() {
        // 2024-02-29T12:00:00Z
        assert_eq!(format_unix_timestamp(1_709_208_000), "2024-02-29T12:00:00Z");
    }
}
