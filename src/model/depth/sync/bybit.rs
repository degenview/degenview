//! Bybit's order book topic: a snapshot when the subscription starts, and again whenever Bybit
//! restarts the book (its `u` is then 1); deltas in between. A snapshot replaces the book; a delta
//! older than the last update seen is dropped. Bybit sends no gaps, only fresh snapshots.

use super::{BookSync, SyncOutcome};
use crate::model::depth::{Book, BookEvent};

#[derive(Debug, Default)]
pub(super) struct SnapshotAndDeltas {
    /// The update id the book is at, once a snapshot is in.
    last: Option<u64>,
}

impl BookSync for SnapshotAndDeltas {
    fn apply(&mut self, book: &mut Book, event: BookEvent) -> SyncOutcome {
        match event {
            BookEvent::Snapshot(snapshot) => {
                book.replace(&snapshot.levels);
                self.last = Some(snapshot.id);
                SyncOutcome::Applied
            }
            BookEvent::Delta(delta) => match self.last {
                Some(last) if delta.last > last => {
                    book.apply(&delta.levels);
                    self.last = Some(delta.last);
                    SyncOutcome::Applied
                }
                _ => SyncOutcome::Stale,
            },
        }
    }

    fn is_synced(&self) -> bool {
        self.last.is_some()
    }

    fn reset(&mut self, book: &mut Book) {
        book.clear();
        self.last = None;
    }
}
