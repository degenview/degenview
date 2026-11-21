//! Lighter's market list, downloaded at most once a day, and the lookups every request makes in it:
//! a symbol is asked for by its number, and a socket message names its market by number.

use super::markets::{Market, id_of, parse_markets, resolve};
use super::rest::get;
use crate::service::ProviderError;
use crate::service::market::catalog::Catalog;

pub(super) static CATALOG: Catalog<Market> = Catalog::new("catalog_lighter");

/// Forgets the list held in memory (after the disk cache was cleared).
pub(super) fn clear() {
    CATALOG.clear_memory();
}

pub(super) fn download() -> Result<Vec<Market>, ProviderError> {
    parse_markets(&get("/orderBooks")?)
}

/// The number of `symbol`, downloading the list when it is not held, or again when it is not in it
/// (Lighter lists markets all the time and the list is held for a day; see
/// [`Catalog::get_or_refresh_on_miss`]); the error says Lighter does not list the market, for a name
/// that is not in it. Waits on the network: not for the UI thread.
pub(super) fn market_id(symbol: &str) -> Result<u32, ProviderError> {
    let markets =
        CATALOG.get_or_refresh_on_miss(download, |markets| id_of(markets, symbol).is_some())?;
    resolve(&markets, symbol)
}
