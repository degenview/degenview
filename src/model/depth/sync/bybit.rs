//! Bybit's order book topic: a snapshot when the subscription starts, and again whenever Bybit
//! restarts the book (its `u` is then 1); deltas in between. A snapshot replaces the book; a delta
//! older than the last update seen is dropped. Bybit sends no gaps, only fresh snapshots.

use super::{BookSync, Verdict};
use crate::model::depth::BookEvent;

/// Snapshots from the socket itself, then deltas.
#[derive(Debug, Default)]
pub struct SnapshotAndDeltas {
    /// The update id the book is at, once a snapshot is in.
    last: Option<u64>,
}

impl BookSync for SnapshotAndDeltas {
    fn judge(&mut self, event: &BookEvent) -> Verdict {
        match event {
            BookEvent::Snapshot(snapshot) => {
                self.last = Some(snapshot.id);
                Verdict::Replace
            }
            BookEvent::Delta(delta) => match self.last {
                Some(last) if delta.last > last => {
                    self.last = Some(delta.last);
                    Verdict::Apply
                }
                _ => Verdict::Stale,
            },
        }
    }

    fn needs_snapshot(&self) -> bool {
        false
    }

    fn is_synced(&self) -> bool {
        self.last.is_some()
    }

    fn reset(&mut self) {
        self.last = None;
    }
}
