pub(crate) use degenview_market::calendar::{
    civil_from_days, days_from_civil, iso8601, parse_iso8601,
};

pub fn price(value: f64) -> String {
    let magnitude = value.abs();
    let decimals = if magnitude >= 1_000.0 {
        2
    } else if magnitude >= 1.0 {
        4
    } else if magnitude >= 0.01 {
        6
    } else {
        8
    };
    let text = format!("{value:.decimals$}");
    if magnitude >= 1_000.0 {
        group_thousands(&text)
    } else {
        text
    }
}

/// The chart card's price line: `$68,432.15` (two to eight decimals, as many as the price needs),
/// or `66.5%` for a prediction market's 0…1 probability.
pub fn headline(value: f64, probability: bool) -> String {
    if probability {
        return format!("{:.1}%", (value * 100.0).clamp(0.0, 100.0));
    }
    let text = format!("{:.8}", value.abs());
    let (integer, fraction) = text.split_once('.').unwrap_or((&text, ""));
    let fraction = fraction.trim_end_matches('0');
    let fraction = format!("{fraction:0<2}");
    let sign = if value < 0.0 { "-" } else { "" };
    format!("{sign}${}.{fraction}", group_thousands(integer))
}

pub(crate) fn group_thousands(text: &str) -> String {
    let (integer, fraction) = text.split_once('.').unwrap_or((text, ""));
    let (sign, digits) = integer
        .strip_prefix('-')
        .map_or(("", integer), |remaining| ("-", remaining));
    let mut output = String::new();
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            output.push(',');
        }
        output.push(character);
    }
    if fraction.is_empty() {
        format!("{sign}{output}")
    } else {
        format!("{sign}{output}.{fraction}")
    }
}

pub fn timestamp(epoch_ms: i64) -> String {
    let seconds = epoch_ms.div_euclid(1_000);
    let days = seconds.div_euclid(86_400);
    let remainder = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        remainder / 3_600,
        remainder % 3_600 / 60
    )
}

/// Reads a UTC date and time the way [`timestamp`] writes it: `YYYY-MM-DD`, `YYYY-MM-DD HH:MM` or
/// `YYYY-MM-DD HH:MM:SS` (a `T` may stand for the space and a trailing `Z` is ignored). Returns
/// epoch milliseconds, or `None` for anything that is not a real calendar moment.
pub(crate) fn parse_timestamp(text: &str) -> Option<i64> {
    let text = text.trim().trim_end_matches(['Z', 'z']).trim();
    let (date, time) = match text.split_once([' ', 'T']) {
        Some((date, time)) => (date, time.trim()),
        None => (text, ""),
    };
    let mut parts = date.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: i64 = parts.next()?.parse().ok()?;
    let day: i64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=9999).contains(&year) || !(1..=12).contains(&month) {
        return None;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days_in_month = match month {
        2 => 28 + i64::from(leap),
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if !(1..=days_in_month).contains(&day) {
        return None;
    }
    let (mut hours, mut minutes, mut seconds) = (0i64, 0i64, 0i64);
    if !time.is_empty() {
        let mut clock = time.split(':');
        hours = clock.next()?.parse().ok()?;
        minutes = clock.next().map_or(Some(0), |part| part.parse().ok())?;
        seconds = clock.next().map_or(Some(0), |part| part.parse().ok())?;
        if clock.next().is_some() {
            return None;
        }
    }
    if !(0..24).contains(&hours) || !(0..60).contains(&minutes) || !(0..60).contains(&seconds) {
        return None;
    }
    let days = days_from_civil(year, month, day);
    Some(((days * 24 + hours) * 60 + minutes) * 60_000 + seconds * 1_000)
}

/// `count` and the noun that agrees with it: "1 alert", "3 alerts".
pub(crate) fn plural<N>(count: N, one: &str, many: &str) -> String
where
    N: std::fmt::Display + PartialEq + From<u8>,
{
    let noun = if count == N::from(1) { one } else { many };
    format!("{count} {noun}")
}

#[cfg(test)]
mod tests {
    #[test]
    fn plural_agrees_with_one_and_with_every_other_count() {
        assert_eq!(super::plural(1usize, "alert", "alerts"), "1 alert");
        assert_eq!(super::plural(0usize, "alert", "alerts"), "0 alerts");
        assert_eq!(super::plural(3u32, "call", "calls"), "3 calls");
    }

    #[test]
    fn timestamps_parse_back_to_what_they_were_written_from() {
        for ms in [
            0i64,
            86_400_000,
            1_700_000_040_000,
            -86_400_000 * 400,
            4_102_444_800_000,
        ] {
            // Seconds are not written, so compare at minute precision.
            let minute = ms - ms.rem_euclid(60_000);
            assert_eq!(
                super::parse_timestamp(&super::timestamp(ms)),
                Some(minute),
                "{ms}"
            );
        }
    }

    #[test]
    fn timestamps_accept_the_usual_spellings_and_refuse_impossible_dates() {
        let day = super::parse_timestamp("2024-02-29").unwrap();
        assert_eq!(super::parse_timestamp("2024-02-29 00:00"), Some(day));
        assert_eq!(super::parse_timestamp("2024-02-29T00:00:00Z"), Some(day));
        assert_eq!(
            super::parse_timestamp("  2024-02-29 12:30  "),
            Some(day + 45_000_000)
        );
        assert_eq!(super::parse_timestamp("1970-01-01 00:00:01"), Some(1_000));
        for bad in [
            "",
            "2023-02-29",
            "2024-13-01",
            "2024-00-10",
            "2024-04-31",
            "2024-01-01 24:00",
            "2024-01-01 12:60",
            "2024-01-01 12:00:60",
            "yesterday",
            "2024-01",
            "2024-01-01-02",
            "2024-01-01 12:00:00:00",
            "0000-01-01",
            "99999-01-01",
            "2024-1-1 x",
        ] {
            assert_eq!(super::parse_timestamp(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn headline_uses_currency_or_probability() {
        assert_eq!(super::headline(68_432.15, false), "$68,432.15");
        assert_eq!(super::headline(2.0, false), "$2.00");
        assert_eq!(super::headline(0.000_012_34, false), "$0.00001234");
        assert_eq!(super::headline(0.665, true), "66.5%");
        assert_eq!(super::headline(1.2, true), "100.0%");
    }

    use super::*;

    #[test]
    fn price_precision_tracks_magnitude() {
        assert_eq!(price(67_432.19), "67,432.19");
        assert_eq!(price(1.23456), "1.2346");
        assert_eq!(price(0.00000278), "0.00000278");
    }

    #[test]
    fn timestamp_is_utc() {
        assert_eq!(timestamp(0), "1970-01-01 00:00");
        assert_eq!(timestamp(1_700_000_000_000), "2023-11-14 22:13");
    }
}
