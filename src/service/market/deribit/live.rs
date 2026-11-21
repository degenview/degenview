//! Deribit's `chart.trades.<instrument>.<resolution>` channel: the forming candle of a market, sent
//! once when subscribed and again with each trade, its volume the running total of the candle.
//! All the markets of a chart share one connection; the channel names its market by name, so
//! nothing needs the market list.

use super::Deribit;
use super::markets::instrument_name;
use super::parse::pushed_candle;
use super::resolution::resolution;
use super::socket::{push, session};
use crate::model::{Kline, Timeframe};
use crate::service::market::live::CandleProtocol;
use crate::service::net::ws::{Frame, Session};

const CHANNEL: &str = "chart.trades.";

/// The channel of every one of `symbols` that is a name Deribit could list, whatever it is cased;
/// a name it does not list is dropped by Deribit when subscribing. `None` when Deribit has no
/// candle of `base`.
pub(super) fn candle_channels(symbols: &[String], base: Timeframe) -> Option<Vec<String>> {
    let resolution = resolution(base)?;
    Some(
        symbols
            .iter()
            .filter_map(|symbol| instrument_name(symbol))
            .map(|name| format!("{CHANNEL}{name}.{resolution}"))
            .collect(),
    )
}

/// `{"jsonrpc":"2.0","method":"subscription","params":{"channel":"chart.trades.BTC-PERPETUAL.1",
/// "data":{"close":56186.5,"high":56350.5,"low":56160.5,"open":56343.0,"tick":1795237380000,
/// "cost":1180980.0,"volume":20.99192082}}}`: the market's name and its forming candle.
pub(super) fn parse_candle(text: &str, base: Timeframe) -> Option<(String, Kline)> {
    let (channel, data) = push(text)?;
    let (name, _resolution) = channel.strip_prefix(CHANNEL)?.rsplit_once('.')?;
    Some((name.to_owned(), pushed_candle(&data, base)?))
}

impl CandleProtocol for Deribit {
    fn sessions(&self, symbols: &[String], base: Timeframe) -> Option<Vec<Session>> {
        let channels = candle_channels(symbols, base)?;
        (!channels.is_empty()).then(|| vec![session(&channels)])
    }

    fn parse(&self, frame: Frame<'_>, base: Timeframe) -> Option<(String, Kline)> {
        parse_candle(frame.text()?, base)
    }
}
