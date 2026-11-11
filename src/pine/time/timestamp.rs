//! Pine's `timestamp()`: argument handling and date-string parsing on top of `PineCalendar`.
//! Ported from `PineTimestamp.swift`.

use std::collections::HashMap;

use super::calendar;
use crate::pine::runtime::value::RuntimeValue;

const MONTH_NAMES: [&str; 12] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

const ORDER: [&str; 6] = ["year", "month", "day", "hour", "minute", "second"];

/// `timestamp(dateString)`, `timestamp(y, m, d[, h, mi, s])` and
/// `timestamp(timezone, y, m, d[, h, mi, s])`, positional or named.
pub fn evaluate(positional: &[RuntimeValue], named: &HashMap<String, RuntimeValue>) -> Option<i64> {
    let mut values = positional.to_vec();
    let mut zone = 0i64;
    let mut named_zone = None;
    if let Some(RuntimeValue::String(text)) = values.first() {
        if values.len() == 1 && named.is_empty() {
            return parse(text);
        }
        match offset(text) {
            Some(minutes) => zone = minutes,
            // `America/New_York`: the offset depends on the date, so it is found after the fields.
            None => named_zone = Some(super::timezone::resolve(text)?),
        }
        values.remove(0);
    }
    let mut fields: [Option<i64>; 6] = [None; 6];
    for (i, value) in values.iter().take(ORDER.len()).enumerate() {
        fields[i] = integer(value);
    }
    for (name, value) in named {
        if let Some(i) = ORDER.iter().position(|o| o == name) {
            fields[i] = integer(value);
        }
    }
    let (year, month, day) = (fields[0]?, fields[1]?, fields[2]?);
    // An `na` argument makes the whole timestamp `na`.
    if values.contains(&RuntimeValue::Na) {
        return None;
    }
    let wall_clock = calendar::make(
        year,
        month,
        day,
        fields[3].unwrap_or(0),
        fields[4].unwrap_or(0),
        fields[5].unwrap_or(0),
        zone,
    )?;
    Some(match named_zone {
        Some(zone) => wall_clock - zone.offset_for_local(wall_clock) * 1_000,
        None => wall_clock,
    })
}

fn integer(value: &RuntimeValue) -> Option<i64> {
    let n = value.number()?;
    if n.abs() >= 1e12 {
        return None;
    }
    if n.fract() == 0.0 && n >= i64::MIN as f64 && n <= i64::MAX as f64 {
        Some(n as i64)
    } else {
        None
    }
}

/// Accepts ISO (`2018-01-01`, `2018-01-01T12:30:00Z`) and the `01 Jan 2018 00:00 +0000` family.
pub fn parse(text: &str) -> Option<i64> {
    let mut year: Option<i64> = None;
    let mut month: Option<i64> = None;
    let mut day: Option<i64> = None;
    let mut time = (0i64, 0i64, 0i64);
    let mut zone = 0i64;

    let mut tokens: Vec<String> = text
        .split([' ', ',', '\t'])
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect();
    if tokens.len() == 1 {
        let token = &tokens[0];
        if token.contains('T') && token.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            tokens = token.splitn(2, 'T').map(str::to_owned).collect();
        }
    }
    let mut numbers: Vec<i64> = Vec::new();
    for token in &tokens {
        let lower = token.to_lowercase();
        if let Some(z) = zone_token(&lower, token) {
            zone = z;
        } else if lower.contains(':') {
            let mut clock = lower.clone();
            if let Some(index) = clock.find(['z', '+', '-']) {
                zone = offset(&clock[index..]).unwrap_or(0);
                clock = clock[..index].to_owned();
            }
            let parts: Vec<i64> = clock.split(':').filter_map(|p| p.parse().ok()).collect();
            if parts.len() < 2 {
                return None;
            }
            time = (
                parts[0],
                parts[1],
                if parts.len() > 2 { parts[2] } else { 0 },
            );
        } else if let Some(index) = month_index(&lower) {
            month = Some(index as i64 + 1);
        } else if lower.contains('-') && lower.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            let parts: Vec<i64> = lower.split('-').filter_map(|p| p.parse().ok()).collect();
            if parts.len() != 3 {
                return None;
            }
            (year, month, day) = (Some(parts[0]), Some(parts[1]), Some(parts[2]));
        } else if let Ok(n) = lower.parse::<i64>() {
            numbers.push(n);
        } else {
            return None;
        }
    }
    for n in numbers {
        if n > 31 || (day.is_some() && year.is_none()) {
            year = year.or(Some(n));
        } else if day.is_none() {
            day = Some(n);
        } else if year.is_none() {
            year = Some(n);
        }
    }
    let (year, month, day) = (year?, month?, day?);
    calendar::make(year, month, day, time.0, time.1, time.2, zone)
}

