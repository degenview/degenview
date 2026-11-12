//! Dates and times as the interface writes them: "Oct 9, 2026", "14:03", "5 min. ago".
//!
//! Pure functions of a timestamp (and the viewer's UTC offset where a zone applies), shared by every
//! screen that prints one.

use crate::format::civil_from_days;
use crate::service::system::timezone::local_utc_offset_secs;

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

fn split_utc(epoch_ms: i64) -> (i64, i64, i64, i64, i64, i64) {
    let seconds = epoch_ms.div_euclid(1_000);
    let (year, month, day) = civil_from_days(seconds.div_euclid(86_400));
    let rest = seconds.rem_euclid(86_400);
    (year, month, day, rest / 3_600, rest % 3_600 / 60, rest % 60)
}

/// "Oct 9, 2026"
pub fn date_text(epoch_ms: i64) -> String {
    let (year, month, day, ..) = split_utc(epoch_ms);
    format!("{} {day}, {year}", MONTHS[(month - 1) as usize])
}

/// "14:03:07"
pub fn time_text(epoch_ms: i64) -> String {
    let (_, _, _, hour, minute, second) = split_utc(epoch_ms);
    format!("{hour:02}:{minute:02}:{second:02}")
}

/// "Oct 9, 14:03"
pub fn compact_text(epoch_ms: i64) -> String {
    let local_ms = epoch_ms + local_utc_offset_secs(epoch_ms) * 1_000;
    let (_, month, day, hour, minute, _) = split_utc(local_ms);
    format!(
        "{} {day}, {hour:02}:{minute:02}",
        MONTHS[(month - 1) as usize]
    )
}

/// Abbreviated relative time: "now", "5 min. ago", "3 hr. ago", "2 days ago".
pub fn relative_time(now_ms: i64, then_ms: i64) -> String {
    let secs = (now_ms - then_ms) / 1_000;
    if secs < 0 {
        return "in the future".to_owned();
    }
    let (value, unit) = match secs {
        0..=4 => return "now".to_owned(),
        5..=59 => (secs, "sec."),
        60..=3_599 => (secs / 60, "min."),
        3_600..=86_399 => (secs / 3_600, "hr."),
        86_400..=604_799 => (
            secs / 86_400,
            if secs / 86_400 == 1 { "day" } else { "days" },
        ),
        604_800..=2_591_999 => (secs / 604_800, "wk."),
        2_592_000..=31_535_999 => (secs / 2_592_000, "mo."),
        _ => (secs / 31_536_000, "yr."),
    };
    format!("{value} {unit} ago")
}

/// "14:03" in the viewer's time zone.
pub fn clock_time(ms: i64, utc_offset_secs: i64) -> String {
    let seconds = (ms.div_euclid(1_000) + utc_offset_secs).rem_euclid(86_400);
    format!("{:02}:{:02}", seconds / 3_600, seconds % 3_600 / 60)
}

/// "Oct 9, 2026 14:03" in the viewer's time zone.
pub fn date_time_text(ms: i64, utc_offset_secs: i64) -> String {
    format!(
        "{} {}",
        date_text(ms + utc_offset_secs * 1_000),
        clock_time(ms, utc_offset_secs)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_date_and_time_text() {
        // 2023-11-14 22:13:20 UTC
        let ms = 1_700_000_000_000;
        assert_eq!(date_text(ms), "Nov 14, 2023");
        assert_eq!(time_text(ms), "22:13:20");
        assert_eq!(compact_text(ms), "Nov 14, 22:13");
        assert_eq!(date_text(0), "Jan 1, 1970");
    }

    #[test]
    fn relative_times_are_abbreviated() {
        let now = 100_000_000_000;
        for (ago_secs, expected) in [
            (0, "now"),
            (4, "now"),
            (30, "30 sec. ago"),
            (120, "2 min. ago"),
            (3 * 3_600, "3 hr. ago"),
            (86_400, "1 day ago"),
            (3 * 86_400, "3 days ago"),
            (14 * 86_400, "2 wk. ago"),
            (90 * 86_400, "3 mo. ago"),
            (800 * 86_400, "2 yr. ago"),
        ] {
            assert_eq!(
                relative_time(now, now - ago_secs * 1_000),
                expected,
                "{ago_secs}"
            );
        }
        assert_eq!(relative_time(now, now + 5_000), "in the future");
    }

    #[test]
    fn clock_times_follow_the_viewers_zone() {
        // 2024-01-15 15:00 UTC.
        assert_eq!(clock_time(1_705_330_800_000, 0), "15:00");
        assert_eq!(clock_time(1_705_330_800_000, 2 * 3_600), "17:00");
        assert_eq!(clock_time(1_705_330_800_000, -8 * 3_600), "07:00");
        assert_eq!(clock_time(1_705_330_800_000 + 90_000, 0), "15:01");
    }

    #[test]
    fn dates_read_in_the_viewers_zone() {
        // 2024-01-15 23:30 UTC is already the 16th in UTC+2.
        let ms = 1_705_361_400_000;
        assert_eq!(date_time_text(ms, 0), "Jan 15, 2024 23:30");
        assert_eq!(date_time_text(ms, 2 * 3_600), "Jan 16, 2024 01:30");
    }
}
