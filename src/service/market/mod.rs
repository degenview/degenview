pub(in crate::service) mod alpaca;
pub(in crate::service) mod binance;
pub(in crate::service) mod bybit;
pub(in crate::service) mod candles;
pub mod catalog;
pub mod cmc;
pub mod coin_index;
pub(in crate::service) mod coinbase;
pub(in crate::service) mod coingecko;
pub(in crate::service) mod dexscreener;
pub mod error;
pub mod fx_rates;
pub(in crate::service) mod gateio;
pub(in crate::service) mod geckoterminal;
pub(in crate::service) mod kalshi;
pub(in crate::service) mod kline_endpoint;
pub(in crate::service) mod kraken;
pub mod live;
pub(in crate::service) mod mexc;
pub mod names;
pub(in crate::service) mod oanda;
pub(in crate::service) mod okx;
pub(in crate::service) mod paging;
pub(in crate::service) mod polymarket;
pub mod retry;
pub mod search_rank;
pub mod trend;

use crate::model::DataSource;
use error::ErrorKind;

/// What each venue's HTTP statuses mean beyond a rate limit (`ProviderError::http`): its own table,
/// declared beside the code that asks it. A venue with no table of its own lists none.
pub(in crate::service) fn statuses(source: DataSource) -> &'static [(u16, ErrorKind)] {
    match source {
        DataSource::Alpaca => alpaca::STATUSES,
        DataSource::Binance => binance::STATUSES,
        DataSource::Bybit => bybit::STATUSES,
        DataSource::Coinbase => coinbase::STATUSES,
        DataSource::Gate => gateio::STATUSES,
        DataSource::Mexc => mexc::STATUSES,
        DataSource::Oanda => oanda::STATUSES,
        DataSource::CoinMarketCap => cmc::STATUSES,
        DataSource::CoinGecko
        | DataSource::DexScreener
        | DataSource::Kalshi
        | DataSource::Kraken
        | DataSource::Okx
        | DataSource::Polymarket => &[],
    }
}
