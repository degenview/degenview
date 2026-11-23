//! Deribit's answers as values: pure functions over the response text or a pushed message.
//!
//! Every REST answer is JSON-RPC: `{"jsonrpc":"2.0","result":…}`, or `{"jsonrpc":"2.0","error":
//! {"code":…,"message":…,"data":{"reason":…}}}`. A candle answer holds parallel arrays (`ticks` of
//! the open times in milliseconds, `open`, `high`, `low`, `close`, `volume` in the base asset and
//! `cost` in dollars); a pushed candle is one object with `tick` and the same fields.

use crate::model::market::bucket::next_bucket_start_ms;
use crate::model::{Kline, Timeframe};
use crate::service::ProviderError;
use crate::service::net::http;
use serde_json::Value;

/// Deribit's code for a refusal because the allowance is used up.
const TOO_MANY_REQUESTS: i64 = 10_028;

/// The `result` of an answer, or the reason it is an error.
pub(super) fn answer(body: &str) -> Result<Value, ProviderError> {
    let mut value: Value = serde_json::from_str(body).map_err(ProviderError::data)?;
    if let Some(error) = value.get("error") {
        return Err(refusal(error));
    }
    match value.get_mut("result") {
        Some(result) => Ok(result.take()),
        None => Err(ProviderError::data("expected a Deribit answer")),
    }
}

/// An `error` object as a [`ProviderError`]: the allowance used up is the rate limit every other
/// provider's HTTP 429 is; the rest are the refusal in Deribit's words.
fn refusal(error: &Value) -> ProviderError {
    let code = error.get("code").and_then(Value::as_i64);
    if code == Some(TOO_MANY_REQUESTS) {
        return ProviderError::Network(http::Error::Status(429));
    }
    let text = |value: Option<&Value>| value.and_then(Value::as_str).map(str::to_owned);
    let message = text(error.get("message")).unwrap_or_else(|| "no reason given".to_owned());
    let reason = text(error.get("data").and_then(|data| data.get("reason")));
    ProviderError::other(match (code, reason) {
        (Some(code), Some(reason)) => format!("Deribit answered {code}: {message} ({reason})"),
        (Some(code), None) => format!("Deribit answered {code}: {message}"),
        (None, _) => format!("Deribit answered: {message}"),
    })
}

/// One candle of length `base`. It closes where this app's bucket does, a millisecond before the
/// next opens.
pub(super) fn kline(
    open_time: i64,
    [open, high, low, close]: [f64; 4],
    volume: f64,
    base: Timeframe,
) -> Kline {
    Kline {
        open_time,
        close_time: next_bucket_start_ms(base, open_time) - 1,
        open,
        high,
        low,
        close,
        volume,
    }
}

/// The values of the array `key` of a candle answer, each read by `read`.
fn column<T>(
    result: &Value,
    key: &str,
    read: fn(&Value) -> Option<T>,
) -> Result<Vec<T>, ProviderError> {
    result
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::data(format!("expected a list of {key}")))?
        .iter()
        .map(|value| read(value).ok_or_else(|| ProviderError::data(format!("invalid field {key}"))))
        .collect()
}

/// The candles of a `get_tradingview_chart_data` answer, oldest first. A window with no candle is
/// `"status":"no_data"` and empty arrays: no candles, not an error.
pub(super) fn parse_candles(body: &str, base: Timeframe) -> Result<Vec<Kline>, ProviderError> {
    let result = answer(body)?;
    let opens = column(&result, "ticks", Value::as_i64)?;
    let [open, high, low, close, volume] =
        ["open", "high", "low", "close", "volume"].map(|key| column(&result, key, Value::as_f64));
    let (open, high, low, close, volume) = (open?, high?, low?, close?, volume?);
    if [&open, &high, &low, &close, &volume]
        .iter()
        .any(|list| list.len() != opens.len())
    {
        return Err(ProviderError::data("the candle lists differ in length"));
    }
    let mut candles: Vec<Kline> = opens
        .iter()
        .enumerate()
        .map(|(index, open_time)| {
            kline(
                *open_time,
                [open[index], high[index], low[index], close[index]],
                volume[index],
                base,
            )
        })
        .collect();
    candles.sort_by_key(|candle| candle.open_time);
    Ok(candles)
}

/// The forming candle a `chart.trades` push carries in its `data`: the running totals of the candle
/// that opened at `tick`.
pub(super) fn pushed_candle(data: &Value, base: Timeframe) -> Option<Kline> {
    let field = |key: &str| data.get(key)?.as_f64();
    Some(kline(
        data.get("tick")?.as_i64()?,
        [
            field("open")?,
            field("high")?,
            field("low")?,
            field("close")?,
        ],
        field("volume")?,
        base,
    ))
}
