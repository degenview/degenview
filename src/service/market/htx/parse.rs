//! HTX's answers as candles: pure functions over the message text. A REST answer and a socket
//! `req` answer carry the same rows, and a pushed candle is one of them too.

use super::socket::HISTORY_ID;
use crate::model::market::bucket::next_bucket_start_ms;
use crate::model::{Kline, Timeframe};
use crate::service::ProviderError;
use crate::service::net::http;
use serde_json::Value;

/// A candle of length `base` from `{"id":1795233600,"open":…,"close":…,"low":…,"high":…,
/// "amount":891.98,"vol":5.0e7,"count":14089}`: `id` is the open time in seconds, `amount` the base
/// asset traded and `vol` the quote asset. The rows of a history answer and the `tick` of a pushed
/// candle are this one shape.
pub(super) fn candle(row: &Value, base: Timeframe) -> Option<Kline> {
    let number = |key: &str| row.get(key)?.as_f64();
    let open_time = row.get("id")?.as_i64()? * 1_000;
    Some(Kline {
        open_time,
        close_time: next_bucket_start_ms(base, open_time) - 1,
        open: number("open")?,
        high: number("high")?,
        low: number("low")?,
        close: number("close")?,
        volume: number("amount")?,
    })
}

/// The HTTP-like status a socket error message starts with: `429 too many request topic is …`. HTX
/// sends a refusal for a request that came too fast as an ordinary `bad-request`.
fn status_prefix(message: &str) -> Option<u16> {
    message.split_once(' ')?.0.parse().ok()
}

/// The provider's refusal in an answer whose `status` is not `ok`.
fn refusal(answer: &Value) -> ProviderError {
    let message = answer
        .get("err-msg")
        .or_else(|| answer.get("message"))
        .and_then(Value::as_str)
        .filter(|message| !message.is_empty())
        .unwrap_or("The request failed");
    match status_prefix(message) {
        Some(429) => ProviderError::Network(http::Error::Status(429)),
        _ => ProviderError::other(format!("HTX: {message}")),
    }
}

/// The answer itself, or the provider's refusal.
pub(super) fn checked(body: &str) -> Result<Value, ProviderError> {
    let answer: Value = serde_json::from_str(body).map_err(ProviderError::data)?;
    check(answer)
}

fn check(answer: Value) -> Result<Value, ProviderError> {
    match answer.get("status").and_then(Value::as_str) {
        Some("ok") => Ok(answer),
        Some(_) => Err(refusal(&answer)),
        None => Err(ProviderError::data("expected a status")),
    }
}

/// The `data` rows of an answer as candles, oldest first.
fn rows(answer: &Value, base: Timeframe) -> Result<Vec<Kline>, ProviderError> {
    let mut candles = answer
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::data("expected a candle list"))?
        .iter()
        .map(|row| candle(row, base).ok_or_else(|| ProviderError::data("invalid candle")))
        .collect::<Result<Vec<_>, _>>()?;
    candles.sort_by_key(|candle| candle.open_time);
    Ok(candles)
}

/// The answer of `/market/history/kline`: the newest candles, the forming one last once sorted.
pub(super) fn parse_klines(body: &str, base: Timeframe) -> Result<Vec<Kline>, ProviderError> {
    rows(&checked(body)?, base)
}

/// What a socket message says to the `req` of [`HISTORY_ID`]: its candles or its refusal. Nothing
/// for any other message (a push, another request's answer).
pub(super) fn history_reply(
    text: &str,
    base: Timeframe,
) -> Option<Result<Vec<Kline>, ProviderError>> {
    let answer: Value = serde_json::from_str(text).ok()?;
    if answer.get("id")?.as_str()? != HISTORY_ID {
        return None;
    }
    Some(check(answer).and_then(|answer| rows(&answer, base)))
}
