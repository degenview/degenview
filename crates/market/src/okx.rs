//! OKX's `/market/candles` and `/market/history-candles`, and the same rows on its websocket.

use serde_json::Value;

use crate::bucket::next_bucket_start_ms;
use crate::fields::{number, timestamp_text};
use crate::{Kline, ParseError, Timeframe};

/// The `data` rows of an answer, or OKX's own message when its `code` says it failed.
pub fn data_rows(body: &str) -> Result<Vec<Value>, ParseError> {
    let value: Value = serde_json::from_str(body)?;
    if value.get("code").and_then(Value::as_str) != Some("0") {
        let message = value
            .get("msg")
            .and_then(Value::as_str)
            .filter(|message| !message.is_empty())
            .unwrap_or("The request failed");
        return Err(ParseError::Refused {
            exchange: "OKX",
            message: message.to_owned(),
            rate_limited: false,
        });
    }
    value
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| ParseError::invalid("expected a data array"))
}

/// Candles of length `base`, oldest first (OKX sends them newest first).
pub fn parse_candles(body: &str, base: Timeframe) -> Result<Vec<Kline>, ParseError> {
    let mut candles = data_rows(body)?
        .iter()
        .map(|row| parse_row(row, base))
        .collect::<Result<Vec<_>, _>>()?;
    candles.sort_by_key(|candle| candle.open_time);
    Ok(candles)
}

/// One row, `[open time ms, open, high, low, close, volume, …]`, all strings. A push on the
/// websocket carries the same rows.
pub fn parse_row(row: &Value, base: Timeframe) -> Result<Kline, ParseError> {
    let open_time = timestamp_text(row, 0)?;
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

    const ROWS: &str = r#"{"code":"0","msg":"","data":[
        ["1700002800000","11","13","10","12","5.5","60","60","0"],
        ["1699999200000","10","12","9","11","42.5","450","450","1"]
    ]}"#;

    #[test]
    fn rows_come_newest_first_and_leave_oldest_first() {
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
        let row = serde_json::json!(["1769904000000", "1", "2", "0.5", "1.5", "3", "4", "4", "1"]);
        let candle = parse_row(&row, Timeframe::MONTH_1).expect("row");
        // 2026-03-01 00:00 UTC, less a millisecond: February has 28 days.
        assert_eq!(candle.close_time, 1_772_323_200_000 - 1);
    }

    #[test]
    fn an_error_answer_is_okxs_message() {
        let body = r#"{"code":"51001","msg":"Instrument ID doesn't exist.","data":[]}"#;
        let error = parse_candles(body, Timeframe::HOUR_1).unwrap_err();
        assert_eq!(error.to_string(), "OKX: Instrument ID doesn't exist.");
        assert!(!error.is_rate_limit());
        assert!(parse_candles("not json", Timeframe::HOUR_1).is_err());
        assert!(parse_candles(r#"{"code":"0"}"#, Timeframe::HOUR_1).is_err());
        let bad_field = r#"{"code":"0","data":[["1700000000000","x","1","1","1","1"]]}"#;
        assert!(parse_candles(bad_field, Timeframe::HOUR_1).is_err());
    }
}
