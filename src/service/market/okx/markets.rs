//! OKX's market list, held for a day, and the search over it.

use super::BASE_URL;
use super::parse::data_rows;
use crate::model::DataSource;
use crate::service::market::catalog::Catalog;
use crate::service::market::search_rank::normalize;
use crate::service::net::http;
use crate::service::{ProviderError, SearchResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The listed markets whose name, without its dash, contains `needle` (letters and digits, upper
/// case).
pub(super) fn matching_pairs(pairs: &[Pair], needle: &str) -> Vec<SearchResult> {
    pairs
        .iter()
        .filter(|pair| normalize(&pair.id).contains(needle))
        .map(|pair| {
            SearchResult::new(DataSource::Okx, pair.id.clone(), pair.id.clone())
                .pair(pair.base.clone(), pair.quote.clone())
        })
        .collect()
}

/// One tradable spot market from `/public/instruments`: all the search needs of it.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct Pair {
    pub(super) id: String,
    pub(super) base: String,
    pub(super) quote: String,
}

/// Every spot market that is live, downloaded at most once a day.
pub(super) static CATALOG: Catalog<Pair> = Catalog::new("catalog_okx");

/// Forgets the list held in memory (after the disk cache was cleared).
pub(in crate::service) fn clear_catalog() {
    CATALOG.clear_memory();
}

pub(super) fn download_pairs() -> Result<Vec<Pair>, ProviderError> {
    let body = http::get_text(
        &format!("{BASE_URL}/public/instruments?instType=SPOT"),
        &[],
        http::Retry::Transient,
    )?;
    parse_pairs(&body)
}

pub(super) fn parse_pairs(body: &str) -> Result<Vec<Pair>, ProviderError> {
    Ok(data_rows(body)?
        .iter()
        .filter(|row| row.get("state").and_then(Value::as_str) == Some("live"))
        .filter_map(|row| {
            let field = |key: &str| row.get(key)?.as_str().filter(|text| !text.is_empty());
            Some(Pair {
                id: field("instId")?.to_owned(),
                base: field("baseCcy")?.to_owned(),
                quote: field("quoteCcy")?.to_owned(),
            })
        })
        .collect())
}
