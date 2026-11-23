//! Gate.io's market list, held for a day, and the search over it.

use super::BASE_URL;
use super::rest::get;
use crate::model::DataSource;
use crate::service::market::catalog::Catalog;
use crate::service::market::search_rank::normalize;
use crate::service::{ProviderError, SearchResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The listed markets whose name, without its underscore, contains `needle` (letters and digits,
/// upper case).
pub(super) fn matching_pairs(pairs: &[Pair], needle: &str) -> Vec<SearchResult> {
    pairs
        .iter()
        .filter(|pair| normalize(&pair.id).contains(needle))
        .map(|pair| {
            SearchResult::new(DataSource::Gate, pair.id.clone(), pair.id.clone())
                .pair(pair.base.clone(), pair.quote.clone())
        })
        .collect()
}

/// One tradable spot market from `currency_pairs`: all the search needs of it.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct Pair {
    pub(super) id: String,
    pub(super) base: String,
    pub(super) quote: String,
}

/// Every spot market that is tradable, downloaded at most once a day.
pub(super) static CATALOG: Catalog<Pair> = Catalog::new("catalog_gateio");

/// Forgets the list held in memory (after the disk cache was cleared).
pub(in crate::service) fn clear_catalog() {
    CATALOG.clear_memory();
}

pub(super) fn download_pairs() -> Result<Vec<Pair>, ProviderError> {
    parse_pairs(&get(&format!("{BASE_URL}/currency_pairs"))?)
}

pub(super) fn parse_pairs(body: &str) -> Result<Vec<Pair>, ProviderError> {
    let value: Value = serde_json::from_str(body).map_err(ProviderError::data)?;
    Ok(value
        .as_array()
        .ok_or_else(|| ProviderError::data("expected a market list"))?
        .iter()
        .filter(|row| row.get("trade_status").and_then(Value::as_str) == Some("tradable"))
        .filter_map(|row| {
            let field = |key: &str| row.get(key)?.as_str().filter(|text| !text.is_empty());
            Some(Pair {
                id: field("id")?.to_owned(),
                base: field("base")?.to_owned(),
                quote: field("quote")?.to_owned(),
            })
        })
        .collect())
}
