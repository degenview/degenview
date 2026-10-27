//! Calendar parts of a bar time, `timeframe.*` derived from the spacing between bars, timeframe
//! strings, and `str.format_time`. A port of `Builtins/PineTime.swift`.
//!
//! Time zones are UTC, fixed offsets (`UTC+3`, `GMT-05:30`) and the named zones of
//! [`timezone`](super::timezone), which carry their daylight-saving rules; any other name reads as UTC.

use super::calendar::{self, Components};
use super::runtime_value::RuntimeValue;

pub const PART_NAMES: [&str; 8] = [
    "year",
    "month",
    "dayofmonth",
    "hour",
    "minute",
    "second",
    "dayofweek",
    "weekofyear",
];

/// Every `timeframe.*` name `timeframe` resolves.
pub const TIMEFRAME_NAMES: [&str; 11] = [
    "timeframe.period",
    "timeframe.main_period",
    "timeframe.multiplier",
    "timeframe.isdaily",
    "timeframe.isweekly",
    "timeframe.ismonthly",
    "timeframe.isseconds",
    "timeframe.isminutes",
    "timeframe.isintraday",
    "timeframe.isdwm",
    "timeframe.isticks",
];

const DAY: f64 = 86_400.0;

/// The calendar fields of `stamp` shifted by `offset_secs`, UTC when zero.
pub fn components(stamp: i64, offset_secs: i64) -> Components {
    calendar::components(stamp + offset_secs * 1_000)
}

pub fn part(name: &str, stamp: i64, offset_secs: i64) -> RuntimeValue {
    let c = components(stamp, offset_secs);
    RuntimeValue::Int(match name {
        "year" => c.year,
        "month" => c.month,
        "dayofmonth" => c.day,
        "hour" => c.hour,
        "minute" => c.minute,
        "second" => c.second,
        "weekofyear" => calendar::week_of_year(&c),
        _ => c.weekday,
    })
}

/// The bar length as a Pine multiplier and unit suffix (`"5"`, `"4"`+`""`, `"1D"`, `"30S"`).
fn period(seconds: f64) -> (i64, &'static str) {
    let rounded = |unit: f64| ((seconds / unit).round() as i64).max(1);
    if seconds >= 28.0 * DAY {
        (rounded(30.0 * DAY), "M")
    } else if seconds >= 7.0 * DAY {
        (rounded(7.0 * DAY), "W")
    } else if seconds >= DAY {
        (rounded(DAY), "D")
    } else if seconds < 60.0 {
        (seconds as i64, "S")
    } else {
        ((seconds / 60.0).round() as i64, "")
    }
}

/// Length in seconds of a timeframe string: `"30S"`, `"5"` and `"60"` (minutes), `"1D"`, `"2W"`,
/// `"3M"`, or a bare unit such as `"D"`. A month counts as 30 days, as `period` does. None when
/// malformed.
pub fn seconds_of_timeframe(text: &str) -> Option<f64> {
    let digits: String = text.chars().take_while(char::is_ascii_digit).collect();
    let unit = text[digits.len()..].to_uppercase();
    let multiplier: f64 = if digits.is_empty() {
        if unit.is_empty() { return None } else { 1.0 }
    } else {
        digits.parse().ok()?
    };
    if multiplier <= 0.0 {
        return None;
    }
    match unit.as_str() {
        "" => Some(multiplier * 60.0),
        "S" => Some(multiplier),
        "D" => Some(multiplier * DAY),
        "W" => Some(multiplier * 7.0 * DAY),
        "M" => Some(multiplier * 30.0 * DAY),
        _ => None,
    }
}

/// The timeframe string for a length in seconds, the inverse of `seconds_of_timeframe`: `60` →
/// `"1"`, `7200` → `"120"`, `86_400` → `"1D"`, `45` → `"45S"`. The largest unit that divides
/// evenly wins, with a month counted as 30 days. None for a length that is not positive.
pub fn timeframe_from_seconds(seconds: i64) -> Option<String> {
    if seconds <= 0 {
        return None;
    }
    let day = DAY as i64;
    for (length, suffix) in [(30 * day, "M"), (7 * day, "W"), (day, "D")] {
        if seconds % length == 0 {
            return Some(format!("{}{suffix}", seconds / length));
        }
    }
    Some(if seconds % 60 == 0 {
        (seconds / 60).to_string()
    } else {
        format!("{seconds}S")
    })
}

