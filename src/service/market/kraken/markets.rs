//! Kraken's market list, held for a day, and the search over it.

use super::parse::result_of;
use super::rest::get;
use super::{BASE_URL, pair};
use crate::model::DataSource;
use crate::service::market::catalog::Catalog;
use crate::service::market::search_rank::normalize;
use crate::service::{ProviderError, SearchResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The listed markets whose name, without its slash, contains `needle` (letters and digits,
/// upper case).
pub(super) fn matching_pairs(pairs: &[Pair], needle: &str) -> Vec<SearchResult> {
    pairs
        .iter()
        .filter(|pair| normalize(&pair.id).contains(needle))
        .map(|pair| {
            SearchResult::new(DataSource::Kraken, pair.id.clone(), pair.id.clone())
                .pair(pair.base.clone(), pair.quote.clone())
        })
        .collect()
}

/// One tradable market from `AssetPairs`: all the search needs of it.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct Pair {
    pub(super) id: String,
    pub(super) base: String,
    pub(super) quote: String,
}

/// Every market that is online, downloaded at most once a day.
pub(super) static CATALOG: Catalog<Pair> = Catalog::new("catalog_kraken");

/// Forgets the list held in memory (after the disk cache was cleared).
pub(in crate::service) fn clear_catalog() {
    CATALOG.clear_memory();
}

pub(super) fn download_pairs() -> Result<Vec<Pair>, ProviderError> {
    parse_pairs(&get(&format!("{BASE_URL}/AssetPairs"))?)
}

/// `wsname` is `XBT/USD`: the market as `BTC/USD`, which is how every Kraken API spells it.
pub(super) fn parse_pairs(body: &str) -> Result<Vec<Pair>, ProviderError> {
    let result = result_of(body)?;
    let pairs = result
        .as_object()
        .ok_or_else(|| ProviderError::data("expected a pair list"))?;
    Ok(pairs
        .values()
        .filter(|pair| pair.get("status").and_then(Value::as_str) == Some("online"))
        .filter_map(|pair| {
            let id = self::pair(pair.get("wsname")?.as_str()?);
            let (base, quote) = id.split_once('/')?;
            let (base, quote) = (base.to_owned(), quote.to_owned());
            (!base.is_empty() && !quote.is_empty() && !quote.contains('/')).then_some(Pair {
                id,
                base,
                quote,
            })
        })
        .collect())
}
