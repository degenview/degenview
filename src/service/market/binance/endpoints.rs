//! Where each of Binance's two markets is served. Everything that differs between spot and the
//! USDⓈ-M perpetuals is a value here, so the code that asks is the same for both.

use super::catalog::Pair;
use crate::model::exchange::Market;
use crate::service::market::catalog::Catalog;
use serde_json::Value;

pub(in crate::service) struct Endpoints {
    pub klines_url: &'static str,
    /// The list of markets the search reads.
    pub exchange_info_url: &'static str,
    /// Whether an entry of that list is a market of this kind that trades.
    pub lists: fn(&Value) -> bool,
    pub catalog: &'static Catalog<Pair>,
    /// The combined-stream socket of klines, without its `?streams=`.
    pub kline_stream_url: &'static str,
    pub stream_host: &'static str,
    pub ticker_url: &'static str,
    /// Whether the 24-hour ticker takes a list of symbols; else it answers one or all.
    pub batched_ticker: bool,
}

static SPOT_CATALOG: Catalog<Pair> = Catalog::new("catalog_binance");
static PERPETUAL_CATALOG: Catalog<Pair> = Catalog::new("catalog_binance_futures");

/// Plain spot `exchangeInfo` is 18 MB (every market ever listed, each with its filters and its
/// permission sets); the `TRADING` ones without permission sets are 2.6 MB.
static SPOT: Endpoints = Endpoints {
    klines_url: "https://api.binance.com/api/v3/klines",
    exchange_info_url: "https://api.binance.com/api/v3/exchangeInfo?symbolStatus=TRADING&showPermissionSets=false",
    lists: super::catalog::is_trading,
    catalog: &SPOT_CATALOG,
    kline_stream_url: "wss://stream.binance.com/stream",
    stream_host: "stream.binance.com",
    ticker_url: "https://api.binance.com/api/v3/ticker/24hr",
    batched_ticker: true,
};

/// The futures API takes no filter on its list (1.2 MB), and its kline streams are served only
/// under `/market`: any other path connects and says nothing.
static PERPETUAL: Endpoints = Endpoints {
    klines_url: "https://fapi.binance.com/fapi/v1/klines",
    exchange_info_url: "https://fapi.binance.com/fapi/v1/exchangeInfo",
    lists: super::catalog::is_trading_perpetual,
    catalog: &PERPETUAL_CATALOG,
    kline_stream_url: "wss://fstream.binance.com/market/stream",
    stream_host: "fstream.binance.com",
    ticker_url: "https://fapi.binance.com/fapi/v1/ticker/24hr",
    batched_ticker: false,
};

pub(in crate::service) fn endpoints(market: Market) -> &'static Endpoints {
    match market {
        Market::Spot => &SPOT,
        Market::Perpetual => &PERPETUAL,
    }
}
