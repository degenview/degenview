//! Comparison overlays on a chart drawn headlessly: where the lines go, what the tags and the
//! crosshair say, and that a replay, a hidden overlay or a price-based chart type shows nothing it
//! should not.

use super::canvas_tests::{HOUR, Harness, SIZE, axis_texts, series};
use super::*;
use crate::model::chart_type::{ChartType, PriceSource};
use crate::model::overlay::{OverlayId, OverlayList, OverlayScale, ScaleChoice};
use crate::model::overlay_feed::{OverlayFeed, OverlayStatus};
use crate::model::{ChartCanvas, InstrumentId};
use eframe::egui::epaint::{ClippedShape, PathShape};

/// The colour of the first overlay added.
const FIRST: Color32 = Color32::from_rgb(0x29, 0x62, 0xFF);

fn candle(open_time: i64, close: f64) -> Kline {
    Kline {
        open_time,
        close_time: open_time + HOUR - 1,
        open: close,
        high: close,
        low: close,
        close,
        volume: 1.0,
    }
}

/// An overlay market's candles on the same hours as the chart's `series`.
pub(super) fn market(count: usize, value: impl Fn(usize) -> f64) -> Vec<Kline> {
    (0..count)
        .map(|n| candle(n as i64 * HOUR, value(n)))
        .collect()
}

pub(super) fn feed(candles: Vec<Kline>) -> OverlayFeed {
    OverlayFeed {
        candles: Arc::new(candles),
        revision: 1,
        status: OverlayStatus::Ready,
    }
}

/// A chart comparing `markets` (each a symbol and its candles).
pub(super) fn harness(markets: &[(&str, Vec<Kline>)]) -> Harness {
    let mut harness = Harness::new(ChartCanvas::default());
    let main = InstrumentId::new(DataSource::Binance, "BTCUSDT");
    let mut list = OverlayList::new();
    let mut feeds = Vec::new();
    for (symbol, candles) in markets {
        list.add(&main, InstrumentId::new(DataSource::Binance, *symbol), None);
        feeds.push(feed(candles.clone()));
    }
    harness
        .card
        .set_overlays(list, feeds, 1, DataSource::Binance, "BTCUSDT");
    harness
}

fn paths_of(shape: &Shape, colour: Color32, out: &mut Vec<Vec<Pos2>>) {
    match shape {
        Shape::Path(PathShape { points, stroke, .. }) => {
            if stroke.color == egui::epaint::ColorMode::Solid(colour) {
                out.push(points.clone());
            }
        }
        Shape::Vec(shapes) => shapes.iter().for_each(|shape| paths_of(shape, colour, out)),
        _ => {}
    }
}

/// Every polyline of `colour` the frame painted.
fn lines(shapes: &[ClippedShape], colour: Color32) -> Vec<Vec<Pos2>> {
    let mut out = Vec::new();
    for clipped in shapes {
        paths_of(&clipped.shape, colour, &mut out);
    }
    out
}

/// A few frames so layout settles, then the last one's shapes.
fn settle(harness: &mut Harness) -> Vec<ClippedShape> {
    let mut last = harness.frame().0;
    for _ in 0..3 {
        last = harness.frame().0;
    }
    last
}

fn plot_covers(points: &[Pos2]) -> (f32, f32) {
    let top = points.iter().map(|p| p.y).fold(f32::MAX, f32::min);
    let bottom = points.iter().map(|p| p.y).fold(f32::MIN, f32::max);
    (top, bottom)
}

#[test]
fn an_overlay_is_one_line_on_the_charts_bars() {
    let mut chart = harness(&[("ETHUSDT", market(300, |n| 4_000.0 + n as f64))]);
    let shapes = settle(&mut chart);
    let runs = lines(&shapes, FIRST);
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert!(runs[0].len() > 50);
    assert!(runs[0].windows(2).all(|pair| pair[1].x > pair[0].x));
    assert!(
        runs[0]
            .iter()
            .all(|p| p.x <= SIZE.x && p.y >= 0.0 && p.y <= SIZE.y)
    );
}

#[test]
fn a_bar_without_a_candle_breaks_the_line_and_is_not_bridged() {
    let mut candles = market(300, |n| 4_000.0 + n as f64);
    candles.retain(|c| !(285..290).contains(&(c.open_time / HOUR)));
    let mut chart = harness(&[("ETHUSDT", candles)]);
    let runs = lines(&settle(&mut chart), FIRST);
    assert_eq!(runs.len(), 2, "one gap, two runs");
    let gap = runs[1][0].x - runs[0].last().unwrap().x;
    let step = runs[0][1].x - runs[0][0].x;
    assert!(
        gap > step * 5.0,
        "the gap spans the missing bars: {gap} vs {step}"
    );
}

