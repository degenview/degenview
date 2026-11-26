//! TradingView's resolution strings: a bare number is minutes, then `S`, `D`, `W` or `M`.

use crate::model::{TimeUnit, Timeframe};

/// The timeframe `text` names; `None` for tick and range resolutions and counts out of range.
pub fn timeframe(text: &str) -> Option<Timeframe> {
    let text = text.trim();
    let digits = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    let (count, unit) = text.split_at(digits);
    let count: u32 = if count.is_empty() {
        1
    } else {
        count.parse().ok()?
    };
    let unit = match unit {
        "" if !text.is_empty() => TimeUnit::Minute,
        "S" => TimeUnit::Second,
        "H" => TimeUnit::Hour,
        "D" => TimeUnit::Day,
        "W" => TimeUnit::Week,
        "M" => TimeUnit::Month,
        _ => return None,
    };
    Timeframe::new(u16::try_from(count).ok()?, unit).ok()
}

/// The length of one bar of `text`, for turning a bar offset into time. A month is taken as 30
/// days: only points past the newest bar need it.
pub fn bar_ms(text: &str) -> Option<i64> {
    let timeframe = timeframe(text)?;
    let unit: i64 = match timeframe.unit() {
        TimeUnit::Second => 1_000,
        TimeUnit::Minute => 60_000,
        TimeUnit::Hour => 3_600_000,
        TimeUnit::Day => 86_400_000,
        TimeUnit::Week => 7 * 86_400_000,
        TimeUnit::Month => 30 * 86_400_000,
    };
    Some(unit * i64::from(timeframe.count()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tradingview_resolutions_read_as_timeframes() {
        for (text, expected) in [
            ("1", Timeframe::MINUTE_1),
            ("5", Timeframe::MINUTE_5),
            ("60", Timeframe::HOUR_1),
            ("240", Timeframe::HOUR_4),
            ("D", Timeframe::DAY_1),
            ("1D", Timeframe::DAY_1),
            ("1W", Timeframe::WEEK_1),
            ("1M", Timeframe::MONTH_1),
            ("12M", Timeframe::YEAR_1),
            ("30S", Timeframe::SECOND_30),
        ] {
            assert_eq!(timeframe(text), Some(expected), "{text}");
        }
    }

    #[test]
    fn what_is_not_a_time_length_is_none() {
        for text in ["", "1T", "100R", "0", "abc"] {
            assert_eq!(timeframe(text), None, "{text:?}");
        }
        assert_eq!(bar_ms("60"), Some(3_600_000));
    }
}
