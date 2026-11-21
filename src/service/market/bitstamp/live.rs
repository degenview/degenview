//! The live candle of a market, built from its trades: Bitstamp's socket has no candle channel, so
//! each market of a connection has a [`TradeFold`] that cuts the trades into candles of the feed's
//! length and sends the forming one on after every trade.
//!
//! Unlike a candle channel's, that candle is not the whole bar: it holds the trades seen since the
//! feed began, which the chart adds to the candle it holds of the bar ([`Klines::Trades`]), so the
//! bar keeps the open and the range it had before the socket connected.

use super::parse::{asks_to_reconnect, trade};
use super::socket::trades_session;
use super::{Bitstamp, step};
use crate::model::market::live::{Klines, TradeFold};
use crate::model::{Kline, Timeframe};
use crate::service::market::live::{CandleProtocol, FrameReader};
use crate::service::net::ws::{Frame, Session};
use std::collections::HashMap;

/// Reads the trades of one connection into the forming candle of each market.
struct Trades {
    base: Timeframe,
    folds: HashMap<String, TradeFold>,
    /// The server asked for a new connection and nobody has acted on it yet.
    reconnect: bool,
}

impl FrameReader for Trades {
    /// A new connection keeps the candles: the trades missed in between are not in them, which the
    /// next REST account of the bar restores, but a fold that started over would send klines with
    /// less volume than the last, which the chart takes for an older account of the bar and drops.
    fn reset(&mut self) {
        self.reconnect = false;
    }

    fn read(&mut self, frame: Frame<'_>) -> Option<(String, Kline)> {
        let text = frame.text()?;
        if asks_to_reconnect(text) {
            self.reconnect = true;
            return None;
        }
        let (market, trade) = trade(text)?;
        let candle = self
            .folds
            .entry(market.clone())
            .or_default()
            .apply(self.base, trade)?;
        Some((market, candle))
    }

    fn reconnect(&mut self) -> bool {
        std::mem::take(&mut self.reconnect)
    }
}

impl CandleProtocol for Bitstamp {
    fn sessions(&self, symbols: &[String], base: Timeframe) -> Option<Vec<Session>> {
        step(base)?;
        let session = trades_session(symbols);
        (!session.subscribe.is_empty()).then(|| vec![session])
    }

    /// A trade names its channel, not its market: it is read through [`Self::reader`].
    fn parse(&self, _: Frame<'_>, _: Timeframe) -> Option<(String, Kline)> {
        None
    }

    /// The candle holds the trades since the socket connected: it adds to what the chart holds of
    /// the bar, which Bitstamp's REST account of it fills.
    fn klines(&self) -> Klines {
        Klines::Trades
    }

    fn reader(&self, base: Timeframe) -> Option<Box<dyn FrameReader>> {
        Some(Box::new(Trades {
            base,
            folds: HashMap::new(),
            reconnect: false,
        }))
    }
}
