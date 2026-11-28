//! Why a market provider gave no candles, search results or quotes. An error is who failed and
//! what kind of failure it was: code decides on the kind, never on a message. The words a person
//! reads are written where they are shown (`service::provider_message`), in the language in use.

use crate::model::{DataSource, Listing, Timeframe};
use crate::service::market::ports::http;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderError {
    /// The market list the answer came from: Binance spot and Binance perpetuals are two.
    pub venue: Listing,
    /// A service that answered in place of the venue's own API, when the words should name it:
    /// GeckoTerminal draws the candles of the pools DEXScreener lists.
    pub via: Option<&'static str>,
    pub kind: ErrorKind,
}

/// What a request asks about, which is what a provider's "unknown market" code is about: a code
/// that says a market is unknown means so only to a request that named one. The envelope readers
/// (`okx::data_rows`, `kraken::result_of`, `mexc::rows_of`) take it, as the statuses tables
/// ([`Statuses`]) are chosen by it, so a whole-list request never reads as `UnknownSymbol`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Asked {
    /// The request names the market(s) it asks for: candles, a ticker asked for by pair.
    Markets,
    /// It names none: the market list, a whole-exchange ticker.
    List,
}

/// What the HTTP statuses of one kind of request mean to a venue, beyond a rate limit. A status
/// means what the endpoint makes of it: a 400 from a request that names a market says the market
/// is unknown, from one that names none (a list, a catalog, an account) it says nothing about any,
/// so a venue declares a table for each kind of request it makes.
pub type Statuses = &'static [(u16, ErrorKind)];

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    /// The provider said too many requests: HTTP 429 or a code of its own envelope.
    RateLimited,
    /// The provider refuses this client with a 403: its country is not served, or it asked too
    /// often.
    Blocked,
    /// The provider refused the credentials sent with the request.
    Unauthorized,
    /// The credentials are fine, but the plan they belong to does not include what was asked for.
    NotInPlan,
    /// No credentials of the provider are saved, so its markets can be neither searched nor
    /// charted.
    MissingKey,
    /// The provider does not know the market the request named.
    UnknownSymbol,
    /// No candle of the venue reaches this timeframe: it is never answered with a nearby one.
    Unsupported { timeframe: Timeframe },
    /// The venue serves candles by length only, not by a time range.
    NoTimeRange,
    /// The provider has nothing for this market or range.
    NoData,
    /// The provider has candles for this market, but none older than the ones held.
    NoOlderData,
    /// Alpaca's free plan serves no bars of this symbol on this feed.
    NotOnFeed { symbol: String, feed: String },
    /// The request failed, or the provider answered a status the venue gives no meaning to.
    Transport(http::Error),
    /// The provider answered with something other than what it documents.
    Malformed(String),
    /// The provider refused the request, in words of its own when it gave any.
    Refused(Option<String>),
    /// A page of a quote request was refused and the markets at fault could not be told apart.
    Unisolated,
    /// The work ended without an answer (its worker stopped).
    Interrupted,
}

/// What a failure is about: how far a quote round has to look for what went wrong.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Scope {
    /// One market of the request: a smaller request can find which.
    Symbol,
    /// The venue as a whole: the key, the region, the endpoint, a ban or an outage.
    Source,
}

impl ErrorKind {
    /// Only an answer the provider documents as "a market in this request is unknown" is
    /// [`Scope::Symbol`]; everything else about a request belongs to the venue and backs it off.
    pub fn scope(&self) -> Scope {
        match self {
            Self::UnknownSymbol => Scope::Symbol,
            _ => Scope::Source,
        }
    }
}

impl ProviderError {
    pub fn new(venue: impl Into<Listing>, kind: ErrorKind) -> Self {
        Self {
            venue: venue.into(),
            via: None,
            kind,
        }
    }

    /// The failure is `service`'s: it answered in place of the venue's own API.
    pub fn through(self, service: &'static str) -> Self {
        Self {
            via: Some(service),
            ..self
        }
    }

    /// A failed request to an endpoint whose statuses mean nothing more than a rate limit: HTTP
    /// 429 is one at every venue, and any other status stays a transport failure.
    pub fn http(venue: impl Into<Listing>, error: http::Error) -> Self {
        Self::http_with(venue, &[], error)
    }

