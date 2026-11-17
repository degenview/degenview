//! Comparison overlays on a chart drawn headlessly: where the lines go, what the tags and the
//! crosshair say, and that a replay, a hidden overlay or a price-based chart type shows nothing it
//! should not.

use super::canvas_tests::{HOUR, Harness, SIZE, axis_texts, series};
use super::*;
use crate::model::chart::kind::{ChartType, PriceSource};
use crate::model::overlay::feed::{OverlayFeed, OverlayStatus};
use crate::model::overlay::{OverlayId, OverlayList, OverlayScale, ScaleChoice};
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
    blocked.status = OverlayStatus::Blocked("Same market as the chart".into());
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
        frozen: false,
        x_fraction,
        time: None,
        price: 120.0,
        owner_id: 1,
    }
}

/// Seconds between two frames while the pointer rests, so a tooltip's delay passes in a few.
const TICK: f64 = 0.2;

/// One frame of a chart in a workspace: what `workspace/grid.rs` does with the chart's answer. A
/// pointer on the card freezes the crosshair where it was; a pointer the plot sees moves it; and
/// with neither the crosshair is gone. Returns whether the card's market was named and what the
/// chart reported.
fn grid_frame(chart: &mut Harness) -> (bool, ChartPointer) {
    let (shapes, pointer) = chart.frame();
    let named = super::canvas_tests::gather(&shapes)
        .texts
        .iter()
        .any(|(_, text, _)| text == "ETHUSDT");
    let mut next = None;
    if pointer.on_overlay_card {
        next = chart.crosshair.map(|crosshair| CrosshairInfo {
            frozen: true,
            ..crosshair
        });
    }
    if let (Some(x_fraction), Some(price)) = (pointer.hover_x_fraction, pointer.hover_price) {
        next = Some(CrosshairInfo {
            frozen: false,
            owner_id: 1,
            x_fraction,
            time: pointer.hover_time,
            price,
        });
    }
    chart.crosshair = next;
    (named, pointer)
}

/// A chart with the pointer resting on its plot, so the card is up, and the card's rectangle.
fn chart_with_card_at_rest() -> (Harness, Rect) {
    let mut chart = harness(&[("ETHUSDT", market(300, |n| 4_000.0 + n as f64))]);
    chart.pointer = Some(Pos2::new(400.0, 300.0));
    for _ in 0..4 {
        grid_frame(&mut chart);
    }
    let rect = overlay_card::card_rect(&chart.ctx, 1).expect("the card is up");
    (chart, rect)
}

/// Puts the pointer at `at` and runs frames: the card must be there in every one, and the chart's
/// answer for "the pointer is on the card" must be the same from the third frame on.
fn assert_steady_at(chart: &mut Harness, at: Pos2, what: &str) {
    chart.pointer = Some(at);
    // Long enough for the tooltips to come up, which must not disturb the card either.
    let frames: Vec<(bool, bool)> = (0..12)
        .map(|_| {
            chart.clock += TICK;
            let (named, pointer) = grid_frame(chart);
            (named, pointer.on_overlay_card)
        })
        .collect();
    assert!(
        frames.iter().all(|(named, _)| *named),
        "{what}: the card went away in some frame; (named, on card) per frame: {frames:?}"
    );
    assert!(
        frames[2..].windows(2).all(|pair| pair[0].1 == pair[1].1),
        "{what}: the card flips between on and off; (named, on card) per frame: {frames:?}"
    );
}

#[test]
fn a_pointer_on_the_card_keeps_it_up_and_steady() {
    let (mut chart, rect) = chart_with_card_at_rest();
    assert_steady_at(&mut chart, rect.center(), "inside the card");
    let (_, pointer) = grid_frame(&mut chart);
    assert!(pointer.on_overlay_card);
}

#[test]
fn a_pointer_just_outside_the_card_keeps_it_up_and_steady() {
    // egui may give the pointer to a widget up to its interact radius away: the card must stay up,
    // and say the same about the pointer from frame to frame, in that margin too.
    for what in ["left of", "under", "right of", "just under"] {
        let (mut chart, rect) = chart_with_card_at_rest();
        let at = match what {
            "left of" => Pos2::new(rect.left() - 3.0, rect.center().y),
            "under" => Pos2::new(rect.center().x, rect.bottom() + 3.0),
            "right of" => Pos2::new(rect.right() + 3.0, rect.center().y),
            _ => Pos2::new(rect.center().x, rect.bottom() + 1.0),
        };
        assert_steady_at(&mut chart, at, what);
    }
}

