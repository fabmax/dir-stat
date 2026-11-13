//! Display formatting for sizes, counts, percentages and times (UX §12).
//! Units are decimal (1 kB = 1,000 B); separators are en-US.
//!
//! The whole UX §12 set lives here, tested together; some formats are first used by the views
//! of later milestones.
#![allow(dead_code)]

use std::time::Duration;

const SIZE_UNITS: [&str; 6] = ["kB", "MB", "GB", "TB", "PB", "EB"];

/// Prefix for a size that is only a lower bound: "≥" plus a narrow no-break space.
pub const AT_LEAST: &str = "\u{2265}\u{202F}";

/// Shown where a value is still pending.
pub const PENDING: &str = "\u{2014}";

/// Size with three significant digits: "512 B", "1.23 GB", "12.3 GB", "123 GB".
pub fn size(bytes: u64) -> String {
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    for unit in SIZE_UNITS {
        value /= 1000.0;
        if let Some(digits) = three_digits(value) {
            return format!("{digits} {unit}");
        }
    }
    unreachable!("u64 sizes stay below 1000 EB")
}

/// A size that is only a lower bound: "≥ 12.3 GB".
pub fn size_at_least(bytes: u64) -> String {
    format!("{AT_LEAST}{}", size(bytes))
}

/// Exact bytes for tooltips: "24,113,402,880 bytes".
pub fn bytes_exact(bytes: u64) -> String {
    plural(bytes, "byte", "bytes")
}

/// Count for columns and pills: "8,912", "12.3k", "420k", "1.21M".
pub fn count(n: u64) -> String {
    if n < 10_000 {
        return thousands(n);
    }
    if n < 1_000_000
        && let Some(digits) = three_digits(n as f64 / 1e3)
    {
        return format!("{digits}k");
    }
    match three_digits(n as f64 / 1e6) {
        Some(digits) => format!("{digits}M"),
        None => format!("{}M", thousands(n / 1_000_000)),
    }
}

/// Full number with en-US thousands separators: "312,004".
pub fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Count for prose with the right noun: "1 item", "312,004 items".
pub fn plural(n: u64, singular: &str, plural: &str) -> String {
    format!(
        "{} {}",
        thousands(n),
        if n == 1 { singular } else { plural }
    )
}

/// Share of `whole`: "42%", "4.2%", "<0.1%", "0%".
pub fn percent(part: u64, whole: u64) -> String {
    if part == 0 || whole == 0 {
        return "0%".into();
    }
    let pct = part as f64 * 100.0 / whole as f64;
    if pct < 0.1 {
        return "<0.1%".into();
    }
    if pct < 10.0 {
        let one_decimal = format!("{pct:.1}");
        if one_decimal != "10.0" {
            return format!("{one_decimal}%");
        }
    }
    format!("{pct:.0}%")
}

/// Elapsed time: "0:42", "12:05", "1:02:33".
pub fn elapsed(duration: Duration) -> String {
    let secs = duration.as_secs();
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Remaining time: "less than a minute left", "about 2 min left", "about 1 h 20 min left".
pub fn eta(remaining: Duration) -> String {
    let secs = remaining.as_secs();
    if secs < 60 {
        return "less than a minute left".into();
    }
    let minutes = (secs + 30) / 60;
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("about {m} min left"),
        (h, 0) => format!("about {h} h left"),
        (h, m) => format!("about {h} h {m} min left"),
    }
}

/// Relative date: "just now", "5 min ago", "yesterday", "3 days ago", "8 months ago".
pub fn relative(age: Duration) -> String {
    let secs = age.as_secs();
    let days = secs / 86_400;
    match secs {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", secs / 60),
        3600..86_400 => plural(secs / 3600, "hour", "hours") + " ago",
        _ if days < 2 => "yesterday".into(),
        _ if days < 30 => format!("{days} days ago"),
        _ if days < 365 => plural(days / 30, "month", "months") + " ago",
        _ => plural(days / 365, "year", "years") + " ago",
    }
}

