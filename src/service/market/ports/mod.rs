//! What the venues need of the world outside them, as traits: the network, the clock, the user's
//! credentials and the lookup cache. A venue is built with the ports it uses and calls nothing
//! global, so its paging, retries and error handling run in a test on recorded answers.
//!
//! The ports are bundled in a [`context::MarketContext`]. The implementations the app runs on
//! (`service::net::http`, the system clock, the Keychain, the lookup cache) are in
//! `service::wiring`, which `layout_tests.rs` keeps the only place that names them.

pub mod cache;
pub mod clock;
pub mod context;
pub mod credentials;
pub mod http;

#[cfg(test)]
pub(in crate::service) mod fakes;
