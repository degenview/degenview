//! What a person reads when a market provider failed: the chart's error slot, the symbol picker,
//! the watchlist, Settings and MCP. `service::market` reports a kind of failure and who failed;
//! the words are written here, in the language in use, so the venues stay free of the interface.

use crate::i18n::tr;
use crate::model::DataSource;
use crate::service::market::error::{ErrorKind, ProviderError};

pub fn message(error: &ProviderError) -> String {
    let provider = error.venue.label();
    match &error.kind {
        ErrorKind::RateLimited => tr!("Rate limited by the provider").to_owned(),
        ErrorKind::RegionBlocked => tr!(
            "{provider} refused the request (HTTP 403): it is not available in every country",
            provider = provider
        ),
        ErrorKind::Unauthorized => match error.venue.source {
            DataSource::Alpaca => {
                tr!("Alpaca refused your API key. Check it in Settings").to_owned()
            }
            DataSource::Oanda => {
                tr!("Oanda refused your access token. Check it in Settings").to_owned()
            }
            _ => tr!(
                "{provider} refused your API key. Check it in Settings",
                provider = provider
            ),
        },
        ErrorKind::MissingKey => match error.venue.source {
            DataSource::Alpaca => {
                tr!("Set up your Alpaca API key before using stock charts.").to_owned()
            }
            DataSource::Oanda => {
                tr!("Set up your Oanda account in Settings before using Oanda charts.").to_owned()
            }
            _ => tr!("Set up this provider's API key in Settings first.").to_owned(),
        },
        ErrorKind::UnknownSymbol => {
            tr!("{provider} does not list this market", provider = provider)
        }
        ErrorKind::Unsupported { timeframe } => {
            tr!(
                "{provider} has no {timeframe} history",
                provider = provider,
                timeframe = timeframe
            )
        }
        ErrorKind::NoTimeRange => tr!(
            "{provider} cannot serve a time range of candles",
            provider = provider
        ),
        ErrorKind::NoData => tr!(
            "{provider} has no price history for this market in this range",
            provider = provider
        ),
        ErrorKind::NoOlderData => tr!(
            "{provider} has no older candles for this market",
            provider = provider
        ),
        ErrorKind::NotOnFeed { symbol, feed } => tr!(
            "No data is available for {symbol} on Alpaca's free {feed} feed.",
            symbol = symbol,
            feed = feed
        ),
        ErrorKind::Transport(error) => tr!("Network error: {error}", error = error),
        ErrorKind::Malformed(detail) => tr!("Data error: {error}", error = detail),
        ErrorKind::Refused(refusal) => tr!(
            "{provider} error: {message}",
            provider = provider,
            message = refusal
        ),
        ErrorKind::Unisolated => tr!(
            "{provider} refused the request, and the markets at fault could not be singled out",
            provider = provider
        ),
        ErrorKind::Interrupted => tr!("The request stopped before it answered").to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DataSource, Timeframe};
    use crate::service::net::http;

    fn said(source: DataSource, kind: ErrorKind) -> String {
        message(&ProviderError::new(source, kind))
    }

    #[test]
    fn a_kind_is_worded_with_the_venue_that_failed() {
        assert_eq!(
            said(DataSource::Bybit, ErrorKind::RegionBlocked),
            "Bybit refused the request (HTTP 403): it is not available in every country"
        );
        assert_eq!(
            said(DataSource::Gate, ErrorKind::UnknownSymbol),
            "Gate.io does not list this market"
        );
        assert_eq!(
            said(
                DataSource::Polymarket,
                ErrorKind::Unsupported {
                    timeframe: Timeframe::MINUTE_1
                }
            ),
            "Polymarket has no 1m history"
        );
        assert_eq!(
            said(
                DataSource::Bybit,
                ErrorKind::Refused("Not supported symbols".into())
            ),
            "Bybit error: Not supported symbols"
        );
    }

    #[test]
    fn a_missing_or_refused_key_names_what_it_is() {
        assert_eq!(
            said(DataSource::Alpaca, ErrorKind::MissingKey),
            "Set up your Alpaca API key before using stock charts."
        );
        assert_eq!(
            said(DataSource::Oanda, ErrorKind::MissingKey),
            "Set up your Oanda account in Settings before using Oanda charts."
        );
        assert_eq!(
            said(DataSource::CoinMarketCap, ErrorKind::MissingKey),
            "Set up this provider's API key in Settings first."
        );
        assert_eq!(
            said(DataSource::Alpaca, ErrorKind::Unauthorized),
            "Alpaca refused your API key. Check it in Settings"
        );
        assert_eq!(
            said(DataSource::CoinMarketCap, ErrorKind::Unauthorized),
            "CoinMarketCap refused your API key. Check it in Settings"
        );
    }

    #[test]
    fn a_failure_of_the_transport_and_the_provider_s_data_keep_their_prefix() {
        assert_eq!(
            said(
                DataSource::Binance,
                ErrorKind::Transport(http::Error::Status(500))
            ),
            "Network error: HTTP 500"
        );
        assert_eq!(
            said(
                DataSource::Binance,
                ErrorKind::Malformed("expected a candle array".into())
            ),
            "Data error: expected a candle array"
        );
        assert_eq!(
            said(DataSource::Binance, ErrorKind::RateLimited),
            "Rate limited by the provider"
        );
    }

    #[test]
    fn a_perpetual_is_named_as_one() {
        let error = ProviderError::unsupported(
            crate::model::Listing::BINANCE_PERPETUAL,
            Timeframe::MONTH_1,
        );
        assert!(message(&error).starts_with("Binance perp has no "));
    }

    #[test]
    fn a_failure_is_worded_in_the_language_but_the_provider_s_own_words_are_not() {
        use crate::i18n::{Locale, with_locale};
        for error in [
            ProviderError::new(DataSource::Bybit, ErrorKind::RateLimited),
            ProviderError::new(DataSource::Oanda, ErrorKind::MissingKey),
            ProviderError::new(DataSource::Gate, ErrorKind::RegionBlocked),
        ] {
            let english = message(&error);
            assert_ne!(with_locale(Locale::PSEUDO, || message(&error)), english);
        }
        let refused = ProviderError::refused(DataSource::Bybit, "HTTP 500 from the provider");
        assert!(
            with_locale(Locale::PSEUDO, || message(&refused))
                .contains("HTTP 500 from the provider")
        );
    }

    #[test]
    fn what_a_provider_has_nothing_for_is_said_for_its_market() {
        assert_eq!(
            said(DataSource::Kalshi, ErrorKind::NoData),
            "Kalshi has no price history for this market in this range"
        );
        assert_eq!(
            said(DataSource::DexScreener, ErrorKind::NoOlderData),
            "DEXScreener has no older candles for this market"
        );
        assert_eq!(
            said(DataSource::CoinGecko, ErrorKind::NoTimeRange),
            "CoinGecko cannot serve a time range of candles"
        );
        assert_eq!(
            said(
                DataSource::Alpaca,
                ErrorKind::NotOnFeed {
                    symbol: "AAPL".into(),
                    feed: "IEX".into()
                }
            ),
            "No data is available for AAPL on Alpaca's free IEX feed."
        );
        assert_eq!(
            said(DataSource::Binance, ErrorKind::Interrupted),
            "The request stopped before it answered"
        );
        assert_eq!(
            said(DataSource::Binance, ErrorKind::Unisolated),
            "Binance refused the request, and the markets at fault could not be singled out"
        );
    }
}
