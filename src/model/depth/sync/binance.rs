//! Binance's depth stream: a REST snapshot with a `lastUpdateId`, then diffs that each cover the
//! update ids `U..=u`. Binance's rule is one for every diff, the first included: a diff that ends
//! at or before the book's update id is old news and is dropped; one that starts after the id
//! plus one has missed updates and the book is rebuilt from a new snapshot; anything between holds
//! the update that comes next and is applied, even when it also repeats updates the book already
//! has (levels carry absolute sizes, so applying them again changes nothing).

use super::{BookSync, Verdict};
use crate::model::depth::BookEvent;

#[derive(Debug, Default)]
enum State {
    /// No snapshot yet, or the stream has a gap: a diff cannot be placed.
    #[default]
    Waiting,
    /// The book holds every update up to this id (the snapshot's, then the last diff applied).
    Synced(u64),
}

/// A snapshot fetched over REST, then diffs that must follow it without a gap.
#[derive(Debug, Default)]
pub struct DiffStream {
    state: State,
}

impl BookSync for DiffStream {
    fn judge(&mut self, event: &BookEvent) -> Verdict {
        let delta = match event {
            BookEvent::Snapshot(snapshot) => {
                self.state = State::Synced(snapshot.id);
                return Verdict::Replace;
            }
            BookEvent::Delta(delta) => delta,
        };
        let State::Synced(id) = self.state else {
            return Verdict::Gap;
        };
        if delta.last <= id {
            return Verdict::Stale;
        }
        if delta.first > id + 1 {
            self.state = State::Waiting;
            return Verdict::Gap;
        }
        self.state = State::Synced(delta.last);
        Verdict::Apply
    }

    fn needs_snapshot(&self) -> bool {
        matches!(self.state, State::Waiting)
    }

    fn is_synced(&self) -> bool {
        !self.needs_snapshot()
    }

    fn reset(&mut self) {
        self.state = State::Waiting;
    }
}
