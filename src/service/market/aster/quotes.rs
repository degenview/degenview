//! Aster's `<symbol>@ticker` topic: the last trade plus the rolling 24 h open and volume of every
//! market asked for, one `SUBSCRIBE` message naming them (the first 200, the socket's limit; the
//! others are priced over REST).

use super::{Aster, SHAPE};
use crate::model::DataSource;
use crate::service::market::shape::binance::parse_ticker;
use crate::service::net::ws::Session;
use crate::service::watchlist_quotes::quote_stream::{QuoteProtocol, StreamTick};

impl QuoteProtocol for Aster {
    fn session(&self, products: &[String]) -> Session {
        SHAPE.ticker_session(products)
    }

    fn parse(&self, text: &str) -> Option<StreamTick> {
        let (product_id, quote) = parse_ticker(text)?;
        Some(StreamTick {
            source: DataSource::Aster,
            product_id,
            quote,
        })
    }
}
