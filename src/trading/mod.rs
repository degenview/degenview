//! Trading, as the screens see it: one vocabulary and one trait for every venue (the paper account
//! and each connected broker). See [`backend::TradingBackend`]. No drawing code lives here.

#![allow(dead_code)] // The order sheet and the orders panel read all of this from the next change on.

pub mod backend;
pub mod broker_backend;
pub mod confirm;
pub mod draft;
pub mod model;
pub mod paper_backend;
pub mod preview;
pub mod task;
pub mod valuation;

#[allow(unused_imports)]
pub use backend::TradingBackend;
#[cfg(test)]
mod conformance_tests;
