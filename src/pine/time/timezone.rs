//! Time zones for `hour(time, tz)`, `time(tf, session, tz)`, `timestamp(tz, …)` and
//! `str.format_time`. The app carries no tz database, so this is a table of the zones scripts
//! actually name (the IANA ids of the major exchanges' countries) with the daylight-saving rules
//! those regions follow today. Fixed offsets (`UTC+3`, `GMT-05:30`) always work; a name outside
//! the table is *unknown*, which callers read as UTC, and `ignored_features` warns about such a
//! name wherever it appears as a string literal.
//!
//! Rules: United States and Canada (second Sunday of March to first Sunday of November since 2007,
//! first Sunday of April to last Sunday of October from 1987), the European Union (last Sunday of
//! March to last Sunday of October, 01:00 UTC), Australia's south-east and New Zealand (southern
//! summer). History before those rules is not modelled.

use super::calendar;

const HOUR: i64 = 3_600;
const DAY_MS: i64 = 86_400_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Rule {
    None,
    UnitedStates,
    Europe,
    Australia,
    NewZealand,
}

/// A resolved zone: its standard offset east of UTC in seconds and its daylight-saving rule.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Zone {
    standard: i64,
    rule: Rule,
}

/// `(name, standard offset in seconds, rule)`. Names compare case-insensitively.
const ZONES: &[(&str, i64, Rule)] = &[
    ("UTC", 0, Rule::None),
    ("GMT", 0, Rule::None),
    ("Z", 0, Rule::None),
    ("Etc/UTC", 0, Rule::None),
    ("Etc/GMT", 0, Rule::None),
    ("Etc/Universal", 0, Rule::None),
    ("Etc/Zulu", 0, Rule::None),
    ("America/New_York", -5 * HOUR, Rule::UnitedStates),
    ("US/Eastern", -5 * HOUR, Rule::UnitedStates),
    ("America/Detroit", -5 * HOUR, Rule::UnitedStates),
    ("America/Toronto", -5 * HOUR, Rule::UnitedStates),
    ("America/Chicago", -6 * HOUR, Rule::UnitedStates),
    ("US/Central", -6 * HOUR, Rule::UnitedStates),
    ("America/Winnipeg", -6 * HOUR, Rule::UnitedStates),
    ("America/Denver", -7 * HOUR, Rule::UnitedStates),
    ("US/Mountain", -7 * HOUR, Rule::UnitedStates),
    ("America/Edmonton", -7 * HOUR, Rule::UnitedStates),
    ("America/Phoenix", -7 * HOUR, Rule::None),
    ("America/Los_Angeles", -8 * HOUR, Rule::UnitedStates),
    ("US/Pacific", -8 * HOUR, Rule::UnitedStates),
    ("America/Vancouver", -8 * HOUR, Rule::UnitedStates),
    ("America/Anchorage", -9 * HOUR, Rule::UnitedStates),
    ("Pacific/Honolulu", -10 * HOUR, Rule::None),
    ("America/Halifax", -4 * HOUR, Rule::UnitedStates),
    ("America/St_Johns", -3 * HOUR - 1_800, Rule::UnitedStates),
    ("America/Mexico_City", -6 * HOUR, Rule::None),
    ("America/Bogota", -5 * HOUR, Rule::None),
    ("America/Lima", -5 * HOUR, Rule::None),
    ("America/Caracas", -4 * HOUR, Rule::None),
    ("America/Sao_Paulo", -3 * HOUR, Rule::None),
    ("America/Argentina/Buenos_Aires", -3 * HOUR, Rule::None),
    ("America/Buenos_Aires", -3 * HOUR, Rule::None),
    ("Europe/London", 0, Rule::Europe),
    ("Europe/Dublin", 0, Rule::Europe),
    ("Europe/Lisbon", 0, Rule::Europe),
    ("Atlantic/Reykjavik", 0, Rule::None),
    ("Europe/Paris", HOUR, Rule::Europe),
    ("Europe/Berlin", HOUR, Rule::Europe),
    ("Europe/Madrid", HOUR, Rule::Europe),
    ("Europe/Rome", HOUR, Rule::Europe),
    ("Europe/Amsterdam", HOUR, Rule::Europe),
    ("Europe/Brussels", HOUR, Rule::Europe),
    ("Europe/Vienna", HOUR, Rule::Europe),
    ("Europe/Zurich", HOUR, Rule::Europe),
    ("Europe/Stockholm", HOUR, Rule::Europe),
    ("Europe/Oslo", HOUR, Rule::Europe),
    ("Europe/Copenhagen", HOUR, Rule::Europe),
    ("Europe/Warsaw", HOUR, Rule::Europe),
    ("Europe/Prague", HOUR, Rule::Europe),
    ("Europe/Budapest", HOUR, Rule::Europe),
    ("Europe/Luxembourg", HOUR, Rule::Europe),
    ("Europe/Athens", 2 * HOUR, Rule::Europe),
    ("Europe/Helsinki", 2 * HOUR, Rule::Europe),
    ("Europe/Kiev", 2 * HOUR, Rule::Europe),
    ("Europe/Kyiv", 2 * HOUR, Rule::Europe),
    ("Europe/Bucharest", 2 * HOUR, Rule::Europe),
    ("Europe/Sofia", 2 * HOUR, Rule::Europe),
    ("Europe/Istanbul", 3 * HOUR, Rule::None),
    ("Europe/Moscow", 3 * HOUR, Rule::None),
    ("Europe/Minsk", 3 * HOUR, Rule::None),
    ("Africa/Johannesburg", 2 * HOUR, Rule::None),
    ("Africa/Lagos", HOUR, Rule::None),
    ("Africa/Nairobi", 3 * HOUR, Rule::None),
    ("Asia/Riyadh", 3 * HOUR, Rule::None),
    ("Asia/Qatar", 3 * HOUR, Rule::None),
    ("Asia/Tehran", 3 * HOUR + 1_800, Rule::None),
    ("Asia/Dubai", 4 * HOUR, Rule::None),
    ("Asia/Karachi", 5 * HOUR, Rule::None),
    ("Asia/Kolkata", 5 * HOUR + 1_800, Rule::None),
    ("Asia/Calcutta", 5 * HOUR + 1_800, Rule::None),
    ("Asia/Dhaka", 6 * HOUR, Rule::None),
    ("Asia/Bangkok", 7 * HOUR, Rule::None),
    ("Asia/Jakarta", 7 * HOUR, Rule::None),
    ("Asia/Ho_Chi_Minh", 7 * HOUR, Rule::None),
    ("Asia/Shanghai", 8 * HOUR, Rule::None),
    ("Asia/Hong_Kong", 8 * HOUR, Rule::None),
    ("Asia/Singapore", 8 * HOUR, Rule::None),
    ("Asia/Taipei", 8 * HOUR, Rule::None),
    ("Asia/Manila", 8 * HOUR, Rule::None),
    ("Asia/Kuala_Lumpur", 8 * HOUR, Rule::None),
    ("Australia/Perth", 8 * HOUR, Rule::None),
    ("Asia/Tokyo", 9 * HOUR, Rule::None),
    ("Asia/Seoul", 9 * HOUR, Rule::None),
    ("Australia/Darwin", 9 * HOUR + 1_800, Rule::None),
    ("Australia/Adelaide", 9 * HOUR + 1_800, Rule::Australia),
    ("Australia/Brisbane", 10 * HOUR, Rule::None),
    ("Australia/Sydney", 10 * HOUR, Rule::Australia),
    ("Australia/Melbourne", 10 * HOUR, Rule::Australia),
    ("Australia/Canberra", 10 * HOUR, Rule::Australia),
    ("Australia/Hobart", 10 * HOUR, Rule::Australia),
    ("Pacific/Auckland", 12 * HOUR, Rule::NewZealand),
    ("Pacific/Fiji", 12 * HOUR, Rule::None),
];

