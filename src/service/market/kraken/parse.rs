//! Kraken's answers as candles: pure functions over the response text.

use crate::model::market::bucket::next_bucket_start_ms;
use crate::model::{Kline, Timeframe};
use crate::service::ProviderError;
use crate::service::net::http;
use serde_json::Value;

/// Whether a Kraken error code says to slow down: `EAPI:Rate limit exceeded`,
/// `EGeneral:Too many requests`, `EService:Throttled`, `EGeneral:Temporary lockout`.
pub(in crate::service) fn is_rate_limit(code: &str) -> bool {
    code.starts_with("EAPI:Rate limit")
        || code.starts_with("EService:Throttled")
        || code.starts_with("EGeneral:Temporary lockout")
        || code.starts_with("EGeneral:Too many requests")
}

/// The `result` of an answer, or the provider's refusal.
pub(in crate::service) fn result_of(body: &str) -> Result<Value, ProviderError> {
    let mut value: Value = serde_json::from_str(body).map_err(ProviderError::data)?;
    if let Some(error) = value
        .get("error")
        .and_then(Value::as_array)
        .and_then(|errors| errors.first())
        .and_then(Value::as_str)
    {
        return Err(refusal(error));
    }
    value
        .get_mut("result")
        .map(Value::take)
        .ok_or_else(|| ProviderError::data("expected a result"))
}

/// What a Kraken error code (`EAPI:Rate limit exceeded`, `EQuery:Unknown asset pair`) means.
pub(super) fn refusal(code: &str) -> ProviderError {
    if is_rate_limit(code) {
        return ProviderError::Network(http::Error::Status(429));
    }
    let text = code.split_once(':').map_or(code, |(_, text)| text);
    ProviderError::other(format!("Kraken: {text}"))
}

/// Candles of length `base`, oldest first.
pub(super) fn parse_ohlc(body: &str, base: Timeframe) -> Result<Vec<Kline>, ProviderError> {
    let result = result_of(body)?;
    let rows = result
        .as_object()
        .and_then(|result| result.iter().find(|(key, _)| key.as_str() != "last"))
        .and_then(|(_, rows)| rows.as_array())
        .ok_or_else(|| ProviderError::data("expected a candle array"))?;
    let mut candles = rows
        .iter()
        .map(|row| parse_row(row, base))
        .collect::<Result<Vec<_>, _>>()?;
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
        .ok_or_else(|| ProviderError::data("invalid timestamp"))?
        * 1_000;
    Ok(Kline {
        open_time,
        close_time: next_bucket_start_ms(base, open_time) - 1,
        open: number(1)?,
        high: number(2)?,
        low: number(3)?,
        close: number(4)?,
        volume: number(6)?,
    })
}