#[test]
fn a_hidden_overlay_draws_nothing_and_a_removed_one_neither() {
    let mut chart = harness(&[("ETHUSDT", market(300, |n| 10.0 + n as f64))]);
    let mut list = chart.card.overlay_list().clone();
    let id = list.as_slice()[0].id;
    list.set_visible(id, false);
    let feeds = vec![feed(market(300, |n| 10.0 + n as f64))];
    chart
        .card
        .set_overlays(list, feeds, 2, DataSource::Binance, "BTCUSDT");
    assert!(lines(&settle(&mut chart), FIRST).is_empty());
    chart.card.set_overlays(
        OverlayList::new(),
        Vec::new(),
        3,
        DataSource::Binance,
        "BTCUSDT",
    );
    assert!(lines(&settle(&mut chart), FIRST).is_empty());
}

#[test]
fn an_overlay_far_from_the_charts_price_is_still_on_the_plot_on_the_main_scale() {
    // BTC-like chart (~100..250 here), an overlay at 0.2: rebased, it starts where the chart does.
    let mut chart = harness(&[("DOGEUSDT", market(300, |n| 0.2 + n as f64 * 0.0005))]);
    let shapes = settle(&mut chart);
    let line = &lines(&shapes, FIRST)[0];
    let (top, bottom) = plot_covers(line);
    assert!(top >= 0.0 && bottom <= SIZE.y, "{top}..{bottom}");
    assert!(line.len() > 50);
}

#[test]
fn on_an_own_scale_the_line_fills_the_plot_whatever_its_magnitude() {
    let mut chart = harness(&[("DOGEUSDT", market(300, |n| 0.2 + n as f64 * 0.0005))]);
    let mut list = chart.card.overlay_list().clone();
    let id = list.as_slice()[0].id;
    assert!(list.set_scale(id, ScaleChoice::Own));
    chart.card.set_overlays(
        list,
        vec![feed(market(300, |n| 0.2 + n as f64 * 0.0005))],
        2,
        DataSource::Binance,
        "BTCUSDT",
    );
    let shapes = settle(&mut chart);
    let line = &lines(&shapes, FIRST)[0];
    let (top, bottom) = plot_covers(line);
    assert!(bottom - top > SIZE.y * 0.5, "{top}..{bottom}");
    assert!(top >= 0.0 && bottom <= SIZE.y);
}

#[test]
fn a_rebased_overlay_reads_as_its_own_percent_change_on_the_percent_axis() {
    // The overlay doubles over the view: its tag reads +100% whatever the chart's price is.
    let candles = market(300, |n| if n == 299 { 20.0 } else { 10.0 });
    let mut chart = harness(&[("ETHUSDT", candles)]);
    chart.scale_mode = PriceScaleMode::Percent;
    let (drawn, _) = chart.draw();
    let texts = axis_texts(&drawn);
    assert!(texts.iter().any(|t| t.contains("+100")), "{texts:?}");
}

#[test]
fn nothing_after_the_replay_cursor_reaches_the_lines_the_range_or_the_tags() {
    let mut future = market(300, |n| 50.0 + n as f64 * 0.01);
    for candle in future.iter_mut().skip(150) {
        candle.close = 5.0e6;
        candle.open = 5.0e6;
        candle.high = 5.0e6;
        candle.low = 5.0e6;
    }
    let mut chart = harness(&[("ETHUSDT", future)]);
    chart.candles = series(150);
    let shapes = settle(&mut chart);
    let line = &lines(&shapes, FIRST)[0];
    let (top, bottom) = plot_covers(line);
    // Were the 5e6 candles read, the range would explode and the line would be a flat sliver.
    assert!(bottom - top > 5.0, "{top}..{bottom}");
    let (drawn, _) = chart.draw();
    assert!(
        axis_texts(&drawn)
            .iter()
            .all(|t| !t.contains("5000000") && !t.contains("5,000,000")),
        "{:?}",
        axis_texts(&drawn)
    );
}

#[test]
fn price_based_chart_types_have_no_shared_time_axis_and_show_no_overlay() {
    let mut chart = harness(&[("ETHUSDT", market(300, |n| 10.0 + n as f64))]);
    chart.card.appearance_mut().style.chart_type = ChartType::Renko;
    let shapes = settle(&mut chart);
    assert!(lines(&shapes, FIRST).is_empty());
}

#[test]
fn the_overlay_of_the_charts_own_market_is_kept_but_not_drawn() {
    let mut chart = harness(&[("ETHUSDT", market(300, |n| 10.0 + n as f64))]);
    let list = chart.card.overlay_list().clone();
    let mut blocked = feed(market(300, |n| 10.0 + n as f64));
    blocked.status = OverlayStatus::Blocked("Same market as the chart");
    chart
        .card
        .set_overlays(list, vec![blocked], 2, DataSource::Binance, "ETHUSDT");
    assert!(lines(&settle(&mut chart), FIRST).is_empty());
    assert_eq!(chart.card.overlay_list().len(), 1);
}

