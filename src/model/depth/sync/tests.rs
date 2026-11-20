use super::*;
use crate::model::depth::{Delta, Levels, Price, Snapshot};

fn price(text: &str) -> Price {
    Price::parse(text).unwrap()
}

fn levels(bids: &[(&str, f64)], asks: &[(&str, f64)]) -> Levels {
    let side = |rows: &[(&str, f64)]| rows.iter().map(|(p, s)| (price(p), *s)).collect();
    Levels {
        bids: side(bids),
        asks: side(asks),
    }
}

fn snapshot(id: u64) -> BookEvent {
    BookEvent::Snapshot(Snapshot {
        id,
        levels: levels(&[("99", 1.0), ("98", 2.0)], &[("101", 1.5), ("102", 2.5)]),
    })
}

fn delta(first: u64, last: u64, bids: &[(&str, f64)], asks: &[(&str, f64)]) -> BookEvent {
    BookEvent::Delta(Delta {
        first,
        last,
        levels: levels(bids, asks),
    })
}

#[test]
fn binance_cannot_place_a_diff_before_a_snapshot() {
    let mut sync = DiffStream::default();
    assert!(sync.needs_snapshot());
    assert!(!sync.is_synced());
    assert_eq!(sync.judge(&delta(5, 6, &[("99", 9.0)], &[])), Verdict::Gap);
}

#[test]
fn binance_drops_diffs_the_snapshot_already_has_and_uses_the_one_that_straddles_it() {
    let mut sync = DiffStream::default();
    assert_eq!(sync.judge(&snapshot(100)), Verdict::Replace);
    assert!(!sync.needs_snapshot());
    let old = delta(90, 100, &[("99", 7.0)], &[]);
    assert_eq!(sync.judge(&old), Verdict::Stale);
    let straddling = delta(98, 103, &[("99", 3.0)], &[("101", 0.0)]);
    assert_eq!(sync.judge(&straddling), Verdict::Apply);
}

#[test]
fn binance_wants_the_first_diff_to_start_no_later_than_the_snapshot_ends() {
    let mut sync = DiffStream::default();
    sync.judge(&snapshot(100));
    let late = delta(102, 105, &[("99", 3.0)], &[]);
    assert_eq!(sync.judge(&late), Verdict::Gap);
    assert!(!sync.is_synced());
    assert!(sync.needs_snapshot());
}

#[test]
fn binance_diffs_must_follow_each_other_and_a_gap_asks_for_a_snapshot() {
    let mut sync = DiffStream::default();
    sync.judge(&snapshot(100));
    assert_eq!(
        sync.judge(&delta(100, 101, &[("99", 2.0)], &[])),
        Verdict::Apply
    );
    assert_eq!(
        sync.judge(&delta(102, 102, &[("98", 5.0)], &[])),
        Verdict::Apply
    );
    assert_eq!(
        sync.judge(&delta(102, 102, &[("98", 6.0)], &[])),
        Verdict::Stale,
        "a repeat is old news"
    );
    let skipped = delta(104, 105, &[("98", 9.0)], &[]);
    assert_eq!(sync.judge(&skipped), Verdict::Gap);
    assert!(sync.needs_snapshot());
    // A new snapshot starts it over.
    assert_eq!(sync.judge(&snapshot(200)), Verdict::Replace);
    assert!(sync.is_synced());
    assert!(!sync.needs_snapshot());
    assert_eq!(
        sync.judge(&delta(201, 201, &[("99", 1.0)], &[])),
        Verdict::Apply
    );
}

#[test]
fn bybit_never_asks_for_a_snapshot_and_ignores_deltas_until_the_socket_sends_one() {
    let mut sync = SnapshotAndDeltas::default();
    assert!(!sync.needs_snapshot());
    assert_eq!(
        sync.judge(&delta(5, 5, &[("99", 9.0)], &[])),
        Verdict::Stale,
        "no snapshot to apply it to"
    );
    assert!(!sync.is_synced());
    assert_eq!(sync.judge(&snapshot(10)), Verdict::Replace);
    assert!(sync.is_synced());
    assert_eq!(
        sync.judge(&delta(11, 11, &[("98", 0.0)], &[("100", 4.0)])),
        Verdict::Apply
    );
    assert_eq!(
        sync.judge(&delta(11, 11, &[("99", 9.0)], &[])),
        Verdict::Stale
    );
    // The exchange restarts its book: a snapshot with u = 1 wins over what was held.
    assert_eq!(sync.judge(&snapshot(1)), Verdict::Replace);
    assert_eq!(
        sync.judge(&delta(2, 2, &[("99", 8.0)], &[])),
        Verdict::Apply
    );
    assert!(!sync.needs_snapshot());
}

#[test]
fn a_dropped_connection_lets_the_book_go_for_both() {
    let syncs: [Box<dyn BookSync>; 2] = [
        Box::new(DiffStream::default()),
        Box::new(SnapshotAndDeltas::default()),
    ];
    for mut sync in syncs {
        sync.judge(&snapshot(10));
        assert!(sync.is_synced());
        sync.reset();
        assert!(!sync.is_synced());
        assert_eq!(
            sync.judge(&delta(11, 11, &[("99", 1.0)], &[])),
            if sync.needs_snapshot() {
                Verdict::Gap
            } else {
                Verdict::Stale
            },
            "a delta cannot be placed on the book that was let go"
        );
    }
}
