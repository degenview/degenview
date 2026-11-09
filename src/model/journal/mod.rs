#![allow(dead_code)] // transient until the analytics and the journal tab use all of it
//! The trading journal's pure domain layer: no egui, no SQLite, no network.
//!
//! Trading sources (the paper engine today; brokers and file imports later) hand over
//! [`Execution`]s through [`source::TradingActivitySource`]. Everything else is derived from
//! executions: [`reconstruct`] groups them into [`Trade`]s and analytics read the trades. See
//! `docs/trading-journal.md`.

pub mod accounting;
pub mod model;
pub mod normalize;
pub mod reconstruct;
pub mod source;
pub mod sources;

#[cfg(test)]
pub(crate) mod testing;
