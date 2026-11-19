//! Binance's `/api/v3/klines`.

use serde_json::Value;

use crate::fields::{candle_array, number};
use crate::{Kline, ParseError};

/// Candles, oldest first (Binance sends them that way): rows of
/// `[open time, open, high, low, close, volume, close time, …]`.
pub fn parse_candles(body: &str) -> Result<Vec<Kline>, ParseError> {
    candle_array(body)?.iter().map(parse_row).collect()
}

fn parse_row(row: &Value) -> Result<Kline, ParseError> {
    let time = |index: usize| -> Result<i64, ParseError> {
        row.get(index)
            .and_then(Value::as_i64)
            .ok_or_else(|| ParseError::invalid(format!("invalid timestamp {index}")))
    };
    Ok(Kline {
        open_time: time(0)?,
        open: number(row, 1)?,
        high: number(row, 2)?,
        low: number(row, 3)?,
        close: number(row, 4)?,
        volume: number(row, 5)?,
        close_time: time(6)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_binance_rows() {
        let body = r#"[[1700000000000,"10.0","12.0","9.0","11.0","42.5",1700003599999]]"#;
        let candles = parse_candles(body).expect("valid response");
        assert_eq!(
            candles,
            [Kline {
                open_time: 1_700_000_000_000,
                close_time: 1_700_003_599_999,
                open: 10.0,
                high: 12.0,
                low: 9.0,
                close: 11.0,
                volume: 42.5,
            }]
        );
    }

    #[test]
    fn rejects_error_objects() {
        let error = parse_candles(r#"{"code":-1121,"msg":"Invalid symbol."}"#)
            .expect_err("object must fail");
        assert_eq!(error.to_string(), "expected a candle array");
    }
}
