//! Market depth: the order book of one market and its trades, folded into the ladder a panel
//! draws. Pure: the sockets that fill a [`DepthRecord`] are in `service::depth`, and each
//! exchange's rules for judging its book updates are a [`BookSync`] in `sync`.

mod book;
mod event;
pub mod group;
mod ladder;
mod market;
mod price;
mod pull_stack;
mod record;
mod sync;
mod tape;
#[cfg(test)]
mod tests;

pub use book::{Book, Delta, Levels, Side, Snapshot};
pub use event::{BookEvent, Event};
pub use ladder::{CENTRE_ROW, Ladder, Link, Row, Scale, View, WINDOW_ROWS};
pub use market::{DepthMarket, market_of};
pub use price::Price;
pub use record::{DepthRecord, SyncOutcome};
pub use sync::{BookSync, DiffStream, SnapshotAndDeltas};
pub use tape::{Tape, Trade};
