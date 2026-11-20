use super::*;
use crate::lock::lock;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

const NOW: i64 = 5_000_000_000;

/// A feed that is running until told it is not.
struct Fake {
    alive: Arc<AtomicBool>,
}

impl Running for Fake {
    fn is_running(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }
}

/// A map of records that are counters, counting how many feeds were started.
struct Rig {
    map: LeaseMap<String, u32, Fake, ()>,
    started: AtomicUsize,
}

impl Rig {
    fn new() -> Self {
        Self {
            map: LeaseMap::default(),
            started: AtomicUsize::new(0),
        }
    }

    fn ask(&mut self, key: &str, now: i64) -> (Arc<Mutex<u32>>, Arc<AtomicBool>) {
        let started = &self.started;
        let mut alive = None;
        let entry = self.map.entry_for(
            &key.to_owned(),
            now,
            || 0,
            |_, _| {
                started.fetch_add(1, Ordering::AcqRel);
                let flag = Arc::new(AtomicBool::new(true));
                alive = Some(Arc::clone(&flag));
                Fake { alive: flag }
            },
        );
        let flag = alive.unwrap_or_else(|| Arc::clone(&entry.feed.alive));
        (Arc::clone(&entry.record), flag)
    }

    fn started(&self) -> usize {
        self.started.load(Ordering::Acquire)
    }

    fn holds(&mut self, key: &str) -> bool {
        self.map.get_mut(&key.to_owned()).is_some()
    }
}

#[test]
fn two_windows_on_two_keys_each_keep_their_feed_and_their_record() {
    let mut rig = Rig::new();
    let (btc, _) = rig.ask("BTC", NOW);
    let (eth, _) = rig.ask("ETH", NOW);
    *lock(&btc) = 1_000;
    // Both windows draw, frame after frame.
    for frame in 1..20 {
        let at = NOW + frame * 16;
        let (again_btc, _) = rig.ask("BTC", at);
        let (again_eth, _) = rig.ask("ETH", at);
        assert!(Arc::ptr_eq(&btc, &again_btc));
        assert!(Arc::ptr_eq(&eth, &again_eth));
    }
    assert_eq!(rig.started(), 2, "a feed was restarted by the other window");
    assert_eq!(*lock(&btc), 1_000, "BTC kept its history");
    assert_eq!(*lock(&eth), 0);
}

#[test]
fn a_late_write_of_an_old_feed_never_reaches_another_keys_record() {
    let mut rig = Rig::new();
    let (btc, _) = rig.ask("BTC", NOW);
    // Nobody asks about BTC for longer than a lease; ETH is asked, and BTC's feed is let go.
    let later = NOW + 10_000;
    let (eth, _) = rig.ask("ETH", later);
    assert!(!rig.holds("BTC"));
    // A BTC socket still reading a frame writes into the record it was given.
    *lock(&btc) = 9_000;
    assert_eq!(*lock(&eth), 0);
    // And asking about BTC again starts over, with a record of its own.
    let (new_btc, _) = rig.ask("BTC", later + 100);
    assert!(!Arc::ptr_eq(&btc, &new_btc));
    assert_eq!(*lock(&new_btc), 0);
}

#[test]
fn a_feed_that_stopped_is_replaced_with_a_fresh_record() {
    let mut rig = Rig::new();
    let (record, alive) = rig.ask("BTC", NOW);
    *lock(&record) = 5;
    alive.store(false, Ordering::Release);
    let (replaced, _) = rig.ask("BTC", NOW + 100);
    assert_eq!(rig.started(), 2);
    assert!(!Arc::ptr_eq(&record, &replaced));
    assert_eq!(*lock(&replaced), 0);
}

#[test]
fn a_key_a_window_is_still_drawing_is_never_let_go_by_another_window() {
    let mut rig = Rig::new();
    rig.ask("BTC", NOW);
    rig.ask("ETH", NOW + 2_000);
    // BTC was asked 2 s ago: well within the lease.
    assert!(rig.holds("BTC"));
    rig.ask("ETH", NOW + 3_500);
    assert!(!rig.holds("BTC"));
}

#[test]
fn a_value_is_kept_while_its_key_holds() {
    let mut memo: Memo<(u64, i64), String> = Memo::default();
    let made = AtomicUsize::new(0);
    let mut get = |key| {
        memo.get_or(key, || {
            made.fetch_add(1, Ordering::AcqRel);
            format!("{key:?}")
        })
    };
    let first = get((1, 10));
    assert!(Arc::ptr_eq(&first, &get((1, 10))));
    let second = get((1, 11));
    assert!(!Arc::ptr_eq(&first, &second));
    assert_eq!(*second, "(1, 11)");
    assert_eq!(made.load(Ordering::Acquire), 2);
}
