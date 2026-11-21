//! Hyperliquid's candles as `Kline`s: pure functions over the response text or a pushed message.
//!
//! A candle is an object, `{"t":open ms,"T":close ms,"s":"BTC","i":"1h","o":"…","c":"…","h":"…",
//! "l":"…","v":"…","n":trades}`, prices and volume as strings. Volume is in the base asset.

use crate::model::market::bucket::next_bucket_start_ms;
use crate::model::{Kline, Timeframe};
use crate::service::ProviderError;
use crate::service::watchlist_quotes::provider::number;
use serde_json::Value;

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

/// The candles of a `candleSnapshot` answer, oldest first.
pub(super) fn parse_candles(body: &str, base: Timeframe) -> Result<Vec<Kline>, ProviderError> {
    let value: Value = serde_json::from_str(body).map_err(ProviderError::data)?;
    let mut candles = value
        .as_array()
        .ok_or_else(|| ProviderError::data("expected a candle list"))?
        .iter()
        .map(|row| candle(row, base))
        .collect::<Result<Vec<_>, _>>()?;
    candles.sort_by_key(|candle| candle.open_time);
    Ok(candles)
}