/// The zone a name stands for: a table entry, or a fixed offset (`UTC+3`, `GMT-05:30`).
pub fn resolve(name: &str) -> Option<Zone> {
    let name = name.trim();
    if let Some(&(_, standard, rule)) = ZONES
        .iter()
        .find(|(candidate, _, _)| candidate.eq_ignore_ascii_case(name))
    {
        return Some(Zone { standard, rule });
    }
    fixed_offset(name).map(|standard| Zone {
        standard,
        rule: Rule::None,
    })
}

/// `UTC+3`, `GMT-05:30`, `utc - 5`: seconds east of UTC.
fn fixed_offset(name: &str) -> Option<i64> {
    let compact: String = name.to_uppercase().chars().filter(|c| *c != ' ').collect();
    for prefix in ["UTC", "GMT"] {
        let Some(offset) = compact.strip_prefix(prefix) else {
            continue;
        };
        let mut chars = offset.chars();
        let sign = match chars.next() {
            Some('+') => 1,
            Some('-') => -1,
            _ => return None,
        };
        let digits: Vec<&str> = chars.as_str().split(':').collect();
        let hours = digits.first()?.parse::<i64>().ok()?;
        let minutes = digits
            .get(1)
            .and_then(|m| m.parse::<i64>().ok())
            .unwrap_or(0);
        let seconds = sign * (hours * 3600 + minutes * 60);
        return (seconds.abs() <= 18 * 3600).then_some(seconds);
    }
    None
}

