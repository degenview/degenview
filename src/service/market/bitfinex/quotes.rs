//! Bitfinex's `ticker` channel: the last trade, the 24 hour change and the volume of a market, pushed
//! as it trades. Every market is its own subscription (a refused one harms none) and the pushes name
//! only their channel number, so the connection keeps the numbers it was told ([`Channels`]). One
//! connection takes only so many ([`CHANNELS_PER_CONNECTION`]): the markets of a watchlist beyond
//! them are priced by REST, which the engine falls back to for a market the socket never answered.

use super::channels::{Channels, Subscription, message};
use super::socket::{CHANNELS_PER_CONNECTION, session, subscribe_ticker};
use super::ticker::quote;
use super::{Bitfinex, symbol};
use crate::model::DataSource;
use crate::service::net::ws::Session;
use crate::service::watchlist_quotes::quote_stream::{QuoteProtocol, StreamTick, TickReader};
use serde_json::Value;

/// Reads the tickers of one connection.
#[derive(Default)]
struct Tickers {
    channels: Channels,
}

impl Tickers {
    /// The market and price a message says; nothing for an acknowledgement, a heartbeat, a refusal
    /// or the push of a channel this connection was not told about.
    fn tick(&mut self, text: &str) -> Option<StreamTick> {
        let value: Value = serde_json::from_str(text).ok()?;
        let (subscription, payload) = self.channels.track(message(&value))?;
        let Subscription::Ticker { market } = subscription else {
            return None;
        };
        Some(StreamTick {
            source: DataSource::Bitfinex,
            product_id: market.clone(),
            quote: quote(payload.as_array()?)?,
        })
    }
}

impl TickReader for Tickers {
    fn reset(&mut self) {
        self.channels.clear();
    }

    fn read(&mut self, text: &str) -> Option<StreamTick> {
        self.tick(text)
    }

    fn reconnect(&mut self) -> bool {
        self.channels.take_reconnect()
    }
}

impl QuoteProtocol for Bitfinex {
    fn session(&self, products: &[String]) -> Session {
        let mut subscriptions: Vec<String> = Vec::new();
        for symbol in products.iter().filter_map(|product| symbol(product)) {
            let subscription = subscribe_ticker(&symbol);
            if !subscriptions.contains(&subscription) {
                subscriptions.push(subscription);
            }
        }
        subscriptions.truncate(CHANNELS_PER_CONNECTION);
        session(subscriptions)
    }

    /// A ticker names its channel, not its market: it is read through [`Self::reader`].
    fn parse(&self, _: &str) -> Option<StreamTick> {
        None
    }

    fn reader(&self) -> Option<Box<dyn TickReader>> {
        Some(Box::new(Tickers::default()))
    }
}
