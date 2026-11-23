//! Bitfinex's `candles` channel: the forming candle of a market, resent with each trade and
//! accompanied by the candle before it. A subscription is answered by a `subscribed` event naming
//! the channel number, then a snapshot of the newest 240 candles, then updates; the pushes carry only
//! the number, so a connection keeps the numbers it was told ([`Channels`]). Every market is its own
//! subscription, so one unknown market (which Bitfinex answers with an empty snapshot) harms none,
//! and a connection takes only so many ([`CHANNELS_PER_CONNECTION`]), so a feed opens one per group.

use super::channels::{Channels, Subscription, message};
use super::parse::latest;
use super::socket::{CHANNELS_PER_CONNECTION, session, subscribe_candles};
use super::{Bitfinex, period, symbol};
use crate::model::{Kline, Timeframe};
use crate::service::market::live::{CandleProtocol, FrameReader};
use crate::service::net::ws::{Frame, Session};
use serde_json::Value;

/// Reads the candles of one connection.
struct Candles {
    channels: Channels,
    base: Timeframe,
}

impl Candles {
    /// The market and candle a message says: the push of a candle channel of this feed's length,
    /// the numbers of the others and the acknowledgements, heartbeats and refusals carrying none.
    fn candle(&mut self, text: &str) -> Option<(String, Kline)> {
        let value: Value = serde_json::from_str(text).ok()?;
        let (subscription, payload) = self.channels.track(message(&value))?;
        match subscription {
            Subscription::Candles { market, period }
                if Some(period.as_str()) == self::period(self.base).as_deref() =>
            {
                Some((market.clone(), latest(payload, self.base)?))
            }
            _ => None,
        }
    }
}

impl FrameReader for Candles {
    fn reset(&mut self) {
        self.channels.clear();
    }

    fn read(&mut self, frame: Frame<'_>) -> Option<(String, Kline)> {
        self.candle(frame.text()?)
    }

    fn reconnect(&mut self) -> bool {
        self.channels.take_reconnect()
    }
}

impl CandleProtocol for Bitfinex {
    fn sessions(&self, symbols: &[String], base: Timeframe) -> Option<Vec<Session>> {
        let period = period(base)?;
        let mut subscriptions: Vec<String> = Vec::new();
        for symbol in symbols.iter().filter_map(|symbol| self::symbol(symbol)) {
            let subscription = subscribe_candles(&period, &symbol);
            if !subscriptions.contains(&subscription) {
                subscriptions.push(subscription);
            }
        }
        let sessions: Vec<Session> = subscriptions
            .chunks(CHANNELS_PER_CONNECTION)
            .map(|group| session(group.to_vec()))
            .collect();
        (!sessions.is_empty()).then_some(sessions)
    }

    /// A candle names its channel, not its market: it is read through [`Self::reader`].
    fn parse(&self, _: Frame<'_>, _: Timeframe) -> Option<(String, Kline)> {
        None
    }

    fn reader(&self, base: Timeframe) -> Option<Box<dyn FrameReader>> {
        Some(Box::new(Candles {
            channels: Channels::default(),
            base,
        }))
    }
}
