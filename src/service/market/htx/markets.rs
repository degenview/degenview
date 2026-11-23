//! HTX's market list, held for a day, and the search over it.

use super::parse::checked;
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
            SearchResult::new(DataSource::Htx, pair.id.clone(), pair.id.clone())
                .pair(pair.base.clone(), pair.quote.clone())
        })
        .collect()
}

/// One online market from `/v2/settings/common/symbols`: all the search needs of it. The id is
/// upper case like every exchange's here; HTX's own is lower case.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct Pair {
    pub(super) id: String,
    pub(super) base: String,
    pub(super) quote: String,
}

/// Every market that is online (HTX lists about 2,200, most of them offline), downloaded at most
/// once a day.
pub(super) static CATALOG: Catalog<Pair> = Catalog::new("catalog_htx");

/// Forgets the list held in memory (after the disk cache was cleared).
pub(super) fn clear_catalog() {
    CATALOG.clear_memory();
}

pub(super) fn download_pairs() -> Result<Vec<Pair>, ProviderError> {
    parse_pairs(&get("/v2/settings/common/symbols")?)
}

/// The online markets of the answer: `sc` is the market's name (`btcusdt`), `bc` and `qc` its two
/// assets (`btc`, `usdt`).
pub(super) fn parse_pairs(body: &str) -> Result<Vec<Pair>, ProviderError> {
    let answer = checked(body)?;
    Ok(answer
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::data("expected a market list"))?
        .iter()
        .filter(|row| row.get("state").and_then(Value::as_str) == Some("online"))
        .filter_map(|row| {
            let field = |key: &str| {
                row.get(key)?
                    .as_str()
                    .filter(|text| !text.is_empty())
                    .map(str::to_uppercase)
            };
            Some(Pair {
                id: field("sc")?,
                base: field("bc")?,
                quote: field("qc")?,
            })
        })
        .collect())
}
