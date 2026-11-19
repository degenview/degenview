use super::*;

const NOW: i64 = 10_000_000_000;

fn binance_spot() -> Venue {
    Venue::ALL[0]
}

fn bybit_perp() -> Venue {
    Venue::ALL[3]
}

fn trade(venue: Venue, taker: Taker, notional: f64, age_ms: i64) -> Trade {
    Trade {
        venue,
        notional,
        taker,
        time_ms: NOW - age_ms,
    }
}

fn flow(trades: &[Trade]) -> Flow {
    let mut flow = Flow::default();
    let mut trades = trades.to_vec();
    trades.sort_by_key(|trade| trade.time_ms);
    for trade in trades {
        flow.push(trade, NOW);
    }
    flow
}

#[test]
fn net_is_what_takers_bought_minus_what_they_sold() {
    let volumes = Volumes {
        buy: 700.0,
        sell: 200.0,
    };
    assert_eq!(volumes.net(), 500.0);
    assert_eq!(volumes.total(), 900.0);
}

#[test]
fn every_venue_has_a_label() {
    let labels: Vec<String> = Venue::ALL.iter().map(|venue| venue.label()).collect();
    assert_eq!(
        labels,
        ["Binance spot", "Binance perp", "Bybit spot", "Bybit perp"]
    );
}

#[test]
fn each_venue_adds_up_its_own_spans() {
    let flow = flow(&[
        trade(binance_spot(), Taker::Buy, 1_000.0, 10_000),
        trade(binance_spot(), Taker::Sell, 400.0, 120_000),
        trade(binance_spot(), Taker::Buy, 50.0, 1_800_000),
        trade(bybit_perp(), Taker::Sell, 900.0, 5_000),
    ]);
    let snapshot = flow.snapshot(NOW);
    let (_, spot) = snapshot.venues[0];
    assert_eq!(
        spot.minute,
        Volumes {
            buy: 1_000.0,
            sell: 0.0
        }
    );
    assert_eq!(
        spot.five_minutes,
        Volumes {
            buy: 1_000.0,
            sell: 400.0
        }
    );
    assert_eq!(
        spot.hour,
        Volumes {
            buy: 1_050.0,
            sell: 400.0
        }
    );
    let (_, perp) = snapshot.venues[3];
    assert_eq!(
        perp.hour,
        Volumes {
            buy: 0.0,
            sell: 900.0
        }
    );
    assert_eq!(
        snapshot.all.hour,
        Volumes {
            buy: 1_050.0,
            sell: 1_300.0
        }
    );
    assert_eq!(snapshot.all.minute.net(), 100.0);
    assert!(snapshot.has_trades);
}

#[test]
fn the_half_hour_is_a_bar_of_net_flow_a_minute_with_the_newest_last() {
    let flow = flow(&[
        trade(binance_spot(), Taker::Buy, 300.0, 10_000),
        trade(bybit_perp(), Taker::Sell, 100.0, 20_000),
        trade(binance_spot(), Taker::Sell, 700.0, 90_000),
        trade(binance_spot(), Taker::Buy, 5.0, 29 * 60_000 + 30_000),
        trade(binance_spot(), Taker::Buy, 9.0, 31 * 60_000),
    ]);
    let minutes = flow.snapshot(NOW).minutes;
    assert_eq!(
        minutes[MINUTES - 1],
        Volumes {
            buy: 300.0,
            sell: 100.0
        }
    );
    assert_eq!(
        minutes[MINUTES - 2],
        Volumes {
            buy: 0.0,
            sell: 700.0
        }
    );
    assert_eq!(minutes[0].buy, 5.0, "the oldest minute still shown");
    let shown: f64 = minutes.iter().map(|minute| minute.total()).sum();
    assert_eq!(
        shown,
        300.0 + 100.0 + 700.0 + 5.0,
        "a trade past the half hour is only in the hour"
    );
}

#[test]
fn spans_roll_off_with_time_and_old_seconds_are_dropped_on_the_next_trade() {
    let mut flow = flow(&[trade(binance_spot(), Taker::Buy, 100.0, 30_000)]);
    assert_eq!(flow.snapshot(NOW).all.minute.buy, 100.0);
    assert_eq!(flow.snapshot(NOW + 40_000).all.minute.buy, 0.0);
    assert!(!flow.snapshot(NOW + HOUR_MS).has_trades);
    flow.push(trade(binance_spot(), Taker::Sell, 1.0, 0), NOW + HOUR_MS);
    assert_eq!(flow.venues[0].len(), 1);
}