#[test]
fn the_crosshair_names_every_overlay_and_says_so_where_a_market_has_no_candle() {
    let mut sparse = market(300, |n| 7.0 + n as f64);
    sparse.retain(|c| c.open_time / HOUR < 200);
    let mut chart = harness(&[
        ("ETHUSDT", market(300, |n| 4_000.0 + n as f64)),
        ("SOLUSDT", sparse),
    ]);
    // Over the newest bars: ETH has candles there, SOL does not.
    chart.crosshair = Some(crosshair_at(&chart, 0.85));
    let (drawn, _) = chart.draw();
    let texts: Vec<_> = drawn
        .texts
        .iter()
        .map(|(_, text, _)| text.as_str())
        .collect();
    assert!(texts.contains(&"ETHUSDT"), "{texts:?}");
    assert!(texts.contains(&"SOLUSDT"), "{texts:?}");
    assert!(
        texts.contains(&"—"),
        "a bar without a candle says so: {texts:?}"
    );
    assert!(
        texts
            .iter()
            .any(|t| t.starts_with("4,2") || t.starts_with("4,1")),
        "{texts:?}"
    );
}

fn crosshair_at(_chart: &Harness, x_fraction: f64) -> CrosshairInfo {
    CrosshairInfo {
        x_fraction,
        price: 120.0,
        owner_id: 1,
    }
}

#[test]
fn many_overlays_draw_and_the_readout_counts_the_rows_it_cannot_fit() {
    let markets: Vec<(String, Vec<Kline>)> = (0..60)
        .map(|n| {
            (
                format!("S{n}USDT"),
                market(300, move |i| 10.0 + n as f64 + i as f64 * 0.1),
            )
        })
        .collect();
    let refs: Vec<(&str, Vec<Kline>)> = markets
        .iter()
        .map(|(s, c)| (s.as_str(), c.clone()))
        .collect();
    let mut chart = harness(&refs);
    chart.crosshair = Some(crosshair_at(&chart, 0.5));
    let (drawn, _) = chart.draw();
    let more = drawn
        .texts
        .iter()
        .find(|(_, text, _)| text.ends_with("more") && text.starts_with('+'));
    assert!(more.is_some(), "a +N more row");
    assert_eq!(chart.card.overlay_list().len(), 60, "nothing was dropped");
}

#[test]
fn the_source_and_scale_are_per_overlay() {
    let mut chart = harness(&[
        ("ETHUSDT", market(300, |n| 4_000.0 + n as f64)),
        ("SOLUSDT", market(300, |n| 150.0 + n as f64)),
    ]);
    let mut list = chart.card.overlay_list().clone();
    let (a, b) = (list.as_slice()[0].id, list.as_slice()[1].id);
    list.set_scale(b, ScaleChoice::Own);
    list.get_mut(a).unwrap().source = PriceSource::High;
    assert_eq!(list.get(b).unwrap().scale, OverlayScale::Own(b));
    let _ = OverlayId(0);
    chart.card.set_overlays(
        list,
        vec![
            feed(market(300, |n| 4_000.0 + n as f64)),
            feed(market(300, |n| 150.0 + n as f64)),
        ],
        2,
        DataSource::Binance,
        "BTCUSDT",
    );
    let shapes = settle(&mut chart);
    assert_eq!(lines(&shapes, FIRST).len(), 1);
}

#[test]
fn the_crosshair_names_a_market_with_nothing_in_view_and_gives_it_a_dash() {
    // OLDUSDT stopped trading long before the bars in view: it has candles but none here.
    let mut chart = harness(&[
        ("ETHUSDT", market(300, |n| 4_000.0 + n as f64)),
        ("OLDUSDT", market(30, |n| 5.0 + n as f64)),
    ]);
    chart.crosshair = Some(crosshair_at(&chart, 0.5));
    let shapes = settle(&mut chart);
    let orange = Color32::from_rgb(0xFF, 0x98, 0x00);
    assert!(lines(&shapes, orange).is_empty(), "nothing of it is drawn");
    let (drawn, _) = chart.draw();
    let texts: Vec<_> = drawn
        .texts
        .iter()
        .map(|(_, text, _)| text.as_str())
        .collect();
    assert!(texts.contains(&"OLDUSDT"), "it still has a row: {texts:?}");
    assert!(texts.contains(&"—"), "with a dash: {texts:?}");
    // A hidden market is not listed.
    let mut list = chart.card.overlay_list().clone();
    let old = list.as_slice()[1].id;
    list.set_visible(old, false);
    let feeds = vec![
        feed(market(300, |n| 4_000.0 + n as f64)),
        feed(market(30, |n| 5.0 + n as f64)),
    ];
    chart
        .card
        .set_overlays(list, feeds, 2, DataSource::Binance, "BTCUSDT");
    let (drawn, _) = chart.draw();
    assert!(drawn.texts.iter().all(|(_, text, _)| text != "OLDUSDT"));
}