/// `timeframe.*` for bars `seconds` apart; None until the spacing is known.
pub fn timeframe(name: &str, bar_seconds: f64) -> Option<RuntimeValue> {
    if bar_seconds <= 0.0 {
        return None;
    }
    let seconds = bar_seconds;
    let is_monthly = seconds >= 28.0 * DAY;
    let is_weekly = !is_monthly && seconds >= 7.0 * DAY;
    let is_daily = !is_weekly && !is_monthly && seconds >= DAY;
    Some(match name {
        "timeframe.period" | "timeframe.main_period" => {
            let (multiplier, suffix) = period(seconds);
            RuntimeValue::String(format!("{multiplier}{suffix}"))
        }
        "timeframe.multiplier" => RuntimeValue::Int(period(seconds).0),
        "timeframe.isdaily" => RuntimeValue::Bool(is_daily),
        "timeframe.isweekly" => RuntimeValue::Bool(is_weekly),
        "timeframe.ismonthly" => RuntimeValue::Bool(is_monthly),
        "timeframe.isseconds" => RuntimeValue::Bool(seconds < 60.0),
        "timeframe.isminutes" => RuntimeValue::Bool((60.0..DAY).contains(&seconds)),
        "timeframe.isintraday" => RuntimeValue::Bool(seconds < DAY),
        "timeframe.isdwm" => RuntimeValue::Bool(seconds >= DAY),
        // Bars here are always time based.
        "timeframe.isticks" => RuntimeValue::Bool(false),
        _ => return None,
    })
}

/// `timeframe.change(timeframe)`: whether `now_ms` opens a new bar of a `tf_seconds`-long
/// timeframe compared with the previous chart bar (`None` for the very first bar, which is always
/// a change). Months follow the calendar, weeks start on Monday, the rest are fixed lengths.
pub fn timeframe_changed(previous_ms: Option<i64>, now_ms: i64, tf_seconds: f64) -> bool {
    let Some(previous_ms) = previous_ms else {
        return true;
    };
    let bucket = |ms: i64| -> i64 {
        if tf_seconds >= 28.0 * DAY && (tf_seconds / (30.0 * DAY)).fract().abs() < 1e-9 {
            let c = calendar::components(ms);
            let months = ((tf_seconds / (30.0 * DAY)).round() as i64).max(1);
            (c.year * 12 + c.month - 1).div_euclid(months)
        } else if tf_seconds >= 7.0 * DAY && (tf_seconds / (7.0 * DAY)).fract().abs() < 1e-9 {
            let weeks = ((tf_seconds / (7.0 * DAY)).round() as i64).max(1);
            // Epoch day 0 was a Thursday; shift so weeks begin on Monday.
            (ms.div_euclid(86_400_000) + 3)
                .div_euclid(7)
                .div_euclid(weeks)
        } else {
            ms.div_euclid((tf_seconds * 1_000.0).max(1.0) as i64)
        }
    };
    bucket(previous_ms) != bucket(now_ms)
}

