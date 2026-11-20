//! What a depth feed needs of an exchange. One implementation per exchange (`binance.rs`,
//! `bybit.rs`); the feed holds a `dyn DepthVenue` and never knows which exchange it runs on.

use super::{DepthError, binance, bybit};
use crate::model::depth::{Event, Price, Snapshot};
use crate::model::exchange::Exchange;
use crate::service::net::ws::Session;

pub trait DepthVenue: Send + Sync {
    /// The socket for the market `symbol` (the exchange's own name for it): the order book and
    /// the trades, with whatever must be sent once connected.
    fn session(&self, symbol: &str) -> Session;

    /// The smallest step the market's price moves in.
    fn tick_size(&self, symbol: &str) -> Result<Price, DepthError>;

    /// The whole book, fetched when the exchange's socket does not send it. `None` for an
    /// exchange whose socket opens with a snapshot of its own.
    fn snapshot(&self, symbol: &str) -> Option<Result<Snapshot, DepthError>>;

    /// What one message of the socket says; nothing for a message that is not news (an
    /// acknowledgement, a heartbeat, another topic).
    fn parse(&self, text: &str) -> Vec<Event>;
}

/// The venue of `exchange`.
pub fn for_exchange(exchange: Exchange) -> Box<dyn DepthVenue> {
    match exchange {
        Exchange::Binance => Box::new(binance::Binance),
        Exchange::Bybit => Box::new(bybit::Bybit),
    }
}