/// The right edge of the plot: where the grid lines stop (the lines that run on to the card's
/// edge, such as the time axis, are longer).
fn plot_right(drawn: &super::canvas_tests::Drawn) -> f32 {
    drawn
        .segments
        .iter()
        .filter(|(points, _)| {
            points[0].y == points[1].y && (points[1].x - points[0].x).abs() > 100.0
        })
        .map(|(points, _)| points[0].x.max(points[1].x))
        .fold(f32::MAX, f32::min)
}

#[test]
fn the_axis_is_wide_enough_for_an_overlays_last_value_tag() {
    // On the Regular axis a compared market's tag reads its own price, which can be far wider than
    // the chart's: the axis has to make room, or the text runs into the plot.
    let plain = {
        let mut chart = Harness::new(ChartCanvas::default());
        chart.draw().0
    };
    let wide = {
        let mut chart = harness(&[("BIGUSDT", market(300, |n| 1_234_567.891 + n as f64))]);
        assert_eq!(chart.scale_mode, PriceScaleMode::Regular);
        chart.draw().0
    };
    assert!(
        plot_right(&wide) < plot_right(&plain) - 8.0,
        "the plot narrowed to fit the tag: {} vs {}",
        plot_right(&wide),
        plot_right(&plain)
    );
    // And the tag itself sits inside the card.
    let tag = wide
        .texts
        .iter()
        .find(|(_, text, _)| text.starts_with("1,23"))
        .expect("the overlay's tag");
    assert!(tag.0.x < SIZE.x - 20.0, "{:?}", tag.0);
}

/// What a frame costs as the number of compared markets grows. A measurement, not a check: run it
/// in release mode, `cargo test --release overlay_cost -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement, not a check"]
fn overlay_cost() {
    use std::time::Instant;
    const BARS: usize = 2_000;
    let main = series(BARS);
    println!(
        "\n{BARS} bars on the chart, {BARS} candles per market, 760x520 headless frame (no GPU)"
    );
    println!(
        "{:>8} {:>14} {:>16} {:>14} {:>12}",
        "markets", "idle frame ms", "every-tick ms", "candles MB", "line runs"
    );
    for count in [0_usize, 1, 5, 10, 25, 50, 100, 300] {
        let candles: Vec<Kline> = (0..BARS)
            .map(|n| candle(n as i64 * HOUR, 100.0 + (n as f64 * 0.01).sin() * 10.0))
            .collect();
        let markets: Vec<(String, Vec<Kline>)> = (0..count)
            .map(|n| (format!("S{n}USDT"), candles.clone()))
            .collect();
        let refs: Vec<(&str, Vec<Kline>)> = markets
            .iter()
            .map(|(s, c)| (s.as_str(), c.clone()))
            .collect();
        let mut chart = harness(&refs);
        chart.candles = main.clone();
        for _ in 0..4 {
            chart.frame();
        }
        let frames = 30;
        let start = Instant::now();
        for _ in 0..frames {
            chart.frame();
        }
        let idle = start.elapsed().as_secs_f64() * 1000.0 / frames as f64;

        // Worst case: every market's candles change before every frame (a live tick on each).
        let list = chart.card.overlay_list().clone();
        let mut revision = 10;
        let start = Instant::now();
        for _ in 0..frames {
            revision += 1;
            let feeds = (0..list.len())
                .map(|n| OverlayFeed {
                    candles: Arc::new(markets[n].1.clone()),
                    revision,
                    status: OverlayStatus::Ready,
                })
                .collect();
            chart.card.set_overlays(
                list.clone(),
                feeds,
                revision,
                DataSource::Binance,
                "BTCUSDT",
            );
            chart.frame();
        }
        let ticking = start.elapsed().as_secs_f64() * 1000.0 / frames as f64;
        let shapes = chart.frame().0;
        let runs: usize = shapes
            .iter()
            .map(|clipped| {
                let mut found = Vec::new();
                for colour in crate::model::overlay::PALETTE {
                    let rgb =
                        Color32::from_rgb((colour >> 16) as u8, (colour >> 8) as u8, colour as u8);
                    paths_of(&clipped.shape, rgb, &mut found);
                }
                found.len()
            })
            .sum();
        let megabytes = (count * BARS * std::mem::size_of::<Kline>()) as f64 / 1e6;
        println!("{count:>8} {idle:>14.2} {ticking:>16.2} {megabytes:>14.1} {runs:>12}");
    }
}
