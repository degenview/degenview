//! BingX's `<market>@kline_<length>` topics: the forming candle of a market, pushed about every
//! half second, its volume the running total of the candle. Each market is its own topic, and an
//! unknown one is refused alone, so the markets of a chart share a connection up to the 200 topics
//! it takes. The push names its market and its length.
//!
//! The prices of a push have the digit beyond the market's tick that BingX's history has
//! (`precision.rs`), so they are cut at the tick of the market (from the market list the chart's
//! candle request holds in memory, read once and again only while it was not there) to be the same
//! bar REST makes of the same minutes.

use super::markets::price_decimals;
use super::parse::pushed_candle;
use super::precision::cut_candle;
use super::socket::{kline_topic, sessions, subscribe, token};
use super::{Bingx, market_id};
use crate::model::{Kline, Timeframe};
use crate::service::market::live::{CandleProtocol, FrameReader};
use crate::service::net::ws::{Frame, Session};
use std::collections::HashMap;

/// Reads the candles of one connection.
pub(super) struct Candles {
    token: &'static str,
    base: Timeframe,
    /// The digits of each market's prices; empty until the market list is held.
    pub(super) decimals: HashMap<String, u32>,
    /// Where the digits are had from while there are none.
    pub(super) learn: fn() -> HashMap<String, u32>,
}

impl Candles {
    /// A reader of candles of `base`, cutting each market's prices at its `decimals`; `None` for a
    /// length the socket has no name for.
    pub(super) fn new(base: Timeframe, decimals: HashMap<String, u32>) -> Option<Self> {
        Some(Self {
            token: token(base)?,
            base,
            decimals,
            learn: price_decimals,
        })
    }
}

impl FrameReader for Candles {
    /// A new connection keeps the digits: they are facts of the markets, not of the connection.
    fn reset(&mut self) {}

    fn read(&mut self, frame: Frame<'_>) -> Option<(String, Kline)> {
        if self.decimals.is_empty() {
            self.decimals = (self.learn)();
        }
        let (market, candle) = pushed_candle(frame.text()?, self.token, self.base)?;
        let candle = match self.decimals.get(&market) {
            Some(decimals) => cut_candle(candle, *decimals),
            None => candle,
        };
        Some((market, candle))
    }
}

impl CandleProtocol for Bingx {
    fn sessions(&self, symbols: &[String], base: Timeframe) -> Option<Vec<Session>> {
        let token = token(base)?;
        let mut subscriptions: Vec<String> = Vec::new();
        for id in symbols.iter().filter_map(|symbol| market_id(symbol)) {
            let subscription = subscribe(&kline_topic(&id, token));
            if !subscriptions.contains(&subscription) {
                subscriptions.push(subscription);
            }
        }
        (!subscriptions.is_empty()).then(|| sessions(subscriptions))
    }

    /// A candle's price is cut to the market's digits, which only [`Self::reader`] knows.
    fn parse(&self, _: Frame<'_>, _: Timeframe) -> Option<(String, Kline)> {
        None
    }

    fn reader(&self, base: Timeframe) -> Option<Box<dyn FrameReader>> {
        Some(Box::new(Candles::new(base, price_decimals())?))
    }
}
