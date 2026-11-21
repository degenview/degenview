//! Deribit's market list, downloaded at most once a day and searched locally.

use super::markets::parse_markets;
use super::rest::get;
use crate::service::ProviderError;
use crate::service::market::catalog::Catalog;

pub(super) static CATALOG: Catalog<String> = Catalog::new("catalog_deribit");

/// Forgets the list held in memory (after the disk cache was cleared).
pub(super) fn clear() {
    CATALOG.clear_memory();
}

pub(super) fn download() -> Result<Vec<String>, ProviderError> {
    parse_markets(&get("/get_instruments?currency=any&kind=future")?)
}
