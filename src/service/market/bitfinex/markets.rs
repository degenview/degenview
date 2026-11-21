//! Bitfinex's market list, held for a day, and the search over it.
//!
//! `/v2/conf/pub:list:pair:exchange` answers `[["AAVE:USD","ADABTC","BTCUSD",…]]`: the pair names
//! as Bitfinex writes them, run together (`BTCUSD`) until an asset has more than three letters, then
//! split by a colon (`AVAX:USD`, `BTC:XAUT`). The API puts a `t` in front of one (`tBTCUSD`); the app
//! keeps the name without it, so a market reads as it does on Bitfinex's own pages.

use super::rest::get_now;
use crate::model::DataSource;
use crate::service::market::catalog::Catalog;
use crate::service::market::search_rank::normalize;
use crate::service::{ProviderError, SearchResult};
use serde_json::Value;

/// The listed pairs, downloaded at most once a day.
pub(super) static CATALOG: Catalog<String> = Catalog::new("catalog_bitfinex");

/// Forgets the list held in memory (after the disk cache was cleared).
pub(super) fn clear_catalog() {
    CATALOG.clear_memory();
}

/// The listed pairs, from Bitfinex. The list has an allowance of its own, outside the candle
/// schedule.
pub(super) fn download_pairs() -> Result<Vec<String>, ProviderError> {
    parse_pairs(&get_now("/conf/pub:list:pair:exchange")?)
}

/// Whether Bitfinex lists `market`. A list that cannot be had is no reason to say it does not.
pub(super) fn is_listed(market: &str) -> bool {
    CATALOG
        .get(download_pairs)
        .map_or(true, |pairs| pairs.iter().any(|pair| pair == market))
}

/// Bitfinex's paper-trading markets (`TESTBTC:TESTUSD`) are listed beside the real ones.
fn is_paper(pair: &str) -> bool {
    pair.starts_with("TEST")
}

/// The real pairs of a conf answer.
pub(super) fn parse_pairs(body: &str) -> Result<Vec<String>, ProviderError> {
    let value: Value = serde_json::from_str(body).map_err(ProviderError::data)?;
    if let Some(refusal) = super::parse::refusal(&value) {
        return Err(refusal);
    }
    Ok(value
        .get(0)
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::data("expected a market list"))?
        .iter()
        .filter_map(Value::as_str)
        .filter(|pair| !pair.is_empty() && !is_paper(pair))
        .map(str::to_owned)
        .collect())
}

/// `needle` (letters and digits, upper case) as Bitfinex writes an asset: it calls Tether `UST`.
fn bitfinex_spelling(needle: &str) -> String {
    needle.replace("USDT", "UST")
}

/// The pairs whose name, spelled with letters and digits only, contains `needle` (upper case; a
/// `USDT` in it finds the `UST` pairs).
pub(super) fn matching_pairs(pairs: &[String], needle: &str) -> Vec<SearchResult> {
    let needle = bitfinex_spelling(needle);
    pairs
        .iter()
        .filter(|pair| normalize(pair).contains(&needle))
        .map(|pair| hit(pair))
        .collect()
}

/// A search hit for a pair, its assets as the profile reads them from the name.
fn hit(pair: &str) -> SearchResult {
    let hit = SearchResult::new(DataSource::Bitfinex, pair, pair);
    match DataSource::Bitfinex.quote_asset(pair) {
        Some(quote) => hit.pair(DataSource::Bitfinex.base_asset(pair), quote),
        None => hit,
    }
}
