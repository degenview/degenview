use super::book::MAX_LEVELS;
use super::tape::{PRICES_KEPT, TAPE_ROWS};
use super::*;
use crate::model::DataSource;
use crate::model::liquidation::Exchange;
use crate::model::taker_flow::Taker;

const NOW: i64 = 1_795_000_000_000;

fn price(text: &str) -> Price {
    Price::parse(text).unwrap()
}

fn side(rows: &[(&str, f64)]) -> Vec<(Price, f64)> {
    rows.iter().map(|(p, s)| (price(p), *s)).collect()
}

fn book_of(bids: &[(&str, f64)], asks: &[(&str, f64)]) -> BookEvent {
    BookEvent::Snapshot(Snapshot {
        id: 1,
        levels: Levels {
            bids: side(bids),
            asks: side(asks),
        },
    })
}

fn change(first: u64, bids: &[(&str, f64)], asks: &[(&str, f64)]) -> Event {
    Event::Book(BookEvent::Delta(Delta {
        first,
        last: first,
        levels: Levels {
            bids: side(bids),
            asks: side(asks),
        },
    }))
}

fn trade(at: &str, size: f64, taker: Taker, time_ms: i64) -> Event {
    Event::Trade(Trade {
        time_ms,
        price: price(at),
        size,
        taker,
    })
}

/// A live record of a Bybit book with a cent tick, bids 99.98 and 99.97, asks 100.01 and 100.03.
fn live() -> DepthRecord {
    let mut record = DepthRecord::new(Exchange::Bybit);
    record.set_tick(price("0.01"));
    record.link(true);
    record.apply(
        Event::Book(book_of(
            &[("99.98", 4.0), ("99.97", 6.0)],
            &[("100.01", 3.0), ("100.03", 5.0)],
        )),
        NOW,
    );
    record
}

fn view(group: &str) -> View {
    View {
        group: Some(price(group)),
        centre: None,
    }
}

fn row_at(ladder: &Ladder, text: &str) -> Row {
    *ladder
        .rows
        .iter()
        .find(|row| row.price == price(text))
        .unwrap_or_else(|| panic!("no row at {text}"))
}

#[test]
fn prices_parse_exactly_and_reject_what_is_not_a_decimal() {
    assert_eq!(
        Price::parse("58884.13000000").unwrap().units(),
        5_888_413_000_000
    );
    assert_eq!(Price::parse("0.00001234").unwrap().units(), 1_234);
    assert_eq!(Price::parse("100").unwrap().units(), 10_000_000_000);
    assert_eq!(Price::parse(".5").unwrap().units(), 50_000_000);
    assert_eq!(Price::parse("1.500000000").unwrap().units(), 150_000_000);
    for bad in [
        "",
        ".",
        "-1",
        "+1",
        "1e5",
        "1,5",
        "abc",
        "1.2.3",
        "1.000000001",
        " 1",
        "99999999999999999999",
    ] {
        assert_eq!(Price::parse(bad), None, "{bad:?}");
    }
}

#[test]
fn a_price_knows_its_places_and_how_to_write_them() {
    assert_eq!(price("0.01").decimals(), 2);
    assert_eq!(price("0.00001").decimals(), 5);
    assert_eq!(price("10").decimals(), 0);
    assert_eq!(price("0.5").decimals(), 1);
    assert_eq!(price("58884.1").format(2), "58,884.10");
    assert_eq!(price("0.00001234").format(8), "0.00001234");
}

#[test]
fn rounding_goes_to_the_multiples_of_a_step() {
    let step = price("10");
    assert_eq!(price("101").floor_to(step), price("100"));
    assert_eq!(price("101").ceil_to(step), price("110"));
    assert_eq!(price("100").floor_to(step), price("100"));
    assert_eq!(price("100").ceil_to(step), price("100"));
    assert_eq!(price("0.01").floor_to(price("0.05")), Price::ZERO);
}

#[test]
fn the_row_sizes_start_at_the_tick_and_the_automatic_one_fits_the_market() {
    let choices = group::choices(price("0.01"));
    assert_eq!(choices[0], price("0.01"));
    assert_eq!(choices[1], price("0.05"));
    assert_eq!(choices.last(), Some(&price("10")));
    assert!(choices.windows(2).all(|pair| pair[0] < pair[1]));
    // Half a percent of 80,000 over 40 rows is 10.
    assert_eq!(group::automatic(price("0.01"), price("80000")), price("10"));
    // A cheap coin gets fine rows, a huge price the largest on offer.
    assert_eq!(
        group::automatic(price("0.0001"), price("1")),
        price("0.0005")
    );
    assert_eq!(
        group::automatic(price("0.0001"), price("1000000000")),
        price("0.1")
    );
}

