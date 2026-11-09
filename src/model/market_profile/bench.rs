//! Timings of the engines on realistic loads, for a person to run in release:
//! `cargo test --release --lib market_profile::bench -- --ignored --nocapture`.
//!
//! They assert nothing about speed (a machine decides that); they print what was measured, and a loose
//! ceiling catches an accidental quadratic.

use super::flow::{CandleBook, FlowKline};
use super::footprint::{self, SetInput as FootprintInput};
use super::svp::{self, SetInput as SvpInput};
use super::tpo::{self, SetInput as TpoInput};
use super::trades::{Trade, TradeBook};
use crate::model::Kline;
use crate::model::chart_type::{FootprintSettings, RowMode, RowsLayout, SvpSettings, TpoSettings};
use std::time::Instant;

const MIN: i64 = 60_000;
const HOUR: i64 = 60 * MIN;
const DAY: i64 = 24 * HOUR;

/// A deterministic price path: a slow wave with some noise, around 60 000.
fn price(i: i64) -> f64 {
    let t = i as f64;
    60_000.0 + 1_500.0 * (t / 900.0).sin() + 120.0 * (t * 0.37).sin() + 40.0 * (t * 1.9).cos()
}

fn candles(count: i64, size: i64, taker: bool) -> Vec<FlowKline> {
    (0..count)
        .map(|i| {
            let open = price(i * size / MIN);
            let close = price((i + 1) * size / MIN);
            let wiggle = 15.0 + 8.0 * ((i % 7) as f64);
            FlowKline {
                kline: Kline {
                    open_time: i * size,
                    close_time: (i + 1) * size - 1,
                    open,
                    high: open.max(close) + wiggle,
                    low: open.min(close) - wiggle,
                    close,
                    volume: 5.0 + (i % 11) as f64,
                },
                taker_buy: taker.then_some(2.0 + (i % 5) as f64),
            }
        })
        .collect()
}

fn bars(count: i64, size: i64) -> Vec<Kline> {
    candles(count, size, false)
        .into_iter()
        .map(|c| c.kline)
        .collect()
}

fn timed<T>(label: &str, run: impl FnOnce() -> T) -> (T, f64) {
    let start = Instant::now();
    let out = run();
    let ms = start.elapsed().as_secs_f64() * 1_000.0;
    println!("{label}: {ms:.1} ms");
    (out, ms)
}

#[test]
#[ignore = "timing run"]
fn bench_footprint_500_hourly_bars_from_one_minute_candles_and_300k_trades() {
    let chart = bars(500, HOUR);
    let mut book = CandleBook::new(MIN, 100_000);
    book.insert(candles(500 * 60, MIN, true));
    let mut trades = TradeBook::with_capacity(1_000_000);
    let trade_list: Vec<Trade> = (0..300_000u64)
        .map(|i| Trade {
            id: i + 1,
            time_ms: (500 * HOUR - 6 * HOUR) + (i as i64) * (6 * HOUR) / 300_000,
            price: price((i as i64) / 50) + ((i % 40) as f64) * 0.5,
            quantity: 0.01 + (i % 13) as f64 * 0.01,
            buyer_is_maker: i % 3 == 0,
        })
        .collect();
    let (_, insert_ms) = timed("insert 300 000 trades", || trades.insert(&trade_list));
    let shape = FootprintSettings {
        row_mode: RowMode::Auto,
        ..FootprintSettings::default()
    }
    .shape();
    let mut cache = footprint::Cache::default();
    let run = |trades: &mut TradeBook, cache: &mut footprint::Cache| {
        footprint::build_set(
            FootprintInput {
                bars: &chart,
                bar_ms: HOUR,
                candles: Some(&book),
                trades: Some(trades),
                exchange_tick: Some(0.01),
                shape,
            },
            cache,
        )
    };
    let (set, cold) = timed("footprint, 500 bars, cold", || run(&mut trades, &mut cache));
    println!(
        "  counts {:?}, grid {:?} ticks a row",
        set.counts,
        set.grid.map(|g| g.ticks_per_row())
    );
    let (_, warm) = timed("footprint, nothing changed", || {
        run(&mut trades, &mut cache)
    });
    let live = Trade {
        id: 300_001,
        time_ms: 500 * HOUR - 1_000,
        price: price(500 * 60 - 1),
        quantity: 0.5,
        buyer_is_maker: false,
    };
    trades.insert(&[live]);
    let (_, live_ms) = timed("footprint, one live trade (forming bar only)", || {
        run(&mut trades, &mut cache)
    });
    assert!(insert_ms < 2_000.0 && cold < 10_000.0, "a loose ceiling");
    assert!(warm <= cold && live_ms <= cold);
}