/// Compact relative date for cleanup rows: "<1 d", "5 d", "8 mo", "2 y".
pub fn relative_compact(age: Duration) -> String {
    let days = age.as_secs() / 86_400;
    match days {
        0 => "<1 d".into(),
        1..30 => format!("{days} d"),
        30..365 => format!("{} mo", days / 30),
        _ => format!("{} y", days / 365),
    }
}

/// Absolute local date for tooltips: "2026-03-14 18:22".
/// `utc_offset_secs` is the local zone's offset at that instant.
pub fn datetime(unix_secs: i64, utc_offset_secs: i32) -> String {
    let local = unix_secs + i64::from(utc_offset_secs);
    let (days, secs_of_day) = (local.div_euclid(86_400), local.rem_euclid(86_400));
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        secs_of_day / 3600,
        secs_of_day / 60 % 60
    )
}

/// Scan rate: "850 items/s", "9.8k items/s", "123k items/s", "1.2M items/s".
pub fn rate(items_per_sec: f64) -> String {
    let n = items_per_sec.max(0.0);
    let compact = if n < 999.5 {
        format!("{n:.0}")
    } else if n < 9_950.0 {
        format!("{:.1}k", n / 1e3)
    } else if n < 999_500.0 {
        format!("{:.0}k", n / 1e3)
    } else {
        format!("{:.1}M", n / 1e6)
    };
    format!("{compact} items/s")
}

/// Formats `value` (≥ 0.5) with three significant digits, keeping trailing zeros.
/// Returns `None` when the result would reach 1000, so the caller moves up a unit.
fn three_digits(value: f64) -> Option<String> {
    let decimals: usize = if value < 10.0 {
        2
    } else if value < 100.0 {
        1
    } else {
        0
    };
    let text = format!("{value:.decimals$}");
    let rounded: f64 = text.parse().expect("formatted float parses");
    let limit = [1000.0, 100.0, 10.0][decimals];
    if rounded < limit {
        Some(text)
    } else if decimals == 0 {
        None
    } else {
        three_digits(rounded)
    }
}

