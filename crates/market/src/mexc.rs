//! MEXC's v3 `/klines`.

use serde_json::Value;

use crate::bucket::{bucket_start_ms, next_bucket_start_ms};
use crate::fields::{candle_array_or_message, number, timestamp_int};
use crate::{Kline, ParseError, Timeframe};

/// Candles of length `base`, oldest first. The days, weeks and months MEXC cut at UTC+8 midnight
/// (its oldest history) are not on the app's grid and are left out.
pub fn parse_candles(body: &str, base: Timeframe) -> Result<Vec<Kline>, ParseError> {
    let rows = candle_array_or_message(body, "MEXC", "msg")?;
    let mut candles = rows
        .iter()
        .map(|row| parse_row(row, base))
        .collect::<Result<Vec<_>, _>>()?;
    candles.retain(|candle| bucket_start_ms(base, candle.open_time) == candle.open_time);
    candles.sort_by_key(|candle| candle.open_time);
    Ok(candles)
}

/// One row, `[open time ms, open, high, low, close, volume, close time ms, quote volume]`.
pub fn parse_row(row: &Value, base: Timeframe) -> Result<Kline, ParseError> {
    let open_time = timestamp_int(row, 0)?;
    Ok(Kline {
        open_time,
        close_time: next_bucket_start_ms(base, open_time) - 1,
        open: number(row, 1)?,
        high: number(row, 2)?,
        low: number(row, 3)?,
        close: number(row, 4)?,
        volume: number(row, 5)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // A one-minute candle: the volume is the sixth field, the close time (the next open) the
    // seventh and the quote volume the eighth.
    const ROWS: &str = r#"[
        [1699999200000,"10.0","12.0","9.0","11.0","42.5",1700002800000,"450"],
        [1700002800000,"11.0","13.0","10.0","12.0","5.5",1700006400000,"60"]
    ]"#;

    #[test]
    fn rows_are_candles_with_the_close_of_their_bucket() {
        let candles = parse_candles(ROWS, Timeframe::HOUR_1).expect("valid response");
        assert_eq!(candles.len(), 2);
        assert_eq!(
            candles[0],
            Kline {
                open_time: 1_699_999_200_000,
                close_time: 1_700_002_799_999,
                open: 10.0,
                high: 12.0,
                low: 9.0,
                close: 11.0,
                volume: 42.5,
            }
        );
    }

    #[test]
    fn candles_cut_at_utc_plus_8_midnight_are_not_on_the_grid_and_are_left_out() {
        // 2018-06-01 16:00 UTC, which MEXC's oldest days open at, and 2018-12-04 00:00 UTC.
        let body = r#"[
            [1527868800000,"1","2","0.5","1.5","3",1527955200000,"4"],
            [1543881600000,"1","2","0.5","1.5","3",1543968000000,"4"]
        ]"#;
        let candles = parse_candles(body, Timeframe::DAY_1).unwrap();
        assert_eq!(candles.len(), 1);
        assert_eq!(candles[0].open_time, 1_543_881_600_000);
        // The same for months: one that opens at 16:00 on the last day of the month before.
        let months = r#"[
            [1506787200000,"1","2","0.5","1.5","3",1509465600000,"4"],
            [1509494400000,"1","2","0.5","1.5","3",1512086400000,"4"]
        ]"#;
        let candles = parse_candles(months, Timeframe::MONTH_1).unwrap();
        assert_eq!(candles.len(), 1);
        assert_eq!(candles[0].open_time, 1_509_494_400_000);
    }

    #[test]
    fn a_month_closes_when_the_next_calendar_month_opens() {
        // 2026-02-01 00:00 UTC.
        let row = serde_json::json!([
            1769904000000_i64,
            "1",
            "2",
            "0.5",
            "1.5",
            "3",
            1772323200000_i64,
            "4"
        ]);
        let candle = parse_row(&row, Timeframe::MONTH_1).expect("row");
        assert_eq!(candle.close_time, 1_772_323_200_000 - 1);
    }

    #[test]
    fn odd_answers_are_errors() {
        assert!(parse_candles("not json", Timeframe::HOUR_1).is_err());
        let error = parse_candles(
            r#"{"code":-1121,"msg":"Invalid symbol."}"#,
            Timeframe::HOUR_1,
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "MEXC: Invalid symbol.");
        assert!(parse_candles(r#"{"x":1}"#, Timeframe::HOUR_1).is_err());
        assert!(parse_candles("[]", Timeframe::HOUR_1).unwrap().is_empty());
        let bad_field = r#"[[1700000000000,"x","1","1","1","1",1,"1"]]"#;
        assert!(parse_candles(bad_field, Timeframe::HOUR_1).is_err());
        assert!(parse_candles(r#"[["x","1","1","1","1","1",1,"1"]]"#, Timeframe::HOUR_1).is_err());
    }
}
