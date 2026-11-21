//! Lighter's `candle/<market>/<resolution>` channel: the forming candle of a market, answered once
//! when subscribed and pushed twice a second while it trades. All the markets of a chart share one
//! connection. The channel names its market by number, which the market list held in memory turns
//! into the symbol (and which tells a socket that starts before the list is held, `None`, to wait for
//! the next frame).

use super::Lighter;
use super::catalog::CATALOG;
use super::interval;
use super::markets::{Market, ids_of, symbol_of};
use super::parse::candle;
use super::socket::session;
use crate::model::{Kline, Timeframe};
use crate::service::market::live::CandleProtocol;
use crate::service::net::ws::{Frame, Session};
use serde_json::Value;

/// `None` when Lighter has no candle of `base`, or the market list is not held yet or lists none of
/// `symbols`: this runs on the UI thread, which never waits for the list (the chart that asked for
/// these markets is fetching it, and the feed is asked for again until it is there).
fn candle_sessions(symbols: &[String], base: Timeframe) -> Option<Vec<Session>> {
    let resolution = interval(base)?;
    let channels = candle_channels(&CATALOG.held()?, symbols, &resolution);
    (!channels.is_empty()).then(|| vec![session(&channels)])
}

/// The channel of every one of `symbols` that Lighter lists.
pub(super) fn candle_channels(
    markets: &[Market],
    symbols: &[String],
    resolution: &str,
) -> Vec<String> {
    ids_of(markets, symbols)
        .map(|id| format!("candle/{id}/{resolution}"))
        .collect()
}

/// `{"candles":[{"t":1795235040000,"o":55777.5,"h":55811.4,"l":55755.1,"c":55755.1,"v":4.98637,
/// "V":278122.9,"i":41936937725}],"channel":"candle:1:1m","timestamp":1795235054631,
/// "type":"update/candle"}`: the market's number and its forming candle, whose volume is the running
/// total. The answer to the subscription has the same shape under `subscribed/candle`.
pub(super) fn parse_candle(text: &str, base: Timeframe) -> Option<(u32, Kline)> {
    let value: Value = serde_json::from_str(text).ok()?;
    if !matches!(
        value.get("type")?.as_str()?,
        "subscribed/candle" | "update/candle"
    ) {
        return None;
    }
    let mut parts = value.get("channel")?.as_str()?.split(':');
    let (Some("candle"), Some(id)) = (parts.next(), parts.next()) else {
        return None;
    };
    let kline = candle(value.get("candles")?.as_array()?.last()?, base).ok()?;
    Some((id.parse().ok()?, kline))
}

impl CandleProtocol for Lighter {
    fn sessions(&self, symbols: &[String], base: Timeframe) -> Option<Vec<Session>> {
        candle_sessions(symbols, base)
    }

    fn parse(&self, frame: Frame<'_>, base: Timeframe) -> Option<(String, Kline)> {
        let (id, kline) = parse_candle(frame.text()?, base)?;
        let markets = CATALOG.held()?;
        Some((symbol_of(&markets, id)?.to_owned(), kline))
    }
}
