//! What a host that keeps one feed per thing viewers look at has in common: a feed starts the
//! first time a panel asks for its key, lives on a short lease the panel renews each frame it is
//! drawn, and is let go (closing its sockets) when nobody has asked for a lease's length.
//!
//! Every key has a feed, record and lease of its own, so two windows looking at two keys do not
//! disturb each other, and a socket of one key that is slow to close can never write into the
//! record of another.

mod memo;
#[cfg(test)]
mod tests;

pub use memo::Memo;

use crate::service::stream::Lease;
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How long a feed runs after a panel last asked: longer than a frame, so a frame that is late
/// does not close the sockets, and short enough that a tab in the background stops them.
pub const LEASE: Duration = Duration::from_secs(3);

/// What a feed must say for the host to keep it: whether it still serves all its sockets.
pub trait Running {
    fn is_running(&self) -> bool;
}

/// One key's feed, and everything that belongs to it: dropping the entry closes its sockets, and
/// what it recorded goes with it.
pub struct Entry<R, F, C> {
    pub record: Arc<Mutex<R>>,
    pub feed: F,
    /// What the host made from the record last, to hand out again while it still holds.
    pub cache: C,
    lease: Lease,
    /// When a panel last asked, in milliseconds since the epoch.
    asked_ms: i64,
}

/// The entries of a host, by key.
pub struct LeaseMap<K, R, F, C> {
    entries: HashMap<K, Entry<R, F, C>>,
}

impl<K, R, F, C> Default for LeaseMap<K, R, F, C> {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }
}

impl<K: Clone + Eq + Hash, R, F: Running, C: Default> LeaseMap<K, R, F, C> {
    /// The entry of `key`, asked for at `now_ms`. A new one, with a record of its own from
    /// `record`, replaces one whose feed does not serve all its sockets any more; and the entries
    /// nobody has asked about for a [`LEASE`] are let go, their sockets closing as they are
    /// dropped. `start` makes the feed, given the record and the lease it must follow.
    pub fn entry_for(
        &mut self,
        key: &K,
        now_ms: i64,
        record: impl FnOnce() -> R,
        start: impl FnOnce(&Arc<Mutex<R>>, &Lease) -> F,
    ) -> &mut Entry<R, F, C> {
        let lapsed = LEASE.as_millis() as i64;
        self.entries
            .retain(|name, entry| name == key || now_ms - entry.asked_ms <= lapsed);
        if self
            .entries
            .get(key)
            .is_some_and(|entry| !entry.feed.is_running())
        {
            self.entries.remove(key);
        }
        let entry = self.entries.entry(key.clone()).or_insert_with(|| {
            let record = Arc::new(Mutex::new(record()));
            let lease = Lease::default();
            // Held before the feed starts, or its watchdog ends it at once.
            lease.renew(LEASE);
            let feed = start(&record, &lease);
            Entry {
                record,
                feed,
                cache: C::default(),
                lease,
                asked_ms: now_ms,
            }
        });
        entry.lease.renew(LEASE);
        entry.asked_ms = now_ms;
        entry
    }

    #[cfg(test)]
    pub fn get_mut(&mut self, key: &K) -> Option<&mut Entry<R, F, C>> {
        self.entries.get_mut(key)
    }
}