    /// A failed request whose statuses mean what `statuses` says, the table of the kind of
    /// endpoint it was made to.
    pub fn http_with(venue: impl Into<Listing>, statuses: Statuses, error: http::Error) -> Self {
        let kind = match error {
            http::Error::Status(429) => ErrorKind::RateLimited,
            http::Error::Status(code) => statuses
                .iter()
                .find(|(status, _)| *status == code)
                .map_or(ErrorKind::Transport(error), |(_, kind)| kind.clone()),
            error => ErrorKind::Transport(error),
        };
        Self::new(venue, kind)
    }

    /// The provider answered with something other than what it documents.
    pub fn malformed(venue: impl Into<Listing>, detail: impl std::fmt::Display) -> Self {
        Self::new(venue, ErrorKind::Malformed(detail.to_string()))
    }

    /// The provider refused the request in `message`, its own words.
    pub fn refused(venue: impl Into<Listing>, message: impl Into<String>) -> Self {
        Self::new(venue, ErrorKind::Refused(Some(message.into())))
    }

    /// The provider refused the request, saying why in `reason` when its answer held one.
    pub fn refused_because(venue: impl Into<Listing>, reason: Option<&str>) -> Self {
        let reason = reason
            .filter(|reason| !reason.is_empty())
            .map(str::to_owned);
        Self::new(venue, ErrorKind::Refused(reason))
    }

    /// `venue` cannot chart `timeframe`.
    pub fn unsupported(venue: impl Into<Listing>, timeframe: Timeframe) -> Self {
        Self::new(venue, ErrorKind::Unsupported { timeframe })
    }

    /// The work for `venue` ended without an answer.
    pub fn interrupted(venue: impl Into<Listing>) -> Self {
        Self::new(venue, ErrorKind::Interrupted)
    }

    /// The venue whose credentials are missing, when that is what failed.
    pub fn missing_key(&self) -> Option<DataSource> {
        (self.kind == ErrorKind::MissingKey).then_some(self.venue.source)
    }

    /// The venue cannot chart the timeframe, so asking again at it is no use.
    pub fn is_unsupported_interval(&self) -> bool {
        matches!(self.kind, ErrorKind::Unsupported { .. })
    }

    /// The failure may not come again: the transport failed or the provider was busy or broken
    /// (see [`http::Error::is_transient`]). An unsupported interval, a missing key, a parse
    /// failure or the provider's own refusal is not.
    pub fn is_transient(&self) -> bool {
        match &self.kind {
            ErrorKind::RateLimited => true,
            ErrorKind::Transport(error) => error.is_transient(),
            _ => false,
        }
    }

