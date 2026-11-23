//! Where watchlist prices come from: one batched call per source. The live provider asks the
//! venue behind the source (`service::market::venue`), whose `ticker.rs` holds its endpoint and
//! its pure parser; tests substitute a recorder. What is shared lives here: the request and failure
//! types, the GET that maps statuses onto failures, and the helpers the parsers use.

use super::engine::Outcome;
use crate::model::watchlist::quote::SourceQuote;
use crate::model::{DataSource, Timeframe};
use crate::service::market::venue;
use crate::service::net::http;
use serde_json::Value;

/// One market to price. `symbol` is the id the provider's quote call takes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QuoteRequest {
    pub symbol: String,
    /// The network of a DEX pair; other sources leave it `None`.
    pub chain: Option<String>,
}

/// Why a batch of quotes did not come back.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum QuoteFailure {
    RateLimited,
    CredentialsMissing,
    /// The provider refused the request itself, as it does for an unknown symbol.
    /// Retrying a smaller batch can find which symbol it was.
    Rejected(String),
    /// Network trouble or a server error; the whole source is treated as down for a moment.
    Failed(String),
}

impl QuoteFailure {
    pub fn message(&self) -> String {
        match self {
            Self::RateLimited => "Rate limited by the provider".to_owned(),
            Self::CredentialsMissing => {
                "Add your Alpaca API key in Settings to see stock quotes".to_owned()
            }
            Self::Rejected(message) | Self::Failed(message) => message.clone(),
        }
    }
}

/// Quotes keyed by `QuoteRequest::symbol`; a symbol the provider did not answer is omitted.
/// An adapter that makes several requests reports the ones that failed in the `Outcome`
/// and keeps the rest; `Err` is for a call that failed as a whole.
pub trait QuoteProvider: Send + Sync {
    fn quotes(
        &self,
        source: DataSource,
        requests: &[QuoteRequest],
    ) -> Result<Outcome, QuoteFailure>;

    /// The network of a DEX pair saved without one, from its address.
    fn resolve_chain(&self, address: &str) -> Option<String>;
}

// MARK: Live provider

pub struct LiveProvider;

impl QuoteProvider for LiveProvider {
    fn quotes(
        &self,
        source: DataSource,
        requests: &[QuoteRequest],
    ) -> Result<Outcome, QuoteFailure> {
        if requests.is_empty() {
            return Ok(Outcome::default());
        }
        venue::of(source).rest_quotes(requests)
    }

    fn resolve_chain(&self, address: &str) -> Option<String> {
        DataSource::ALL
            .into_iter()
            .find_map(|source| venue::of(source).resolve_chain(address))
    }
}

/// A GET that maps HTTP failures onto `QuoteFailure`.
pub(in crate::service) fn get(
    source: DataSource,
    url: &str,
    headers: &[(&str, &str)],
) -> Result<String, QuoteFailure> {
    http::get_text(url, headers, http::Retry::Never).map_err(|error| failure(source, error))
}

/// [`get`] for a provider whose public queries are POSTed as JSON.
pub(in crate::service) fn post_json(
    source: DataSource,
    url: &str,
    body: &str,
) -> Result<String, QuoteFailure> {
    http::post_json(url, body, http::Retry::Never).map_err(|error| failure(source, error))
}

fn failure(source: DataSource, error: http::Error) -> QuoteFailure {
    match error {
        http::Error::Status(code) => classify_status(source, code),
        error => QuoteFailure::Failed(format!("Network error: {error}")),
    }
}

/// What an HTTP status means for a quote request. Only a status the provider documents
/// as "a symbol in this request is unknown" is `Rejected`, which makes the engine search
/// for the bad symbol; every other refusal (credentials, region, a changed endpoint, a
/// ban) belongs to the whole source and backs it off instead. What a status means for one
/// venue is that venue's [`Venue::classify_status`].
pub fn classify_status(source: DataSource, code: u16) -> QuoteFailure {
    if code == 429 {
        return QuoteFailure::RateLimited;
    }
    venue::of(source)
        .classify_status(code)
        .unwrap_or_else(|| QuoteFailure::Failed(format!("The provider answered HTTP {code}")))
}

/// The answer for a status the provider documents as "a symbol in this request is unknown".
pub(in crate::service) fn rejected(code: u16) -> QuoteFailure {
    QuoteFailure::Rejected(format!("The provider answered HTTP {code}"))
}

/// The answer for a venue that refuses some countries, and a client that asks too often, with a 403.
pub(in crate::service) fn refused_by_region(source: DataSource) -> QuoteFailure {
    QuoteFailure::Failed(format!(
        "{} refused the request (HTTP 403): it is not available in every country",
        source.display_name()
    ))
}

/// How many answers in a row may fail before an adapter stops asking: the endpoint is
/// down, and the rest of the batch would only add requests.
pub(in crate::service) const CONSECUTIVE_FAILURES: usize = 3;

pub(in crate::service) fn symbols_of(requests: &[QuoteRequest]) -> Vec<String> {
    requests
        .iter()
        .map(|request| request.symbol.clone())
        .collect()
}

/// The hourly points a prediction market's quote reads: a month of them, from which the price a
/// day before the last one is picked.
const PREDICTION_POINTS: u32 = 720;

/// The price a day before the last point of `series` (oldest first), or the oldest one when the
/// market has not been traded that long.
fn day_before(series: &[crate::model::Kline]) -> Option<f64> {
    const DAY_MS: i64 = 86_400_000;
    let last = series.last()?;
    series
        .iter()
        .find(|point| point.open_time >= last.open_time - DAY_MS)
        .map(|point| point.close)
}

