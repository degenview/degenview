//! Where each of Binance's two markets is served. Everything that differs between spot and the
//! USDⓈ-M perpetuals is a value here, so the code that asks is the same for both.

use crate::model::exchange::Market;
use crate::model::{DataSource, Listing};
use serde_json::Value;

pub(in crate::service) struct Endpoints {
    /// The market list these are the endpoints of: whose failures they report.
    pub listing: Listing,
    pub klines_url: &'static str,
    /// The list of markets the search reads.
    pub exchange_info_url: &'static str,
    /// Whether an entry of that list is a market of this kind that trades.
    pub lists: fn(&Value) -> bool,
    /// The combined-stream socket of klines, without its `?streams=`.
    pub kline_stream_url: &'static str,
    pub stream_host: &'static str,
    pub ticker_url: &'static str,
    /// How many symbols the 24-hour ticker takes by name in one request; `None` for an API that
    /// ignores a list: it takes one name, or none and answers every market.
    pub ticker_batch: Option<usize>,
}

/// Plain spot `exchangeInfo` is 18 MB (every market ever listed, each with its filters and its
/// permission sets); the `TRADING` ones without permission sets are 2.6 MB.
static SPOT: Endpoints = Endpoints {
    listing: Listing::spot(DataSource::Binance),
    klines_url: "https://api.binance.com/api/v3/klines",
    exchange_info_url: "https://api.binance.com/api/v3/exchangeInfo?symbolStatus=TRADING&showPermissionSets=false",
    lists: super::catalog::is_trading,
    kline_stream_url: "wss://stream.binance.com/stream",
    stream_host: "stream.binance.com",
    ticker_url: "https://api.binance.com/api/v3/ticker/24hr",
    ticker_batch: Some(100),
};

/// The futures API takes no filter on its list (1.2 MB), and its kline streams are served only
/// under `/market`: any other path connects and says nothing.
static PERPETUAL: Endpoints = Endpoints {
    listing: Listing::BINANCE_PERPETUAL,
    klines_url: "https://fapi.binance.com/fapi/v1/klines",
    exchange_info_url: "https://fapi.binance.com/fapi/v1/exchangeInfo",
    lists: super::catalog::is_trading_perpetual,
    kline_stream_url: "wss://fstream.binance.com/market/stream",
    stream_host: "fstream.binance.com",
    ticker_url: "https://fapi.binance.com/fapi/v1/ticker/24hr",
    ticker_batch: None,
};

pub(in crate::service) fn endpoints(market: Market) -> &'static Endpoints {
    match market {
        Market::Spot => &SPOT,
        Market::Perpetual => &PERPETUAL,
    }
}
