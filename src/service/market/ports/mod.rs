//! What the venues need of the world outside them, as traits: the network, the clock, the user's
//! credentials and the lookup cache. A venue is built with the ports it uses and calls nothing
//! global, so its paging, retries and error handling run in a test on recorded answers.
//!
//! A port speaks in its own types (`http::Request`, `http::Error`, `http::Retry`,
//! `cache::Cached`), never the infrastructure's, so what a venue is written against does not
//! depend on `service::net` or `service::cache`. The ports are bundled in a
//! [`context::MarketContext`]. The implementations the app runs on (`service::net::http`, the
//! system clock, the Keychain, the lookup cache) are in `service::wiring`, which converts between
//! the two sets of types. `layout_tests.rs` fails for production code under `service::market` that
//! names the infrastructure (by `use`, alias or full path) or calls `service::for_source` or a
//! `service::fetch_*`, which answer from the wiring; its known-debt list (`fx_rates`,
//! `coin_index`) only shrinks.

pub mod cache;
pub mod clock;
pub mod context;
pub mod credentials;
pub mod http;

#[cfg(test)]
pub(in crate::service) mod fakes;
