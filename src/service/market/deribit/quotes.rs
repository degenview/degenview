//! Deribit's `ticker.<instrument>.agg2` channel: the last trade, the 24 h change and the day's
//! turnover of a market, answered once when subscribed and pushed about once a second while it
//! changes (`100ms` is the faster choice; a watchlist does not need it). One connection, a
//! subscription per market; the push names its market by name.

use super::Deribit;
use super::markets::instrument_name;
use super::socket::{push, session};
use super::ticker::ticker_quote;
use crate::model::DataSource;
use crate::service::net::ws::Session;
use crate::service::watchlist_quotes::quote_stream::{QuoteProtocol, StreamTick};

const CHANNEL: &str = "ticker.";
const INTERVAL: &str = "agg2";

/// The channel of every one of `products` that is a name Deribit could list.
pub(super) fn ticker_channels(products: &[String]) -> Vec<String> {
    products
        .iter()
        .filter_map(|product| instrument_name(product))
        .map(|name| format!("{CHANNEL}{name}.{INTERVAL}"))
        .collect()
}

/// `{"jsonrpc":"2.0","method":"subscription","params":{"channel":"ticker.BTC-PERPETUAL.agg2",
/// "data":{"timestamp":1795237424233,"stats":{"price_change":-8.6794,"volume_usd":2364179910.0,…},
/// "instrument_name":"BTC-PERPETUAL","last_price":56190.0,…}}}`.
pub(super) fn parse_tick(text: &str) -> Option<StreamTick> {
    let (channel, data) = push(text)?;
    if !channel.starts_with(CHANNEL) {
        return None;
    }
    let (product_id, quote) = ticker_quote(&data)?;
    Some(StreamTick {
        source: DataSource::Deribit,
        product_id,
        quote,
    })
}

impl QuoteProtocol for Deribit {
    fn session(&self, products: &[String]) -> Session {
        session(&ticker_channels(products))
    }

    fn parse(&self, text: &str) -> Option<StreamTick> {
        parse_tick(text)
    }
}
