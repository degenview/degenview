//! Bitfinex's answers as values: pure functions over the response or message text.
//!
//! A candle is `[MTS, OPEN, CLOSE, HIGH, LOW, VOLUME]` (the close comes before the high), MTS the
//! open time in milliseconds, VOLUME in the base asset. A refusal is `["error", code, "message"]`,
//! with a status of 429 or 500 for the ones that matter here.

use crate::model::market::bucket::next_bucket_start_ms;
use crate::model::{Kline, Timeframe};
use crate::service::ProviderError;
use crate::service::net::http;
use serde_json::Value;

/// The `["error", code, …]` code of a client that asks too fast.
const RATE_LIMIT_CODE: i64 = 11010;

/// What a client too fast for Bitfinex is told, by HTTP 429 or by the `11010` refusal: the provider
/// said too many requests.
pub(super) fn too_fast() -> ProviderError {
    ProviderError::Network(http::Error::Status(429))
}

/// One candle of length `base` from a row `[MTS, OPEN, CLOSE, HIGH, LOW, VOLUME]`: a REST row and
/// the update of a candle channel are this one shape. It closes where this app's bucket does, a
/// millisecond before the next opens.
pub(super) fn candle(row: &Value, base: Timeframe) -> Option<Kline> {
    let row = row.as_array()?;
    let number = |index: usize| row.get(index)?.as_f64();
    let open_time = row.first()?.as_i64()?;
    Some(Kline {
        open_time,
        close_time: next_bucket_start_ms(base, open_time) - 1,
        open: number(1)?,
        close: number(2)?,
        high: number(3)?,
        low: number(4)?,
        volume: number(5)?,
    })
}

/// The refusal an `["error", code, "message"]` answer is, or `None` for anything else.
pub(super) fn refusal(value: &Value) -> Option<ProviderError> {
    let parts = value.as_array()?;
    if parts.first()?.as_str() != Some("error") {
        return None;
    }
    if parts.get(1).and_then(Value::as_i64) == Some(RATE_LIMIT_CODE) {
        return Some(too_fast());
    }
    let message = parts
        .get(2)
        .and_then(Value::as_str)
        .filter(|message| !message.is_empty())
        .unwrap_or("The request failed");
    Some(ProviderError::other(format!("Bitfinex: {message}")))
}

/// The candles of a `hist` answer, oldest first (Bitfinex sends the newest first). An empty list is
/// an empty answer, which is a market with nothing in the range or a client asking too fast: the
/// caller decides (`rest::page_before_with`).
pub(super) fn parse_candles(body: &str, base: Timeframe) -> Result<Vec<Kline>, ProviderError> {
    let value: Value = serde_json::from_str(body).map_err(ProviderError::data)?;
    if let Some(refusal) = refusal(&value) {
        return Err(refusal);
    }
    let mut candles = value
        .as_array()
        .ok_or_else(|| ProviderError::data("expected a candle list"))?
        .iter()
        .map(|row| candle(row, base).ok_or_else(|| ProviderError::data("invalid candle")))
        .collect::<Result<Vec<_>, _>>()?;
    candles.sort_by_key(|candle| candle.open_time);
    Ok(candles)
}

/// The newest candle a candle channel's push holds: the single row of an update, or the first of the
/// 240 rows of the snapshot that follows a subscription (an empty snapshot is a market with no
/// candle, which Bitfinex also gives a market it does not list).
pub(super) fn latest(payload: &Value, base: Timeframe) -> Option<Kline> {
    let rows = payload.as_array()?;
    if rows.first()?.is_array() {
        rows.iter()
            .filter_map(|row| candle(row, base))
            .max_by_key(|candle| candle.open_time)
    } else {
        candle(payload, base)
    }
}
