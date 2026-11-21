//! BingX's market list, held for a day, and the search over it.
//!
//! `/openApi/spot/v1/common/symbols` answers every market ever listed (2,304 on 2026-11-21, 680 KB)
//! as `{"symbol":"BTC-USDT","status":1,"tickSize":0.01,…}`. `status` 1 is a market in trading; the
//! rest (0 is delisted, and 5, 10 and 25 are markets BingX has not opened or has halted) are left
//! out. `tickSize` gives the digits of a price (`precision.rs`).

use super::parse::answer;
use super::precision::decimals_of_tick;
use super::rest::get;
use crate::model::DataSource;
use crate::service::market::catalog::Catalog;
use crate::service::{ProviderError, SearchResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `status` of a market in trading.
const TRADING: i64 = 1;

/// One market in trading: all the search and the candles need of it. The id is BingX's own,
/// `BTC-USDT`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct Pair {
    pub(super) id: String,
    pub(super) base: String,
    pub(super) quote: String,
    /// The digits of its prices after the decimal point.
    pub(super) decimals: u32,
}

/// The markets in trading, downloaded at most once a day.
pub(super) static CATALOG: Catalog<Pair> = Catalog::new("catalog_bingx");

/// Forgets the list held in memory (after the disk cache was cleared).
pub(super) fn clear_catalog() {
    CATALOG.clear_memory();
}

pub(super) fn download_pairs() -> Result<Vec<Pair>, ProviderError> {
    parse_pairs(&get("/openApi/spot/v1/common/symbols")?)
}

/// The markets in trading of an answer.
pub(super) fn parse_pairs(body: &str) -> Result<Vec<Pair>, ProviderError> {
    let data = answer(body)?.data()?;
    Ok(data
        .get("symbols")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::data("expected a market list"))?
        .iter()
        .filter(|row| row.get("status").and_then(Value::as_i64) == Some(TRADING))
        .filter_map(|row| {
            let id = row.get("symbol")?.as_str()?.to_uppercase();
            let (base, quote) = id.split_once('-')?;
            Some(Pair {
                base: base.to_owned(),
                quote: quote.to_owned(),
                decimals: decimals_of_tick(row.get("tickSize")?.as_f64()?)?,
                id,
            })
        })
        .filter(|pair| !pair.base.is_empty() && !pair.quote.is_empty() && !pair.quote.contains('-'))
        .collect())
}

/// The markets whose name contains `needle` (letters and digits, upper case).
pub(super) fn matching_pairs(pairs: &[Pair], needle: &str) -> Vec<SearchResult> {
    pairs
        .iter()
        .filter(|pair| pair.id.replace('-', "").contains(needle))
        .map(|pair| {
            SearchResult::new(DataSource::Bingx, pair.id.clone(), pair.id.clone())
                .pair(pair.base.clone(), pair.quote.clone())
        })
        .collect()
}

/// The pair of the market `id` (upper case) among those in trading.
pub(super) fn find<'a>(pairs: &'a [Pair], id: &str) -> Option<&'a Pair> {
    pairs.iter().find(|pair| pair.id == id)
}

/// The digits of the prices of each of `pairs`, by id.
pub(super) fn decimals_of(pairs: &[Pair]) -> std::collections::HashMap<String, u32> {
    pairs
        .iter()
        .map(|pair| (pair.id.clone(), pair.decimals))
        .collect()
}

/// The digits of the prices of every market in trading, by id, from the list held in memory
/// (a chart's own candle request is what reads it); empty while none is held, in which case prices
/// are used as they come. It never waits on the network or the disk: the socket threads that ask
/// must not block a reconnect on a 680 KB download.
pub(super) fn price_decimals() -> std::collections::HashMap<String, u32> {
    CATALOG
        .held()
        .map(|pairs| decimals_of(&pairs))
        .unwrap_or_default()
}
