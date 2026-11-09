//! Periods, custom sessions and the TPO letter of a block of time.
//!
//! All boundaries are worked in a zone: `None` is UTC (a crypto market's day), otherwise a zone from
//! `pine::timezone` (daylight-saving aware, so a New York day is 23 or 25 hours twice a year). Nothing
//! here knows about holidays or early closes: the app carries no exchange calendar.
//!
//! A *period* is what a TPO profile covers (days, weeks from Monday, calendar months). A *session*
//! is a custom window `HHMM-HHMM[:days]`; overlapping windows are cut at their intersections so each
//! piece gets its own profile, as TradingView documents.

use crate::pine::timezone::Zone;
use serde::{Deserialize, Serialize};

pub const DAY_MS: i64 = 86_400_000;
const MINUTE_MS: i64 = 60_000;

/// The most local days a custom-session search will walk, so a bad range cannot loop for long.
const MAX_SEARCH_DAYS: i64 = 4_000;

/// A half-open span of time `[start_ms, end_ms)`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Session {
    pub start_ms: i64,
    pub end_ms: i64,
}

impl Session {
    pub fn contains(&self, time_ms: i64) -> bool {
        time_ms >= self.start_ms && time_ms < self.end_ms
    }

    pub fn length_ms(&self) -> i64 {
        self.end_ms - self.start_ms
    }
}

/// How long a profile's period is. Counts are at least one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Period {
    Days(u32),
    Weeks(u32),
    Months(u32),
}

impl Period {
    fn count(self) -> i64 {
        i64::from(match self {
            Self::Days(n) | Self::Weeks(n) | Self::Months(n) => n.max(1),
        })
    }
}

// MARK: Zones

fn offset_ms(zone: Option<Zone>, utc_ms: i64) -> i64 {
    zone.map_or(0, |zone| zone.offset_at(utc_ms) * 1_000)
}

/// The wall-clock reading of a UTC instant, written as if it were UTC.
fn to_local(zone: Option<Zone>, utc_ms: i64) -> i64 {
    utc_ms.saturating_add(offset_ms(zone, utc_ms))
}

/// The UTC instant of a wall-clock reading.
fn from_local(zone: Option<Zone>, local_ms: i64) -> i64 {
    let offset = zone.map_or(0, |zone| zone.offset_for_local(local_ms) * 1_000);
    local_ms.saturating_sub(offset)
}

// MARK: Calendar arithmetic (proleptic Gregorian, days since 1970-01-01)

/// The `(year, month 1-12)` of a day number.
fn civil_month(day: i64) -> (i64, i64) {
    let z = day + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (if month <= 2 { year + 1 } else { year }, month)
}

/// The day number of the first of a month.
pub fn first_of_month(year: i64, month: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year.rem_euclid(400);
    let mp = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * mp + 2) / 5;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `0` is Sunday ... `6` is Saturday (1970-01-01 was a Thursday).
fn weekday(day: i64) -> i64 {
    (day + 4).rem_euclid(7)
}

// MARK: Periods

/// The period `time_ms` falls in: whole days from the epoch in groups of `n`, weeks (Monday to
/// Sunday) in groups of `n`, or calendar months in groups of `n`, all in `zone` (`None` is UTC).
pub fn period_containing(period: Period, time_ms: i64, zone: Option<Zone>) -> Session {
    let n = period.count();
    let day = to_local(zone, time_ms).div_euclid(DAY_MS);
    let (first_day, end_day) = match period {
        Period::Days(_) => {
            let group = day.div_euclid(n) * n;
            (group, group + n)
        }
        Period::Weeks(_) => {
            // Day 4 (1970-01-05) is a Monday, so (day + 3) / 7 numbers the weeks from Monday.
            let week = (day + 3).div_euclid(7);
            let group = week.div_euclid(n) * n;
            (group * 7 - 3, (group + n) * 7 - 3)
        }
        Period::Months(_) => {
            let (year, month) = civil_month(day);
            let index = year * 12 + (month - 1);
            let group = index.div_euclid(n) * n;
            let month_start =
                |index: i64| first_of_month(index.div_euclid(12), index.rem_euclid(12) + 1);
            (month_start(group), month_start(group + n))
        }
    };
    Session {
        start_ms: from_local(zone, first_day * DAY_MS),
        end_ms: from_local(zone, end_day * DAY_MS),
    }
}

