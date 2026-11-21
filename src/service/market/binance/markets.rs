//! Binance's market list, held for a day, and the search over it.

use crate::service::market::catalog::Catalog;
use crate::service::{ProviderError, SearchResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The listed markets whose name contains `needle` (letters and digits, upper case).
pub(super) fn matching_pairs(pairs: &[Pair], needle: &str) -> Vec<SearchResult> {
    pairs
        .iter()
        .filter(|pair| pair.symbol.contains(needle))
        .map(|pair| {
            SearchResult::new(
                crate::model::DataSource::Binance,
                pair.symbol.clone(),
                pair.symbol.clone(),
            )
            .pair(pair.base.clone(), pair.quote.clone())
        })
        .collect()
}

/// One tradable market from `exchangeInfo`: all the search needs of it.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct Pair {
    pub(super) symbol: String,
    pub(super) base: String,
    pub(super) quote: String,
}

/// Every market that is trading, downloaded at most once a day.
pub(super) static CATALOG: Catalog<Pair> = Catalog::new("catalog_binance");

/// Forgets the list held in memory (after the disk cache was cleared).
pub(in crate::service) fn clear_catalog() {
    CATALOG.clear_memory();
}

pub(super) fn download_pairs() -> Result<Vec<Pair>, ProviderError> {
    let body = crate::service::net::http::get_text(
        "https://api.binance.com/api/v3/exchangeInfo",
        &[],
        crate::service::net::http::Retry::Transient,
    )?;
    parse_pairs(&body)
}

pub(super) fn parse_pairs(body: &str) -> Result<Vec<Pair>, ProviderError> {
    let value: Value = serde_json::from_str(body).map_err(ProviderError::data)?;
    Ok(value
        .get("symbols")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|symbol| symbol.get("status").and_then(Value::as_str) == Some("TRADING"))
        .filter_map(|symbol| {
            Some(Pair {
                symbol: symbol.get("symbol")?.as_str()?.to_owned(),
                base: symbol.get("baseAsset")?.as_str()?.to_owned(),
                quote: symbol.get("quoteAsset")?.as_str()?.to_owned(),
            })
        })
        .collect())
}
