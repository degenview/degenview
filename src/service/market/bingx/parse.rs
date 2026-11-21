//! BingX's answers as values: pure functions over the response or message text.
//!
//! Every REST answer is `{"code":0,"timestamp":…,"data":…}`, failures included (HTTP 200 with a
//! `code` and a `msg`). A candle is a row `[open_ms, open, high, low, close, volume, close_ms,
//! quote_volume]`, newest first, from either candle endpoint; the volume is the base asset. A
//! candle of the socket is `{"code":0,"data":{"K":{"t":…,"o":"…","h":"…","l":"…","c":"…","v":"…",
//! "i":"1min","s":"BTC-USDT"},"e":"kline","s":"BTC-USDT"},"dataType":"BTC-USDT@kline_1min"}`.

use crate::model::market::bucket::next_bucket_start_ms;
use crate::model::{Kline, Timeframe};
use crate::service::ProviderError;
use crate::service::net::http;
use crate::service::watchlist_quotes::provider::number;
use serde_json::Value;

/// `code` of an answer that has what was asked.
const OK: i64 = 0;

/// `code` of an answer that has nothing for the request: no candles in the range asked (before the
/// market began, past what the endpoint keeps), or a market it does not know. One code for all, told
/// apart only by its text.
const NO_DATA: i64 = 100_204;

/// `code` of a client asking too often (BingX's documented code; not provoked while building this).
const RATE_LIMITED: i64 = 100_410;

/// What an answer holds.
#[derive(Debug, PartialEq)]
pub(super) enum Answer {
    Data(Value),
    /// Nothing for the request: see [`NO_DATA`].
    NoData,
}

impl Answer {
    /// The data, for a call that has no use for an answer of nothing.
    pub(super) fn data(self) -> Result<Value, ProviderError> {
        match self {
            Self::Data(data) => Ok(data),
            Self::NoData => Err(ProviderError::other("BingX has no data for the request")),
        }
    }
}

/// The answer of a REST call, or the provider's refusal of it.
pub(super) fn answer(body: &str) -> Result<Answer, ProviderError> {
    let mut value: Value = serde_json::from_str(body).map_err(ProviderError::data)?;
    let code = value
        .get("code")
        .and_then(Value::as_i64)
        .ok_or_else(|| ProviderError::data("expected a code"))?;
    match code {
        OK => Ok(Answer::Data(
            value
                .get_mut("data")
                .map(Value::take)
                .unwrap_or(Value::Null),
        )),
        NO_DATA => Ok(Answer::NoData),
        RATE_LIMITED => Err(ProviderError::Network(http::Error::Status(429))),
        _ => {
            let message = value
                .get("msg")
                .and_then(Value::as_str)
                .filter(|message| !message.is_empty())
                .unwrap_or("The request failed");
            Err(ProviderError::other(format!("BingX: {message}")))
        }
    }
}

/// One candle of length `base` from a row of a candle answer. It closes where this app's bucket
/// does, a millisecond before the next opens.
fn candle(row: &Value, base: Timeframe) -> Option<Kline> {
    let row = row.as_array()?;
    let open_time = number(row.first())? as i64;
    Some(Kline {
        open_time,
        close_time: next_bucket_start_ms(base, open_time) - 1,
        open: number(row.get(1))?,
        high: number(row.get(2))?,
        low: number(row.get(3))?,
        close: number(row.get(4))?,
        volume: number(row.get(5))?,
    })
}

/// The candles of a candle answer, oldest first. An answer of nothing is no candles: the range is
/// before the market or past what the endpoint keeps.
pub(super) fn parse_candles(body: &str, base: Timeframe) -> Result<Vec<Kline>, ProviderError> {
    let Answer::Data(data) = answer(body)? else {
        return Ok(Vec::new());
    };
    let mut candles = data
        .as_array()
        .ok_or_else(|| ProviderError::data("expected a candle list"))?
        .iter()
        .map(|row| candle(row, base).ok_or_else(|| ProviderError::data("invalid candle")))
        .collect::<Result<Vec<_>, _>>()?;
    candles.sort_by_key(|candle| candle.open_time);
    Ok(candles)
}

/// The market (as BingX names it) and the candle of length `base` that a `kline` push of `token`
/// (BingX's socket name for that length) says. Nothing for an acknowledgement, a refusal, a ping,
/// another topic, another length or a candle that cannot be read.
pub(super) fn pushed_candle(text: &str, token: &str, base: Timeframe) -> Option<(String, Kline)> {
    let value: Value = serde_json::from_str(text).ok()?;
    let data = value.get("data")?;
    if data.get("e")?.as_str()? != "kline" {
        return None;
    }
    let candle = data.get("K")?;
    if candle.get("i")?.as_str()? != token {
        return None;
    }
    let open_time = candle.get("t")?.as_i64()?;
    Some((
        data.get("s")?.as_str()?.to_owned(),
        Kline {
            open_time,
            close_time: next_bucket_start_ms(base, open_time) - 1,
            open: number(candle.get("o"))?,
            high: number(candle.get("h"))?,
            low: number(candle.get("l"))?,
            close: number(candle.get("c"))?,
            volume: number(candle.get("v"))?,
        },
    ))
}
