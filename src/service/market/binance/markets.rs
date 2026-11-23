//! Binance's market list, held for a day: where the shared search keeps it.

use crate::service::market::catalog::Catalog;
use crate::service::market::shape::binance::Pair;

pub(super) static CATALOG: Catalog<Pair> = Catalog::new("catalog_binance");
