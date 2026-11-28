//! What the app's venues run on: the real network, clock, Keychain and lookup cache, behind the
//! ports `service::market::ports` defines, and the one set of venues built with them.
//!
//! The venues are shared by every caller in `service` (`for_source`, the search, the chart, the
//! alerts and the cache clearing alike), none of which holds a context to pass down, so they are
//! built once, on first use. The module is private to `service`: the layers above reach a venue
//! through a function of `service` (`for_source`, `cmc_test_connection`, ...), never through this.
//! A venue is given its ports and names none of this, which is how a test gives it fakes
//! (`service::market::ports::fakes`); `layout_tests.rs` keeps `service::market` off it.

mod cache;
mod clock;
mod credentials;
mod http;

use crate::service::market::ports::context::MarketContext;
use crate::service::market::venues::Venues;
use std::sync::{Arc, OnceLock};

/// The ports of the running app.
fn context() -> MarketContext {
    MarketContext {
        http: Arc::new(http::NetHttp),
        clock: Arc::new(clock::SystemClock),
        credentials: Arc::new(credentials::KeychainCredentials::app()),
        cache: Arc::new(cache::SharedLookups),
    }
}

/// The app's venues.
pub fn venues() -> &'static Venues {
    static VENUES: OnceLock<Venues> = OnceLock::new();
    VENUES.get_or_init(|| Venues::new(&context()))
}

#[cfg(test)]
mod tests;