#[test]
#[ignore = "timing run"]
fn bench_tpo_120_days_of_five_minute_candles() {
    let days = 120;
    let chart = bars(days * 24, HOUR);
    let mut book = CandleBook::new(5 * MIN, 100_000);
    book.insert(candles(days * 288, 5 * MIN, false));
    let shape = TpoSettings::default().shape();
    let mut cache = tpo::Cache::default();
    let run = |book: &CandleBook, cache: &mut tpo::Cache| {
        tpo::build_set(
            TpoInput {
                bars: &chart,
                bar_ms: HOUR,
                candles: book,
                exchange_tick: Some(0.01),
                shape,
                edits: &[],
                until_ms: i64::MAX,
                now_ms: i64::MAX,
            },
            cache,
        )
    };
    let (set, cold) = timed("TPO, 120 daily profiles, cold", || run(&book, &mut cache));
    println!(
        "  {} profiles, {} ticks a row, widest {} letters",
        set.profiles.len(),
        set.grid.map_or(0, |g| g.ticks_per_row()),
        set.profiles.iter().map(|p| p.width()).max().unwrap_or(0)
    );
    let (_, warm) = timed("TPO, nothing changed", || run(&book, &mut cache));
    let mut live = book.clone();
    live.insert([candles(days * 288, 5 * MIN, false).pop().unwrap()]);
    let newest = FlowKline {
        kline: Kline {
            open_time: days * DAY - 5 * MIN,
            close_time: days * DAY - 1,
            open: 61_000.0,
            high: 61_300.0,
            low: 60_900.0,
            close: 61_100.0,
            volume: 9.0,
        },
        taker_buy: None,
    };
    live.insert([newest]);
    let (_, live_ms) = timed("TPO, the newest candle revised", || run(&live, &mut cache));
    assert!(cold < 15_000.0);
    assert!(warm <= cold && live_ms <= cold);
}

#[test]
#[ignore = "timing run"]
fn bench_session_volume_profile_200_sessions_with_developing_levels() {
    let days = 200;
    let chart = bars(days * 24, HOUR);
    let mut book = CandleBook::new(5 * MIN, 100_000);
    book.insert(candles(days * 288, 5 * MIN, true));
    for (label, developing) in [
        ("without developing levels", false),
        ("with developing levels", true),
    ] {
        let settings = SvpSettings {
            developing_poc: developing,
            developing_value_area: developing,
            layout: RowsLayout::NumberOfRows,
            ..SvpSettings::default()
        };
        let shape = settings.shape();
        let mut cache = svp::Cache::default();
        let run = |book: &CandleBook, cache: &mut svp::Cache| {
            svp::build_set(
                SvpInput {
                    bars: &chart,
                    bar_ms: HOUR,
                    candles: book,
                    exchange_tick: Some(0.01),
                    shape,
                    until_ms: i64::MAX,
                    now_ms: i64::MAX,
                },
                cache,
            )
        };
        let (set, cold) = timed(
            &format!("session profiles, 200 sessions, {label}, cold"),
            || run(&book, &mut cache),
        );
        println!("  {} profiles", set.profiles.len());
        let (_, warm) = timed("session profiles, nothing changed", || {
            run(&book, &mut cache)
        });
        assert!(cold < 20_000.0 && warm <= cold);
    }
}
