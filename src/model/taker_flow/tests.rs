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
fn a_trade_already_older_than_an_hour_is_not_added_and_clearing_forgets_all() {
    let mut flow = Flow::default();
    flow.push(trade(binance_spot(), Taker::Buy, 1.0, HOUR_MS + 1_000), NOW);
    assert_eq!(flow.snapshot(NOW), Snapshot::default());
    let before = flow.revision();
    flow.push(trade(binance_spot(), Taker::Buy, 1.0, 0), NOW);
    flow.clear();
    assert_eq!(flow.revision(), before + 2);
    assert_eq!(flow.snapshot(NOW), Snapshot::default());
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