/// Last price of a prediction market's hourly series, and its price a day earlier as the
/// comparison.
pub(in crate::service) fn prediction(source: DataSource, requests: &[QuoteRequest]) -> Outcome {
    let mut outcome = Outcome::default();
    let mut streak = 0;
    for (index, request) in requests.iter().enumerate() {
        let series = match crate::service::fetch_klines(
            source,
            &request.symbol,
            Timeframe::HOUR_1,
            PREDICTION_POINTS,
        ) {
            Ok(series) => {
                streak = 0;
                series
            }
            Err(error) => {
                streak += 1;
                let stop = streak >= CONSECUTIVE_FAILURES;
                let upto = if stop { requests.len() } else { index + 1 };
                outcome.failed.push((
                    symbols_of(&requests[index..upto]),
                    QuoteFailure::Failed(error.to_string()),
                ));
                if stop {
                    break;
                }
                continue;
            }
        };
        if let (Some(before), Some(last)) = (day_before(&series), series.last()) {
            outcome.quotes.insert(
                request.symbol.clone(),
                SourceQuote {
                    price: last.close,
                    previous_day_price: Some(before),
                    ..SourceQuote::default()
                },
            );
        }
    }
    outcome
}

// MARK: Parsers

pub(in crate::service) fn json(body: &str) -> Result<Value, QuoteFailure> {
    serde_json::from_str(body).map_err(|error| QuoteFailure::Failed(format!("Data error: {error}")))
}

pub(in crate::service) fn number(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

pub(in crate::service) const DOLLAR_QUOTES: [&str; 8] = [
    "USDT", "USDC", "BUSD", "FDUSD", "TUSD", "USDP", "DAI", "USD",
];

/// Sources that report a 24h percent change rather than a price:
/// `previous = price / (1 + pct / 100)`.
pub(in crate::service) fn from_change_percent(
    price: f64,
    change_percent_24h: Option<f64>,
) -> SourceQuote {
    SourceQuote {
        price,
        previous_day_price: change_percent_24h
            .filter(|percent| *percent > -100.0)
            .map(|percent| price / (1.0 + percent / 100.0)),
        ..SourceQuote::default()
    }
}

/// A request for `symbol` on a source with no chain, for the venues' parser tests.
#[cfg(test)]
pub(in crate::service) fn request(symbol: &str) -> QuoteRequest {
    QuoteRequest {
        symbol: symbol.to_owned(),
        chain: None,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_prediction_quote_compares_with_the_price_a_day_earlier() {
        use crate::model::Kline;
        let point = |hours: i64, price: f64| Kline {
            open_time: hours * 3_600_000,
            close_time: hours * 3_600_000,
            open: price,
            high: price,
            low: price,
            close: price,
            volume: 0.0,
        };
        let month: Vec<Kline> = (0..=720).map(|hour| point(hour, hour as f64)).collect();
        assert_eq!(super::day_before(&month), Some(696.0));
        // A market younger than a day compares with its first price.
        assert_eq!(
            super::day_before(&[point(0, 0.4), point(5, 0.5)]),
            Some(0.4)
        );
        assert_eq!(super::day_before(&[]), None);
    }

    #[test]
    fn only_a_symbol_level_refusal_makes_the_engine_search_for_the_bad_symbol() {
        use super::classify_status as classify;
        let rejected = |source, code| matches!(classify(source, code), QuoteFailure::Rejected(_));
        // Documented "unknown symbol in this request" answers.
        assert!(rejected(DataSource::Binance, 400));
        assert!(rejected(DataSource::Alpaca, 400));
        assert!(rejected(DataSource::Coinbase, 404));
        // Everything that is about the request, the key, the region or the endpoint.
        for (source, code) in [
            (DataSource::Binance, 401),
            (DataSource::Binance, 403),
            (DataSource::Binance, 404),
            (DataSource::Binance, 418),
            (DataSource::Binance, 451),
            (DataSource::Alpaca, 401),
            (DataSource::Alpaca, 403),
            (DataSource::Alpaca, 404),
            (DataSource::CoinGecko, 400),
            (DataSource::CoinGecko, 404),
            (DataSource::DexScreener, 404),
            (DataSource::Coinbase, 400),
            (DataSource::Coinbase, 403),
        ] {
            assert!(
                matches!(classify(source, code), QuoteFailure::Failed(_)),
                "{source:?} {code}"
            );
        }
        for source in [
            DataSource::Binance,
            DataSource::Alpaca,
            DataSource::Coinbase,
        ] {
            assert_eq!(classify(source, 429), QuoteFailure::RateLimited);
        }
        assert!(
            classify(DataSource::Alpaca, 401)
                .message()
                .contains("API key"),
            "a refused key says so"
        );
    }

    use super::*;

    #[test]
    fn percent_change_of_minus_100_or_worse_has_no_reference() {
        assert_eq!(
            from_change_percent(5.0, Some(-100.0)).previous_day_price,
            None
        );
        assert_eq!(
            from_change_percent(5.0, Some(-150.0)).previous_day_price,
            None
        );
        assert_eq!(from_change_percent(5.0, None).previous_day_price, None);
        assert_eq!(
            from_change_percent(50.0, Some(-50.0)).previous_day_price,
            Some(100.0)
        );
    }

    #[test]
    fn failures_describe_themselves() {
        assert_eq!(
            QuoteFailure::RateLimited.message(),
            "Rate limited by the provider"
        );
        assert!(
            QuoteFailure::CredentialsMissing
                .message()
                .contains("Alpaca")
        );
        assert_eq!(QuoteFailure::Failed("x".into()).message(), "x");
    }
}
