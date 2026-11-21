//! The API Binance defined and others copied: klines as arrays of strings, an `exchangeInfo`
//! market list, a 24 h ticker, and one combined stream socket that names its topics
//! `<symbol>@kline_<interval>` and `<symbol>@ticker`.
//!
//! A venue is a [`Config`]: its hosts, its row limit and which listed markets it serves. Each file
//! holds one concern, the parsers pure over the response text and the I/O on [`Config`] kept apart
//! from them: `klines` and `rest` (candles and paging), `exchange_info` and `markets` (the market
//! list and search), `ticker` (watchlist prices), `stream` (the sockets).

mod exchange_info;
mod klines;
mod markets;
mod rest;
mod stream;
#[cfg(test)]
mod tests;
mod ticker;

pub(in crate::service) use exchange_info::{Pair, trading};
pub(in crate::service) use stream::parse_kline;
pub(in crate::service) use ticker::parse_quotes;

use crate::model::DataSource;
use crate::service::market::catalog::Catalog;
use serde_json::Value;
use std::time::Duration;

/// What differs between the venues that speak this API.
pub(in crate::service) struct Config {
    pub source: DataSource,
    /// The klines endpoint, without its query.
    pub klines_url: &'static str,
    /// The most rows one klines request returns.
    pub max_per_request: usize,
    pub exchange_info_url: &'static str,
    /// Whether a listed market (one entry of `exchangeInfo`'s `symbols`) is served: [`trading`], or
    /// [`perpetual`] too for a futures venue.
    pub serves: fn(&Value) -> bool,
    /// The venue's market list, held for a day.
    pub catalog: &'static Catalog<Pair>,
    /// The combined stream socket's host.
    pub stream_host: &'static str,
    /// The most topics one connection takes; a chart's markets past it get another connection.
    pub max_streams: usize,
    /// How long a socket may say nothing before it is reopened: short for a venue whose markets
    /// always trade, long for one that lists markets that sleep (stocks outside their session).
    pub max_silence: Duration,
}
