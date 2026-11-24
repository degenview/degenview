//! A chart that keeps its card while its market changes (the Pine Studio's preview): what the
//! card draws for the new market after a price range was placed for the old one, and that what a
//! script drew for the old one is never drawn over it.

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
    harness.card.set_timeframe(Timeframe::DAY_1);
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
    // A wick is a filled rectangle about one pixel wide.
    gather(&shapes)
        .rects
        .iter()
        .map(|(rect, _)| rect)
        .filter(|rect| rect.width() <= 2.0 && rect.height() > 0.0)
        .filter(|rect| {
            (0.0..=SIZE.y).contains(&rect.min.y)
                && (0.0..=SIZE.y).contains(&rect.max.y)
                && (0.0..=SIZE.x).contains(&rect.min.x)
                && (0.0..=SIZE.x).contains(&rect.max.x)
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

/// A script that repaints every candle in fuchsia (`#E040FB`), a colour nothing else on the chart
/// uses.
const REPAINT: &str = "//@version=6\nindicator(\"Repaint\", overlay = true)\n\
    plotcandle(open, high, low, close, color = color.fuchsia, wickcolor = color.fuchsia, \
    bordercolor = color.fuchsia)\n";

fn repainted_candles(harness: &mut Harness) -> usize {
    let (shapes, _) = harness.frame();
    gather(&shapes)
        .rects
        .iter()
        .filter(|(_, fill)| *fill == Color32::from_rgb(0xe0, 0x40, 0xfb))
        .count()
}

/// Frames until the script has repainted the candles on screen.
fn wait_for_repaint(harness: &mut Harness) {
    for _ in 0..300 {
        if repainted_candles(harness) > 0 {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("the script never repainted the candles");
}

#[test]
fn a_script_that_repaints_candles_does_not_draw_them_over_another_timeframe() {
    let mut h = harness(DataSource::Binance, series(120));
    h.card.set_pine_source(Some(REPAINT.to_owned()));
    wait_for_repaint(&mut h);

    // The workspace's timeframe changed and the new market candles arrived: the first frame over
    // them must not carry what the run over the old timeframe drew.
    h.card.set_timeframe(Timeframe::HOUR_4);
    h.candles = aapl(40);
    assert_eq!(repainted_candles(&mut h), 0);
    wait_for_repaint(&mut h);
}

#[test]
fn a_script_that_repaints_candles_does_not_draw_them_over_another_market() {
    let mut h = harness(DataSource::Binance, series(120));
    h.card.set_pine_source(Some(REPAINT.to_owned()));
    wait_for_repaint(&mut h);

    h.candles = aapl(40);
    h.source = DataSource::Alpaca;
    assert_eq!(repainted_candles(&mut h), 0);
    wait_for_repaint(&mut h);
}
