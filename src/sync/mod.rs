//! iCloud Sync. SQLite stays the source of truth; CloudKit is only a transport. The pure logic
//! here (policy, records, merge, outbox) knows nothing about CloudKit so it is testable without
//! Apple credentials. See docs/icloud-sync.md.
// The sync layer lands in stages; items are wired up as later stages consume them.
#![allow(dead_code)]

pub mod apply;
pub mod core;
pub mod merge;
pub mod outbox;
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
mod core_tests;
#[cfg(test)]
mod script_tests;
