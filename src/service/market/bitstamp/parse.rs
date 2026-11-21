//! Bitstamp's answers as values: pure functions over the response or message text.
//!
//! A candle is `{"timestamp":"1795237200","open":"56222.13","high":…,"low":…,"close":…,
//! "volume":"41.70547100"}`: every number a string, the time the open in seconds, the volume in the
//! base asset. A trade of the socket is
//! `{"data":{"price":58029.52,"amount":0.00172369,"microtimestamp":"1795244813629000",…},
//! "channel":"live_trades_btcusd","event":"trade"}`.

use crate::model::market::bucket::next_bucket_start_ms;
use crate::model::market::live::Trade;
use crate::model::{Kline, Timeframe};
use crate::service::ProviderError;
use crate::service::watchlist_quotes::provider::number;
use serde_json::Value;

/// What a trade channel's name starts with; the market follows.
pub(super) const TRADES_CHANNEL: &str = "live_trades_";

/// The event with which the server, about to restart, asks its clients to connect again.
const RECONNECT_EVENT: &str = "bts:request_reconnect";

/// Whether `text` is the server's request to reconnect.
pub(super) fn asks_to_reconnect(text: &str) -> bool {
    text.contains(RECONNECT_EVENT)
        && serde_json::from_str::<Value>(text)
            .is_ok_and(|value| value.get("event").and_then(Value::as_str) == Some(RECONNECT_EVENT))
}

/// One candle of length `base` from a row of an `ohlc` answer. It closes where this app's bucket
/// does, a millisecond before the next opens.
fn candle(row: &Value, base: Timeframe) -> Option<Kline> {
    let open_time = number(row.get("timestamp"))? as i64 * 1_000;
    Some(Kline {
        open_time,
        close_time: next_bucket_start_ms(base, open_time) - 1,
        open: number(row.get("open"))?,
        high: number(row.get("high"))?,
        low: number(row.get("low"))?,
        close: number(row.get("close"))?,
        volume: number(row.get("volume"))?,
    })
}

/// The rows of an `ohlc` answer as candles of length `base`, in the order Bitstamp sent them, a
/// row that cannot be read as `None`: for a caller that would rather drop it than refuse the answer
/// (the daily closes of the exchange rates).
pub(in crate::service::market) fn candle_rows(
    body: &str,
    base: Timeframe,
) -> Result<Vec<Option<Kline>>, ProviderError> {
    let value: Value = serde_json::from_str(body).map_err(ProviderError::data)?;
    let rows = value
        .pointer("/data/ohlc")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::data("expected a candle list"))?;
    Ok(rows.iter().map(|row| candle(row, base)).collect())
}

/// The candles of an `ohlc` answer, oldest first. Bitstamp fills a minute nobody traded in with a
/// flat candle of no volume, so a market has a candle for every length since its first, and an
/// empty list is a range before it.
pub(super) fn parse_candles(body: &str, base: Timeframe) -> Result<Vec<Kline>, ProviderError> {
    let mut candles = candle_rows(body, base)?
        .into_iter()
        .map(|candle| candle.ok_or_else(|| ProviderError::data("invalid candle")))
        .collect::<Result<Vec<_>, _>>()?;
    candles.sort_by_key(|candle| candle.open_time);
    Ok(candles)
}

/// The market (upper case, as the app spells it) and the trade a `live_trades_<market>` message
/// says. Nothing for an acknowledgement, a heartbeat, an error, a request to reconnect or a trade
/// that cannot be read.
pub(super) fn trade(text: &str) -> Option<(String, Trade)> {
    let value: Value = serde_json::from_str(text).ok()?;
    if value.get("event")?.as_str()? != "trade" {
        return None;
    }
    let market = value
        .get("channel")?
        .as_str()?
        .strip_prefix(TRADES_CHANNEL)?
        .to_uppercase();
    let data = value.get("data")?;
    let microseconds = number(data.get("microtimestamp"))? as i64;
    let trade = Trade {
        time_ms: microseconds / 1_000,
        price: number(data.get("price"))?,
        size: number(data.get("amount"))?,
    };
    trade.is_sound().then_some((market, trade))
}
