#![allow(dead_code)] // transient until the analytics and the journal tab use all of it
//! The trading journal's pure domain layer: no egui, no SQLite, no network.
//!
//! Trading sources (the paper engine today; brokers and file imports later) hand over
//! [`Execution`]s through [`source::TradingActivitySource`]. Everything else is derived from
//! executions: [`reconstruct`] groups them into [`Trade`]s and analytics read the trades. See
//! `docs/trading-journal.md`.

pub mod accounting;
pub mod analytics;
pub mod backup;
pub mod breakdown;
pub mod calendar;
pub mod cells;
pub mod compare;
pub mod csv;
pub mod daily;
pub mod edge;
pub mod equity;
pub mod excursion;
pub mod filters;
pub mod format;
pub mod marks;
pub mod metric;
pub mod model;
pub mod normalize;
pub mod overview;
pub mod playbook;
pub mod process;
pub mod reconstruct;
pub mod report;
pub mod reviews;
pub mod risk;
pub mod source;
pub mod sources;
pub mod table;

#[cfg(test)]
pub(crate) mod testing;