#[test]
fn a_trade_out_of_order_lands_in_its_own_second() {
    let mut flow = Flow::default();
    flow.push(trade(binance_spot(), Taker::Buy, 10.0, 1_000), NOW);
    flow.push(trade(binance_spot(), Taker::Sell, 20.0, 5_000), NOW);
    flow.push(trade(binance_spot(), Taker::Buy, 30.0, 5_100), NOW);
    let seconds: Vec<i64> = flow.venues[0].iter().map(|bucket| bucket.second).collect();
    assert!(seconds.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(
        flow.snapshot(NOW).all.minute,
        Volumes {
            buy: 40.0,
            sell: 20.0
        }
    );
}

#[test]
fn memory_stays_one_total_per_second_however_many_trades() {
    let mut flow = Flow::default();
    for index in 0..5_000 {
        flow.push(
            trade(binance_spot(), Taker::Buy, 1.0, 1_000 + index % 900),
            NOW,
        );
    }
    assert!(flow.venues[0].len() <= 2);
    assert_eq!(flow.snapshot(NOW).all.hour.buy, 5_000.0);
}

#[test]
fn a_trade_already_older_than_an_hour_is_not_added() {
    let mut flow = Flow::default();
    flow.push(trade(binance_spot(), Taker::Buy, 1.0, HOUR_MS + 1_000), NOW);
    assert_eq!(flow.snapshot(NOW), Snapshot::default());
    let before = flow.revision();
    flow.push(trade(binance_spot(), Taker::Buy, 1.0, 0), NOW);
    assert_eq!(flow.revision(), before + 1);
    assert!(flow.snapshot(NOW).has_trades);
}

#[test]
fn a_crypto_markets_asset_is_its_base_and_nothing_else_has_one() {
    use crate::model::DataSource;
    assert_eq!(
        asset_of(DataSource::Binance, "BTCUSDT").as_deref(),
        Some("BTC")
    );
    assert_eq!(
        asset_of(DataSource::Coinbase, "eth-usd").as_deref(),
        Some("ETH")
    );
    assert_eq!(
        asset_of(DataSource::Kraken, "XBT/USD").as_deref(),
        Some("BTC")
    );
    assert_eq!(
        asset_of(DataSource::Kraken, "XDG/USD").as_deref(),
        Some("DOGE")
    );
    assert_eq!(
        asset_of(DataSource::Okx, "SOL-USDT").as_deref(),
        Some("SOL")
    );
    assert_eq!(asset_of(DataSource::Alpaca, "AAPL"), None);
    assert_eq!(asset_of(DataSource::Polymarket, "will-it-rain"), None);
    assert_eq!(
        asset_of(DataSource::Binance, "BTC USDT; DROP"),
        Some("BTC".to_owned())
    );
    assert_eq!(asset_of(DataSource::Binance, ""), None);
}

#[test]
fn a_one_letter_asset_has_flow_however_its_exchange_spells_the_pair() {
    use crate::model::DataSource;
    for (source, symbol) in [
        (DataSource::Binance, "SUSDT"),
        (DataSource::Bybit, "SUSDT"),
        (DataSource::Bybit, "SUSDC"),
        (DataSource::Mexc, "SUSDT"),
        (DataSource::Gate, "S_USDT"),
        (DataSource::Okx, "S-USDT"),
        (DataSource::Coinbase, "S-USD"),
    ] {
        assert_eq!(
            asset_of(source, symbol).as_deref(),
            Some("S"),
            "{source:?} {symbol}"
        );
    }
    // Two letters and more are unchanged, and so is a wrapped asset that is no pair.
    assert_eq!(
        asset_of(DataSource::Binance, "ENSUSDT").as_deref(),
        Some("ENS")
    );
    assert_eq!(
        asset_of(DataSource::Binance, "WBTC").as_deref(),
        Some("WBTC")
    );
}

fn linked(venues: &[Venue], at: i64) -> Flow {
    let mut flow = Flow::default();
    for venue in venues {
        flow.link(*venue, true, at);
    }
    flow
}

#[test]
fn a_span_is_whole_only_for_the_time_the_sockets_were_connected() {
    let mut flow = linked(&Venue::ALL, NOW - 30_000);
    flow.push(trade(binance_spot(), Taker::Buy, 100.0, 1_000), NOW);
    let snapshot = flow.snapshot(NOW);
    // Thirty seconds were watched: the minute, the five minutes and the hour are all partial.
    assert_eq!(snapshot.all.observed.minute, 30_000);
    assert!(!snapshot.all.observed.minute_full());
    assert!(!snapshot.all.observed.five_minutes_full());
    assert!(!snapshot.all.observed.hour_full());
    // A minute later the last minute is whole, the longer spans still are not.
    let snapshot = flow.snapshot(NOW + 60_000);
    assert!(snapshot.all.observed.minute_full());
    assert!(!snapshot.all.observed.five_minutes_full());
}

#[test]
fn a_quiet_market_that_was_watched_is_told_from_one_that_was_not() {
    let watched = linked(&Venue::ALL, NOW - HOUR_MS).snapshot(NOW);
    assert!(!watched.has_trades);
    assert!(watched.all.observed.hour_full());
    assert!(
        watched
            .minutes_observed
            .iter()
            .all(|ms| full(*ms, VENUES * MINUTE_MS))
    );
    // Nothing was ever connected: not one span is whole, not one bar was observed.
    let unwatched = Flow::default().snapshot(NOW);
    assert!(!unwatched.all.observed.minute_full());
    assert_eq!(unwatched.minutes_observed, [0; MINUTES]);
}

#[test]
fn an_outage_leaves_a_hole_in_the_minutes_it_covered() {
    let mut flow = linked(&Venue::ALL, NOW - 10 * 60_000);
    // Every socket drops for the minute that ended 5 minutes ago, and comes back.
    for venue in Venue::ALL {
        flow.link(venue, false, NOW - 6 * 60_000);
        flow.link(venue, true, NOW - 5 * 60_000);
    }
    let snapshot = flow.snapshot(NOW);
    let slot = |ago: usize| snapshot.minutes_observed[MINUTES - 1 - ago];
    assert_eq!(slot(0), VENUES * MINUTE_MS);
    assert_eq!(slot(4), VENUES * MINUTE_MS);
    assert_eq!(slot(5), 0, "the minute with the outage was not observed");
    assert_eq!(slot(6), VENUES * MINUTE_MS);
    // Four minutes plus the hour's worth around the hole: the five minutes ends the outage.
    assert!(snapshot.all.observed.five_minutes_full());
    assert_eq!(snapshot.all.observed.hour, 9 * 60_000);
}

#[test]
fn a_minute_one_venue_missed_is_partial_and_not_empty() {
    let mut flow = linked(&Venue::ALL[..3], NOW - HOUR_MS);
    flow.link(Venue::ALL[3], true, NOW - 30_000);
    let snapshot = flow.snapshot(NOW);
    let last = snapshot.minutes_observed[MINUTES - 1];
    assert_eq!(last, 3 * MINUTE_MS + 30_000);
    assert!(last > 0 && !full(last, VENUES * MINUTE_MS));
    let before = snapshot.minutes_observed[MINUTES - 2];
    assert_eq!(before, 3 * MINUTE_MS, "the fourth had not connected yet");
}

#[test]
fn one_venue_that_is_down_makes_the_totals_partial_and_the_others_whole() {
    let mut flow = linked(&Venue::ALL, NOW - HOUR_MS);
    flow.link(bybit_perp(), false, NOW - 20_000);
    let snapshot = flow.snapshot(NOW);
    assert!(snapshot.venues[0].1.observed.minute_full());
    assert!(!snapshot.venues[3].1.observed.minute_full());
    assert_eq!(snapshot.venues[3].1.observed.minute, 40_000);
    assert!(
        !snapshot.all.observed.minute_full(),
        "the least observed rules"
    );
}

#[test]
fn a_reconnect_without_a_drop_in_between_closes_the_old_link() {
    let mut flow = Flow::default();
    flow.link(binance_spot(), true, NOW - 50_000);
    flow.link(binance_spot(), true, NOW - 20_000);
    let observed = flow.snapshot(NOW).venues[0].1.observed;
    assert_eq!(observed.minute, 50_000, "never counted twice");
}