/// Whether `text` is spelled like an IANA id (`Region/City`) — what scripts pass that the table
/// may not know.
pub fn looks_like_iana(text: &str) -> bool {
    let mut parts = text.split('/');
    let (Some(region), Some(city)) = (parts.next(), parts.next()) else {
        return false;
    };
    const REGIONS: [&str; 10] = [
        "Africa",
        "America",
        "Antarctica",
        "Asia",
        "Atlantic",
        "Australia",
        "Europe",
        "Indian",
        "Pacific",
        "Etc",
    ];
    REGIONS.contains(&region)
        && !city.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '+' | '-'))
}

impl Zone {
    /// Seconds east of UTC at the UTC instant `utc_ms`.
    pub fn offset_at(&self, utc_ms: i64) -> i64 {
        if self.rule != Rule::None && self.in_daylight_time(utc_ms) {
            self.standard + HOUR
        } else {
            self.standard
        }
    }

    /// Seconds east of UTC for a wall-clock reading `local_ms` (the local time written as if it
    /// were UTC). In the hour a spring-forward skips, the standard offset applies; in the hour a
    /// fall-back repeats, the first (daylight) reading.
    pub fn offset_for_local(&self, local_ms: i64) -> i64 {
        let daylight = self.standard + HOUR;
        if self.rule != Rule::None && self.in_daylight_time(local_ms - daylight * 1_000) {
            daylight
        } else {
            self.standard
        }
    }

    fn in_daylight_time(&self, utc_ms: i64) -> bool {
        let year = calendar::components(utc_ms + self.standard * 1_000).year;
        let (start, end) = self.transitions(year);
        match self.rule {
            // Southern hemisphere: daylight time spans the new year.
            Rule::Australia | Rule::NewZealand => utc_ms >= start || utc_ms < end,
            _ => utc_ms >= start && utc_ms < end,
        }
    }

    /// The UTC instants daylight time begins and ends in `year` (in the southern hemisphere the
    /// end falls earlier in the year than the start).
    fn transitions(&self, year: i64) -> (i64, i64) {
        let at = |month: i64, day: i64, seconds_utc: i64| {
            calendar::make(year, month, day, 0, 0, 0, 0).unwrap_or(0) + seconds_utc * 1_000
        };
        let s = self.standard;
        match self.rule {
            Rule::UnitedStates => {
                let (start_day, end_day, start_month, end_month) = if year >= 2007 {
                    (nth_sunday(year, 3, 2), nth_sunday(year, 11, 1), 3, 11)
                } else {
                    (nth_sunday(year, 4, 1), last_sunday(year, 10), 4, 10)
                };
                // 02:00 local on both days: standard time at the start, daylight time at the end.
                (
                    at(start_month, start_day, 2 * HOUR - s),
                    at(end_month, end_day, 2 * HOUR - (s + HOUR)),
                )
            }
            Rule::Europe => (
                at(3, last_sunday(year, 3), HOUR),
                at(10, last_sunday(year, 10), HOUR),
            ),
            Rule::Australia => (
                at(10, nth_sunday(year, 10, 1), 2 * HOUR - s),
                at(4, nth_sunday(year, 4, 1), 2 * HOUR - s),
            ),
            Rule::NewZealand => (
                at(9, last_sunday(year, 9), 2 * HOUR - s),
                at(4, nth_sunday(year, 4, 1), 2 * HOUR - s),
            ),
            Rule::None => (0, 0),
        }
    }
}

/// The day of the month of the `n`th Sunday.
fn nth_sunday(year: i64, month: i64, n: i64) -> i64 {
    let first = calendar::make(year, month, 1, 0, 0, 0, 0).unwrap_or(0);
    // `weekday` is 1 for Sunday.
    let weekday = calendar::components(first).weekday;
    let first_sunday = 1 + (8 - weekday) % 7;
    first_sunday + (n - 1) * 7
}

