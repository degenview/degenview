//! Binance's depth stream: a REST snapshot with a `lastUpdateId`, then diffs that each cover the
//! update ids `U..=u`. The first diff used must straddle the snapshot (`U <= id + 1 <= u`), and
//! every later one must start where the last ended (`U == previous u + 1`).

use super::{BookSync, SyncOutcome};
use crate::model::depth::{Book, BookEvent};

#[derive(Debug, Default)]
enum State {
    /// No snapshot yet: a diff cannot be placed.
    #[default]
    Waiting,
    /// The snapshot with this id is in; the first diff to use has not arrived.
    Snapshot(u64),
    /// Diffs apply up to this update id.
    Following(u64),
}

#[derive(Debug, Default)]
pub(super) struct DiffStream {
    state: State,
}

impl BookSync for DiffStream {
    fn apply(&mut self, book: &mut Book, event: BookEvent) -> SyncOutcome {
        let delta = match event {
            BookEvent::Snapshot(snapshot) => {
                book.replace(&snapshot.levels);
                self.state = State::Snapshot(snapshot.id);
                return SyncOutcome::Applied;
            }
            BookEvent::Delta(delta) => delta,
        };
        let fits = match self.state {
            State::Waiting => false,
            State::Snapshot(id) => {
                if delta.last <= id {
                    return SyncOutcome::Stale;
                }
                delta.first <= id + 1
            }
            State::Following(previous) => {
                if delta.last <= previous {
                    return SyncOutcome::Stale;
                }
                delta.first == previous + 1
            }
        };
        if !fits {
            book.clear();
            self.state = State::Waiting;
            return SyncOutcome::NeedsSnapshot;
        }
        book.apply(&delta.levels);
        self.state = State::Following(delta.last);
        SyncOutcome::Applied
    }

    fn is_synced(&self) -> bool {
        !matches!(self.state, State::Waiting)
    }

    fn reset(&mut self, book: &mut Book) {
        book.clear();
        self.state = State::Waiting;
    }
}
