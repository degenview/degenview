//! Bitstamp's market list, held for a day, and the search over it.
//!
//! `/api/v2/markets/` answers a row per market, spot and perpetual:
//! `{"name":"BTC/USD","market_symbol":"btcusd","base_currency":"BTC","counter_currency":"USD",
//! "trading":"Enabled","matching":"Enabled","market_type":"SPOT",…}`. Only the spot markets that
//! can be traded are listed here (272 rows on 2026-11-21: 252 spot, one of them with matching
//! disabled, and 20 perpetuals).

use super::rest::get;
use crate::model::DataSource;
use crate::service::market::catalog::Catalog;
use crate::service::{ProviderError, SearchResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One tradable spot market: all the search needs of it. The id is upper case like every
/// exchange's here; Bitstamp's own is lower case.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct Pair {
    pub(super) id: String,
    pub(super) base: String,
    pub(super) quote: String,
}

/// The tradable spot markets, downloaded at most once a day.
pub(super) static CATALOG: Catalog<Pair> = Catalog::new("catalog_bitstamp");

/// Forgets the list held in memory (after the disk cache was cleared).
pub(super) fn clear_catalog() {
    CATALOG.clear_memory();
}

pub(super) fn download_pairs() -> Result<Vec<Pair>, ProviderError> {
    parse_pairs(&get("/markets/")?)
}

/// The tradable spot markets of an answer.
pub(super) fn parse_pairs(body: &str) -> Result<Vec<Pair>, ProviderError> {
    let value: Value = serde_json::from_str(body).map_err(ProviderError::data)?;
    Ok(value
        .as_array()
        .ok_or_else(|| ProviderError::data("expected a market list"))?
        .iter()
        .filter(|row| {
            ["market_type", "trading", "matching"]
                .into_iter()
                .zip(["SPOT", "Enabled", "Enabled"])
                .all(|(field, wanted)| row.get(field).and_then(Value::as_str) == Some(wanted))
        })
        .filter_map(|row| {
            let field = |key: &str| {
                row.get(key)?
                    .as_str()
                    .filter(|text| !text.is_empty())
                    .map(str::to_uppercase)
            };
            Some(Pair {
                id: field("market_symbol")?,
                base: field("base_currency")?,
                quote: field("counter_currency")?,
            })
        })
        .collect())
}

/// The markets whose name contains `needle` (letters and digits, upper case).
pub(super) fn matching_pairs(pairs: &[Pair], needle: &str) -> Vec<SearchResult> {
    pairs
        .iter()
        .filter(|pair| pair.id.contains(needle))
        .map(|pair| {
            SearchResult::new(DataSource::Bitstamp, pair.id.clone(), pair.id.clone())
                .pair(pair.base.clone(), pair.quote.clone())
        })
        .collect()
}
