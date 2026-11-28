//! The ports a venue is built with.

use super::cache::CacheStore;
use super::clock::Clock;
use super::credentials::Credentials;
use super::http::Http;
use crate::service::market::catalog::registry::Registry;
use std::sync::Arc;

#[derive(Clone)]
pub struct MarketContext {
    pub http: Arc<dyn Http>,
    pub clock: Arc<dyn Clock>,
    pub credentials: Arc<dyn Credentials>,
    pub cache: Arc<dyn CacheStore>,
    /// The catalogs built from this context, which forget their lists when the cache is cleared.
    pub catalogs: Arc<Registry>,
}
