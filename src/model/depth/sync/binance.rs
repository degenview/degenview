//! Binance's depth stream: a REST snapshot with a `lastUpdateId`, then diffs that each cover the
//! update ids `U..=u`. The first diff used must straddle the snapshot (`U <= id + 1 <= u`), and
//! every later one must start where the last ended (`U == previous u + 1`).

use super::{BookSync, Verdict};
use crate::model::depth::BookEvent;

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

/// A snapshot fetched over REST, then diffs that must follow it without a gap.
#[derive(Debug, Default)]
pub struct DiffStream {
    state: State,
}

impl BookSync for DiffStream {
    fn judge(&mut self, event: &BookEvent) -> Verdict {
        let delta = match event {
            BookEvent::Snapshot(snapshot) => {
                self.state = State::Snapshot(snapshot.id);
                return Verdict::Replace;
            }
            BookEvent::Delta(delta) => delta,
        };
        let fits = match self.state {
            State::Waiting => false,
            State::Snapshot(id) => {
                if delta.last <= id {
                    return Verdict::Stale;
                }
                delta.first <= id + 1
            }
            State::Following(previous) => {
                if delta.last <= previous {
                    return Verdict::Stale;
                }
                delta.first == previous + 1
            }
        };
        if !fits {
            self.state = State::Waiting;
            return Verdict::Gap;
        }
        self.state = State::Following(delta.last);
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
