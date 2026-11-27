//! Which markets have an order book to show.

use crate::model::exchange::{Exchange, Venue};
use crate::model::market::binance::split_market;
use crate::model::{DataSource, InstrumentId};

/// A market on an exchange whose book can be followed: the key of one feed.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct DepthMarket {
    pub venue: Venue,
    /// The exchange's own name for it: `BTCUSDT`.
    pub symbol: String,
}

/// The book of the market a chart shows, when its exchange is one that sends books.
pub fn market_of(source: DataSource, symbol: &str) -> Option<DepthMarket> {
    let exchange = match source {
        DataSource::Binance => Exchange::Binance,
        DataSource::Bybit => Exchange::Bybit,
        _ => return None,
    };
    let instrument = InstrumentId::new(source, symbol);
    let canonical = instrument.canonical_symbol();
    // The exchange's own name of the pair: Binance's APIs do not take the app's suffix.
    let name = match exchange {
        Exchange::Binance => split_market(&canonical).0,
        Exchange::Bybit => &canonical,
    };
    Some(DepthMarket {
        venue: Venue {
            exchange,
            market: instrument.market(),
        },
        symbol: name.to_owned(),
    })
}