#[test]
fn bids_fold_down_and_asks_fold_up_so_a_row_is_never_both() {
    let record = live();
    let ladder = record.ladder(view("0.05"), NOW);
    assert_eq!(ladder.link, Link::Live);
    // 99.98 and 99.97 round down to 99.95; 100.01 and 100.03 round up to 100.05.
    let bids = row_at(&ladder, "99.95");
    assert_eq!((bids.bid, bids.ask), (10.0, 0.0));
    let asks = row_at(&ladder, "100.05");
    assert_eq!((asks.bid, asks.ask), (0.0, 8.0));
    assert_eq!(ladder.largest_size, 10.0);
    assert!(
        ladder
            .rows
            .windows(2)
            .all(|pair| pair[0].price > pair[1].price)
    );
    assert_eq!(ladder.rows.len(), WINDOW_ROWS);
    // The middle of 99.98 and 100.01 is 99.995, in the row of 99.95.
    assert_eq!(ladder.centre(), Some(price("99.95")));
    assert_eq!(ladder.best_bid, Some(price("99.98")));
    assert_eq!(ladder.spread(), Some(price("0.03")));
}

#[test]
fn a_viewer_can_centre_the_window_elsewhere() {
    let record = live();
    let ladder = record.ladder(
        View {
            group: Some(price("0.01")),
            centre: Some(price("100.50")),
        },
        NOW,
    );
    assert_eq!(ladder.centre(), Some(price("100.50")));
    assert_eq!(ladder.rows.len(), WINDOW_ROWS);
    assert_eq!(row_at(&ladder, "100.03").ask, 5.0);
}

#[test]
fn a_group_the_market_does_not_offer_is_replaced_by_the_automatic_one() {
    let record = live();
    let ladder = record.ladder(view("3"), NOW);
    assert_eq!(ladder.group, group::automatic(price("0.01"), price("100")));
}

#[test]
fn trades_fold_to_where_the_taker_hit_and_show_since_the_first() {
    let mut record = live();
    record.apply(trade("100.01", 2.0, Taker::Buy, NOW + 1_000), NOW + 1_000);
    record.apply(trade("99.98", 1.5, Taker::Sell, NOW + 2_000), NOW + 2_000);
    record.apply(trade("100.02", 0.5, Taker::Buy, NOW + 3_000), NOW + 3_000);
    let ladder = record.ladder(view("0.05"), NOW + 3_000);
    let up = row_at(&ladder, "100.05");
    assert_eq!((up.bought, up.sold), (2.5, 0.0));
    let down = row_at(&ladder, "99.95");
    assert_eq!((down.bought, down.sold), (0.0, 1.5));
    assert_eq!(ladder.largest_traded, 2.5);
    assert_eq!(ladder.since_ms, Some(NOW + 1_000));
    assert_eq!(ladder.trades.len(), 3);
    assert_eq!(ladder.trades[0].price, price("100.02"), "newest first");
    assert_eq!(ladder.last_trade.map(|t| t.taker), Some(Taker::Buy));
}

#[test]
fn the_tape_keeps_the_latest_trades_and_prunes_far_prices() {
    let mut tape = Tape::default();
    for index in 0..(TAPE_ROWS as i64 + 50) {
        tape.push(Trade {
            time_ms: index,
            price: Price::from_units(100_000_000 + index),
            size: 1.0,
            taker: Taker::Buy,
        });
    }
    assert_eq!(tape.recent().len(), TAPE_ROWS);
    assert_eq!(
        tape.recent().next().map(|t| t.time_ms),
        Some(TAPE_ROWS as i64 + 49)
    );
    assert_eq!(tape.since_ms(), Some(0));
    assert_eq!(
        tape.traded().count(),
        TAPE_ROWS + 50,
        "below the limit, nothing goes"
    );

    for index in 0..(3 * PRICES_KEPT as i64) {
        tape.push(Trade {
            time_ms: index,
            price: Price::from_units(200_000_000 + index * 1_000),
            size: 1.0,
            taker: Taker::Sell,
        });
    }
    let mid = Price::from_units(200_000_000);
    tape.prune(mid);
    assert_eq!(tape.traded().count(), PRICES_KEPT);
    assert!(
        tape.traded().any(|(price, _)| price == mid),
        "the nearest stay"
    );
    let farthest = tape.traded().map(|(price, _)| price).max().unwrap();
    assert!(farthest < Price::from_units(200_000_000 + 3_000 * PRICES_KEPT as i64));
}

#[test]
fn stacked_and_pulled_size_shows_but_size_that_traded_does_not() {
    let mut record = live();
    // 3 more bid at 99.98 (stacked); an offer of 5 at 100.03 withdrawn (pulled).
    record.apply(
        change(2, &[("99.98", 7.0)], &[("100.03", 0.0)]),
        NOW + 1_000,
    );
    let ladder = record.ladder(view("0.01"), NOW + 1_500);
    assert_eq!(row_at(&ladder, "99.98").pull_stack, 3.0);
    assert_eq!(row_at(&ladder, "100.03").pull_stack, -5.0);

    // A seller hits 2 of the bid, and the book shows 2 less: not a pull.
    record.apply(trade("99.98", 2.0, Taker::Sell, NOW + 2_000), NOW + 2_000);
    record.apply(change(3, &[("99.98", 5.0)], &[]), NOW + 2_000);
    let ladder = record.ladder(view("0.01"), NOW + 2_500);
    assert_eq!(row_at(&ladder, "99.98").pull_stack, 3.0);
}