/// Index of the month a word names: its full name, or an abbreviation of 3–4 letters.
fn month_index(word: &str) -> Option<usize> {
    let trimmed = word.trim_matches('.');
    if let Some(exact) = MONTH_NAMES.iter().position(|m| *m == trimmed) {
        return Some(exact);
    }
    if !(3..=4).contains(&trimmed.len()) {
        return None;
    }
    MONTH_NAMES.iter().position(|m| m.starts_with(trimmed))
}

fn zone_token(lower: &str, token: &str) -> Option<i64> {
    let looks_like_zone = lower == "z"
        || lower.starts_with("gmt")
        || lower.starts_with("utc")
        || ((lower.starts_with('+') || lower.starts_with('-')) && lower.len() >= 2);
    if looks_like_zone { offset(token) } else { None }
}

/// Minutes east of UTC for `Z`, `+0200`, `-05:00`, `GMT+2`, `UTC-3`, `UTC`.
pub fn offset(text: &str) -> Option<i64> {
    let mut s = text.to_uppercase();
    if s == "Z" || s == "UTC" || s == "GMT" {
        return Some(0);
    }
    if s.starts_with("GMT") || s.starts_with("UTC") {
        s = s[3..].to_owned();
    }
    let sign = s.chars().next()?;
    if sign != '+' && sign != '-' {
        return None;
    }
    let digits: String = s[1..].chars().filter(|c| *c != ':').collect();
    if digits.is_empty() || digits.len() > 4 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let value: i64 = digits.parse().unwrap_or(0);
    let minutes = if digits.len() <= 2 {
        value * 60
    } else {
        value / 100 * 60 + value % 100
    };
    Some(if sign == '-' { -minutes } else { minutes })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_named_zone_follows_daylight_saving_time() {
        let at = |zone: &str, month: i64, hour: i64| {
            evaluate(
                &[
                    RuntimeValue::String(zone.to_owned()),
                    RuntimeValue::Int(2024),
                    RuntimeValue::Int(month),
                    RuntimeValue::Int(15),
                    RuntimeValue::Int(hour),
                    RuntimeValue::Int(30),
                ],
                &HashMap::new(),
            )
        };
        // 09:30 in New York is 14:30 UTC in January and 13:30 UTC in July.
        assert_eq!(
            at("America/New_York", 1, 9),
            calendar::make(2024, 1, 15, 14, 30, 0, 0)
        );
        assert_eq!(
            at("America/New_York", 7, 9),
            calendar::make(2024, 7, 15, 13, 30, 0, 0)
        );
        assert_eq!(at("UTC+2", 1, 9), calendar::make(2024, 1, 15, 7, 30, 0, 0));
        assert_eq!(at("Mars/Olympus", 1, 9), None, "an unknown zone gives na");
    }

    #[test]
    fn parses_iso_dates() {
        assert_eq!(parse("2018-01-01"), calendar::make(2018, 1, 1, 0, 0, 0, 0));
        assert_eq!(
            parse("2018-01-01T12:30:00Z"),
            calendar::make(2018, 1, 1, 12, 30, 0, 0)
        );
    }

    #[test]
    fn parses_named_months_and_offsets() {
        assert_eq!(
            parse("01 Jan 2018 00:00 +0000"),
            calendar::make(2018, 1, 1, 0, 0, 0, 0)
        );
        // `-05:00` is 300 minutes west of UTC.
        assert_eq!(
            parse("2018-01-01T00:00:00-05:00"),
            calendar::make(2018, 1, 1, 0, 0, 0, -300)
        );
    }

    #[test]
    fn offset_parses_common_zones() {
        assert_eq!(offset("Z"), Some(0));
        assert_eq!(offset("+0200"), Some(120));
        assert_eq!(offset("-05:00"), Some(-300));
        assert_eq!(offset("GMT+2"), Some(120));
        assert_eq!(offset("bogus"), None);
    }

    #[test]
    fn evaluate_accepts_positional_fields() {
        let values = [
            RuntimeValue::Int(2023),
            RuntimeValue::Int(11),
            RuntimeValue::Int(14),
        ];
        assert_eq!(
            evaluate(&values, &HashMap::new()),
            calendar::make(2023, 11, 14, 0, 0, 0, 0)
        );
    }
}
