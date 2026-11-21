//! Bybit's market list, held for a day, and the search over it.

use super::BASE_URL;
use super::parse::result_of;
use super::rest::get;
use crate::model::DataSource;
use crate::service::market::catalog::Catalog;
use crate::service::{ProviderError, SearchResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The listed markets whose name contains `needle` (letters and digits, upper case).
pub(super) fn matching_pairs(pairs: &[Pair], needle: &str) -> Vec<SearchResult> {
    pairs
        .iter()
        .filter(|pair| pair.id.contains(needle))
        .map(|pair| {
            SearchResult::new(DataSource::Bybit, pair.id.clone(), pair.id.clone())
                .pair(pair.base.clone(), pair.quote.clone())
        })
        .collect()
}

/// One tradable spot market from `instruments-info`: all the search needs of it.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct Pair {
    pub(super) id: String,
    pub(super) base: String,
    pub(super) quote: String,
}

/// Every spot market that is trading, downloaded at most once a day.
pub(super) static CATALOG: Catalog<Pair> = Catalog::new("catalog_bybit");

/// Forgets the list held in memory (after the disk cache was cleared).
pub(in crate::service) fn clear_catalog() {
    CATALOG.clear_memory();
}

/// The most pages of the market list followed: 509 markets fit one page of 1,000.
pub(super) const CATALOG_PAGES: usize = 10;

pub(super) fn download_pairs() -> Result<Vec<Pair>, ProviderError> {
    let mut pairs = Vec::new();
    let mut cursor = String::new();
    for _ in 0..CATALOG_PAGES {
        let url = format!("{BASE_URL}/instruments-info?category=spot&limit=1000{cursor}");
        let (page, next) = parse_pairs(&get(&url)?)?;
        pairs.extend(page);
        match next {
            Some(next) => cursor = format!("&cursor={next}"),
            None => break,
        }
    }
    Ok(pairs)
}

/// The trading markets of a page and the cursor of the next one, if there is one.
pub(super) fn parse_pairs(body: &str) -> Result<(Vec<Pair>, Option<String>), ProviderError> {
    let result = result_of(body)?;
    let pairs = result
        .get("list")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::data("expected a market list"))?
        .iter()
        .filter(|row| row.get("status").and_then(Value::as_str) == Some("Trading"))
        .filter_map(|row| {
            let field = |key: &str| row.get(key)?.as_str().filter(|text| !text.is_empty());
            Some(Pair {
                id: field("symbol")?.to_owned(),
                base: field("baseCoin")?.to_owned(),
                quote: field("quoteCoin")?.to_owned(),
            })
        })
        .collect();
    let next = result
        .get("nextPageCursor")
        .and_then(Value::as_str)
        .filter(|cursor| !cursor.is_empty())
        .map(str::to_owned);
    Ok((pairs, next))
}
