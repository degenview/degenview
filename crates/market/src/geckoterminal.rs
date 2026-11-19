//! GeckoTerminal's pool `ohlcv` endpoint.

use serde_json::Value;

use crate::{Kline, ParseError};

/// Candles of buckets `bucket_ms` long, newest first as GeckoTerminal sends them: rows of
/// `[timestamp seconds, open, high, low, close, volume in USD]`. A bucket closes a millisecond
/// before the next opens, as on an exchange. A malformed row is skipped.
pub fn parse_candles(body: &str, bucket_ms: i64) -> Result<Vec<Kline>, ParseError> {
    let value: Value = serde_json::from_str(body)?;
    let rows = value
        .pointer("/data/attributes/ohlcv_list")
        .and_then(Value::as_array)
        .ok_or_else(|| ParseError::NoData("No chart data for this pool".to_owned()))?;
    let mut candles = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(fields) = row.as_array() else {
            continue;
        };
        if fields.len() < 6 {
            continue;
        }
        let (Some(seconds), Some(open), Some(high), Some(low), Some(close)) = (
            fields[0].as_f64(),
            fields[1].as_f64(),
            fields[2].as_f64(),
            fields[3].as_f64(),
            fields[4].as_f64(),
        ) else {
            continue;
        };
        let volume = fields[5].as_f64().unwrap_or(0.0);
        let open_time = (seconds * 1_000.0) as i64;
        candles.push(Kline {
            open_time,
            close_time: open_time + bucket_ms - 1,
            open,
            high,
            low,
            close,
            volume,
        });
    }
    Ok(candles)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR_MS: i64 = 3_600_000;

    #[test]
    fn rows_become_candles_and_bad_rows_are_skipped() {
        let body = r#"{"data":{"attributes":{"ohlcv_list":[
            [1700003600,11.0,13.0,10.0,12.0,5.5],
            [1700000000,10.0,12.0,9.0,11.0,42.5],
            [1700000000,10.0],
            ["x",1,1,1,1,1],
            5
        ]}}}"#;
        let candles = parse_candles(body, HOUR_MS).unwrap();
        assert_eq!(candles.len(), 2);
        assert_eq!(
            candles[1],
            Kline {
                open_time: 1_700_000_000_000,
                close_time: 1_700_003_599_999,
                open: 10.0,
                high: 12.0,
                low: 9.0,
                close: 11.0,
                volume: 42.5,
            }
        );
    }

    #[test]
    fn a_pool_without_a_list_has_no_data() {
        let error = parse_candles(r#"{"data":{"attributes":{}}}"#, HOUR_MS).unwrap_err();
        assert!(matches!(error, ParseError::NoData(_)));
        assert_eq!(error.to_string(), "No chart data for this pool");
        assert!(parse_candles("not json", HOUR_MS).is_err());
    }
}
