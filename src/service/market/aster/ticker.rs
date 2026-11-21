//! The REST call behind a watchlist price of Aster. Its `/ticker/24hr` takes one symbol or none: a
//! list is ignored and every market answered, so the one call without a symbol serves any batch.

use crate::model::DataSource;
use crate::model::watchlist::quote::SourceQuote;
use crate::service::market::shape::binance::parse_quotes;
use crate::service::watchlist_quotes::provider::{QuoteFailure, QuoteRequest, get};
use std::collections::HashMap;

const URL: &str = "https://fapi.asterdex.com/fapi/v1/ticker/24hr";

pub(super) fn fetch_quotes(
    requests: &[QuoteRequest],
) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let body = get(DataSource::Aster, URL, &[])?;
    parse_quotes(DataSource::Aster, &body, requests)
}
