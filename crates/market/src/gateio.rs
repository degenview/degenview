//! Gate.io's v4 `/spot/candlesticks`.

use serde_json::Value;

use crate::bucket::next_bucket_start_ms;
use crate::fields::{candle_array, number, timestamp_text};
use crate::{Kline, ParseError, Timeframe};

/// Candles of length `base`, oldest first.
pub fn parse_candles(body: &str, base: Timeframe) -> Result<Vec<Kline>, ParseError> {
    let mut candles = candle_array(body)?
        .iter()
        .map(|row| parse_row(row, base))
        .collect::<Result<Vec<_>, _>>()?;
    candles.sort_by_key(|candle| candle.open_time);
    Ok(candles)
}

/// One row, `[time s, quote volume, close, high, low, open, base volume, closed]`, all strings.
pub fn parse_row(row: &Value, base: Timeframe) -> Result<Kline, ParseError> {
    let open_time = timestamp_text(row, 0)? * 1_000;
    Ok(Kline {
        open_time,
        close_time: next_bucket_start_ms(base, open_time) - 1,
        open: number(row, 5)?,
        high: number(row, 3)?,
        low: number(row, 4)?,
        close: number(row, 2)?,
        volume: number(row, 6)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // The close is the third field and the open the sixth; the base volume is the seventh.
    const ROWS: &str = r#"[
        ["1699999200","450","11","12","9","10","42.5","true"],
        ["1700002800","60","12","13","10","11","5.5","false"]
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
        assert_eq!(candles[1].open_time, 1_700_002_800_000);
    }

    #[test]
    fn a_month_closes_when_the_next_calendar_month_opens() {
        // 2026-02-01 00:00 UTC.
        let row = serde_json::json!(["1769904000", "4", "1.5", "2", "0.5", "1", "3", "true"]);
        let candle = parse_row(&row, Timeframe::MONTH_1).expect("row");
        assert_eq!(candle.close_time, 1_772_323_200_000 - 1);
    }

    #[test]
    fn odd_answers_are_errors() {
        assert!(parse_candles("not json", Timeframe::HOUR_1).is_err());
        assert!(parse_candles(r#"{"label":"INVALID_PARAM_VALUE"}"#, Timeframe::HOUR_1).is_err());
        assert!(parse_candles("[]", Timeframe::HOUR_1).unwrap().is_empty());
        let bad_field = r#"[["1700000000","1","x","1","1","1","1","true"]]"#;
        assert!(parse_candles(bad_field, Timeframe::HOUR_1).is_err());
        assert!(
            parse_candles(
                r#"[["x","1","1","1","1","1","1","true"]]"#,
                Timeframe::HOUR_1
            )
            .is_err()
        );
    }
}
