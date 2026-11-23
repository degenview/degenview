//! Bybit's answers as candles: pure functions over the response text.

use crate::model::market::bucket::next_bucket_start_ms;
use crate::model::{Kline, Timeframe};
use crate::service::ProviderError;
use crate::service::net::http;
use serde_json::Value;

/// Whether a `retCode` says to slow down: 10006 (too many visits) and 10018 (the IP limit).
pub(in crate::service) fn is_rate_limit(code: i64) -> bool {
    matches!(code, 10_006 | 10_018)
}

/// The `result` of an answer, or the provider's refusal.
pub(super) fn result_of(body: &str) -> Result<Value, ProviderError> {
    let mut value: Value = serde_json::from_str(body).map_err(ProviderError::data)?;
    let code = value
        .get("retCode")
        .and_then(Value::as_i64)
        .ok_or_else(|| ProviderError::data("expected a retCode"))?;
    if code != 0 {
        if is_rate_limit(code) {
            return Err(ProviderError::Network(http::Error::Status(429)));
        }
        let message = value
            .get("retMsg")
            .and_then(Value::as_str)
            .filter(|message| !message.is_empty())
            .unwrap_or("The request failed");
        return Err(ProviderError::other(format!("Bybit: {message}")));
    }
    value
        .get_mut("result")
        .map(Value::take)
        .ok_or_else(|| ProviderError::data("expected a result"))
}

/// Candles of length `base`, oldest first.
pub(super) fn parse_candles(body: &str, base: Timeframe) -> Result<Vec<Kline>, ProviderError> {
    let result = result_of(body)?;
    let mut candles = result
        .get("list")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::data("expected a candle list"))?
        .iter()
        .map(|row| parse_row(row, base))
        .collect::<Result<Vec<_>, _>>()?;
    candles.sort_by_key(|candle| candle.open_time);
    Ok(candles)
}

/// One kline of a push (`start`, `open`, …) is the same row as an object: see `live.rs`.
pub(super) fn parse_row(row: &Value, base: Timeframe) -> Result<Kline, ProviderError> {
    let text = |index: usize| -> Result<&str, ProviderError> {
        row.get(index)
            .and_then(Value::as_str)
            .ok_or_else(|| ProviderError::data(format!("invalid field {index}")))
    };
    let number = |index: usize| -> Result<f64, ProviderError> {
        text(index)?
            .parse()
            .map_err(|_| ProviderError::data(format!("invalid field {index}")))
    };
    let open_time: i64 = text(0)?
        .parse()
        .map_err(|_| ProviderError::data("invalid timestamp"))?;
    Ok(Kline {
        open_time,
        close_time: next_bucket_start_ms(base, open_time) - 1,
        open: number(1)?,
        high: number(2)?,
        low: number(3)?,
        close: number(4)?,
        volume: number(5)?,
    })
}