fn last_sunday(year: i64, month: i64) -> i64 {
    let next_month = calendar::make(year, month + 1, 1, 0, 0, 0, 0).unwrap_or(0);
    let last = calendar::components(next_month - DAY_MS);
    last.day - (last.weekday - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(year: i64, month: i64, day: i64, hour: i64, minute: i64) -> i64 {
        calendar::make(year, month, day, hour, minute, 0, 0).unwrap()
    }

    fn offset(name: &str, at: i64) -> i64 {
        resolve(name).unwrap().offset_at(at) / HOUR
    }

    #[test]
    fn fixed_offsets_and_names_resolve() {
        assert_eq!(resolve("UTC+3").map(|z| z.offset_at(0)), Some(3 * HOUR));
        assert_eq!(resolve("utc - 5").map(|z| z.offset_at(0)), Some(-5 * HOUR));
        assert_eq!(
            resolve("GMT+05:30").map(|z| z.offset_at(0)),
            Some(5 * HOUR + 1_800)
        );
        assert_eq!(resolve("UTC+99"), None);
        assert_eq!(resolve("UTCX"), None);
        assert_eq!(resolve("Mars/Olympus"), None);
        assert_eq!(resolve("america/new_york"), resolve("America/New_York"));
    }

    #[test]
    fn the_united_states_changes_on_the_second_sunday_of_march_and_the_first_of_november() {
        // 2024: DST began Sunday March 10 at 07:00 UTC and ended Sunday November 3 at 06:00 UTC.
        assert_eq!(offset("America/New_York", utc(2024, 3, 10, 6, 59)), -5);
        assert_eq!(offset("America/New_York", utc(2024, 3, 10, 7, 0)), -4);
        assert_eq!(offset("America/New_York", utc(2024, 11, 3, 5, 59)), -4);
        assert_eq!(offset("America/New_York", utc(2024, 11, 3, 6, 0)), -5);
        assert_eq!(offset("America/Chicago", utc(2024, 7, 1, 12, 0)), -5);
        assert_eq!(offset("America/Los_Angeles", utc(2024, 1, 15, 12, 0)), -8);
        assert_eq!(offset("America/Phoenix", utc(2024, 7, 1, 12, 0)), -7);
        // Before 2007 the change was the first Sunday of April: 2006-03-20 was still winter.
        assert_eq!(offset("America/New_York", utc(2006, 3, 20, 12, 0)), -5);
        assert_eq!(offset("America/New_York", utc(2006, 4, 3, 12, 0)), -4);
    }

    #[test]
    fn europe_changes_at_one_utc_on_the_last_sundays_of_march_and_october() {
        // 2024: March 31 and October 27.
        assert_eq!(offset("Europe/London", utc(2024, 3, 31, 0, 59)), 0);
        assert_eq!(offset("Europe/London", utc(2024, 3, 31, 1, 0)), 1);
        assert_eq!(offset("Europe/Berlin", utc(2024, 10, 27, 0, 59)), 2);
        assert_eq!(offset("Europe/Berlin", utc(2024, 10, 27, 1, 0)), 1);
        assert_eq!(offset("Europe/Moscow", utc(2024, 7, 1, 0, 0)), 3);
    }

    #[test]
    fn the_southern_hemisphere_spans_the_new_year() {
        // Sydney: DST from the first Sunday of October to the first Sunday of April (2024-04-07).
        assert_eq!(offset("Australia/Sydney", utc(2024, 1, 15, 0, 0)), 11);
        assert_eq!(offset("Australia/Sydney", utc(2024, 7, 1, 0, 0)), 10);
        assert_eq!(offset("Australia/Sydney", utc(2024, 12, 1, 0, 0)), 11);
        assert_eq!(offset("Pacific/Auckland", utc(2024, 7, 1, 0, 0)), 12);
        assert_eq!(offset("Pacific/Auckland", utc(2024, 12, 1, 0, 0)), 13);
        assert_eq!(offset("Asia/Tokyo", utc(2024, 7, 1, 0, 0)), 9);
    }

    #[test]
    fn a_wall_clock_reading_maps_back_to_the_right_offset() {
        let new_york = resolve("America/New_York").unwrap();
        let local = |month, day, hour| calendar::make(2024, month, day, hour, 30, 0, 0).unwrap();
        // 09:30 on a winter and on a summer day.
        assert_eq!(new_york.offset_for_local(local(1, 15, 9)), -5 * HOUR);
        assert_eq!(new_york.offset_for_local(local(7, 15, 9)), -4 * HOUR);
        // The day DST starts: 09:30 is already summer time.
        assert_eq!(new_york.offset_for_local(local(3, 10, 9)), -4 * HOUR);
    }

    #[test]
    fn iana_looking_names_are_told_from_other_text() {
        assert!(looks_like_iana("America/New_York"));
        assert!(looks_like_iana("Etc/GMT+5"));
        assert!(!looks_like_iana("BINANCE:BTCUSDT"));
        assert!(!looks_like_iana("a/b"));
        assert!(!looks_like_iana("Asia"));
    }
}
