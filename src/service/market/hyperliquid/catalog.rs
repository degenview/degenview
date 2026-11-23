//! Hyperliquid's market list, downloaded at most once a day: the main dex, every HIP-3 dex and the
//! spot pairs, one `/info` query each.

use super::markets::{Market, parse_dexes, parse_perps, parse_spot};
use super::rest::info;
use crate::lock::lock;
use crate::service::ProviderError;
use crate::service::market::catalog::Catalog;
use serde_json::json;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub(super) static CATALOG: Catalog<Market> = Catalog::new("catalog_hyperliquid");

/// Forgets the list held in memory (after the disk cache was cleared).
pub(super) fn clear() {
    CATALOG.clear_memory();
}

/// Reads the market list into memory for the candle feed to tell the names Hyperliquid lists from
/// the ones it does not (see `live.rs`), unless it is there or was asked for in the last five
/// minutes and could not be had. Waits on the network: for a worker, never the UI thread.
pub(super) fn warm() {
    static FAILED: Mutex<Option<Instant>> = Mutex::new(None);
    if CATALOG.held().is_some() {
        return;
    }
    if lock(&FAILED).is_some_and(|at| at.elapsed() < RETRY_AFTER_FAILURE) {
        return;
    }
    *lock(&FAILED) = CATALOG.get(download).is_err().then(Instant::now);
}

/// How long a market list that could not be had is left alone.
const RETRY_AFTER_FAILURE: Duration = Duration::from_secs(300);

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
