//! Why a market provider gave no candles or no search results. The messages are the ones the
//! chart, the symbol picker and MCP show; code decides on the variant, never on the text.

use super::alpaca::MISSING_KEY;
use crate::model::{DataSource, Timeframe};
use crate::service::net::http;

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ProviderError {
    /// The request failed, or the provider answered with an error status.
    #[error("Network error: {0}")]
    Network(#[from] http::Error),
    /// The same for Alpaca, whose failures name it: they are usually its keys.
    #[error("Alpaca error: {0}")]
    Alpaca(http::Error),
    /// The same for CoinMarketCap, whose failures name it: they are usually its key or plan.
    #[error("CoinMarketCap error: {0}")]
    CoinMarketCap(http::Error),
    /// The provider answered with something other than what it documents.
    #[error("Data error: {0}")]
    Data(String),
    /// No Alpaca keys are saved, so stocks can be neither searched nor charted.
    #[error("{MISSING_KEY}")]
    MissingKey,
    /// The provider has nothing for this market, interval or range, or refused it in its own words.
    #[error("{0}")]
    Other(String),
    /// The provider lists no market of this name: asking again is no use until it lists one.
    #[error("{}", unknown_market_message(.provider, .symbol, .hint))]
    UnknownMarket {
        provider: DataSource,
        symbol: String,
        /// How the provider writes its names, or empty.
        hint: &'static str,
    },
    /// No candle of the source reaches this timeframe: it is never answered with a nearby one.
    #[error("{} has no {timeframe} history", .provider.display_name())]
    UnsupportedInterval {
        provider: DataSource,
        timeframe: Timeframe,
    },
}

impl ProviderError {
    /// `provider` lists no market `symbol`. `hint` says how it writes its names, or is empty.
    pub fn unknown_market(
        provider: DataSource,
        symbol: impl Into<String>,
        hint: &'static str,
    ) -> Self {
        Self::UnknownMarket {
            provider,
            symbol: symbol.into(),
            hint,
        }
    }

    /// A [`Self::Data`] from a parser's error.
    pub fn data(error: impl std::fmt::Display) -> Self {
        Self::Data(error.to_string())
    }

    pub fn other(message: impl Into<String>) -> Self {
        Self::Other(message.into())
    }

    /// The provider answered HTTP 403, which it gives a country it does not serve.
    pub fn refused_by_region(provider: DataSource) -> Self {
        Self::Other(format!(
            "{} refused the request (HTTP 403): it is not available in every country",
            provider.display_name()
        ))
    }

    /// `provider` cannot chart `timeframe`.
    pub fn unsupported(provider: DataSource, timeframe: Timeframe) -> Self {
        Self::UnsupportedInterval {
            provider,
            timeframe,
        }
    }

    /// The source cannot chart the timeframe, so asking again at it is no use.
    pub fn is_unsupported_interval(&self) -> bool {
        matches!(self, Self::UnsupportedInterval { .. })
    }

    /// The provider said too many requests (HTTP 429).
    pub fn is_rate_limited(&self) -> bool {
        match self {
            Self::Network(error) | Self::Alpaca(error) | Self::CoinMarketCap(error) => {
                error.status() == Some(429)
            }
            _ => false,
        }
    }
}

fn unknown_market_message(provider: &DataSource, symbol: &str, hint: &str) -> String {
    let name = provider.display_name();
    if hint.is_empty() {
        format!("{name} does not list {symbol}")
    } else {
        format!("{name} does not list {symbol}; {hint}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_name_who_failed() {
        let status = http::Error::Status(429);
        assert_eq!(
            ProviderError::from(status.clone()).to_string(),
            "Network error: HTTP 429"
        );
        assert_eq!(
            ProviderError::Alpaca(status.clone()).to_string(),
            "Alpaca error: HTTP 429"
        );
        assert_eq!(
            ProviderError::CoinMarketCap(status).to_string(),
            "CoinMarketCap error: HTTP 429"
        );
        assert_eq!(
            ProviderError::data("expected a candle array").to_string(),
            "Data error: expected a candle array"
        );
        assert_eq!(ProviderError::MissingKey.to_string(), MISSING_KEY);
        assert_eq!(
            ProviderError::other("No CoinMarketCap data").to_string(),
            "No CoinMarketCap data"
        );
        assert_eq!(
            ProviderError::unsupported(DataSource::Polymarket, Timeframe::MINUTE_1).to_string(),
            "Polymarket has no 1m history"
        );
    }

    #[test]
    fn an_unknown_market_says_who_does_not_list_it_and_is_decided_by_its_variant() {
        let hinted = ProviderError::unknown_market(
            DataSource::Hyperliquid,
            "btc",
            "names are case-sensitive",
        );
        assert_eq!(
            hinted.to_string(),
            "Hyperliquid does not list btc; names are case-sensitive"
        );
        assert!(matches!(hinted, ProviderError::UnknownMarket { .. }));
        // A message that reads like one is not.
        assert!(!matches!(
            ProviderError::other("Bitstamp does not list FOOUSD"),
            ProviderError::UnknownMarket { .. }
        ));
    }

    #[test]
    fn a_refusal_whose_body_was_kept_is_a_rate_limit_all_the_same() {
        let refused = |status| http::Error::Refused {
            status,
            body: "slow down".to_owned(),
        };
        assert!(ProviderError::Network(refused(429)).is_rate_limited());
        assert!(!ProviderError::Network(refused(500)).is_rate_limited());
        assert_eq!(
            ProviderError::Network(refused(429)).to_string(),
            "Network error: HTTP 429"
        );
    }

    #[test]
    fn only_an_unsupported_interval_is_one() {
        let unsupported = ProviderError::unsupported(DataSource::CoinGecko, Timeframe::MINUTE_1);
        assert!(unsupported.is_unsupported_interval());
        assert!(!unsupported.is_rate_limited());
        // A message that happens to read like one is not.
        assert!(!ProviderError::other("CoinGecko has no 1m history").is_unsupported_interval());
        assert!(!ProviderError::MissingKey.is_unsupported_interval());
    }

    #[test]
    fn only_a_429_is_a_rate_limit() {
        assert!(ProviderError::Network(http::Error::Status(429)).is_rate_limited());
        assert!(ProviderError::Alpaca(http::Error::Status(429)).is_rate_limited());
        assert!(!ProviderError::Network(http::Error::Status(500)).is_rate_limited());
        // A message that happens to hold the digits is not a rate limit.
        assert!(!ProviderError::other("No data for 429 days").is_rate_limited());
        assert!(!ProviderError::MissingKey.is_rate_limited());
    }

    #[test]
    fn a_regional_refusal_is_worded_once_for_a_chart_and_for_a_price() {
        use crate::service::watchlist_quotes::{QuoteFailure, provider::refused_by_region};
        for source in [
            DataSource::Bybit,
            DataSource::Gate,
            DataSource::Mexc,
            DataSource::Htx,
        ] {
            let message = ProviderError::refused_by_region(source).to_string();
            assert_eq!(
                message,
                format!(
                    "{} refused the request (HTTP 403): it is not available in every country",
                    source.display_name()
                )
            );
            assert_eq!(refused_by_region(source), QuoteFailure::Failed(message));
        }
    }
}
