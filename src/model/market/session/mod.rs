//! The regular US equity session in New York time: Monday to Friday, 09:30 to 16:00, with the
//! clock changes of the United States (second Sunday of March to first Sunday of November).
//!
//! Holidays and early closes are not modelled. A holiday holds no bar, so nothing is built for it;
//! an early close (13:00) holds none after it, and its last bucket closes where the full session
//! would, 16:00.

use crate::format::{civil_from_days, days_from_civil};

const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 86_400_000;

/// The session opens this far into the New York day: 09:30.
pub const OPEN_MS: i64 = 9 * HOUR_MS + HOUR_MS / 2;
/// The session closes this far into the New York day: 16:00.
pub const CLOSE_MS: i64 = 16 * HOUR_MS;

/// UTC offset of America/New_York at `t_ms`, in milliseconds: -5 h, or -4 h during US daylight
/// time (02:00 local on the second Sunday of March to 02:00 on the first Sunday of November).
pub fn eastern_offset_ms(t_ms: i64) -> i64 {
    let (year, ..) = civil_from_days(t_ms.div_euclid(DAY_MS));
    // The changes in UTC: 02:00 EST is 07:00 UTC in spring, 02:00 EDT is 06:00 UTC in autumn.
    let spring = nth_sunday(year, 3, 2) * DAY_MS + 7 * HOUR_MS;
    let autumn = nth_sunday(year, 11, 1) * DAY_MS + 6 * HOUR_MS;
    if (spring..autumn).contains(&t_ms) {
        -4 * HOUR_MS
    } else {
        -5 * HOUR_MS
    }
}

/// Days since the epoch of the `nth` Sunday of a month.
fn nth_sunday(year: i64, month: i64, nth: i64) -> i64 {
    let first = days_from_civil(year, month, 1);
    // Sunday is weekday 6 with Monday = 0.
    let weekday = (first + 3).rem_euclid(7);
    first + (6 - weekday).rem_euclid(7) + (nth - 1) * 7
}

/// The New York calendar day (days since the epoch) that holds `t_ms`.
pub fn local_day(t_ms: i64) -> i64 {
    (t_ms + eastern_offset_ms(t_ms)).div_euclid(DAY_MS)
}

/// Whether the New York day `day` is a weekday. 1970-01-01 was a Thursday.
pub fn is_weekday(day: i64) -> bool {
    (day + 3).rem_euclid(7) < 5
}

/// The instant the session of New York day `day` opens (09:30), in UTC milliseconds. A weekend
/// has no session, but the instant is as defined for any day.
pub fn open_ms(day: i64) -> i64 {
    local_instant(day, OPEN_MS)
}

/// The instant the session of New York day `day` closes (16:00), in UTC milliseconds.
pub fn close_ms(day: i64) -> i64 {
    local_instant(day, CLOSE_MS)
}

/// Whether the regular session is open at `t_ms`.
pub fn is_open(t_ms: i64) -> bool {
    let day = local_day(t_ms);
    is_weekday(day) && (open_ms(day)..close_ms(day)).contains(&t_ms)
}

/// UTC milliseconds of the New York wall-clock time `ms_of_day` on `day`. The offset is the one in
/// force at noon that day: a clock change happens at 02:00, never during the session.
fn local_instant(day: i64, ms_of_day: i64) -> i64 {
    let noon_utc = day * DAY_MS + 17 * HOUR_MS;
    day * DAY_MS + ms_of_day - eastern_offset_ms(noon_utc)
}

#[cfg(test)]
mod tests;
