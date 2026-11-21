//! The REST call behind a watchlist price of this venue: the batch ticker URL.

use super::SHAPE;
use crate::model::DataSource;
use crate::model::watchlist::quote::SourceQuote;
use crate::service::market::shape::binance::parse_quotes;
use crate::service::watchlist_quotes::provider::{QuoteFailure, QuoteRequest, get};
use std::collections::HashMap;

/// The batch ticker URL. `symbols` is a JSON array, so its brackets and quotes are
/// percent-encoded: sent raw they are not valid in a URL, the HTTP client refuses the request,
/// and every Binance quote in every watchlist stays empty.
pub(in crate::service) fn binance_url(requests: &[QuoteRequest]) -> String {
    let list = requests
        .iter()
        .map(|request| format!("%22{}%22", request.symbol.to_uppercase()))
        .collect::<Vec<_>>()
        .join("%2C");
    format!("https://api.binance.com/api/v3/ticker/24hr?symbols=%5B{list}%5D&type=MINI")
}

pub(super) fn fetch_quotes(
    requests: &[QuoteRequest],
) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let body = get(DataSource::Binance, &binance_url(requests), &[])?;
    parse_quotes(SHAPE.source, &body, requests)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::watchlist_quotes::provider::request;

    #[test]
    fn the_binance_url_encodes_its_symbol_list() {
        let url = binance_url(&[request("btcusdt"), request("ETHUSDT")]);
        assert_eq!(
            url,
            "https://api.binance.com/api/v3/ticker/24hr?symbols=%5B%22BTCUSDT%22%2C%22ETHUSDT%22%5D&type=MINI"
        );
        assert!(!url.contains(['[', ']', '"']), "only URL-safe characters");
    }
}
