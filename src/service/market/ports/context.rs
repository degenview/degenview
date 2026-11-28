//! The ports a venue is built with.

use super::cache::CacheStore;
use super::clock::Clock;
use super::credentials::Credentials;
use super::http::Http;
use std::sync::Arc;

#[derive(Clone)]
pub struct MarketContext {
    pub http: Arc<dyn Http>,
    pub clock: Arc<dyn Clock>,
    pub credentials: Arc<dyn Credentials>,
    pub cache: Arc<dyn CacheStore>,
}
