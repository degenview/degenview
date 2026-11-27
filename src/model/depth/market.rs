//! Which markets have an order book to show.

use crate::model::exchange::{Exchange, Market, Venue};
use crate::model::{DataSource, InstrumentId};

/// A market on an exchange whose book can be followed: the key of one feed.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct DepthMarket {
    pub venue: Venue,
    /// The exchange's own name for it: `BTCUSDT`.
    pub symbol: String,
}

/// The book of the market a chart shows, when its exchange is one that sends books. Charts are
/// spot, so these are spot books.
pub fn market_of(source: DataSource, symbol: &str) -> Option<DepthMarket> {
    let exchange = match source {
        DataSource::Binance => Exchange::Binance,
        DataSource::Bybit => Exchange::Bybit,
        _ => return None,
    };
    Some(DepthMarket {
        venue: Venue {
            exchange,
            market: Market::Spot,
        },
        symbol: InstrumentId::new(source, symbol).canonical_symbol(),
    })
}
