//! Everything known about one market's depth, behind one door. The service feeds it what the
//! exchange says, the host asks it for ladders, and neither sees the book, the rules that keep it
//! right, or the trades behind it.

use super::group;
use super::ladder::{Ladder, Link, View};
use super::pull_stack::PullStack;
use super::sync::{BookSync, Verdict};
use super::{Book, BookEvent, Delta, Event, Price, Side, Snapshot, Tape};
use crate::model::taker_flow::Taker;

/// What an update did to the record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SyncOutcome {
    /// The record changed.
    Applied,
    /// The update was older than the book, or came before it could be used: ignored.
    Stale,
    /// The stream skipped something, or never started from a snapshot: the book has been emptied
    /// and cannot be trusted again until a snapshot arrives (see
    /// [`DepthRecord::needs_snapshot`]).
    NeedsSnapshot,
}

pub struct DepthRecord {
    book: Book,
    sync: Box<dyn BookSync>,
    tape: Tape,
    pull_stack: PullStack,
    tick: Option<Price>,
    connected: bool,
    /// Whether the socket was ever up: after that, a missing book is a reconnection.
    was_connected: bool,
    unlisted: bool,
    revision: u64,
}

impl DepthRecord {
    /// An empty record whose book `sync` judges the updates of.
    pub fn new(sync: Box<dyn BookSync>) -> Self {
        Self {
            book: Book::default(),
            sync,
            tape: Tape::default(),
            pull_stack: PullStack::default(),
            tick: None,
            connected: false,
            was_connected: false,
            unlisted: false,
            revision: 0,
        }
    }

    /// Counts the changes that can alter a ladder, so that one made for an earlier count is known
    /// to be out of date.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The market's smallest price step, once the exchange has said.
    pub fn set_tick(&mut self, tick: Price) {
        self.tick = Some(tick);
        self.unlisted = false;
        self.revision += 1;
    }

    /// The exchange does not list this market.
    pub fn mark_unlisted(&mut self) {
        self.unlisted = true;
        self.revision += 1;
    }

    /// The socket came up or went down. Going down lets the book go: it would be out of date by
    /// the time the socket is back, and the exchange sends it again.
    pub fn link(&mut self, up: bool) {
        self.connected = up;
        self.was_connected |= up;
        if !up {
            self.sync.reset();
            self.book.clear();
            self.pull_stack.clear();
        }
        self.revision += 1;
    }

    /// Takes in what the exchange said at `now_ms`. `NeedsSnapshot` says the book has been let go
    /// and a snapshot must be fetched and applied before it is of use.
    pub fn apply(&mut self, event: Event, now_ms: i64) -> SyncOutcome {
        let outcome = match event {
            Event::Book(event) => self.apply_book(event, now_ms),
            Event::Trade(trade) => {
                self.tape.push(trade);
                let side = match trade.taker {
                    Taker::Buy => Side::Ask,
                    Taker::Sell => Side::Bid,
                };
                self.pull_stack
                    .traded(side, trade.price, trade.size, now_ms);
                if let Some(mid) = self.book.mid() {
                    self.tape.prune(mid);
                }
                SyncOutcome::Applied
            }
        };
        if outcome != SyncOutcome::Stale {
            self.revision += 1;
        }
        outcome
    }

    /// Whether the book must be started from a fetched snapshot, given to [`Self::apply`], before
    /// the updates of the socket can be placed.
    pub fn needs_snapshot(&self) -> bool {
        self.sync.needs_snapshot()
    }

    fn apply_book(&mut self, event: BookEvent, now_ms: i64) -> SyncOutcome {
        let verdict = self.sync.judge(&event);
        let (BookEvent::Snapshot(Snapshot { levels, .. }) | BookEvent::Delta(Delta { levels, .. })) =
            &event;
        match verdict {
            Verdict::Replace => {
                self.book.replace(levels);
                self.pull_stack.clear();
                SyncOutcome::Applied
            }
            Verdict::Apply => {
                for change in self.book.apply(levels) {
                    self.pull_stack.changed(
                        change.side,
                        change.price,
                        change.before,
                        change.after,
                        now_ms,
                    );
                }
                SyncOutcome::Applied
            }
            Verdict::Stale => SyncOutcome::Stale,
            Verdict::Gap => {
                self.book.clear();
                self.pull_stack.clear();
                SyncOutcome::NeedsSnapshot
            }
        }
    }

    /// The ladder for `view` as of `now_ms`. While the book is not there to show, the ladder
    /// says why and has no rows.
    pub fn ladder(&self, view: View, now_ms: i64) -> Ladder {
        if self.unlisted {
            return Ladder::waiting(Link::Unlisted, self.tick);
        }
        let shown = self
            .tick
            .filter(|_| self.connected && self.sync.is_synced())
            .zip(self.book.mid());
        let Some((tick, mid)) = shown else {
            let link = if self.was_connected && !self.connected {
                Link::Reconnecting
            } else {
                Link::Connecting
            };
            return Ladder::waiting(link, self.tick);
        };
        let group = view
            .group
            .filter(|group| group::choices(tick).contains(group))
            .unwrap_or_else(|| group::automatic(tick, mid));
        let centre = view.centre.unwrap_or(mid);
        Ladder::build(
            &self.book,
            &self.tape,
            &self.pull_stack,
            tick,
            group,
            centre,
            now_ms,
        )
    }
}