/// Days since 1970-01-01 to (year, month, day), proleptic Gregorian (H. Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: u64 = 60;
    const HOUR: u64 = 3600;
    const DAY: u64 = 86_400;

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    #[test]
    fn sizes() {
        assert_eq!(size(0), "0 B");
        assert_eq!(size(512), "512 B");
        assert_eq!(size(999), "999 B");
        assert_eq!(size(1000), "1.00 kB");
        assert_eq!(size(1_230_000_000), "1.23 GB");
        assert_eq!(size(12_300_000_000), "12.3 GB");
        assert_eq!(size(123_000_000_000), "123 GB");
        assert_eq!(size(4_500_000_000), "4.50 GB");
        assert_eq!(size(1_000_000_000_000), "1.00 TB");
        assert_eq!(size(2_000_000_000_000_000), "2.00 PB");
        assert_eq!(size(u64::MAX), "18.4 EB");
    }

    #[test]
    fn size_rounding_moves_up_a_unit() {
        assert_eq!(size(999_600_000), "1.00 GB");
        assert_eq!(size(999_499), "999 kB");
        assert_eq!(size(9_996), "10.0 kB");
        assert_eq!(size(99_960), "100 kB");
    }

    #[test]
    fn size_lower_bound_and_pending() {
        assert_eq!(size_at_least(12_300_000_000), "\u{2265}\u{202F}12.3 GB");
        assert_eq!(PENDING, "—");
    }

    #[test]
    fn exact_bytes() {
        assert_eq!(bytes_exact(24_113_402_880), "24,113,402,880 bytes");
        assert_eq!(bytes_exact(1), "1 byte");
        assert_eq!(bytes_exact(0), "0 bytes");
    }

    #[test]
    fn counts() {
        assert_eq!(count(0), "0");
        assert_eq!(count(8_912), "8,912");
        assert_eq!(count(9_999), "9,999");
        assert_eq!(count(10_000), "10.0k");
        assert_eq!(count(12_345), "12.3k");
        assert_eq!(count(420_000), "420k");
        assert_eq!(count(999_999), "1.00M");
        assert_eq!(count(1_210_000), "1.21M");
        assert_eq!(count(123_400_000), "123M");
        assert_eq!(count(2_500_000_000), "2,500M");
    }

    #[test]
    fn prose_counts() {
        assert_eq!(thousands(312_004), "312,004");
        assert_eq!(thousands(1_000_000), "1,000,000");
        assert_eq!(thousands(999), "999");
        assert_eq!(plural(312_004, "item", "items"), "312,004 items");
        assert_eq!(plural(1, "item", "items"), "1 item");
        assert_eq!(
            plural(1, "folder", "folders") + " couldn't be read",
            "1 folder couldn't be read"
        );
    }

    #[test]
    fn percentages() {
        assert_eq!(percent(42, 100), "42%");
        assert_eq!(percent(42, 1000), "4.2%");
        assert_eq!(percent(1, 1000), "0.1%");
        assert_eq!(percent(1, 10_000), "<0.1%");
        assert_eq!(percent(0, 100), "0%");
        assert_eq!(percent(5, 0), "0%");
        assert_eq!(percent(100, 100), "100%");
        assert_eq!(percent(9_996, 100_000), "10%");
        assert_eq!(percent(996, 1000), "100%");
    }

    #[test]
    fn elapsed_times() {
        assert_eq!(elapsed(secs(42)), "0:42");
        assert_eq!(elapsed(secs(12 * MIN + 5)), "12:05");
        assert_eq!(elapsed(secs(HOUR + 2 * MIN + 33)), "1:02:33");
    }

    #[test]
    fn etas() {
        assert_eq!(eta(secs(20)), "less than a minute left");
        assert_eq!(eta(secs(2 * MIN)), "about 2 min left");
        assert_eq!(eta(secs(90)), "about 2 min left");
        assert_eq!(eta(secs(HOUR)), "about 1 h left");
        assert_eq!(eta(secs(HOUR + 20 * MIN)), "about 1 h 20 min left");
    }

    #[test]
    fn relative_dates() {
        assert_eq!(relative(secs(10)), "just now");
        assert_eq!(relative(secs(5 * MIN)), "5 min ago");
        assert_eq!(relative(secs(HOUR)), "1 hour ago");
        assert_eq!(relative(secs(3 * HOUR)), "3 hours ago");
        assert_eq!(relative(secs(DAY + HOUR)), "yesterday");
        assert_eq!(relative(secs(3 * DAY)), "3 days ago");
        assert_eq!(relative(secs(35 * DAY)), "1 month ago");
        assert_eq!(relative(secs(8 * 30 * DAY)), "8 months ago");
        assert_eq!(relative(secs(2 * 365 * DAY)), "2 years ago");
    }

    #[test]
    fn compact_relative_dates() {
        assert_eq!(relative_compact(secs(3 * HOUR)), "<1 d");
        assert_eq!(relative_compact(secs(5 * DAY)), "5 d");
        assert_eq!(relative_compact(secs(8 * 30 * DAY)), "8 mo");
        assert_eq!(relative_compact(secs(2 * 365 * DAY)), "2 y");
    }

    #[test]
    fn absolute_dates() {
        // 2026-03-14 17:22 UTC, shown in UTC+1.
        assert_eq!(datetime(1_773_508_920, 3600), "2026-03-14 18:22");
        assert_eq!(datetime(0, 0), "1970-01-01 00:00");
        assert_eq!(datetime(-60, 0), "1969-12-31 23:59");
        assert_eq!(datetime(951_782_400, 0), "2000-02-29 00:00");
    }

    #[test]
    fn rates() {
        assert_eq!(rate(9_800.0), "9.8k items/s");
        assert_eq!(rate(850.0), "850 items/s");
        assert_eq!(rate(123_456.0), "123k items/s");
        assert_eq!(rate(1_234_567.0), "1.2M items/s");
        assert_eq!(rate(9_990.0), "10k items/s");
    }
}
