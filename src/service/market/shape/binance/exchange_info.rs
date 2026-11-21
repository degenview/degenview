//! `exchangeInfo` as a list of markets, and the search over it: pure functions.

use crate::model::DataSource;
use crate::service::{ProviderError, SearchResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One tradable market from `exchangeInfo`: all the search needs of it.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(in crate::service) struct Pair {
    pub symbol: String,
    pub base: String,
    pub quote: String,
}

/// Whether a listed market is trading (not halted, settling or awaiting its listing).
pub(in crate::service) fn trading(market: &Value) -> bool {
    market.get("status").and_then(Value::as_str) == Some("TRADING")
}

/// The listed markets `serves` accepts, each with all its fields.
pub(super) fn parse_pairs(
    body: &str,
    serves: fn(&Value) -> bool,
) -> Result<Vec<Pair>, ProviderError> {
    let value: Value = serde_json::from_str(body).map_err(ProviderError::data)?;
    Ok(value
        .get("symbols")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|market| serves(market))
        .filter_map(|market| {
            Some(Pair {
                symbol: market.get("symbol")?.as_str()?.to_owned(),
                base: market.get("baseAsset")?.as_str()?.to_owned(),
                quote: market.get("quoteAsset")?.as_str()?.to_owned(),
            })
        })
        .collect())
}

/// The listed markets whose name contains `needle` (letters and digits, upper case).
pub(super) fn matching_pairs(
    source: DataSource,
    pairs: &[Pair],
    needle: &str,
) -> Vec<SearchResult> {
    pairs
        .iter()
        .filter(|pair| pair.symbol.contains(needle))
        .map(|pair| {
            let mut hit = SearchResult::new(source, pair.symbol.clone(), pair.symbol.clone())
                .pair(pair.base.clone(), pair.quote.clone());
            hit.kind = source.profile().default_kind();
            hit
        })
        .collect()
}