// MARK: Custom sessions

/// One window of a custom session: `HHMM-HHMM` optionally followed by `:` and the days it runs on
/// (digits 1 to 7, Sunday is 1: TradingView's numbering).
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct SessionSpan {
    /// Minutes after local midnight the window opens (0..1440).
    pub start_min: u16,
    /// Minutes after local midnight it closes (0..=1440); not after `start_min` means overnight.
    pub end_min: u16,
    /// Bit `d` is set when the window opens on weekday `d` (0 Sunday .. 6 Saturday).
    pub days: u8,
}

impl SessionSpan {
    pub const ALL_DAYS: u8 = 0b0111_1111;

    /// `None` unless the text is a valid window.
    pub fn parse(text: &str) -> Option<Self> {
        let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        let mut parts = compact.split(':');
        let window = parts.next()?;
        let days_text = parts.next();
        if parts.next().is_some() {
            return None;
        }
        let (start, end) = window.split_once('-')?;
        let (start_min, end_min) = (minute_of_day(start)?, minute_of_day(end)?);
        if start_min >= 1_440 {
            return None;
        }
        let days = match days_text {
            None => Self::ALL_DAYS,
            Some(digits) if !digits.is_empty() => {
                let mut mask = 0u8;
                for c in digits.chars() {
                    let d = c.to_digit(10).filter(|d| (1..=7).contains(d))?;
                    mask |= 1 << (d - 1);
                }
                mask
            }
            Some(_) => return None,
        };
        Some(Self {
            start_min,
            end_min,
            days,
        })
    }

    /// How long the window is, in minutes: an overnight window wraps, and equal ends are a day.
    pub fn length_min(&self) -> i64 {
        let length = (i64::from(self.end_min) - i64::from(self.start_min)).rem_euclid(1_440);
        if length == 0 { 1_440 } else { length }
    }

    /// The window as `HHMM-HHMM`, with `:days` unless it runs every day (what [`Self::parse`] reads).
    pub fn to_text(self) -> String {
        let clock = |minutes: u16| format!("{:02}{:02}", minutes / 60, minutes % 60);
        let mut text = format!("{}-{}", clock(self.start_min), clock(self.end_min));
        if self.days != Self::ALL_DAYS {
            text.push(':');
            for day in 0..7u8 {
                if self.days & (1 << day) != 0 {
                    text.push(char::from(b'1' + day));
                }
            }
        }
        text
    }

    fn runs_on(&self, weekday: i64) -> bool {
        self.days & (1 << weekday) != 0
    }
}

fn minute_of_day(text: &str) -> Option<u16> {
    if text.len() != 4 || !text.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let value: u16 = text.parse().ok()?;
    let (hour, minute) = (value / 100, value % 100);
    (minute < 60 && (hour < 24 || (hour == 24 && minute == 0))).then_some(hour * 60 + minute)
}

