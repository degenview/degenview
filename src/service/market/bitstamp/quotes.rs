//! The price of a market from its trades: Bitstamp has no ticker channel, so the last trade of
//! `live_trades_<market>` is the price. A trade says nothing of the price 24 hours ago or the
//! volume, so the quote engine takes those from the REST call (`ticker.rs`): see
//! [`QuoteProtocol::ticks_carry_reference`].

use super::Bitstamp;
use super::parse::{asks_to_reconnect, trade};
use super::socket::trades_session;
use crate::model::DataSource;
use crate::model::watchlist::quote::SourceQuote;
use crate::service::net::ws::Session;
use crate::service::watchlist_quotes::quote_stream::{QuoteProtocol, StreamTick, TickReader};

/// The market and price of a trade message.
fn tick(text: &str) -> Option<StreamTick> {
    let (product_id, trade) = trade(text)?;
    Some(StreamTick {
        source: DataSource::Bitstamp,
        product_id,
        quote: SourceQuote {
            price: trade.price,
            timestamp_ms: Some(trade.time_ms),
            ..SourceQuote::default()
        },
    })
}

/// Reads the trades of one connection, and whether the server asked for a new one.
#[derive(Default)]
struct Prices {
    reconnect: bool,
}

impl TickReader for Prices {
    fn reset(&mut self) {
        self.reconnect = false;
    }

    fn read(&mut self, text: &str) -> Option<StreamTick> {
        if asks_to_reconnect(text) {
            self.reconnect = true;
            return None;
        }
        tick(text)
    }

    fn reconnect(&mut self) -> bool {
        std::mem::take(&mut self.reconnect)
    }
}

impl QuoteProtocol for Bitstamp {
    fn session(&self, products: &[String]) -> Session {
        trades_session(products)
    }

    /// A trade says its market, but the server's request to reconnect needs a reader's memory.
    fn parse(&self, _: &str) -> Option<StreamTick> {
        None
    }

    fn reader(&self) -> Option<Box<dyn TickReader>> {
        Some(Box::new(Prices::default()))
    }

    fn ticks_carry_reference(&self) -> bool {
        false
    }
}
