//! Kraken's `OHLC`.

use serde_json::Value;

use crate::bucket::next_bucket_start_ms;
use crate::fields::number;
use crate::{Kline, ParseError, Timeframe};

/// Whether a Kraken error code says to slow down: `EAPI:Rate limit exceeded`,
/// `EGeneral:Too many requests`, `EService:Throttled`, `EGeneral:Temporary lockout`.
pub fn is_rate_limit(code: &str) -> bool {
    code.starts_with("EAPI:Rate limit")
        || code.starts_with("EService:Throttled")
        || code.starts_with("EGeneral:Temporary lockout")
        || code.starts_with("EGeneral:Too many requests")
}

/// The `result` of an answer, or Kraken's refusal.
pub fn result_of(body: &str) -> Result<Value, ParseError> {
    let mut value: Value = serde_json::from_str(body)?;
    if let Some(code) = value
        .get("error")
        .and_then(Value::as_array)
        .and_then(|errors| errors.first())
        .and_then(Value::as_str)
    {
        return Err(refusal(code));
    }
    value
        .get_mut("result")
        .map(Value::take)
        .ok_or_else(|| ParseError::invalid("expected a result"))
}

/// What a Kraken error code (`EAPI:Rate limit exceeded`, `EQuery:Unknown asset pair`) means.
fn refusal(code: &str) -> ParseError {
    ParseError::Refused {
        exchange: "Kraken",
        message: code
            .split_once(':')
            .map_or(code, |(_, text)| text)
            .to_owned(),
        rate_limited: is_rate_limit(code),
    }
}

/// Candles of length `base`, oldest first.
pub fn parse_candles(body: &str, base: Timeframe) -> Result<Vec<Kline>, ParseError> {
    let result = result_of(body)?;
    let rows = result
        .as_object()
        .and_then(|result| result.iter().find(|(key, _)| key.as_str() != "last"))
        .and_then(|(_, rows)| rows.as_array())
        .ok_or_else(|| ParseError::invalid("expected a candle array"))?;
    let mut candles = rows
        .iter()
        .map(|row| parse_row(row, base))
        .collect::<Result<Vec<_>, _>>()?;
    candles.sort_by_key(|candle| candle.open_time);
    Ok(candles)
}

/// One row, `[time s, open, high, low, close, vwap, volume, count]`.
fn parse_row(row: &Value, base: Timeframe) -> Result<Kline, ParseError> {
    let open_time = row
        .get(0)
        .and_then(Value::as_i64)
        .ok_or_else(|| ParseError::invalid("invalid timestamp"))?
        * 1_000;
    Ok(Kline {
        open_time,
        close_time: next_bucket_start_ms(base, open_time) - 1,
        open: number(row, 1)?,
        high: number(row, 2)?,
        low: number(row, 3)?,
        close: number(row, 4)?,
        volume: number(row, 6)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const OHLC: &str = r#"{"error":[],"result":{"BTC/USD":[
        [1699999200,"10.0","12.0","9.0","11.0","10.5","42.5",7],
        [1700002800,"11.0","13.0","10.0","12.0","11.5","5.5",3]
    ],"last":1699999200}}"#;

    #[test]
    fn rows_are_candles_with_the_close_of_their_bucket_and_the_last_key_is_not_a_pair() {
        let candles = parse_candles(OHLC, Timeframe::HOUR_1).expect("valid response");
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
    fn a_refusal_is_krakens_message_and_a_rate_limit_is_one() {
        let unknown = r#"{"error":["EQuery:Unknown asset pair"]}"#;
        let error = parse_candles(unknown, Timeframe::HOUR_1).unwrap_err();
        assert_eq!(error.to_string(), "Kraken: Unknown asset pair");
        assert!(!error.is_rate_limit());
        for code in [
            "EAPI:Rate limit exceeded",
            "EGeneral:Too many requests",
            "EService:Throttled: 1700000000",
        ] {
            let limited = format!(r#"{{"error":["{code}"]}}"#);
            let error = parse_candles(&limited, Timeframe::HOUR_1).unwrap_err();
            assert!(error.is_rate_limit(), "{code}: {error}");
        }
        assert!(parse_candles("not json", Timeframe::HOUR_1).is_err());
        assert!(parse_candles(r#"{"error":[]}"#, Timeframe::HOUR_1).is_err());
        assert!(parse_candles(r#"{"error":[],"result":{"last":1}}"#, Timeframe::HOUR_1).is_err());
        let bad_field =
            r#"{"error":[],"result":{"BTC/USD":[[1699999200,"x","1","1","1","1","1",1]]}}"#;
        assert!(parse_candles(bad_field, Timeframe::HOUR_1).is_err());
    }
}
