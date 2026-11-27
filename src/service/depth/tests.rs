use super::venue::Sessions;
use super::*;
use crate::model::depth::{
    BookSync, Delta, DiffStream, Levels, Link, Price, Snapshot, SnapshotAndDeltas, View,
};
use crate::model::exchange::{Exchange, Market, Venue};
use crate::service::net::ws::{Keepalive, Session};
use std::sync::atomic::{AtomicUsize, Ordering};

fn price(text: &str) -> Price {
    Price::parse(text).unwrap()
}

fn levels(bid: &str, ask: &str) -> Levels {
    Levels {
        bids: vec![(price(bid), 1.0)],
        asks: vec![(price(ask), 1.0)],
    }
}

/// A venue whose messages are the update ids of a diff stream, and whose snapshot is at id 100.
struct Scripted {
    snapshots: AtomicUsize,
    sends_snapshots: bool,
    tick: Result<Price, DepthError>,
}

impl Scripted {
    fn new() -> Self {
        Self {
            snapshots: AtomicUsize::new(0),
            sends_snapshots: false,
            tick: Ok(price("0.01")),
        }
    }

    /// A venue whose socket sends its own snapshots, like Bybit.
    fn sending_snapshots() -> Self {
        Self {
            sends_snapshots: true,
            ..Self::new()
        }
    }
}

impl DepthVenue for Scripted {
    fn sessions(&self, _symbol: &str) -> Sessions {
        Sessions {
            book: Session {
                url: String::new(),
                host: String::new(),
                port: 0,
                subscribe: Vec::new(),
                max_data_silence: Duration::from_secs(1),
                keepalive: Keepalive::Frame,
            },
            trades: None,
        }
    }

    fn tick_size(&self, _symbol: &str) -> Result<Price, DepthError> {
        self.tick.clone()
    }

    fn book_sync(&self) -> Box<dyn BookSync> {
        if self.sends_snapshots {
            Box::new(SnapshotAndDeltas::default())
        } else {
            Box::new(DiffStream::default())
        }
    }

    fn snapshot(&self, _symbol: &str) -> Result<Snapshot, DepthError> {
        if self.sends_snapshots {
            return Err(DepthError::Unsupported);
        }
        let tried = self.snapshots.fetch_add(1, Ordering::AcqRel);
        if tried == 0 {
            Err(DepthError::Parse("not yet"))
        } else {
            Ok(Snapshot {
                id: 100,
                levels: levels("99", "101"),
            })
        }
    }

    /// `"101 105"`: a diff over update ids 101 to 105. Anything else is not news.
    fn parse(&self, text: &str) -> Vec<Event> {
        let ids: Vec<u64> = text.split(' ').filter_map(|id| id.parse().ok()).collect();
        let [first, last] = ids[..] else {
            return Vec::new();
        };
        vec![Event::Book(BookEvent::Delta(Delta {
            previous: None,
            first,
            last,
            levels: levels("99", "101"),
        }))]
    }
}

/// A record of `venue`'s market with the socket up.
fn record(venue: &Scripted) -> Mutex<DepthRecord> {
    let mut record = DepthRecord::new(venue.book_sync());
    record.set_tick(price("0.01"));
    record.link(true);
    Mutex::new(record)
}

/// A view of the market with room for 40 rows.
fn view() -> View {
    View {
        group: None,
        centre: None,
        rows: 40,
    }
}

fn live(record: &Mutex<DepthRecord>) -> bool {
    lock(record).snapshot(view(), now_ms()).link == Link::Live
}

/// Follows `venue` on a worker that is running, or `stopped` already.
fn follow<'a>(
    worker: &'a Worker,
    venue: &'a Scripted,
    record: &'a Mutex<DepthRecord>,
) -> Follower<'a> {
    Follower {
        worker,
        venue,
        symbol: "BTCUSDT",
        record,
    }
}

#[test]
fn a_snapshot_is_fetched_until_it_comes_and_starts_the_book() {
    let (venue, worker) = (Scripted::new(), Worker::detached(false));
    let record = record(&venue);
    follow(&worker, &venue, &record).start_book();
    assert_eq!(
        venue.snapshots.load(Ordering::Acquire),
        2,
        "the first try failed"
    );
    assert!(live(&record));
}

#[test]
fn a_stream_that_skips_ahead_is_started_again_from_a_new_snapshot() {
    let (venue, worker) = (Scripted::new(), Worker::detached(false));
    let record = record(&venue);
    let follower = follow(&worker, &venue, &record);
    follower.start_book();
    assert!(follower.take("100 103"));
    assert!(live(&record));
    let fetched = venue.snapshots.load(Ordering::Acquire);
    // 105 does not follow 103: the book is let go and a snapshot restores it.
    assert!(follower.take("105 106"));
    assert_eq!(venue.snapshots.load(Ordering::Acquire), fetched + 1);
    assert!(live(&record));
}

