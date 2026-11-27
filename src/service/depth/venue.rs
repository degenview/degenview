//! What a depth feed needs of an exchange. One implementation per exchange (`binance.rs`,
//! `bybit.rs`); the feed holds a `dyn DepthVenue` and never knows which exchange it runs on.

use super::{DepthError, binance, bybit};
use crate::model::depth::{BookSync, Event, Price, Snapshot};
use crate::model::exchange::{Exchange, Market, Venue};
use crate::service::net::ws::Session;

/// The sockets of one market, with whatever must be sent once connected.
pub struct Sessions {
    /// The order book, and the trades where one socket carries both. The book is only as good as
    /// this one: it going down lets the book go.
    pub book: Session,
    /// The trades, from an exchange that serves them apart. It going down costs only trades.
    pub trades: Option<Session>,
}

pub trait DepthVenue: Send + Sync {
    /// The sockets for the market `symbol` (the exchange's own name for it).
    fn sessions(&self, symbol: &str) -> Sessions;

    /// The smallest step the market's price moves in.
    fn tick_size(&self, symbol: &str) -> Result<Price, DepthError>;

    /// The rules that judge this exchange's book updates. Which book protocol an exchange speaks
    /// is stated here and nowhere else.
    fn book_sync(&self) -> Box<dyn BookSync>;

    /// The whole book, fetched over REST. Called only while the venue's [`BookSync`] says it
    /// `needs_snapshot`; a venue whose socket sends its own snapshots answers
    /// [`DepthError::Unsupported`] and is never asked.
    fn snapshot(&self, symbol: &str) -> Result<Snapshot, DepthError>;

    /// What one message of the socket says; nothing for a message that is not news (an
    /// acknowledgement, a heartbeat, another topic).
    fn parse(&self, text: &str) -> Vec<Event>;
}

/// The venue of one exchange's market.
pub fn for_venue(venue: Venue) -> Box<dyn DepthVenue> {
    match (venue.exchange, venue.market) {
        (Exchange::Binance, Market::Spot) => Box::new(binance::Binance),
        (Exchange::Binance, Market::Perpetual) => Box::new(binance::BinanceFutures),
        // No chart is a Bybit perpetual.
        (Exchange::Bybit, _) => Box::new(bybit::Bybit),
    }
}
