//! iCloud Sync. SQLite stays the source of truth; CloudKit is only a transport. The pure logic
//! here (policy, records, merge, outbox) knows nothing about CloudKit so it is testable without
//! Apple credentials. See docs/icloud-sync.md.
// The sync layer lands in stages; items are wired up as later stages consume them.
#![allow(dead_code)]

pub mod account;
pub mod apply;
pub mod core;
pub mod ids;
pub mod initial;
pub mod merge;
pub mod native;
pub mod outbox;
pub mod paper;
pub mod policy;
pub mod project;
pub mod record;
pub mod scripts;
pub mod transport;
pub mod validate;

#[cfg(test)]
pub mod mock;

#[cfg(test)]
mod apply_tests;
#[cfg(test)]
mod asset_tests;
#[cfg(test)]
mod core_tests;
#[cfg(test)]
mod crash_tests;
#[cfg(test)]
mod initial_tests;
#[cfg(test)]
mod paper_tests;
#[cfg(test)]
mod perf_tests;
#[cfg(test)]
mod portfolio_tests;
#[cfg(test)]
mod script_tests;
