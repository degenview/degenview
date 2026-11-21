//! Deribit's market list as names, and the search and the lookups over it. Pure: `catalog.rs`
//! downloads and keeps it.
//!
//! `get_instruments` lists every future of every currency; a perpetual has `settlement_period`
//! `perpetual`, and one no longer traded is not `is_active`. Its name is the market's id as every
//! call takes it: `BTC-PERPETUAL` and `ETH-PERPETUAL` are inverse (quoted in dollars, settled in
//! the coin), `SOL_USDC-PERPETUAL` is linear (settled in USDC).

use super::parse::answer;
use crate::model::DataSource;
use crate::model::market::MarketKind;
use crate::service::market::search_rank::normalize;
use crate::service::{ProviderError, SearchResult};
use serde_json::Value;

/// What every perpetual's name ends in.
const SUFFIX: &str = "-PERPETUAL";

/// The names of the active perpetuals of a `get_instruments` answer, in name order (the inverse
/// `BTC-PERPETUAL` sorts before the linear `BTC_USDC-PERPETUAL`).
pub(super) fn parse_markets(body: &str) -> Result<Vec<String>, ProviderError> {
    let result = answer(body)?;
    let mut names: Vec<String> = result
        .as_array()
        .ok_or_else(|| ProviderError::data("expected a market list"))?
        .iter()
        .filter(|row| {
            row.get("settlement_period").and_then(Value::as_str) == Some("perpetual")
                && row.get("is_active").and_then(Value::as_bool) == Some(true)
        })
        .filter_map(|row| {
            row.get("instrument_name")?
                .as_str()
                .filter(|name| name.ends_with(SUFFIX))
                .map(str::to_owned)
        })
        .collect();
    names.sort();
    Ok(names)
}

/// The name Deribit takes for `symbol`, however it was typed: it refuses a name that is not upper
/// case. `None` for text that cannot be a name (nothing else is ever put in a request).
pub(super) fn instrument_name(symbol: &str) -> Option<String> {
    let name = symbol.trim().to_ascii_uppercase();
    (!name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_')))
    .then_some(name)
}

/// [`instrument_name`] for a market a request names: the error says Deribit does not list it when
/// `symbol` cannot be one.
pub(super) fn require_name(symbol: &str) -> Result<String, ProviderError> {
    instrument_name(symbol).ok_or_else(|| not_listed(symbol))
}

/// The error for a market Deribit does not list.
pub(super) fn not_listed(symbol: &str) -> ProviderError {
    ProviderError::unknown_market(
        DataSource::Deribit,
        symbol,
        "its perpetuals are named like BTC-PERPETUAL or SOL_USDC-PERPETUAL",
    )
}

/// The currency a perpetual is settled in, which is how Deribit groups its markets: the coin of an
/// inverse one (`BTC` of `BTC-PERPETUAL`), the quote of a linear one (`USDC` of
/// `SOL_USDC-PERPETUAL`). `None` for a name that is no perpetual's.
pub(super) fn settlement_of(name: &str) -> Option<&str> {
    let stem = name.strip_suffix(SUFFIX)?;
    let currency = stem.split_once('_').map_or(stem, |(_, quote)| quote);
    (!currency.is_empty()).then_some(currency)
}

/// The names that contain `needle` (upper case) once spelled with letters and digits only, as
/// search results.
pub(super) fn matching(names: &[String], needle: &str) -> Vec<SearchResult> {
    names
        .iter()
        .filter(|name| normalize(name).contains(needle))
        .map(|name| {
            let mut hit = SearchResult::new(DataSource::Deribit, name.clone(), name.clone());
            hit.kind = MarketKind::Perp;
            hit
        })
        .collect()
}
