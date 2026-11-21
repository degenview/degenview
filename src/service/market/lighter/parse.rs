//! Lighter's answers as values: pure functions over the response text or a pushed message.
//!
//! Every REST answer is an object that opens with `"code"` (200, or the code of the refusal and its
//! `"message"`). A candle is `{"t":open ms,"o":…,"h":…,"l":…,"c":…,"v":base volume,"V":quote
//! volume,"i":last trade index}`, prices and volumes as numbers.

use crate::model::market::bucket::next_bucket_start_ms;
use crate::model::{Kline, Timeframe};
use crate::service::ProviderError;
use crate::service::watchlist_quotes::provider::number;
use serde_json::Value;

/// The object an answer is, when its `code` says it is one.
pub(super) fn answer(body: &str) -> Result<Value, ProviderError> {
    let value: Value = serde_json::from_str(body).map_err(ProviderError::data)?;
    match value.get("code").and_then(Value::as_i64) {
        Some(200) => Ok(value),
        Some(code) => Err(ProviderError::other(format!(
            "Lighter answered {code}: {}",
            value
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("no reason given")
        ))),
        None => Err(ProviderError::data("expected a Lighter answer")),
    }
}

/// One candle of length `base`. It closes where this app's bucket does, a millisecond before the
/// next opens.
pub(super) fn candle(row: &Value, base: Timeframe) -> Result<Kline, ProviderError> {
    let field = |key: &str| {
        number(row.get(key)).ok_or_else(|| ProviderError::data(format!("invalid field {key}")))
    };
    let open_time = row
        .get("t")
        .and_then(Value::as_i64)
        .ok_or_else(|| ProviderError::data("invalid timestamp"))?;
    Ok(Kline {
        open_time,
        close_time: next_bucket_start_ms(base, open_time) - 1,
        open: field("o")?,
        high: field("h")?,
        low: field("l")?,
        close: field("c")?,
        volume: field("v")?,
    })
}

/// The candles of a `/candles` answer, oldest first.
pub(super) fn parse_candles(body: &str, base: Timeframe) -> Result<Vec<Kline>, ProviderError> {
    let value = answer(body)?;
    let mut candles = value
        .get("c")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::data("expected a candle list"))?
        .iter()
        .map(|row| candle(row, base))
        .collect::<Result<Vec<_>, _>>()?;
    candles.sort_by_key(|candle| candle.open_time);
    Ok(candles)
}
