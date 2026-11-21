//! Gate.io's answers as candles: pure functions over the response text.

use crate::model::market::bucket::next_bucket_start_ms;
use crate::model::{Kline, Timeframe};
use crate::service::ProviderError;
use serde_json::Value;

/// Candles of length `base`, oldest first.
pub(super) fn parse_candles(body: &str, base: Timeframe) -> Result<Vec<Kline>, ProviderError> {
    let value: Value = serde_json::from_str(body).map_err(ProviderError::data)?;
    let mut candles = value
        .as_array()
        .ok_or_else(|| ProviderError::data("expected a candle array"))?
        .iter()
        .map(|row| parse_row(row, base))
        .collect::<Result<Vec<_>, _>>()?;
    candles.sort_by_key(|candle| candle.open_time);
    Ok(candles)
}

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
    let seconds: i64 = text(0)?
        .parse()
        .map_err(|_| ProviderError::data("invalid timestamp"))?;
    let open_time = seconds * 1_000;
    Ok(Kline {
        open_time,
        close_time: next_bucket_start_ms(base, open_time) - 1,
        open: number(5)?,
        high: number(3)?,
        low: number(4)?,
        close: number(2)?,
        volume: number(6)?,
    })
}