/// The sessions of `spans` that overlap `[from_ms, to_ms)`, oldest first. Overlapping windows are
/// cut at their intersections: every stretch some window covers is one session. Windows are cut to
/// the stretches they cover, never merged with a neighbour that merely touches them.
pub fn custom_sessions(
    spans: &[SessionSpan],
    zone: Option<Zone>,
    from_ms: i64,
    to_ms: i64,
) -> Vec<Session> {
    if spans.is_empty() || to_ms <= from_ms {
        return Vec::new();
    }
    // Start a day early: an overnight window that opened the evening before.
    let first_day = to_local(zone, from_ms).div_euclid(DAY_MS) - 1;
    let last_day = to_local(zone, to_ms).div_euclid(DAY_MS) + 1;
    if last_day - first_day > MAX_SEARCH_DAYS {
        return Vec::new();
    }
    let mut raw: Vec<Session> = Vec::new();
    for day in first_day..=last_day {
        for span in spans.iter().filter(|span| span.runs_on(weekday(day))) {
            let start_local = day * DAY_MS + i64::from(span.start_min) * MINUTE_MS;
            let end_local = start_local + span.length_min() * MINUTE_MS;
            raw.push(Session {
                start_ms: from_local(zone, start_local),
                end_ms: from_local(zone, end_local),
            });
        }
    }
    let mut edges: Vec<i64> = raw
        .iter()
        .flat_map(|session| [session.start_ms, session.end_ms])
        .collect();
    edges.sort_unstable();
    edges.dedup();
    edges
        .windows(2)
        .filter(|pair| {
            raw.iter()
                .any(|s| s.start_ms <= pair[0] && pair[1] <= s.end_ms)
        })
        .map(|pair| Session {
            start_ms: pair[0],
            end_ms: pair[1],
        })
        .filter(|session| session.end_ms > from_ms && session.start_ms < to_ms)
        .collect()
}

/// The custom session `time_ms` is in, `None` between sessions.
pub fn custom_session_containing(
    spans: &[SessionSpan],
    zone: Option<Zone>,
    time_ms: i64,
) -> Option<Session> {
    custom_sessions(spans, zone, time_ms - 2 * DAY_MS, time_ms + 2 * DAY_MS)
        .into_iter()
        .find(|session| session.contains(time_ms))
}

// MARK: Blocks and letters

/// The block sizes TradingView offers for a TPO profile, in minutes. 30 is the default.
pub const BLOCK_MINUTES: [u32; 7] = [5, 10, 15, 30, 60, 120, 240];

/// Which block of a profile `time_ms` falls in: whole `block_ms` from the profile's start. `None`
/// before the profile starts or for a block that is not positive.
pub fn block_index(profile_start_ms: i64, time_ms: i64, block_ms: i64) -> Option<u32> {
    if block_ms <= 0 || time_ms < profile_start_ms {
        return None;
    }
    u32::try_from((time_ms - profile_start_ms) / block_ms).ok()
}

