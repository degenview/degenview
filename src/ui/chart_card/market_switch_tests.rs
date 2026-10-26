//! A chart that keeps its card while its market changes (the Script Manager's preview): what the
//! card draws for the new market after a price range was placed for the old one.

use super::canvas_tests::{Harness, SIZE, gather, series};
use super::*;
use crate::model::ChartCanvas;

const DAY: i64 = 86_400_000;
/// 2026-01-05T05:00:00Z, a Monday midnight in New York.
const START: i64 = 1_767_589_200_000;

/// Daily stock bars as Alpaca serves them: stamped at midnight New York, closing the instant
/// they open.
fn aapl(count: usize) -> Vec<Kline> {
    (0..count)
        .map(|index| {
            let open_time = START + index as i64 * DAY;
            let close = 250.0 + (index as f64 * 0.7).sin() * 8.0 + index as f64 * 0.2;
            Kline {
                open_time,
                close_time: open_time,
                open: close - 1.0,
                high: close + 2.0,
                low: close - 3.0,
                close,
                volume: 1_000_000.0 + index as f64,
            }
        })
        .collect()
}

fn harness(source: DataSource, candles: Vec<Kline>) -> Harness {
    let mut harness = Harness::new(ChartCanvas::default());
    harness.card.set_timeframe(Timeframe::Day);
    harness.source = source;
    harness.candles = candles;
    harness
}

/// The candle wicks that land inside the window.
fn wicks_in_window(harness: &mut Harness) -> usize {
    for _ in 0..6 {
        harness.frame();
    }
    let (shapes, _) = harness.frame();
    gather(&shapes)
        .segments
        .iter()
        .filter(|(points, _)| points[0].x == points[1].x && points[0].y != points[1].y)
        .filter(|(points, _)| {
            points
                .iter()
                .all(|p| (0.0..=SIZE.y).contains(&p.y) && (0.0..=SIZE.x).contains(&p.x))
        })
        .count()
}

#[test]
fn stock_bars_that_close_as_they_open_are_drawn() {
    let mut h = harness(DataSource::Alpaca, aapl(120));
    assert!(wicks_in_window(&mut h) > 100);
}

#[test]
fn a_price_range_placed_for_another_market_leaves_the_new_one_off_screen_until_reset() {
    let mut h = harness(DataSource::Binance, series(120));
    h.card.place_prices_for_test(90_000.0, 100_000.0);
    wicks_in_window(&mut h);

    h.candles = aapl(120);
    h.source = DataSource::Alpaca;
    assert!(
        wicks_in_window(&mut h) < 20,
        "the old range still holds the axis"
    );

    h.card.reset_view();
    assert!(wicks_in_window(&mut h) > 100);
}
