use super::*;
use crate::model::depth::{Delta, Levels, Link, Price, Snapshot, View};
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
            sends_snapshots: true,
            tick: Ok(price("0.01")),
        }
    }
}

impl DepthVenue for Scripted {
    fn session(&self, _symbol: &str) -> Session {
        Session {
            url: String::new(),
            host: String::new(),
            port: 0,
            subscribe: Vec::new(),
            max_data_silence: Duration::from_secs(1),
            keepalive: Keepalive::Frame,
        }
    }

    fn tick_size(&self, _symbol: &str) -> Result<Price, DepthError> {
        self.tick.clone()
    }

    fn snapshot(&self, _symbol: &str) -> Option<Result<Snapshot, DepthError>> {
        if !self.sends_snapshots {
            return None;
        }
        let tried = self.snapshots.fetch_add(1, Ordering::AcqRel);
        Some(if tried == 0 {
            Err(DepthError::Parse("not yet"))
        } else {
            Ok(Snapshot {
                id: 100,
                levels: levels("99", "101"),
            })
        })
    }

    /// `"101 105"`: a diff over update ids 101 to 105. Anything else is not news.
    fn parse(&self, text: &str) -> Vec<Event> {
        let ids: Vec<u64> = text.split(' ').filter_map(|id| id.parse().ok()).collect();
        let [first, last] = ids[..] else {
            return Vec::new();
        };
        vec![Event::Book(BookEvent::Delta(Delta {
            first,
            last,
            levels: levels("99", "101"),
        }))]
    }
}

fn record() -> Mutex<DepthRecord> {
    let mut record = DepthRecord::new(Exchange::Binance);
    record.set_tick(price("0.01"));
    record.link(true);
    Mutex::new(record)
}

fn live(record: &Mutex<DepthRecord>) -> bool {
    lock(record).ladder(View::default(), now_ms()).link == Link::Live
}

#[test]
fn a_snapshot_is_fetched_until_it_comes_and_starts_the_book() {
    let (venue, record, running) = (Scripted::new(), record(), AtomicBool::new(false));
    start_book(&running, &venue, "BTCUSDT", &record);
    assert_eq!(
        venue.snapshots.load(Ordering::Acquire),
        2,
        "the first try failed"
    );
    assert!(live(&record));
}

#[test]
fn a_stream_that_skips_ahead_is_started_again_from_a_new_snapshot() {
    let (venue, record, running) = (Scripted::new(), record(), AtomicBool::new(false));
    start_book(&running, &venue, "BTCUSDT", &record);
    assert!(take(&running, &venue, "BTCUSDT", &record, "100 103"));
    assert!(live(&record));
    let fetched = venue.snapshots.load(Ordering::Acquire);
    // 105 does not follow 103: the book is let go and a snapshot restores it.
    assert!(take(&running, &venue, "BTCUSDT", &record, "105 106"));
    assert_eq!(venue.snapshots.load(Ordering::Acquire), fetched + 1);
    assert!(live(&record));
}

#[test]
fn a_message_that_is_not_news_changes_nothing() {
    let (venue, record, running) = (Scripted::new(), record(), AtomicBool::new(false));
    start_book(&running, &venue, "BTCUSDT", &record);
    let revision = lock(&record).revision();
    assert!(!take(&running, &venue, "BTCUSDT", &record, "pong"));
    assert!(
        !take(&running, &venue, "BTCUSDT", &record, "50 60"),
        "older than the snapshot"
    );
    assert_eq!(lock(&record).revision(), revision);
}

#[test]
fn an_exchange_that_sends_its_own_snapshots_is_never_asked_for_one() {
    let mut venue = Scripted::new();
    venue.sends_snapshots = false;
    let (record, running) = (record(), AtomicBool::new(false));
    start_book(&running, &venue, "BTCUSDT", &record);
    assert_eq!(venue.snapshots.load(Ordering::Acquire), 0);
    assert!(!live(&record), "nothing to show until the socket sends one");
}

#[test]
fn giving_up_on_a_snapshot_when_the_feed_is_shut_down() {
    let (venue, record, stopped) = (Scripted::new(), record(), AtomicBool::new(true));
    start_book(&stopped, &venue, "BTCUSDT", &record);
    assert_eq!(venue.snapshots.load(Ordering::Acquire), 0);
}

#[test]
fn a_tick_size_is_learned_and_an_unlisted_market_is_marked_so() {
    let record = Mutex::new(DepthRecord::new(Exchange::Binance));
    let venue = Scripted::new();
    assert_eq!(learn_tick(&venue, "BTCUSDT", &record), Some(price("0.01")));

    let mut unlisted = Scripted::new();
    unlisted.tick = Err(DepthError::UnknownSymbol("NOPEUSDT".to_owned()));
    assert_eq!(learn_tick(&unlisted, "NOPEUSDT", &record), None);
    assert_eq!(
        lock(&record).ladder(View::default(), now_ms()).link,
        Link::Unlisted
    );

    let mut offline = Scripted::new();
    offline.tick = Err(DepthError::Parse("tick size"));
    let record = Mutex::new(DepthRecord::new(Exchange::Binance));
    assert_eq!(learn_tick(&offline, "BTCUSDT", &record), None);
    assert_eq!(
        lock(&record).ladder(View::default(), now_ms()).link,
        Link::Connecting
    );
}

/// Listens to the real sockets of BTCUSDT until each venue has a book and a trade, and prints the
/// best bid and ask and a trade of each. Run with
/// `cargo test --lib service::depth::tests::live_books -- --ignored --nocapture`.
#[test]
#[ignore = "needs the network"]
fn live_books_fill() {
    for exchange in [Exchange::Binance, Exchange::Bybit] {
        let market = DepthMarket {
            venue: Venue {
                exchange,
                market: Market::Spot,
            },
            symbol: "BTCUSDT".to_owned(),
        };
        let record = Arc::new(Mutex::new(DepthRecord::new(exchange)));
        let lease = Lease::default();
        lease.renew(Duration::from_secs(120));
        let feed = DepthFeed::start(market, Arc::clone(&record), lease, Arc::new(|| {}));
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        let ladder = loop {
            std::thread::sleep(Duration::from_millis(500));
            let ladder = lock(&record).ladder(View::default(), now_ms());
            if ladder.link == Link::Live && !ladder.trades.is_empty() {
                break ladder;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "{exchange:?}: {:?}",
                ladder.link
            );
        };
        assert!(feed.is_running());
        let (bid, ask) = (ladder.best_bid.unwrap(), ladder.best_ask.unwrap());
        assert!(bid < ask, "a crossed book");
        let trade = ladder.trades[0];
        println!(
            "{}: tick {}, best bid {} ask {}, last trade {} x {} ({:?}), {} rows around {}",
            exchange.name(),
            ladder.tick.format(ladder.tick.decimals()),
            bid.format(ladder.tick.decimals()),
            ask.format(ladder.tick.decimals()),
            trade.price.format(ladder.tick.decimals()),
            trade.size,
            trade.taker,
            ladder.rows.len(),
            ladder.centre().unwrap().format(ladder.tick.decimals()),
        );
        // Let the diffs flow for a while: the book must stay uncrossed.
        for _ in 0..20 {
            std::thread::sleep(Duration::from_millis(500));
            let ladder = lock(&record).ladder(View::default(), now_ms());
            assert_eq!(ladder.link, Link::Live);
            assert!(ladder.best_bid < ladder.best_ask, "crossed after diffs");
        }
    }
}