/// The TPO letter of a block: A to Z, then a to z, then the sequence repeats.
pub fn letter(block: u32) -> char {
    let n = block % 52;
    if n < 26 {
        (b'A' + n as u8) as char
    } else {
        (b'a' + (n - 26) as u8) as char
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::timezone;

    const HOUR: i64 = 3_600_000;

    fn day(year: i64, month: i64, d: i64) -> i64 {
        first_of_month(year, month) + d - 1
    }

    fn ms(year: i64, month: i64, d: i64, hour: i64, minute: i64) -> i64 {
        day(year, month, d) * DAY_MS + hour * HOUR + minute * MINUTE_MS
    }

    fn new_york() -> Option<Zone> {
        timezone::resolve("America/New_York")
    }

    // MARK: Calendar

    #[test]
    fn civil_arithmetic_round_trips_and_knows_the_weekday() {
        assert_eq!(first_of_month(1970, 1), 0);
        assert_eq!(first_of_month(2000, 3), 11_017);
        assert_eq!(civil_month(11_017), (2000, 3));
        assert_eq!(civil_month(11_016), (2000, 2));
        assert_eq!(civil_month(day(2024, 12, 31)), (2024, 12));
        assert_eq!(civil_month(day(2025, 1, 1)), (2025, 1));
        assert_eq!(weekday(0), 4); // Thursday
        assert_eq!(weekday(day(2024, 3, 10)), 0); // Sunday
        assert_eq!(weekday(day(2024, 3, 11)), 1); // Monday
    }

    // MARK: Periods

    #[test]
    fn a_utc_day_is_midnight_to_midnight() {
        let t = ms(2024, 5, 17, 13, 45);
        let s = period_containing(Period::Days(1), t, None);
        assert_eq!(s.start_ms, ms(2024, 5, 17, 0, 0));
        assert_eq!(s.end_ms, ms(2024, 5, 18, 0, 0));
        assert!(s.contains(t));
    }

    #[test]
    fn a_new_york_day_is_23_hours_in_march_and_25_in_november() {
        let spring = period_containing(Period::Days(1), ms(2024, 3, 10, 12, 0), new_york());
        assert_eq!(spring.length_ms(), 23 * HOUR);
        assert_eq!(spring.start_ms, ms(2024, 3, 10, 5, 0)); // 00:00 EST = 05:00 UTC
        let fall = period_containing(Period::Days(1), ms(2024, 11, 3, 12, 0), new_york());
        assert_eq!(fall.length_ms(), 25 * HOUR);
        assert_eq!(fall.start_ms, ms(2024, 11, 3, 4, 0)); // 00:00 EDT = 04:00 UTC
        let ordinary = period_containing(Period::Days(1), ms(2024, 6, 12, 12, 0), new_york());
        assert_eq!(ordinary.length_ms(), 24 * HOUR);
    }

    #[test]
    fn a_week_runs_monday_to_sunday() {
        // 2024-05-15 is a Wednesday.
        let s = period_containing(Period::Weeks(1), ms(2024, 5, 15, 9, 0), None);
        assert_eq!(s.start_ms, ms(2024, 5, 13, 0, 0));
        assert_eq!(s.end_ms, ms(2024, 5, 20, 0, 0));
        // Monday 00:00 itself starts a new week; Sunday 23:59 is still the old one.
        assert_eq!(
            period_containing(Period::Weeks(1), ms(2024, 5, 13, 0, 0), None).start_ms,
            ms(2024, 5, 13, 0, 0)
        );
        assert_eq!(
            period_containing(Period::Weeks(1), ms(2024, 5, 12, 23, 59), None).start_ms,
            ms(2024, 5, 6, 0, 0)
        );
    }

    #[test]
    fn months_follow_the_calendar_including_leap_february_and_the_new_year() {
        let feb = period_containing(Period::Months(1), ms(2024, 2, 10, 0, 0), None);
        assert_eq!(feb.start_ms, ms(2024, 2, 1, 0, 0));
        assert_eq!(feb.end_ms, ms(2024, 3, 1, 0, 0));
        assert_eq!(feb.length_ms(), 29 * DAY_MS);
        let dec = period_containing(Period::Months(1), ms(2024, 12, 31, 23, 0), None);
        assert_eq!(dec.end_ms, ms(2025, 1, 1, 0, 0));
        let quarter = period_containing(Period::Months(3), ms(2024, 5, 1, 0, 0), None);
        assert_eq!(quarter.start_ms, ms(2024, 4, 1, 0, 0));
        assert_eq!(quarter.end_ms, ms(2024, 7, 1, 0, 0));
    }

    #[test]
    fn several_days_group_from_the_epoch_and_periods_tile_without_gaps() {
        let a = period_containing(Period::Days(3), ms(2024, 5, 17, 0, 0), None);
        assert_eq!(a.length_ms(), 3 * DAY_MS);
        let b = period_containing(Period::Days(3), a.end_ms, None);
        assert_eq!(b.start_ms, a.end_ms);
        let before = period_containing(Period::Days(3), a.start_ms - 1, None);
        assert_eq!(before.end_ms, a.start_ms);
        // A zero count is read as one.
        assert_eq!(
            period_containing(Period::Days(0), a.start_ms, None).length_ms(),
            DAY_MS
        );
    }

    #[test]
    fn times_before_the_epoch_still_resolve() {
        let s = period_containing(Period::Days(1), -1, None);
        assert_eq!((s.start_ms, s.end_ms), (-DAY_MS, 0));
        assert_eq!(period_containing(Period::Months(1), -1, None).end_ms, 0);
    }

    // MARK: Custom sessions

    #[test]
    fn a_window_parses_with_and_without_days() {
        let s = SessionSpan::parse("0930-1600").unwrap();
        assert_eq!(
            (s.start_min, s.end_min, s.days),
            (570, 960, SessionSpan::ALL_DAYS)
        );
        assert_eq!(s.length_min(), 390);
        let weekdays = SessionSpan::parse("0930-1600:23456").unwrap();
        assert_eq!(weekdays.days, 0b0011_1110);
        assert_eq!(SessionSpan::parse("2200-0600").unwrap().length_min(), 480);
        assert_eq!(SessionSpan::parse("0000-0000").unwrap().length_min(), 1_440);
        assert_eq!(SessionSpan::parse("0000-2400").unwrap().length_min(), 1_440);
    }

    #[test]
    fn a_window_reads_back_as_the_text_it_came_from() {
        for text in [
            "0930-1600",
            "0930-1600:23456",
            "2200-0600",
            "0000-2400",
            "0000-0000:17",
        ] {
            assert_eq!(SessionSpan::parse(text).unwrap().to_text(), text);
        }
    }

    #[test]
    fn a_bad_window_does_not_parse() {
        for text in [
            "",
            "0930",
            "09301600",
            "0930-",
            "0960-1000",
            "2500-0100",
            "2400-0100",
            "0930-1600:",
            "0930-1600:8",
            "0930-1600:0",
            "0930-1600:1:2",
            "abcd-efgh",
        ] {
            assert!(SessionSpan::parse(text).is_none(), "{text:?}");
        }
    }

    #[test]
    fn a_daily_window_is_one_session_per_day_it_runs() {
        let span = [SessionSpan::parse("0930-1600:23456").unwrap()];
        // 2024-05-10 Fri .. 2024-05-14 Tue; the weekend has none.
        let sessions = custom_sessions(&span, None, ms(2024, 5, 10, 0, 0), ms(2024, 5, 15, 0, 0));
        let starts: Vec<i64> = sessions.iter().map(|s| s.start_ms).collect();
        assert_eq!(
            starts,
            [
                ms(2024, 5, 10, 9, 30),
                ms(2024, 5, 13, 9, 30),
                ms(2024, 5, 14, 9, 30)
            ]
        );
        assert!(sessions.iter().all(|s| s.length_ms() == 390 * MINUTE_MS));
    }

    #[test]
    fn an_overnight_window_belongs_to_the_day_it_opened_on() {
        // Friday-only 22:00-06:00 runs into Saturday morning.
        let span = [SessionSpan::parse("2200-0600:6").unwrap()];
        let found = custom_session_containing(&span, None, ms(2024, 5, 11, 3, 0)).unwrap();
        assert_eq!(found.start_ms, ms(2024, 5, 10, 22, 0));
        assert_eq!(found.end_ms, ms(2024, 5, 11, 6, 0));
        assert!(custom_session_containing(&span, None, ms(2024, 5, 11, 12, 0)).is_none());
    }

    #[test]
    fn a_session_search_that_starts_mid_overnight_window_still_finds_it() {
        let span = [SessionSpan::parse("2200-0600").unwrap()];
        let sessions = custom_sessions(&span, None, ms(2024, 5, 11, 1, 0), ms(2024, 5, 11, 2, 0));
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].start_ms, ms(2024, 5, 10, 22, 0));
    }

    #[test]
    fn overlapping_windows_are_cut_at_their_intersections() {
        let spans = [
            SessionSpan::parse("0900-1100").unwrap(),
            SessionSpan::parse("1000-1200").unwrap(),
        ];
        let sessions = custom_sessions(&spans, None, ms(2024, 5, 10, 0, 0), ms(2024, 5, 11, 0, 0));
        let bounds: Vec<(i64, i64)> = sessions.iter().map(|s| (s.start_ms, s.end_ms)).collect();
        assert_eq!(
            bounds,
            [
                (ms(2024, 5, 10, 9, 0), ms(2024, 5, 10, 10, 0)),
                (ms(2024, 5, 10, 10, 0), ms(2024, 5, 10, 11, 0)),
                (ms(2024, 5, 10, 11, 0), ms(2024, 5, 10, 12, 0)),
            ]
        );
    }

    #[test]
    fn a_gap_between_windows_is_not_a_session() {
        let spans = [
            SessionSpan::parse("0900-1000").unwrap(),
            SessionSpan::parse("1300-1400").unwrap(),
        ];
        let sessions = custom_sessions(&spans, None, ms(2024, 5, 10, 0, 0), ms(2024, 5, 11, 0, 0));
        assert_eq!(sessions.len(), 2);
        assert!(custom_session_containing(&spans, None, ms(2024, 5, 10, 11, 0)).is_none());
    }

    #[test]
    fn custom_windows_follow_daylight_saving() {
        let span = [SessionSpan::parse("0930-1600").unwrap()];
        // 09:30 New York is 14:30 UTC in winter and 13:30 UTC in summer.
        let winter = custom_session_containing(&span, new_york(), ms(2024, 1, 10, 16, 0)).unwrap();
        assert_eq!(winter.start_ms, ms(2024, 1, 10, 14, 30));
        let summer = custom_session_containing(&span, new_york(), ms(2024, 7, 10, 16, 0)).unwrap();
        assert_eq!(summer.start_ms, ms(2024, 7, 10, 13, 30));
        // On the spring-forward Sunday itself the window is already on summer time.
        let change = custom_session_containing(&span, new_york(), ms(2024, 3, 10, 16, 0)).unwrap();
        assert_eq!(change.start_ms, ms(2024, 3, 10, 13, 30));
    }

    #[test]
    fn an_absurd_search_range_returns_nothing_instead_of_looping() {
        let span = [SessionSpan::parse("0930-1600").unwrap()];
        assert!(custom_sessions(&span, None, 0, 100_000 * DAY_MS).is_empty());
        assert!(custom_sessions(&[], None, 0, DAY_MS).is_empty());
        assert!(custom_sessions(&span, None, DAY_MS, 0).is_empty());
    }

    // MARK: Blocks and letters

    #[test]
    fn letters_run_a_to_z_then_a_to_z_lowercase_then_repeat() {
        assert_eq!(letter(0), 'A');
        assert_eq!(letter(25), 'Z');
        assert_eq!(letter(26), 'a');
        assert_eq!(letter(51), 'z');
        assert_eq!(letter(52), 'A');
        assert_eq!(letter(52 + 26), 'a');
        assert_eq!(letter(u32::MAX), letter(u32::MAX % 52));
    }

    #[test]
    fn blocks_count_from_the_start_of_the_profile() {
        let start = ms(2024, 5, 10, 0, 0);
        let block = 30 * MINUTE_MS;
        assert_eq!(block_index(start, start, block), Some(0));
        assert_eq!(block_index(start, start + block - 1, block), Some(0));
        assert_eq!(block_index(start, start + block, block), Some(1));
        assert_eq!(block_index(start, start + 47 * block + 5, block), Some(47));
        assert_eq!(block_index(start, start - 1, block), None);
        assert_eq!(block_index(start, start, 0), None);
    }

    #[test]
    fn the_offered_block_sizes_include_the_default() {
        assert!(BLOCK_MINUTES.contains(&30));
        assert!(BLOCK_MINUTES.windows(2).all(|w| w[0] < w[1]));
    }
}
