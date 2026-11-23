//! Lighter's market list as `Market`s, and the search and the lookups over it. Pure: `catalog.rs`
//! downloads and keeps it.
//!
//! `/orderBooks` lists every market: `market_type` is `perp` or `spot`, and a perpetual no longer
//! traded is `inactive`. Only the active perpetuals are served.

use super::parse::answer;
use crate::model::DataSource;
use crate::model::market::MarketKind;
use crate::service::market::search_rank::normalize;
use crate::service::{ProviderError, SearchResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One market the search can offer.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct Market {
    /// What the API takes: `1` for BTC.
    pub(super) id: u32,
    /// What Lighter, and so this app, calls it: `BTC`.
    pub(super) symbol: String,
}

impl Market {
    fn hit(&self) -> SearchResult {
        let mut hit = SearchResult::new(
            DataSource::Lighter,
            self.symbol.clone(),
            self.symbol.clone(),
        );
        hit.kind = MarketKind::Perp;
        hit
    }
}

/// The active perpetuals of an `/orderBooks` answer.
pub(super) fn parse_markets(body: &str) -> Result<Vec<Market>, ProviderError> {
    let value = answer(body)?;
    let rows = value
        .get("order_books")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::data("expected a market list"))?;
    Ok(rows
        .iter()
        .filter(|row| {
            row.get("market_type").and_then(Value::as_str) == Some("perp")
                && row.get("status").and_then(Value::as_str) == Some("active")
        })
        .filter_map(|row| {
            Some(Market {
                id: u32::try_from(row.get("market_id")?.as_u64()?).ok()?,
                symbol: row
                    .get("symbol")?
                    .as_str()
                    .filter(|symbol| !symbol.is_empty())?
                    .to_owned(),
            })
        })
        .collect())
}

/// The number the API takes for `symbol`, however it is cased.
pub(super) fn id_of(markets: &[Market], symbol: &str) -> Option<u32> {
    markets
        .iter()
        .find(|market| market.symbol.eq_ignore_ascii_case(symbol.trim()))
        .map(|market| market.id)
}

/// [`id_of`] for a market a request names: the error says Lighter does not list it when it is not
/// in `markets`.
pub(super) fn resolve(markets: &[Market], symbol: &str) -> Result<u32, ProviderError> {
    id_of(markets, symbol).ok_or_else(|| {
        ProviderError::unknown_market(
            DataSource::Lighter,
            symbol,
            "its perpetuals are named like BTC, 1000PEPE or US500",
        )
    })
}

/// The numbers of the markets `symbols` name, leaving out the ones `markets` does not list.
pub(super) fn ids_of<'a>(
    markets: &'a [Market],
    symbols: &'a [String],
) -> impl Iterator<Item = u32> + 'a {
    symbols.iter().filter_map(|symbol| id_of(markets, symbol))
}

/// The symbol of the market a socket message names by its number.
pub(super) fn symbol_of(markets: &[Market], id: u32) -> Option<&str> {
    markets
        .iter()
        .find(|market| market.id == id)
        .map(|market| market.symbol.as_str())
}

/// The markets whose symbol, spelled with letters and digits only, contains `needle` (upper case).
pub(super) fn matching(markets: &[Market], needle: &str) -> Vec<SearchResult> {
    markets
        .iter()
        .filter(|market| normalize(&market.symbol).contains(needle))
        .map(Market::hit)
        .collect()
}
