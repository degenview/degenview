//! Coinbase Exchange response shapes. The request itself (HTTP, paging, rate limits) is the
//! caller's.

use serde_json::Value;
use thiserror::Error;

use crate::Kline;

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    #[error("expected a candle array")]
    NotAnArray,
}

/// Reads a `/products/{id}/candles` body: rows of `[time s, low, high, open, close, volume]`,
/// newest first. The result is oldest first, a bar being `granularity_secs` long. A row that is
/// not six numbers is skipped (a missing volume reads as 0).
pub fn parse_candles(body: &str, granularity_secs: i64) -> Result<Vec<Kline>, ParseError> {
    let value: Value = serde_json::from_str(body)?;
    let rows = value.as_array().ok_or(ParseError::NotAnArray)?;
    let mut candles = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(fields) = row.as_array() else {
            continue;
        };
        if fields.len() < 6 {
            continue;
        }
        let Some(seconds) = fields[0].as_f64() else {
            continue;
        };
        let Some(low) = fields[1].as_f64() else {
            continue;
        };
        let Some(high) = fields[2].as_f64() else {
            continue;
        };
        let Some(open) = fields[3].as_f64() else {
            continue;
        };
        let Some(close) = fields[4].as_f64() else {
            continue;
        };
        let volume = fields[5].as_f64().unwrap_or(0.0);
        let open_time = (seconds * 1_000.0) as i64;
        candles.push(Kline {
            open_time,
            close_time: open_time + granularity_secs * 1_000 - 1,
            open,
            high,
            low,
            close,
            volume,
        });
    }
    candles.sort_by_key(|candle| candle.open_time);
    Ok(candles)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BODY: &str =
        "[[1700003600,99.0,103.0,101.0,102.5,12.5],[1700000000,98.0,102.0,100.0,101.0,9.0]]";

    #[test]
    fn rows_come_back_oldest_first_with_their_columns() {
        let candles = parse_candles(BODY, 3600).unwrap();
        assert_eq!(candles.len(), 2);
        assert_eq!(candles[0].open_time, 1_700_000_000_000);
        assert_eq!(
            candles[1],
            Kline {
                open_time: 1_700_003_600_000,
                close_time: 1_700_007_199_999,
                open: 101.0,
                high: 103.0,
                low: 99.0,
                close: 102.5,
                volume: 12.5,
            }
        );
        assert!(candles[1].is_up());
    }

    #[test]
    fn a_malformed_row_is_skipped_and_a_missing_volume_is_zero() {
        let body = r#"[[1700000000,98.0,102.0,100.0,101.0,null],[1700003600,"x",1,1,1,1],[1],5]"#;
        let candles = parse_candles(body, 60).unwrap();
        assert_eq!(candles.len(), 1);
        assert_eq!(candles[0].volume, 0.0);
    }

    #[test]
    fn an_error_object_is_not_a_candle_array() {
        let error = parse_candles(r#"{"message":"NotFound"}"#, 60).unwrap_err();
        assert!(matches!(error, ParseError::NotAnArray));
        assert_eq!(error.to_string(), "expected a candle array");
        assert!(matches!(
            parse_candles("<html>", 60),
            Err(ParseError::Json(_))
        ));
    }
}
