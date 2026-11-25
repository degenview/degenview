use super::*;

fn utc(year: i64, month: i64, date: i64, hour: i64, minute: i64) -> i64 {
    days_from_civil(year, month, date) * DAY_MS + hour * HOUR_MS + minute * 60_000
}

#[test]
fn the_offset_follows_the_us_clock_changes() {
    // 2024: the second Sunday of March is the 10th, the first of November the 3rd.
    assert_eq!(
        nth_sunday(2024, 3, 2),
        days_from_civil(2024, 3, 10),
        "spring"
    );
    assert_eq!(
        nth_sunday(2024, 11, 1),
        days_from_civil(2024, 11, 3),
        "autumn"
    );
    assert_eq!(eastern_offset_ms(utc(2024, 3, 10, 6, 59)), -5 * HOUR_MS);
    assert_eq!(eastern_offset_ms(utc(2024, 3, 10, 7, 0)), -4 * HOUR_MS);
    assert_eq!(eastern_offset_ms(utc(2024, 11, 3, 5, 59)), -4 * HOUR_MS);
    assert_eq!(eastern_offset_ms(utc(2024, 11, 3, 6, 0)), -5 * HOUR_MS);
}

#[test]
fn the_session_opens_at_half_past_nine_new_york_time() {
    // 24 Nov 2026 is in standard time (13:30 UTC would be 08:30), 14:30 UTC is 09:30.
    let winter = local_day(utc(2026, 11, 24, 15, 0));
    assert_eq!(open_ms(winter), utc(2026, 11, 24, 14, 30));
    assert_eq!(close_ms(winter), utc(2026, 11, 24, 21, 0));
    // 9 Jul 2026 is in daylight time: 13:30 UTC.
    let summer = local_day(utc(2026, 7, 9, 15, 0));
    assert_eq!(open_ms(summer), utc(2026, 7, 9, 13, 30));
    assert_eq!(close_ms(summer), utc(2026, 7, 9, 20, 0));
}

#[test]
fn the_session_is_open_on_weekdays_from_the_open_to_the_close() {
    let open = utc(2026, 11, 24, 14, 30);
    assert!(!is_open(open - 1));
    assert!(is_open(open));
    assert!(is_open(utc(2026, 11, 24, 20, 59)));
    assert!(!is_open(utc(2026, 11, 24, 21, 0)));
    // Saturday 28 November 2026.
    assert!(!is_open(utc(2026, 11, 28, 16, 0)));
}

#[test]
fn the_new_york_day_is_not_the_utc_day() {
    // 02:00 UTC on the 25th is 21:00 on the 24th in New York.
    assert_eq!(
        local_day(utc(2026, 11, 25, 2, 0)),
        days_from_civil(2026, 11, 24)
    );
    assert_eq!(
        local_day(utc(2026, 11, 25, 5, 0)),
        days_from_civil(2026, 11, 25)
    );
}
