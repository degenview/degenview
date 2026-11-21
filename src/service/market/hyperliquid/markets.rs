//! Hyperliquid's market list as `Market`s, from the three answers that make it, and the search over
//! it. Pure: `catalog.rs` downloads and keeps it.
//!
//! - `meta` lists the perpetuals of the main dex (`BTC`), `meta` with a `dex` those of a HIP-3 dex
//!   under its prefix (`xyz:SP500`), and `perpDexs` the dexes. A market no longer traded is
//!   flagged `isDelisted` and left out.
//! - `spotMeta` lists the spot pairs by the indices of their two tokens. A pair is charted by its
//!   `name`, `PURR/USDC` or `@107` for one Hyperliquid numbers, and shown by its tokens.

use super::names::name_of;
use crate::model::DataSource;
use crate::model::market::MarketKind;
use crate::service::market::search_rank::normalize;
use crate::service::{NAME_SEPARATOR, ProviderError, SearchResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// One market the search can offer.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct Market {
    /// What the API takes: `BTC`, `xyz:SP500`, `@107`.
    pub(super) id: String,
    /// What else the market is called, shown beside it and searched with it.
    pub(super) also_called: Option<String>,
    /// A spot pair's tokens.
    pub(super) pair: Option<(String, String)>,
    pub(super) kind: MarketKind,
}

impl Market {
    /// The market as the picker shows it: a pair by its tokens (`HYPE/USDC`), a perpetual by its
    /// name, with what it is also called after it.
    fn label(&self) -> String {
        match (&self.pair, &self.also_called) {
            (Some((base, quote)), _) => format!("{base}/{quote}"),
            (None, Some(name)) => format!("{}{NAME_SEPARATOR}{name}", self.id),
            (None, None) => self.id.clone(),
        }
    }

    fn matches(&self, needle: &str) -> bool {
        normalize(&self.label()).contains(needle) || normalize(&self.id).contains(needle)
    }

    fn hit(&self) -> SearchResult {
        let mut hit = SearchResult::new(DataSource::Hyperliquid, self.label(), self.id.clone());
        if let Some((base, quote)) = &self.pair {
            hit = hit.pair(base.clone(), quote.clone());
        }
        hit.kind = self.kind;
        hit
    }
}

/// The markets whose name, spelled with letters and digits only, contains `needle` (upper case).
pub(super) fn matching(markets: &[Market], needle: &str) -> Vec<SearchResult> {
    markets
        .iter()
        .filter(|market| market.matches(needle))
        .map(Market::hit)
        .collect()
}

fn parse_json(body: &str) -> Result<Value, ProviderError> {
    serde_json::from_str(body).map_err(ProviderError::data)
}

/// The perpetuals of a `meta` answer that are still traded.
pub(super) fn parse_perps(body: &str) -> Result<Vec<Market>, ProviderError> {
    let value = parse_json(body)?;
    let universe = value
        .get("universe")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::data("expected a market list"))?;
    Ok(universe
        .iter()
        .filter(|row| row.get("isDelisted").and_then(Value::as_bool) != Some(true))
        .filter_map(|row| {
            let id = row.get("name")?.as_str().filter(|name| !name.is_empty())?;
            Some(Market {
                id: id.to_owned(),
                also_called: name_of(id).map(str::to_owned),
                pair: None,
                kind: MarketKind::Perp,
            })
        })
        .collect())
}

/// The names of the HIP-3 dexes in a `perpDexs` answer: a list that opens with `null` for the main
/// dex.
pub(super) fn parse_dexes(body: &str) -> Result<Vec<String>, ProviderError> {
    let value = parse_json(body)?;
    Ok(value
        .as_array()
        .ok_or_else(|| ProviderError::data("expected a dex list"))?
        .iter()
        .filter_map(|dex| dex.get("name")?.as_str().filter(|name| !name.is_empty()))
        .map(str::to_owned)
        .collect())
}

/// The spot pairs of a `spotMeta` answer. A pair names its tokens by their `index`, which is not
/// their position in the list.
pub(super) fn parse_spot(body: &str) -> Result<Vec<Market>, ProviderError> {
    let value = parse_json(body)?;
    let tokens: HashMap<i64, &str> = value
        .get("tokens")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::data("expected a token list"))?
        .iter()
        .filter_map(|token| Some((token.get("index")?.as_i64()?, token.get("name")?.as_str()?)))
        .collect();
    let universe = value
        .get("universe")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::data("expected a pair list"))?;
    Ok(universe
        .iter()
        .filter_map(|row| {
            let id = row.get("name")?.as_str().filter(|name| !name.is_empty())?;
            let [base, quote] = row.get("tokens")?.as_array()?.as_slice() else {
                return None;
            };
            let token = |index: &Value| tokens.get(&index.as_i64()?).map(|name| (*name).to_owned());
            Some(Market {
                id: id.to_owned(),
                also_called: None,
                pair: Some((token(base)?, token(quote)?)),
                kind: MarketKind::Spot,
            })
        })
        .collect())
}
