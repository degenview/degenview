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

/// ISO-8601 UTC instant, the form Coinbase's `start`/`end` candle query expects.
pub(crate) fn iso8601(epoch_ms: i64) -> String {
    let seconds = epoch_ms.div_euclid(1_000);
    let days = seconds.div_euclid(86_400);
    let remainder = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.000Z",
        remainder / 3_600,
        remainder % 3_600 / 60,
        remainder % 60
    )
}

pub(crate) fn civil_from_days(days_since_epoch: i64) -> (i64, i64, i64) {
    let days = days_since_epoch + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    (year_of_era + era * 400 + i64::from(month <= 2), month, day)
}

/// Inverse of `civil_from_days`, exposed for calendar-bucket folding in services.
/// ISO-8601 UTC instant (`2024-01-15T15:00:00Z`, optional fractional seconds) in epoch ms.
pub(crate) fn parse_iso8601(text: &str) -> Option<i64> {
    let text = text.trim().strip_suffix('Z').unwrap_or(text.trim());
    let (date, time) = text.split_once('T')?;
    let mut date_parts = date.split('-');
    let (year, month, day) = (
        date_parts.next()?.parse::<i64>().ok()?,
        date_parts.next()?.parse::<i64>().ok()?,
        date_parts.next()?.parse::<i64>().ok()?,
    );
    let time = time.split(['+', '-']).next()?;
    let (clock, fraction) = time.split_once('.').unwrap_or((time, ""));
    let mut clock_parts = clock.split(':');
    let hour = clock_parts.next()?.parse::<i64>().ok()?;
    let minute = clock_parts.next()?.parse::<i64>().ok()?;
    let second = clock_parts
        .next()
        .map_or(Some(0), |s| s.parse::<i64>().ok())?;
    let millis = fraction
        .chars()
        .chain("000".chars())
        .take(3)
        .collect::<String>()
        .parse::<i64>()
        .ok()?;
    let days = crate::format::days_from_civil(year, month, day);
    Some(((days * 86_400 + hour * 3_600 + minute * 60 + second) * 1_000) + millis)
}

pub(crate) fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_prime = (month + 9) % 12;
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn iso_timestamps_read_back_in_epoch_ms() {
        assert_eq!(parse_iso8601("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_iso8601("2024-01-15T15:00:00Z"),
            Some(1_705_330_800_000)
        );
        assert_eq!(
            parse_iso8601("2024-01-15T15:00:00.5Z"),
            Some(1_705_330_800_500)
        );
        assert_eq!(
            parse_iso8601("2024-01-15T15:00:00.123456Z"),
            Some(1_705_330_800_123)
        );
        // Kraken's websocket writes nine fractional digits.
        assert_eq!(
            parse_iso8601("2026-11-17T10:04:00.000000000Z"),
            Some(1_794_909_840_000)
        );
        assert_eq!(parse_iso8601("yesterday"), None);
    }
}
