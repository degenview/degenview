//! Lighter's `market_stats/<market>` channel: the last trade, the 24 h change and the day's
//! turnover of a market, answered once when subscribed and pushed about once a second. One
//! connection, a subscription per market.
//!
//! The channel is named by the market's number, which `session` resolves from the market list, so
//! it may wait for the network (it runs on the quote thread, not the UI's). When the list cannot
//! be had the socket has no subscription and the REST prices, asked every ten seconds, are all
//! there is. The pushed stats name their market by symbol.

use super::Lighter;
use super::catalog::{CATALOG, download};
use super::markets::{Market, ids_of};
use super::socket::session;
use super::ticker::stats_quote;
use crate::model::DataSource;
use crate::service::net::ws::Session;
use crate::service::watchlist_quotes::quote_stream::{QuoteProtocol, StreamTick};
use serde_json::Value;

fn quote_session(products: &[String]) -> Session {
    let channels = CATALOG
        .get(download)
        .map(|markets| stats_channels(&markets, products))
        .unwrap_or_default();
    session(&channels)
}

/// The channel of every one of `products` that Lighter lists.
pub(super) fn stats_channels(markets: &[Market], products: &[String]) -> Vec<String> {
    ids_of(markets, products)
        .map(|id| format!("market_stats/{id}"))
        .collect()
}

/// `{"channel":"market_stats:1","market_stats":{"symbol":"BTC","market_id":1,"last_trade_price":
/// "55839.2","daily_quote_token_volume":1726197258.8,"daily_price_change":-9.88,…},
/// "timestamp":1795235085178,"type":"update/market_stats"}`; the answer to the subscription is the
/// same under `subscribed/market_stats`.
pub(super) fn parse_tick(text: &str) -> Option<StreamTick> {
    let value: Value = serde_json::from_str(text).ok()?;
    if !matches!(
        value.get("type")?.as_str()?,
        "subscribed/market_stats" | "update/market_stats"
    ) {
        return None;
    }
    let (product_id, mut quote) = stats_quote(value.get("market_stats")?)?;
    quote.timestamp_ms = value.get("timestamp").and_then(Value::as_i64);
    Some(StreamTick {
        source: DataSource::Lighter,
        product_id,
        quote,
    })
}

impl QuoteProtocol for Lighter {
    fn session(&self, products: &[String]) -> Session {
        quote_session(products)
    }

    fn parse(&self, text: &str) -> Option<StreamTick> {
        parse_tick(text)
    }
}
