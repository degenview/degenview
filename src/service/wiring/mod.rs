//! What the app's venues run on: the real network, clock, Keychain and lookup cache, behind the
//! ports `service::market::ports` defines, and the one set of venues built with them.
//!
//! The venues are shared by every caller (`service::for_source`, the search, the chart and the
//! alerts alike), none of which holds a context to pass down, so they are built once, on first
//! use. Nothing under `service::market` reaches for this: a venue is given its ports, which is
//! how a test gives it fakes (`service::market::ports::fakes`).

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
        credentials: Arc::new(credentials::KeychainCredentials),
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