    /// The provider said too many requests.
    pub fn is_rate_limited(&self) -> bool {
        self.kind == ErrorKind::RateLimited
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::market::gateio;

    #[test]
    fn a_429_is_a_rate_limit_at_every_venue() {
        for source in DataSource::ALL {
            let error = ProviderError::http(source, http::Error::Status(429));
            assert_eq!(error.kind, ErrorKind::RateLimited, "{source:?}");
            assert!(error.is_rate_limited() && error.is_transient());
        }
    }

    #[test]
    fn a_request_reads_the_statuses_of_its_own_table_and_leaves_the_rest_a_transport_failure() {
        let ask = |code| {
            ProviderError::http_with(
                DataSource::Gate,
                gateio::MARKET_STATUSES,
                http::Error::Status(code),
            )
        };
        assert_eq!(ask(403).kind, ErrorKind::Blocked);
        assert!(
            !ask(403).is_transient(),
            "a refusal is not a passing failure"
        );
        assert_eq!(ask(400).kind, ErrorKind::UnknownSymbol);
        assert_eq!(
            ask(500).kind,
            ErrorKind::Transport(http::Error::Status(500))
        );
        let down = ProviderError::http_with(
            DataSource::Gate,
            gateio::MARKET_STATUSES,
            http::Error::Network("reset".to_owned()),
        );
        assert_eq!(
            down.kind,
            ErrorKind::Transport(http::Error::Network("reset".to_owned()))
        );
    }

    #[test]
    fn what_a_status_means_depends_on_whether_the_request_names_a_market() {
        let status = |statuses| {
            ProviderError::http_with(DataSource::Gate, statuses, http::Error::Status(400)).kind
        };
        assert_eq!(status(gateio::MARKET_STATUSES), ErrorKind::UnknownSymbol);
        assert_eq!(
            status(gateio::LIST_STATUSES),
            ErrorKind::Transport(http::Error::Status(400))
        );
        assert_eq!(
            ProviderError::http(DataSource::Gate, http::Error::Status(400)).kind,
            ErrorKind::Transport(http::Error::Status(400))
        );
    }

    #[test]
    fn only_a_failure_that_may_pass_is_transient() {
        for status in [408, 425, 429, 500, 503] {
            let error = ProviderError::http(DataSource::Binance, http::Error::Status(status));
            assert!(error.is_transient(), "{status}");
        }
        let reset = ProviderError::http(
            DataSource::Binance,
            http::Error::Network("reset".to_owned()),
        );
        assert!(reset.is_transient());
        for error in [
            ProviderError::http(DataSource::Binance, http::Error::Status(400)),
            ProviderError::http(DataSource::Binance, http::Error::Status(404)),
            ProviderError::http(DataSource::Binance, http::Error::BodyTooLarge(1)),
            ProviderError::malformed(DataSource::Binance, "bad"),
            ProviderError::refused(DataSource::Binance, "no market"),
            ProviderError::new(DataSource::Alpaca, ErrorKind::MissingKey),
            ProviderError::new(DataSource::Alpaca, ErrorKind::Unauthorized),
            ProviderError::new(DataSource::Gate, ErrorKind::Blocked),
            ProviderError::unsupported(DataSource::Gate, Timeframe::SECOND_1),
            ProviderError::interrupted(DataSource::Gate),
        ] {
            assert!(!error.is_transient(), "{error:?}");
        }
    }

    #[test]
    fn only_an_unsupported_interval_is_one() {
        let unsupported = ProviderError::unsupported(DataSource::CoinGecko, Timeframe::MINUTE_1);
        assert!(unsupported.is_unsupported_interval());
        assert!(!unsupported.is_rate_limited());
        // A refusal that happens to read like one is not.
        assert!(
            !ProviderError::refused(DataSource::CoinGecko, "CoinGecko has no 1m history")
                .is_unsupported_interval()
        );
        assert!(
            !ProviderError::new(DataSource::Alpaca, ErrorKind::MissingKey)
                .is_unsupported_interval()
        );
    }

    #[test]
    fn only_a_rate_limit_is_one() {
        assert!(ProviderError::new(DataSource::Bybit, ErrorKind::RateLimited).is_rate_limited());
        let busy = ProviderError::http(DataSource::Bybit, http::Error::Status(500));
        assert!(!busy.is_rate_limited());
        // A message that happens to hold the digits is not a rate limit.
        let refused = ProviderError::refused(DataSource::Bybit, "No data for 429 days");
        assert!(!refused.is_rate_limited());
        assert!(!ProviderError::new(DataSource::Oanda, ErrorKind::MissingKey).is_rate_limited());
    }

    #[test]
    fn only_a_missing_key_names_its_venue_as_one() {
        let missing = ProviderError::new(DataSource::Oanda, ErrorKind::MissingKey);
        assert_eq!(missing.missing_key(), Some(DataSource::Oanda));
        let refused = ProviderError::new(DataSource::Oanda, ErrorKind::Unauthorized);
        assert_eq!(refused.missing_key(), None);
    }

    #[test]
    fn only_a_documented_unknown_market_is_a_symbol_problem() {
        assert_eq!(ErrorKind::UnknownSymbol.scope(), Scope::Symbol);
        for kind in [
            ErrorKind::RateLimited,
            ErrorKind::Blocked,
            ErrorKind::Unauthorized,
            ErrorKind::NotInPlan,
            ErrorKind::MissingKey,
            ErrorKind::NoData,
            ErrorKind::Transport(http::Error::Status(500)),
            ErrorKind::Malformed("x".to_owned()),
            ErrorKind::Refused(Some("x".to_owned())),
            ErrorKind::Refused(None),
            ErrorKind::Unisolated,
            ErrorKind::Interrupted,
        ] {
            assert_eq!(kind.scope(), Scope::Source, "{kind:?}");
        }
    }
}
