//! OKX's answers as candles: pure functions over the response text.

use crate::model::market::bucket::next_bucket_start_ms;
use crate::model::{Kline, Timeframe};
use crate::service::ProviderError;
use serde_json::Value;

/// The `data` rows of an answer, or OKX's own message when its `code` says it failed.
pub(super) fn data_rows(body: &str) -> Result<Vec<Value>, ProviderError> {
    let value: Value = serde_json::from_str(body).map_err(ProviderError::data)?;
    if value.get("code").and_then(Value::as_str) != Some("0") {
        let message = value
            .get("msg")
            .and_then(Value::as_str)
            .filter(|message| !message.is_empty())
            .unwrap_or("The request failed");
        return Err(ProviderError::other(format!("OKX: {message}")));
    }
    value
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| ProviderError::data("expected a data array"))
}

/// Candles of length `base`, oldest first.
pub(super) fn parse_candles(body: &str, base: Timeframe) -> Result<Vec<Kline>, ProviderError> {
    let mut candles = data_rows(body)?
        .iter()
        .map(|row| parse_row(row, base))
        .collect::<Result<Vec<_>, _>>()?;
    candles.sort_by_key(|candle| candle.open_time);
    Ok(candles)
}

pub(in crate::service) fn parse_row(row: &Value, base: Timeframe) -> Result<Kline, ProviderError> {
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