#[test]
fn a_pointer_on_a_rows_buttons_keeps_the_card_up_and_steady() {
    let (mut chart, rest) = chart_with_card_at_rest();
    // On the card the buttons make room, so the card widens; the last button is at its right end.
    chart.pointer = Some(rest.center());
    for _ in 0..4 {
        grid_frame(&mut chart);
    }
    let wide = overlay_card::card_rect(&chart.ctx, 1).expect("the card is up");
    assert!(wide.width() > rest.width(), "{wide:?} vs {rest:?}");
    assert_steady_at(
        &mut chart,
        Pos2::new(wide.right() - 14.0, wide.center().y),
        "on the buttons",
    );
}

/// The texts of the tooltips a frame drew: the row's, or a button's.
fn tooltips(chart: &mut Harness) -> Vec<String> {
    chart.clock += TICK;
    let drawn = super::canvas_tests::gather(&chart.frame().0);
    drawn
        .texts
        .into_iter()
        .map(|(_, text, _)| text)
        .filter(|text| {
            text.contains("· Binance") || ["Hide", "Settings", "Remove"].contains(&text.as_str())
        })
        .collect()
}

#[test]
fn a_button_has_its_own_tooltip_even_after_the_rows_has_come_up() {
    let (mut chart, rest) = chart_with_card_at_rest();
    chart.pointer = Some(rest.center());
    let mut seen = Vec::new();
    for _ in 0..8 {
        seen = tooltips(&mut chart);
    }
    assert_eq!(seen, ["ETHUSDT · Binance"], "the row's tooltip is up");
    let wide = overlay_card::card_rect(&chart.ctx, 1).expect("the card is up");
    // The remove button, at the right end of the row.
    chart.pointer = Some(Pos2::new(wide.right() - 14.0, wide.center().y));
    for _ in 0..8 {
        seen = tooltips(&mut chart);
    }
    assert_eq!(
        seen,
        ["Remove"],
        "the button says what it does, not the row"
    );
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
fn the_readout_has_the_same_padding_above_its_first_row_and_below_its_last() {
    let mut chart = harness(&[
        ("ETHUSDT", market(300, |n| 4_000.0 + n as f64)),
        ("SOLUSDT", market(300, |n| 150.0 + n as f64)),
    ]);
    chart.crosshair = Some(crosshair_at(&chart, 0.5));
    let (drawn, _) = chart.draw();
    let names = ["ETHUSDT", "SOLUSDT"];
    let centres: Vec<f32> = names
        .iter()
        .map(|name| {
            drawn
                .texts
                .iter()
                .find(|(_, text, _)| text == name)
                .unwrap_or_else(|| panic!("a row for {name}: {:?}", drawn.texts))
                .0
                .y
        })
        .collect();
    let box_fill = theme::chrome().gamma_multiply(0.94);
    let (boxed, _) = drawn
        .rects
        .iter()
        .find(|(rect, fill)| {
            *fill == box_fill && rect.contains(Pos2::new(rect.center().x, centres[0]))
        })
        .expect("the readout box");
    let above = centres[0] - boxed.top();
    let below = boxed.bottom() - centres[1];
    assert!(
        (above - below).abs() < 0.01,
        "padding above {above} and below {below}"
    );
}

#[test]
fn the_card_with_one_market_is_as_tall_as_the_ohlc_box_above_it() {
    let mut chart = harness(&[("ETHUSDT", market(300, |n| 4_000.0 + n as f64))]);
    chart.crosshair = Some(crosshair_at(&chart, 0.5));
    let (drawn, _) = chart.draw();
    let box_fill = theme::chrome().gamma_multiply(0.94);
    let mut boxes: Vec<Rect> = drawn
        .rects
        .iter()
        .filter(|(_, fill)| *fill == box_fill)
        .map(|(rect, _)| *rect)
        .collect();
    boxes.sort_by(|a, b| a.top().total_cmp(&b.top()));
    assert!(boxes.len() >= 2, "the OHLC box and the card: {boxes:?}");
    assert!(
        (boxes[0].height() - boxes[1].height()).abs() < 0.5,
        "{} vs {}",
        boxes[0].height(),
        boxes[1].height()
    );
}

#[test]
fn a_frozen_crosshair_keeps_the_ohlc_box_and_the_card_but_draws_no_lines_or_tags() {
    let mut chart = harness(&[("ETHUSDT", market(300, |n| 4_000.0 + n as f64))]);
    let live = crosshair_at(&chart, 0.5);
    chart.crosshair = Some(live);
    let (moving, _) = chart.draw();
    chart.crosshair = Some(CrosshairInfo {
        frozen: true,
        ..live
    });
    let (frozen, _) = chart.draw();
    let has = |drawn: &super::canvas_tests::Drawn, text: &str| {
        drawn.texts.iter().any(|(_, t, _)| t.starts_with(text))
    };
    for text in ["O ", "C ", "Vol ", "ETHUSDT"] {
        assert!(
            has(&moving, text) && has(&frozen, text),
            "{text} stays: {:?} / {:?}",
            moving.texts.iter().map(|t| t.1.clone()).collect::<Vec<_>>(),
            frozen.texts.iter().map(|t| t.1.clone()).collect::<Vec<_>>()
        );
    }
    assert!(
        frozen.segments.len() < moving.segments.len(),
        "the crosshair's lines are gone: {} vs {}",
        frozen.segments.len(),
        moving.segments.len()
    );
    assert!(
        frozen.texts.len() < moving.texts.len(),
        "so are the price and time tags: {} vs {}",
        frozen.texts.len(),
        moving.texts.len()
    );
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
    // A hidden market stays on the card, so it can be shown again; it says it is hidden.
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
    assert!(drawn.texts.iter().any(|(_, text, _)| text == "OLDUSDT"));
    assert!(drawn.texts.iter().any(|(_, text, _)| text == "hidden"));
}

#[test]
fn the_card_is_only_there_while_the_crosshair_is_and_has_no_controls_until_a_row_is_hovered() {
    let mut chart = harness(&[("ETHUSDT", market(300, |n| 4_000.0 + n as f64))]);
    let (drawn, _) = chart.draw();
    assert!(
        drawn.texts.iter().all(|(_, text, _)| text != "ETHUSDT"),
        "no crosshair, no card"
    );
    chart.crosshair = Some(crosshair_at(&chart, 0.5));
    let (drawn, _) = chart.draw();
    assert!(drawn.texts.iter().any(|(_, text, _)| text == "ETHUSDT"));
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

/// A chart comparing two markets under a crosshair, in the pseudo-locale: the header's hints, the
/// readout's labels, the compared-markets card and the chart's refresh
/// warning. `DEGENVIEW_GALLERY_DIR=<dir>` writes the pictures.
#[test]
fn a_compared_chart_draws_in_the_pseudo_locale() {
    use crate::i18n::{Locale, with_locale};
    use crate::ui::chart::painted_text;
    use crate::ui::render_tests::{Screen, gallery_dir};

    let _theme = crate::ui::theme::RestoreDefaultTheme;
    for (theme_name, app_theme) in [
        ("dark", crate::ui::theme::AppTheme::Dark),
        ("light", crate::ui::theme::AppTheme::Light),
    ] {
        let mut chart = harness(&[
            ("ETHUSDT", market(300, |n| 4_000.0 + n as f64)),
            ("SOLUSDT", market(300, |n| 90.0 + n as f64 / 4.0)),
        ]);
        chart.replay_active = true;
        chart.error = Some(ProviderError::other("down"));
        chart.crosshair = Some(crosshair_at(&chart, 0.85));
        let mut screen = Screen::new(SIZE);
        crate::ui::theme::apply(&screen.ctx, app_theme);
        let mut last = None;
        with_locale(Locale::Pseudo, || {
            for _ in 0..4 {
                let candles = chart.candles.clone();
                let (card, indicators, crosshair, replay_active) = (
                    &mut chart.card,
                    &chart.indicators,
                    chart.crosshair,
                    chart.replay_active,
                );
                let error = chart.error.as_ref();
                last = Some(screen.run(Vec::new(), |ui| {
                    let mut said = ChartCardAction::default();
                    card.header(
                        ui,
                        "BTCUSDT",
                        DataSource::Binance,
                        indicators,
                        &candles,
                        replay_active,
                        false,
                        &[],
                        &mut said,
                        1,
                    );
                    let mut drawing = DrawingFrame::new(&[], crate::model::Tool::Cursor, None, 1)
                        .with_view(Timeframe::HOUR_1, false);
                    let mut replay = ReplayChartFrame {
                        selecting: false,
                        active: replay_active,
                        picked_ms: None,
                        picked_range: None,
                    };
                    card.body(
                        ui,
                        "BTCUSDT",
                        &candles,
                        false,
                        error,
                        PriceScaleMode::Regular,
                        DataSource::Binance,
                        indicators,
                        crosshair.as_ref(),
                        1,
                        &mut drawing,
                        &mut replay,
                    );
                }));
            }
        });
        let last = last.take().expect("a frame");
        let texts = painted_text::of(&last);
        painted_text::assert_painted(&texts, "Replay");
        // The wide card has room for the detailed wording, which ends with the error.
        assert!(
            texts
                .iter()
                .any(|text| text.starts_with("[Çöûļðñ'ţ ŕéfŕéšĥ") && text.contains("(down)")),
            "the refresh warning: {texts:#?}"
        );
        assert!(
            texts.iter().any(|text| text.starts_with("[Ö ~] ")),
            "the readout's open label: {texts:#?}"
        );
        if gallery_dir().is_some() {
            eprintln!("{:?}", painted_text::unbracketed(&texts));
            screen.save(&format!("pseudo-compared-chart-{theme_name}"), last);
        }
    }
}
