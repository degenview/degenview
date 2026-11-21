//! Hyperliquid's market list, downloaded at most once a day: the main dex, every HIP-3 dex and the
//! spot pairs, one `/info` query each.

use super::markets::{Market, parse_dexes, parse_perps, parse_spot};
use super::rest::info;
use crate::service::ProviderError;
use crate::service::market::catalog::Catalog;
use serde_json::json;

pub(super) static CATALOG: Catalog<Market> = Catalog::new("catalog_hyperliquid");

/// Forgets the list held in memory (after the disk cache was cleared).
pub(super) fn clear() {
    CATALOG.clear_memory();
}

/// Every market, or the error of the first query that failed: a list without a dex would be held
/// for a day, so a partial answer is none.
pub(super) fn download() -> Result<Vec<Market>, ProviderError> {
    let mut markets = parse_perps(&info(&json!({"type": "meta"}))?)?;
    for dex in parse_dexes(&info(&json!({"type": "perpDexs"}))?)? {
        markets.extend(parse_perps(&info(&json!({"type": "meta", "dex": dex}))?)?);
    }
    markets.extend(parse_spot(&info(&json!({"type": "spotMeta"}))?)?);
    Ok(markets)
}