#[test]
fn a_message_that_is_not_news_changes_nothing() {
    let (venue, worker) = (Scripted::new(), Worker::detached(false));
    let record = record(&venue);
    let follower = follow(&worker, &venue, &record);
    follower.start_book();
    let revision = lock(&record).revision();
    assert!(!follower.take("pong"));
    assert!(!follower.take("50 60"), "older than the snapshot");
    assert_eq!(lock(&record).revision(), revision);
}

#[test]
fn an_exchange_that_sends_its_own_snapshots_is_never_asked_for_one() {
    let (venue, worker) = (Scripted::sending_snapshots(), Worker::detached(false));
    let record = record(&venue);
    let follower = follow(&worker, &venue, &record);
    follower.start_book();
    assert_eq!(venue.snapshots.load(Ordering::Acquire), 0);
    assert!(!live(&record), "nothing to show until the socket sends one");
    // A delta before the first snapshot is old news, not a reason to fetch.
    assert!(!follower.take("101 102"));
    assert_eq!(venue.snapshots.load(Ordering::Acquire), 0);
}

#[test]
fn giving_up_on_a_snapshot_when_the_feed_is_shut_down() {
    let (venue, worker) = (Scripted::new(), Worker::detached(true));
    let record = record(&venue);
    follow(&worker, &venue, &record).start_book();
    assert_eq!(venue.snapshots.load(Ordering::Acquire), 0);
}

#[test]
fn a_tick_size_is_learned_and_an_unlisted_market_is_marked_so() {
    let worker = Worker::detached(false);
    let venue = Scripted::new();
    let record = Mutex::new(DepthRecord::new(venue.book_sync()));
    assert_eq!(
        follow(&worker, &venue, &record).learn_tick(),
        Some(price("0.01"))
    );

    let mut unlisted = Scripted::new();
    unlisted.tick = Err(DepthError::UnknownSymbol("NOPEUSDT".to_owned()));
    assert_eq!(follow(&worker, &unlisted, &record).learn_tick(), None);
    assert_eq!(
        lock(&record).snapshot(view(), now_ms()).link,
        Link::Unlisted
    );

    let mut offline = Scripted::new();
    offline.tick = Err(DepthError::Parse("tick size"));
    let record = Mutex::new(DepthRecord::new(offline.book_sync()));
    assert_eq!(follow(&worker, &offline, &record).learn_tick(), None);
    assert_eq!(
        lock(&record).snapshot(view(), now_ms()).link,
        Link::Connecting
    );
}

#[test]
fn each_exchange_judges_its_book_by_its_own_rules() {
    let market = |exchange| DepthMarket {
        venue: Venue {
            exchange,
            market: Market::Spot,
        },
        symbol: "BTCUSDT".to_owned(),
    };
    assert!(new_record(&market(Exchange::Binance)).needs_snapshot());
    assert!(!new_record(&market(Exchange::Bybit)).needs_snapshot());
}

/// Listens to the real sockets of BTCUSDT until each venue has a book and a trade, and prints the
/// best bid and ask and a trade of each. Run with
/// `cargo test --lib service::depth::tests::live_books_fill -- --ignored --nocapture`.
#[test]
#[ignore = "needs the network"]
fn live_books_fill() {
    for (exchange, kind) in [
        (Exchange::Binance, Market::Spot),
        (Exchange::Binance, Market::Perpetual),
        (Exchange::Bybit, Market::Spot),
    ] {
        let market = DepthMarket {
            venue: Venue {
                exchange,
                market: kind,
            },
            symbol: "BTCUSDT".to_owned(),
        };
        let record = Arc::new(Mutex::new(new_record(&market)));
        let lease = Lease::default();
        lease.renew(Duration::from_secs(120));
        let feed = DepthFeed::start(market, Arc::clone(&record), lease, Arc::new(|| {}));
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        let shown = loop {
            std::thread::sleep(Duration::from_millis(500));
            let shown = lock(&record).snapshot(view(), now_ms());
            if shown.link == Link::Live && !shown.tape.trades.is_empty() {
                break shown;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "{exchange:?}: {:?}",
                shown.link
            );
        };
        assert!(feed.is_running());
        let (bid, ask) = (
            shown.ladder.best_bid.unwrap(),
            shown.ladder.best_ask.unwrap(),
        );
        assert!(bid < ask, "a crossed book");
        let trade = shown.tape.trades[0];
        println!(
            "{}: tick {}, best bid {} ask {}, last trade {} x {} ({:?}), {} rows around {}",
            exchange.name(),
            shown.tick.format(shown.tick.decimals()),
            bid.format(shown.tick.decimals()),
            ask.format(shown.tick.decimals()),
            trade.price.format(shown.tick.decimals()),
            trade.size,
            trade.taker,
            shown.ladder.rows.len(),
            shown.ladder.centre().unwrap().format(shown.tick.decimals()),
        );
        // Let the diffs flow for a while: the book must stay uncrossed.
        for _ in 0..20 {
            std::thread::sleep(Duration::from_millis(500));
            let shown = lock(&record).snapshot(view(), now_ms());
            assert_eq!(shown.link, Link::Live);
            assert!(
                shown.ladder.best_bid < shown.ladder.best_ask,
                "crossed after diffs"
            );
        }
    }
}