#[test]
fn what_was_stacked_more_than_ten_seconds_ago_is_forgotten() {
    let mut record = live();
    record.apply(change(2, &[("99.98", 7.0)], &[]), NOW + 1_000);
    let soon = record.ladder(view("0.01"), NOW + 9_000);
    assert_eq!(row_at(&soon, "99.98").pull_stack, 3.0);
    let later = record.ladder(view("0.01"), NOW + 12_000);
    assert_eq!(row_at(&later, "99.98").pull_stack, 0.0);
}

#[test]
fn a_ladder_says_why_it_is_empty() {
    let mut record = DepthRecord::new(Exchange::Binance);
    assert_eq!(record.ladder(View::default(), NOW).link, Link::Connecting);
    record.set_tick(price("0.01"));
    record.link(true);
    // Up, but no snapshot yet.
    let waiting = record.ladder(View::default(), NOW);
    assert_eq!(waiting.link, Link::Connecting);
    assert!(waiting.rows.is_empty());
    assert_eq!(waiting.tick, price("0.01"));

    let mut record = live();
    record.link(false);
    let dropped = record.ladder(View::default(), NOW);
    assert_eq!(dropped.link, Link::Reconnecting);
    assert!(dropped.rows.is_empty(), "the old book is not shown");
    // Back up: the book returns with the next snapshot, not before.
    record.link(true);
    assert_eq!(record.ladder(View::default(), NOW).link, Link::Connecting);
    record.apply(Event::Book(book_of(&[("99", 1.0)], &[("101", 1.0)])), NOW);
    assert_eq!(record.ladder(View::default(), NOW).link, Link::Live);

    record.mark_unlisted();
    assert_eq!(record.ladder(View::default(), NOW).link, Link::Unlisted);
}

#[test]
fn a_gap_in_a_binance_stream_empties_the_ladder_until_a_snapshot() {
    let mut record = DepthRecord::new(Exchange::Binance);
    record.set_tick(price("0.01"));
    record.link(true);
    record.apply(Event::Book(book_of(&[("99", 1.0)], &[("101", 1.0)])), NOW);
    assert_eq!(record.ladder(View::default(), NOW).link, Link::Live);
    let skipped = Event::Book(BookEvent::Delta(Delta {
        first: 50,
        last: 51,
        levels: Levels::default(),
    }));
    assert_eq!(record.apply(skipped, NOW), SyncOutcome::NeedsSnapshot);
    assert_eq!(record.ladder(View::default(), NOW).link, Link::Connecting);
}

#[test]
fn only_what_changes_a_ladder_moves_the_revision() {
    let mut record = live();
    let before = record.revision();
    let stale = change(1, &[("99.98", 1.0)], &[]);
    assert_eq!(record.apply(stale, NOW), SyncOutcome::Stale);
    assert_eq!(record.revision(), before);
    record.apply(change(2, &[("99.98", 1.0)], &[]), NOW);
    assert!(record.revision() > before);
}

#[test]
fn the_book_keeps_the_nearest_levels_and_drops_zero_sizes() {
    let mut book = Book::default();
    let levels = Levels {
        bids: (1..=(MAX_LEVELS as i64 + 10))
            .map(|n| (Price::from_units(n * 1_000_000), 1.0))
            .collect(),
        asks: (1..=(MAX_LEVELS as i64 + 10))
            .map(|n| (Price::from_units((1_000_000 + n) * 1_000_000), 1.0))
            .collect(),
    };
    book.apply(&levels);
    assert_eq!(
        book.best_bid(),
        Some(Price::from_units((MAX_LEVELS as i64 + 10) * 1_000_000))
    );
    assert_eq!(
        book.best_ask(),
        Some(Price::from_units(1_000_001 * 1_000_000))
    );
    let furthest_bid = book
        .bids_between(Price::ZERO, Price::from_units(i64::MAX))
        .next();
    assert_eq!(
        furthest_bid.map(|(p, _)| p),
        Some(Price::from_units(11 * 1_000_000))
    );
    book.apply(&Levels {
        bids: vec![(Price::from_units((MAX_LEVELS as i64 + 10) * 1_000_000), 0.0)],
        asks: Vec::new(),
    });
    assert_eq!(
        book.best_bid(),
        Some(Price::from_units((MAX_LEVELS as i64 + 9) * 1_000_000))
    );
}

#[test]
fn only_spot_markets_of_binance_and_bybit_have_a_book() {
    let btc = market_of(DataSource::Binance, "BTCUSDT").unwrap();
    assert_eq!(btc.exchange, Exchange::Binance);
    assert_eq!(btc.symbol, "BTCUSDT");
    let eth = market_of(DataSource::Bybit, "eth").unwrap();
    assert_eq!(
        (eth.exchange, eth.symbol.as_str()),
        (Exchange::Bybit, "ETHUSDT")
    );
    for source in [
        DataSource::Coinbase,
        DataSource::Kraken,
        DataSource::Alpaca,
        DataSource::CoinGecko,
    ] {
        assert_eq!(market_of(source, "BTCUSDT"), None, "{source:?}");
    }
}
