//! A market's recent closes, as a sparkline or a small chart draws them: what a read found, and
//! which closes of a provider's candles fall inside the span shown.

use crate::model::Kline;
use crate::model::market::session;

const DAY_MS: i64 = 86_400_000;

/// What is known about a market's recent closes. A market not heard about yet has no `Trend`
/// at all (it is still loading).
#[derive(Clone, Debug, PartialEq)]
pub enum Trend {
    /// Closes, oldest first.
    Ready(Vec<f32>),
    /// The market has no history to show.
    Unavailable,
}

/// The closes of the daily candles that opened within `days` calendar days of the newest one,
/// oldest first. A calendar span, not a candle count, so a stock's week is its five sessions and
/// a coin's is seven days.
pub fn closes_within(candles: &[Kline], days: u32) -> Vec<f32> {
    let Some(newest) = candles.last() else {
        return Vec::new();
    };
    let since = newest.open_time - i64::from(days.saturating_sub(1)) * DAY_MS;
    candles
        .iter()
        .filter(|candle| candle.open_time >= since)
        .map(|candle| candle.close as f32)
        .collect()
}

/// The closes of the candles that opened within the 24 hours up to the newest one, oldest first.
/// Counted back from the newest candle, so a market resting over a weekend still has its last day.
pub fn last_day_closes(candles: &[Kline]) -> Vec<f32> {
    let Some(newest) = candles.last() else {
        return Vec::new();
    };
    let since = newest.open_time - DAY_MS;
    candles
        .iter()
        .filter(|candle| candle.open_time > since)
        .map(|candle| candle.close as f32)
        .collect()
}

/// The closes of the newest candle's New York day: a US stock's last session, whose change is the
/// one quoted, rather than 24 hours that would reach into the session before.
pub fn last_session_closes(candles: &[Kline]) -> Vec<f32> {
    let Some(newest) = candles.last() else {
        return Vec::new();
    };
    let day = session::local_day(newest.open_time);
    candles
        .iter()
        .filter(|candle| session::local_day(candle.open_time) == day)
        .map(|candle| candle.close as f32)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(index: i64, close: f64) -> Kline {
        Kline {
            open_time: index * DAY_MS,
            close_time: (index + 1) * DAY_MS - 1,
            open: close,
            high: close,
            low: close,
            close,
            volume: 0.0,
        }
    }

    #[test]
    fn keeps_the_days_of_the_span_ending_at_the_newest_candle() {
        let candles: Vec<Kline> = (0..10).map(|index| day(index, index as f64)).collect();
        assert_eq!(
            closes_within(&candles, 7),
            vec![3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]
        );
        assert_eq!(closes_within(&candles[..3], 7), vec![0.0, 1.0, 2.0]);
        assert!(closes_within(&[], 7).is_empty());
    }

    #[test]
    fn a_last_day_ends_at_the_newest_candle_and_a_session_at_its_new_york_day() {
        let hour = |index: i64| Kline {
            open_time: index * 3_600_000,
            ..day(0, index as f64)
        };
        let candles: Vec<Kline> = (0..60).map(hour).collect();
        assert_eq!(last_day_closes(&candles).len(), 24);
        assert_eq!(last_day_closes(&candles).first(), Some(&36.0));
        // Hour 59 is 11:00 UTC on 3 January 1970, which began in New York at 05:00 UTC.
        assert_eq!(last_session_closes(&candles).first(), Some(&53.0));
        assert!(last_day_closes(&[]).is_empty() && last_session_closes(&[]).is_empty());
    }

    #[test]
    fn a_span_counts_calendar_days_not_candles() {
        // Weekdays only, as a stock trades: Monday 0 to Friday 4, then Monday 7 to Friday 11.
        let candles: Vec<Kline> = [0, 1, 2, 3, 4, 7, 8, 9, 10, 11]
            .into_iter()
            .map(|index| day(index, index as f64))
            .collect();
        assert_eq!(closes_within(&candles, 7), vec![7.0, 8.0, 9.0, 10.0, 11.0]);
    }
}
