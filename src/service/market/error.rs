//! Why a market provider gave no candles or no search results. The messages are the ones the
//! chart, the symbol picker and MCP show; code decides on the variant, never on the text.

use super::alpaca::MISSING_KEY;
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
}

impl ProviderError {
    /// A [`Self::Data`] from a parser's error.
    pub fn data(error: impl std::fmt::Display) -> Self {
        Self::Data(error.to_string())
    }

    pub fn other(message: impl Into<String>) -> Self {
        Self::Other(message.into())
    }

    /// The provider said too many requests (HTTP 429).
    pub fn is_rate_limited(&self) -> bool {
        matches!(
            self,
            Self::Network(http::Error::Status(429))
                | Self::Alpaca(http::Error::Status(429))
                | Self::CoinMarketCap(http::Error::Status(429))
        )
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
}
