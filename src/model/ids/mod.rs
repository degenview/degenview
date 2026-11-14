//! The id space engines mint into. Portfolios, paper-trading accounts and everything inside them
//! are numbered 1, 2, 3… by each Mac on its own; iCloud Sync tags those counters when they cross
//! into a synced record (`sync::record::ids`), so another Mac's ids arrive at or above
//! [`LEGACY_LIMIT`] and an engine's own counter never reaches them.

/// Counter ids stay far below this: a Mac would need 16 million portfolios or orders.
pub const LEGACY_LIMIT: u64 = 1 << 24;
/// Tagged ids live in `[GLOBAL_BASE, GLOBAL_BASE << 1)`, above counters and below nanosecond ids.
pub const GLOBAL_BASE: u64 = 1 << 55;
