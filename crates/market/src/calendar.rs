//! Calendar dates and ISO-8601 instants in UTC, as epoch days and epoch milliseconds.

/// The proleptic Gregorian date `(year, month, day)` of `days_since_epoch` (days since
/// 1970-01-01, negative before it).
pub fn civil_from_days(days_since_epoch: i64) -> (i64, i64, i64) {
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

/// Inverse of [`civil_from_days`]: days since 1970-01-01 of a Gregorian date, for calendar-bucket
/// folding.
pub fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_prime = (month + 9) % 12;
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// ISO-8601 UTC instant at whole seconds, the form Coinbase's `start`/`end` candle query expects.
pub fn iso8601(epoch_ms: i64) -> String {
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

pub fn parse_iso8601(text: &str) -> Option<i64> {
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
    let days = days_from_civil(year, month, day);
    Some(((days * 86_400 + hour * 3_600 + minute * 60 + second) * 1_000) + millis)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_days_and_dates_are_inverse() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
        assert_eq!(days_from_civil(2024, 2, 29), 19_782);
        for days in (-800_000..800_000).step_by(997) {
            let (year, month, day) = civil_from_days(days);
            assert_eq!(days_from_civil(year, month, day), days, "{days}");
        }
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

    #[test]
    fn written_instants_parse_back() {
        assert_eq!(iso8601(1_705_330_800_000), "2024-01-15T15:00:00.000Z");
        for ms in [0, 1_700_000_000_000, -86_400_000 * 400] {
            assert_eq!(parse_iso8601(&iso8601(ms)), Some(ms), "{ms}");
        }
    }
}
