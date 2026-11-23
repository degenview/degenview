//! MEXC's answers as candles: pure functions over the response text.

use crate::model::market::bucket::{bucket_start_ms, next_bucket_start_ms};
use crate::model::{Kline, Timeframe};
use crate::service::ProviderError;
use serde_json::Value;

/// Candles of length `base`, oldest first. The days, weeks and months MEXC cut at UTC+8 midnight
/// (its oldest history) are not on this app's grid and are left out.
pub(super) fn parse_candles(body: &str, base: Timeframe) -> Result<Vec<Kline>, ProviderError> {
    let value: Value = serde_json::from_str(body).map_err(ProviderError::data)?;
    let rows = match value.as_array() {
        Some(rows) => rows,
        None => {
            let message = value.get("msg").and_then(Value::as_str);
            return Err(message.map_or_else(
                || ProviderError::data("expected a candle array"),
                |message| ProviderError::other(format!("MEXC: {message}")),
            ));
        }
    };
    let mut candles = rows
        .iter()
        .map(|row| parse_row(row, base))
        .collect::<Result<Vec<_>, _>>()?;
    candles.retain(|candle| bucket_start_ms(base, candle.open_time) == candle.open_time);
    candles.sort_by_key(|candle| candle.open_time);
    Ok(candles)
}

pub(super) fn parse_row(row: &Value, base: Timeframe) -> Result<Kline, ProviderError> {
    let number = |index: usize| -> Result<f64, ProviderError> {
        row.get(index)
            .and_then(Value::as_str)
            .and_then(|text| text.parse().ok())
            .ok_or_else(|| ProviderError::data(format!("invalid field {index}")))
    };
    let open_time = row
        .get(0)
        .and_then(Value::as_i64)
        .ok_or_else(|| ProviderError::data("invalid timestamp"))?;
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
