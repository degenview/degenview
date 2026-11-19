//! Bybit's v5 `/market/kline`.

use serde_json::Value;

use crate::bucket::next_bucket_start_ms;
use crate::fields::{number, timestamp_text};
use crate::{Kline, ParseError, Timeframe};

/// Whether a `retCode` says to slow down: 10006 (too many visits) and 10018 (the IP limit).
pub fn is_rate_limit(code: i64) -> bool {
    matches!(code, 10_006 | 10_018)
}

/// The `result` of an answer, or Bybit's refusal.
pub fn result_of(body: &str) -> Result<Value, ParseError> {
    let mut value: Value = serde_json::from_str(body)?;
    let code = value
        .get("retCode")
        .and_then(Value::as_i64)
        .ok_or_else(|| ParseError::invalid("expected a retCode"))?;
    if code != 0 {
        let message = value
            .get("retMsg")
            .and_then(Value::as_str)
            .filter(|message| !message.is_empty())
            .unwrap_or("The request failed");
        return Err(ParseError::Refused {
            exchange: "Bybit",
            message: message.to_owned(),
            rate_limited: is_rate_limit(code),
        });
    }
    value
        .get_mut("result")
        .map(Value::take)
        .ok_or_else(|| ParseError::invalid("expected a result"))
}

/// Candles of length `base`, oldest first (Bybit sends them newest first).
pub fn parse_candles(body: &str, base: Timeframe) -> Result<Vec<Kline>, ParseError> {
    let result = result_of(body)?;
    let mut candles = result
        .get("list")
        .and_then(Value::as_array)
        .ok_or_else(|| ParseError::invalid("expected a candle list"))?
        .iter()
        .map(|row| parse_row(row, base))
        .collect::<Result<Vec<_>, _>>()?;
    candles.sort_by_key(|candle| candle.open_time);
    Ok(candles)
}

/// One row, `[start ms, open, high, low, close, volume, turnover]`, all strings.
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

    const ROWS: &str = r#"{"retCode":0,"retMsg":"OK","result":{"category":"spot","symbol":"BTCUSDT","list":[
        ["1700002800000","11","13","10","12","5.5","60"],
        ["1699999200000","10","12","9","11","42.5","450"]
    ]}}"#;

    #[test]
    fn rows_come_newest_first_and_leave_oldest_first_with_the_close_of_their_bucket() {
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
        let row = serde_json::json!(["1769904000000", "1", "2", "0.5", "1.5", "3", "4"]);
        let candle = parse_row(&row, Timeframe::MONTH_1).expect("row");
        assert_eq!(candle.close_time, 1_772_323_200_000 - 1);
    }

    #[test]
    fn a_refusal_is_bybits_message_and_the_limits_are_rate_limits() {
        let unknown = r#"{"retCode":10001,"retMsg":"Not supported symbols","result":{}}"#;
        let error = parse_candles(unknown, Timeframe::HOUR_1).unwrap_err();
        assert_eq!(error.to_string(), "Bybit: Not supported symbols");
        assert!(!error.is_rate_limit());
        for code in [10_006, 10_018] {
            let limited =
                format!(r#"{{"retCode":{code},"retMsg":"Too many visits!","result":{{}}}}"#);
            let error = parse_candles(&limited, Timeframe::HOUR_1).unwrap_err();
            assert!(error.is_rate_limit(), "{code}: {error}");
        }
        assert!(parse_candles("not json", Timeframe::HOUR_1).is_err());
        assert!(parse_candles(r#"{"retCode":0,"result":{}}"#, Timeframe::HOUR_1).is_err());
        assert!(parse_candles(r#"{"result":{"list":[]}}"#, Timeframe::HOUR_1).is_err());
        let bad_field =
            r#"{"retCode":0,"result":{"list":[["1700000000000","x","1","1","1","1","1"]]}}"#;
        assert!(parse_candles(bad_field, Timeframe::HOUR_1).is_err());
    }
}
