use super::*;
use crate::model::depth::{Delta, Levels, Price, Side, Snapshot};

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

fn binance() -> (Box<dyn BookSync>, Book) {
    (for_exchange(Exchange::Binance), Book::default())
}

#[test]
fn binance_cannot_place_a_diff_before_a_snapshot() {
    let (mut sync, mut book) = binance();
    assert!(!sync.is_synced());
    let outcome = sync.apply(&mut book, delta(5, 6, &[("99", 9.0)], &[]));
    assert_eq!(outcome, SyncOutcome::NeedsSnapshot);
    assert_eq!((book.best_bid(), book.best_ask()), (None, None));
}

#[test]
fn binance_drops_diffs_the_snapshot_already_has_and_uses_the_one_that_straddles_it() {
    let (mut sync, mut book) = binance();
    assert_eq!(sync.apply(&mut book, snapshot(100)), SyncOutcome::Applied);
    let old = delta(90, 100, &[("99", 7.0)], &[]);
    assert_eq!(sync.apply(&mut book, old), SyncOutcome::Stale);
    assert_eq!(book.size_at(Side::Bid, price("99")), 1.0);
    let straddling = delta(98, 103, &[("99", 3.0)], &[("101", 0.0)]);
    assert_eq!(sync.apply(&mut book, straddling), SyncOutcome::Applied);
    assert_eq!(book.size_at(Side::Bid, price("99")), 3.0);
    assert_eq!(book.best_ask(), Some(price("102")));
}

#[test]
fn binance_wants_the_first_diff_to_start_no_later_than_the_snapshot_ends() {
    let (mut sync, mut book) = binance();
    sync.apply(&mut book, snapshot(100));
    let late = delta(102, 105, &[("99", 3.0)], &[]);
    assert_eq!(sync.apply(&mut book, late), SyncOutcome::NeedsSnapshot);
    assert_eq!(book.best_bid(), None, "a book with a hole in it is let go");
    assert!(!sync.is_synced());
}

#[test]
fn binance_diffs_must_follow_each_other_and_a_gap_empties_the_book() {
    let (mut sync, mut book) = binance();
    sync.apply(&mut book, snapshot(100));
    assert_eq!(
        sync.apply(&mut book, delta(100, 101, &[("99", 2.0)], &[])),
        SyncOutcome::Applied
    );
    assert_eq!(
        sync.apply(&mut book, delta(102, 102, &[("98", 5.0)], &[])),
        SyncOutcome::Applied
    );
    assert_eq!(
        sync.apply(&mut book, delta(102, 102, &[("98", 6.0)], &[])),
        SyncOutcome::Stale,
        "a repeat is old news"
    );
    let skipped = delta(104, 105, &[("98", 9.0)], &[]);
    assert_eq!(sync.apply(&mut book, skipped), SyncOutcome::NeedsSnapshot);
    assert_eq!((book.best_bid(), book.best_ask()), (None, None));
    // A new snapshot starts it over.
    assert_eq!(sync.apply(&mut book, snapshot(200)), SyncOutcome::Applied);
    assert!(sync.is_synced());
    assert_eq!(book.best_bid(), Some(price("99")));
}

#[test]
fn bybit_replaces_the_book_with_each_snapshot_and_applies_deltas_to_it() {
    let (mut sync, mut book) = (for_exchange(Exchange::Bybit), Book::default());
    assert_eq!(
        sync.apply(&mut book, delta(5, 5, &[("99", 9.0)], &[])),
        SyncOutcome::Stale,
        "no snapshot to apply it to"
    );
    assert!(!sync.is_synced());
    sync.apply(&mut book, snapshot(10));
    assert!(sync.is_synced());
    assert_eq!(
        sync.apply(&mut book, delta(11, 11, &[("98", 0.0)], &[("100", 4.0)])),
        SyncOutcome::Applied
    );
    assert_eq!(book.best_ask(), Some(price("100")));
    assert_eq!(book.size_at(Side::Bid, price("98")), 0.0);
    assert_eq!(
        sync.apply(&mut book, delta(11, 11, &[("99", 9.0)], &[])),
        SyncOutcome::Stale
    );
    // The exchange restarts its book: a snapshot with u = 1 wins over what was held.
    sync.apply(&mut book, snapshot(1));
    assert_eq!(book.best_ask(), Some(price("101")));
    assert_eq!(
        sync.apply(&mut book, delta(2, 2, &[("99", 8.0)], &[])),
        SyncOutcome::Applied
    );
}

#[test]
fn a_dropped_connection_lets_the_book_go_for_both() {
    for exchange in [Exchange::Binance, Exchange::Bybit] {
        let (mut sync, mut book) = (for_exchange(exchange), Book::default());
        sync.apply(&mut book, snapshot(10));
        sync.reset(&mut book);
        assert_eq!((book.best_bid(), book.best_ask()), (None, None));
        assert!(!sync.is_synced());
    }
}
