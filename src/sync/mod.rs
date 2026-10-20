//! iCloud Sync. SQLite stays the source of truth; CloudKit is only a transport. The pure logic
//! here (policy, records, merge, outbox) knows nothing about CloudKit so it is testable without
//! Apple credentials. See docs/icloud-sync.md.
// The sync layer lands in stages; items are wired up as later stages consume them.
#![allow(dead_code)]

pub mod policy;
pub mod record;
pub mod validate;