/// The offset in seconds east of UTC that `name` has at the UTC instant `utc_ms`: a fixed offset
/// (`UTC+3`), or a zone from [`timezone`](super::timezone)'s table with its daylight-saving rule.
/// No name, or one the table does not know, reads as UTC.
pub fn zone_offset_secs(name: Option<&str>, utc_ms: i64) -> i64 {
    name.filter(|name| !name.is_empty())
        .and_then(super::timezone::resolve)
        .map_or(0, |zone| zone.offset_at(utc_ms))
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

/// `str.format_time`: `format` is a Unicode date pattern (the letters Pine and Java share: `yyyy`,
/// `MM`, `MMM`, `dd`, `HH`, `hh`, `mm`, `ss`, `SSS`, `a`, `EEE`, `Z`…), `zone` a UTC offset name.
/// Without a zone it is UTC, which is what the engine reports as the exchange zone.
pub fn formatted(milliseconds: i64, format: Option<&str>, zone: Option<&str>) -> String {
    let offset = zone_offset_secs(zone, milliseconds);
    let c = components(milliseconds, offset);
    let millis = (milliseconds + offset * 1_000).rem_euclid(1_000);
    let pattern: Vec<char> = format.unwrap_or("yyyy-MM-dd'T'HH:mm:ssZ").chars().collect();
    let mut output = String::new();
    let mut index = 0;
    while index < pattern.len() {
        let letter = pattern[index];
        if letter == '\'' {
            // Quoted text; '' is a literal quote.
            index += 1;
            if pattern.get(index) == Some(&'\'') {
                output.push('\'');
                index += 1;
                continue;
            }
            while index < pattern.len() {
                if pattern[index] == '\'' {
                    if pattern.get(index + 1) == Some(&'\'') {
                        output.push('\'');
                        index += 2;
                        continue;
                    }
                    break;
                }
                output.push(pattern[index]);
                index += 1;
            }
            index += 1;
            continue;
        }
        if !letter.is_ascii_alphabetic() {
            output.push(letter);
            index += 1;
            continue;
        }
        let run = pattern[index..]
            .iter()
            .take_while(|c| **c == letter)
            .count();
        index += run;
        let number = |value: i64, width: usize| format!("{value:0width$}");
        let hour12 = if c.hour % 12 == 0 { 12 } else { c.hour % 12 };
        let weekday = WEEKDAYS[(c.weekday - 1).rem_euclid(7) as usize];
        let month = MONTHS[(c.month - 1).rem_euclid(12) as usize];
        output.push_str(&match letter {
            'y' | 'u' => {
                if run == 2 {
                    number(c.year.rem_euclid(100), 2)
                } else {
                    number(c.year, run)
                }
            }
            'M' | 'L' => match run {
                1 | 2 => number(c.month, run),
                3 => month[..3].to_owned(),
                _ => month.to_owned(),
            },
            'd' => number(c.day, run),
            'H' => number(c.hour, run),
            'h' => number(hour12, run),
            'm' => number(c.minute, run),
            's' => number(c.second, run),
            'S' => format!("{millis:03}")
                .chars()
                .chain(std::iter::repeat('0'))
                .take(run)
                .collect(),
            'a' => (if c.hour < 12 { "AM" } else { "PM" }).to_owned(),
            'E' => {
                if run >= 4 {
                    weekday.to_owned()
                } else {
                    weekday[..3].to_owned()
                }
            }
            'Z' => offset_text(offset, false),
            'z' | 'X' | 'x' => offset_text(offset, run >= 3),
            other => other.to_string().repeat(run),
        });
    }
    output
}

/// "+0300" or, with a colon, "+03:00".
fn offset_text(offset_secs: i64, colon: bool) -> String {
    let sign = if offset_secs < 0 { '-' } else { '+' };
    let minutes = offset_secs.abs() / 60;
    let (hours, minutes) = (minutes / 60, minutes % 60);
    if colon {
        format!("{sign}{hours:02}:{minutes:02}")
    } else {
        format!("{sign}{hours:02}{minutes:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2024-03-05 14:30:15.250 UTC, a Tuesday.
    const STAMP: i64 = 1_709_649_015_250;

    #[test]
    fn parts_follow_the_zone_offset() {
        assert_eq!(part("year", STAMP, 0), RuntimeValue::Int(2024));
        assert_eq!(part("month", STAMP, 0), RuntimeValue::Int(3));
        assert_eq!(part("dayofmonth", STAMP, 0), RuntimeValue::Int(5));
        assert_eq!(part("hour", STAMP, 0), RuntimeValue::Int(14));
        assert_eq!(part("minute", STAMP, 0), RuntimeValue::Int(30));
        assert_eq!(part("second", STAMP, 0), RuntimeValue::Int(15));
        assert_eq!(
            part("dayofweek", STAMP, 0),
            RuntimeValue::Int(3),
            "Tuesday: Sunday is 1"
        );
        assert_eq!(part("hour", STAMP, 3 * 3600), RuntimeValue::Int(17));
        assert_eq!(
            part("dayofmonth", STAMP, 10 * 3600),
            RuntimeValue::Int(6),
            "past midnight"
        );
        for name in PART_NAMES {
            assert!(matches!(part(name, STAMP, 0), RuntimeValue::Int(_)));
        }
    }

    #[test]
    fn bar_spacing_names_the_timeframe() {
        let value = |name: &str, seconds: f64| timeframe(name, seconds);
        assert_eq!(
            value("timeframe.period", 60.0),
            Some(RuntimeValue::String("1".into()))
        );
        assert_eq!(
            value("timeframe.period", 3_600.0),
            Some(RuntimeValue::String("60".into()))
        );
        assert_eq!(
            value("timeframe.period", 30.0),
            Some(RuntimeValue::String("30S".into()))
        );
        assert_eq!(
            value("timeframe.period", 86_400.0),
            Some(RuntimeValue::String("1D".into()))
        );
        assert_eq!(
            value("timeframe.period", 604_800.0),
            Some(RuntimeValue::String("1W".into()))
        );
        assert_eq!(
            value("timeframe.period", 2_592_000.0),
            Some(RuntimeValue::String("1M".into()))
        );
        assert_eq!(
            value("timeframe.multiplier", 900.0),
            Some(RuntimeValue::Int(15))
        );
        assert_eq!(
            value("timeframe.isintraday", 900.0),
            Some(RuntimeValue::Bool(true))
        );
        assert_eq!(
            value("timeframe.isminutes", 900.0),
            Some(RuntimeValue::Bool(true))
        );
        assert_eq!(
            value("timeframe.isseconds", 900.0),
            Some(RuntimeValue::Bool(false))
        );
        assert_eq!(
            value("timeframe.isdaily", 86_400.0),
            Some(RuntimeValue::Bool(true))
        );
        assert_eq!(
            value("timeframe.isdwm", 86_400.0),
            Some(RuntimeValue::Bool(true))
        );
        assert_eq!(
            value("timeframe.isweekly", 604_800.0),
            Some(RuntimeValue::Bool(true))
        );
        assert_eq!(
            value("timeframe.isdaily", 604_800.0),
            Some(RuntimeValue::Bool(false))
        );
        assert_eq!(
            value("timeframe.ismonthly", 2_419_200.0),
            Some(RuntimeValue::Bool(true))
        );
        assert_eq!(
            value("timeframe.isticks", 60.0),
            Some(RuntimeValue::Bool(false))
        );
        assert_eq!(
            value("timeframe.period", 0.0),
            None,
            "spacing not known yet"
        );
        assert_eq!(value("timeframe.nope", 60.0), None);
        for name in TIMEFRAME_NAMES {
            assert!(timeframe(name, 3_600.0).is_some(), "{name}");
        }
    }

    #[test]
    fn timeframe_strings_convert_both_ways() {
        for (text, seconds) in [
            ("30S", 30.0),
            ("5", 300.0),
            ("60", 3_600.0),
            ("1D", 86_400.0),
            ("2W", 1_209_600.0),
            ("3M", 7_776_000.0),
            ("D", 86_400.0),
            ("m", 60.0),
        ] {
            // A bare "m" is a lowercase M unit: one month.
            let expected = if text == "m" {
                30.0 * 86_400.0
            } else {
                seconds
            };
            assert_eq!(seconds_of_timeframe(text), Some(expected), "{text}");
        }
        for bad in ["", "0", "0D", "5X", "-5", "1.5"] {
            assert_eq!(seconds_of_timeframe(bad), None, "{bad:?}");
        }
        assert_eq!(timeframe_from_seconds(60).as_deref(), Some("1"));
        assert_eq!(timeframe_from_seconds(7_200).as_deref(), Some("120"));
        assert_eq!(timeframe_from_seconds(86_400).as_deref(), Some("1D"));
        assert_eq!(timeframe_from_seconds(45).as_deref(), Some("45S"));
        assert_eq!(timeframe_from_seconds(604_800).as_deref(), Some("1W"));
        assert_eq!(timeframe_from_seconds(2_592_000).as_deref(), Some("1M"));
        assert_eq!(timeframe_from_seconds(0), None);
        assert_eq!(timeframe_from_seconds(-60), None);
        for seconds in [30_i64, 60, 300, 3_600, 86_400, 604_800, 1_209_600] {
            let text = timeframe_from_seconds(seconds).unwrap();
            assert_eq!(seconds_of_timeframe(&text), Some(seconds as f64), "{text}");
        }
    }

    #[test]
    fn zone_names_resolve_with_their_daylight_saving_rules() {
        let winter = calendar::make(2024, 1, 15, 12, 0, 0, 0).unwrap();
        let summer = calendar::make(2024, 7, 15, 12, 0, 0, 0).unwrap();
        assert_eq!(zone_offset_secs(None, winter), 0);
        assert_eq!(zone_offset_secs(Some(""), winter), 0);
        assert_eq!(zone_offset_secs(Some("UTC+3"), winter), 3 * 3600);
        assert_eq!(
            zone_offset_secs(Some("GMT-9:30"), winter),
            -(9 * 3600 + 1800)
        );
        assert_eq!(zone_offset_secs(Some("UTC+99"), winter), 0, "out of range");
        assert_eq!(
            zone_offset_secs(Some("America/New_York"), winter),
            -5 * 3600
        );
        assert_eq!(
            zone_offset_secs(Some("America/New_York"), summer),
            -4 * 3600
        );
        assert_eq!(
            zone_offset_secs(Some("Mars/Olympus"), summer),
            0,
            "unknown: UTC"
        );
    }

    #[test]
    fn a_named_zone_shifts_the_calendar_parts_and_the_formatted_text() {
        // 2024-07-15 12:00 UTC is 08:00 in New York (summer time) and 21:00 in Tokyo.
        let noon = calendar::make(2024, 7, 15, 12, 0, 0, 0).unwrap();
        assert_eq!(
            formatted(noon, Some("HH:mm"), Some("America/New_York")),
            "08:00"
        );
        assert_eq!(formatted(noon, Some("HH:mm"), Some("Asia/Tokyo")), "21:00");
        let january = calendar::make(2024, 1, 15, 12, 0, 0, 0).unwrap();
        assert_eq!(
            formatted(january, Some("HH:mm"), Some("America/New_York")),
            "07:00"
        );
    }

    #[test]
    fn dates_format_with_unicode_patterns() {
        let f = |pattern: &str| formatted(STAMP, Some(pattern), None);
        assert_eq!(f("yyyy-MM-dd HH:mm:ss"), "2024-03-05 14:30:15");
        assert_eq!(f("yy/M/d"), "24/3/5");
        assert_eq!(f("MMM d, yyyy"), "Mar 5, 2024");
        assert_eq!(f("MMMM"), "March");
        assert_eq!(f("EEE"), "Tue");
        assert_eq!(f("EEEE"), "Tuesday");
        assert_eq!(f("hh:mm a"), "02:30 PM");
        assert_eq!(f("h a"), "2 PM");
        assert_eq!(f("SSS"), "250");
        assert_eq!(f("ss.SS"), "15.25");
        assert_eq!(f("HH 'o''clock' mm"), "14 o'clock 30");
        assert_eq!(f("yyyy-MM-dd'T'HH:mm:ssZ"), "2024-03-05T14:30:15+0000");
        assert_eq!(
            formatted(STAMP, None, None),
            "2024-03-05T14:30:15+0000",
            "the default pattern"
        );
        assert_eq!(
            formatted(STAMP, Some("HH:mm Z"), Some("UTC+3")),
            "17:30 +0300"
        );
        assert_eq!(
            formatted(STAMP, Some("HH:mm zzz"), Some("GMT-05:30")),
            "09:00 -05:30"
        );
        let midnight = 1_709_596_800_000;
        assert_eq!(formatted(midnight, Some("hh a"), None), "12 AM");
        assert_eq!(
            formatted(midnight + 12 * 3_600_000, Some("h a"), None),
            "12 PM"
        );
    }

    #[test]
    fn timeframe_change_follows_buckets() {
        let hour = 3_600_000;
        let day = 24 * hour;
        assert!(
            timeframe_changed(None, 0, 3600.0),
            "the first bar always changes"
        );
        assert!(!timeframe_changed(Some(hour), hour + 60_000, 3600.0));
        assert!(timeframe_changed(Some(hour), 2 * hour, 3600.0));
        assert!(!timeframe_changed(
            Some(day + hour),
            day + 2 * hour,
            86_400.0
        ));
        assert!(timeframe_changed(Some(day - hour), day, 86_400.0));
        // 2024-03-31 (Sun) to 2024-04-01 (Mon): a new week, and a new month.
        let sunday = 1_711_843_200_000; // 2024-03-31T00:00:00Z
        assert!(timeframe_changed(
            Some(sunday),
            sunday + day,
            7.0 * 86_400.0
        ));
        assert!(!timeframe_changed(
            Some(sunday + day),
            sunday + 2 * day,
            7.0 * 86_400.0
        ));
        assert!(timeframe_changed(
            Some(sunday),
            sunday + day,
            30.0 * 86_400.0
        ));
        assert!(!timeframe_changed(
            Some(sunday - day),
            sunday,
            30.0 * 86_400.0
        ));
    }
}
