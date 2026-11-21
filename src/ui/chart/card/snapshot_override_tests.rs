//! What a snapshot may draw differently from the live frame (`SnapshotFrame`): other bars and
//! prices, frozen script outputs, no indicators, a restyled canvas. And that none of it reaches
//! the card.

use super::canvas_tests::{Drawn, Harness, gather, wait_for_run};
use super::snapshot::SnapshotGeometry;
use super::snapshot_tests::{dumped, everything_on, input, settled};
use super::*;
use crate::model::ChartCanvas;
use crate::model::chart::canvas::visible_window;
use crate::model::chart::snapshot::{SnapshotDimensions, SnapshotScale};
use crate::model::chart::viewport::zoom::Selection;
use crate::model::chart::viewport::{HorizontalViewport, VerticalViewport};
use crate::ui::chart::canvas::Restyle;
use eframe::egui::epaint::ClippedShape;

const PANE_SCRIPT: &str = "//@version=6\nindicator(\"Pane\")\nplot(close)\n";
const REACHING_SCRIPT: &str = "//@version=6
indicator(\"Reaching\", overlay = true)
if barstate.islast
    line.new(bar_index, close, bar_index + 12, close * 5)
";

/// A plot that is the close until bar 200, then twenty times it: far above anything the
/// candles of a chart scrolled back to bars 160-200 reach.
const JUMPING_OVERLAY: &str = "//@version=6
indicator(\"Jumping overlay\", overlay = true)
plot(bar_index < 200 ? close : close * 20)
";
const JUMPING_PANE: &str = "//@version=6
indicator(\"Jumping pane\")
plot(bar_index < 200 ? close : close * 20)
";
const FAR_DRAWING: &str = "//@version=6
indicator(\"Far drawing\", overlay = true)
if barstate.islast
    line.new(bar_index, close, bar_index + 500, close)
";

/// A plot twenty times the close, far above the candles; `scale` is the declaration's argument.
fn far_overlay(scale: &str) -> String {
    format!(
        "//@version=6
indicator(\"Far overlay\", overlay = true{scale})
plot(close * 20)
"
    )
}

/// A line drawn on the last bar, anchored 50 bars ahead and extended left across every bar on
/// screen, at twenty times the close; `rise` is how far its second end lies above its first.
fn extended_line_script(extend: &str, rise: f64) -> String {
    format!(
        "//@version=6
indicator(\"Extended\", overlay = true)
if barstate.islast
    line.new(bar_index + 50, close * 20, bar_index + 60, close * 20 + {rise:?}, extend = extend.{extend})
"
    )
}

/// A pane with the close and a label two slots past the last bar, at a price far below it.
const MARGIN_LABEL_PANE: &str = "//@version=6
indicator(\"Margin label\")
plot(close)
if barstate.islast
    label.new(bar_index + 2, 99.0, \"x\")
";

fn painted(harness: &Harness, input: &SnapshotInput) -> Vec<ClippedShape> {
    let rect = Rect::from_min_size(Pos2::ZERO, harness.card.snapshot_size());
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        screen_rect: Some(rect),
        ..Default::default()
    };
    let mut output = ctx.run_ui(raw, |ui| {
        harness
            .card
            .paint_snapshot(&ui.painter_at(rect), rect, input)
            .expect("a chart to paint");
    });
    output.textures_delta.clear();
    std::mem::take(&mut output.shapes)
}

fn geometry(harness: &Harness, input: &SnapshotInput) -> SnapshotGeometry {
    harness
        .card
        .snapshot_geometry(input, harness.card.snapshot_size(), 1.0)
        .expect("a chart to measure")
}

fn showing(horizontal: HorizontalViewport, prices: Option<(f64, f64)>) -> ChartViewport {
    ChartViewport {
        horizontal,
        vertical: VerticalViewport {
            manual: prices.map(|(low, high)| PriceRange::new(low, high)),
            ..VerticalViewport::default()
        },
    }
}

fn settled_harness() -> Harness {
    let mut harness = Harness::new(ChartCanvas::default());
    settled(&mut harness);
    harness
}

/// A chart with one script pane and every built-in indicator, drawn until the run arrived.
fn busy_harness() -> (Harness, u64) {
    let mut harness = Harness::new(ChartCanvas::default());
    harness.indicators = everything_on();
    let id = harness.card.add_pine_instance(None, "Pane", PANE_SCRIPT);
    wait_for_run(&mut harness, id);
    settled(&mut harness);
    assert_eq!(harness.card.pane_count(), 1, "the script has its pane");
    (harness, id)
}

// MARK: Bars and prices

#[test]
fn an_override_viewport_draws_exactly_the_bars_and_prices_asked_for() {
    let harness = settled_harness();
    let mut input = input(&harness, false);
    input.frame.viewport = Some(showing(
        HorizontalViewport {
            visible: 40,
            right_offset: 25.0,
        },
        Some((150.0, 300.0)),
    ));
    let view = geometry(&harness, &input).view;
    let window = visible_window(300, 40, 25.0, view.reserved);
    assert_eq!((view.start, view.end), (window.start, window.end));
    assert_eq!(view.lead, window.lead);
    assert_eq!((view.minimum, view.maximum), (150.0, 300.0));
    let slots = (40 + view.reserved) as f32;
    assert!((view.slot_width - view.plot.width() / slots).abs() < 1e-4);
    // What is painted is on that scale: no price on the axis is outside the range.
    let drawn = gather(&painted(&harness, &input));
    let axis: Vec<f64> = drawn
        .texts
        .iter()
        .filter(|(pos, ..)| pos.x > view.plot.right())
        .filter_map(|(_, text, _)| text.parse().ok())
        .collect();
    assert!(axis.len() >= 3, "{axis:?}");
    assert!(axis.iter().all(|price| (150.0..=300.0).contains(price)));
}

/// The chart scrolled back to bars 160-200 of 300, prices left to fit.
fn scrolled_back() -> ChartViewport {
    showing(
        HorizontalViewport {
            visible: 40,
            right_offset: 120.0,
        },
        None,
    )
}

#[test]
fn scripts_scrolled_back_fit_the_prices_on_screen_not_the_newest_bars() {
    let mut harness = Harness::new(ChartCanvas::default());
    let overlay = harness
        .card
        .add_pine_instance(None, "Jumping overlay", JUMPING_OVERLAY);
    wait_for_run(&mut harness, overlay);
    let pane = harness
        .card
        .add_pine_instance(None, "Jumping pane", JUMPING_PANE);
    wait_for_run(&mut harness, pane);
    settled(&mut harness);
    assert_eq!(harness.card.pane_count(), 1);

    let mut input = input(&harness, false);
    input.frame.viewport = Some(scrolled_back());
    let view = geometry(&harness, &input).view;
    assert!(view.end <= 200, "only bars before the jump are on screen");
    assert!(
        view.maximum < 400.0,
        "the price axis ends at {}: the candles are not squeezed under today's overlay",
        view.maximum
    );
    let axis: Vec<f64> = gather(&painted(&harness, &input))
        .texts
        .iter()
        .filter(|(pos, ..)| pos.x > view.plot.right() && pos.y > view.plot.bottom())
        .filter_map(|(_, text, _)| text.replace(',', "").parse().ok())
        .collect();
    assert!(axis.len() >= 2, "the pane writes its scale: {axis:?}");
    assert!(
        axis.iter().all(|value| *value < 400.0),
        "the pane's scale fits the bars on screen: {axis:?}"
    );
    // At the right edge the same scripts do reach today's values.
    input.frame.viewport = Some(showing(
        HorizontalViewport {
            visible: 40,
            right_offset: 0.0,
        },
        None,
    ));
    assert!(geometry(&harness, &input).view.maximum > 1_000.0);
}

#[test]
fn only_a_level_extended_across_the_screen_is_on_the_scrolled_back_axis() {
    let maximum_scrolled_back = |extend: &str, rise: f64| {
        let mut harness = Harness::new(ChartCanvas::default());
        let id =
            harness
                .card
                .add_pine_instance(None, "Extended", &extended_line_script(extend, rise));
        wait_for_run(&mut harness, id);
        settled(&mut harness);
        let mut input = input(&harness, false);
        input.frame.viewport = Some(scrolled_back());
        geometry(&harness, &input).view.maximum
    };
    assert!(
        maximum_scrolled_back("left", 0.0) > 4_000.0,
        "a level's price fits the axis wherever it crosses the bars on screen"
    );
    assert!(
        maximum_scrolled_back("none", 0.0) < 400.0,
        "the same level without extend is far off screen"
    );
    for rise in [1.0, 10.0, 1_000.0] {
        assert!(
            maximum_scrolled_back("left", rise) < 400.0,
            "a line rising {rise} does not stretch the axis, as on TradingView"
        );
    }
}

#[test]
fn a_scale_none_overlay_leaves_the_price_axis_to_the_candles() {
    let maximum = |scale: &str| {
        let mut harness = Harness::new(ChartCanvas::default());
        let id = harness
            .card
            .add_pine_instance(None, "Far overlay", &far_overlay(scale));
        wait_for_run(&mut harness, id);
        settled(&mut harness);
        let input = input(&harness, false);
        geometry(&harness, &input).view.maximum
    };
    assert!(
        maximum("") > 4_000.0,
        "an overlay on the price axis stretches it to its own values"
    );
    let own = maximum(", scale = scale.none");
    assert!(own < 400.0, "the candles keep their range: {own}");
}

/// The stroked path with the most points on the plot: a script's plot, the longest line there.
fn longest_path(drawn: &Drawn) -> &[Pos2] {
    drawn
        .paths
        .iter()
        .max_by_key(|points| points.len())
        .expect("a path was stroked")
}

#[test]
fn a_scale_none_overlay_is_fitted_to_its_own_hidden_scale_inside_the_plot() {
    let mut harness = Harness::new(ChartCanvas::default());
    let id =
        harness
            .card
            .add_pine_instance(None, "Far overlay", &far_overlay(", scale = scale.none"));
    wait_for_run(&mut harness, id);
    settled(&mut harness);
    let mut input = input(&harness, false);
    input.frame.viewport = Some(showing(
        HorizontalViewport {
            visible: 100,
            right_offset: 0.0,
        },
        None,
    ));
    let view = geometry(&harness, &input).view;
    let drawn = gather(&painted(&harness, &input));
    let line = longest_path(&drawn);
    assert!(line.len() >= 90, "the plot is drawn: {} points", line.len());
    let (top, bottom) = line
        .iter()
        .fold((f32::MAX, f32::MIN), |(top, bottom), point| {
            (top.min(point.y), bottom.max(point.y))
        });
    assert!(
        top >= view.plot.top() && bottom <= view.plot.bottom(),
        "inside the plot: {top}..{bottom} of {:?}",
        view.plot
    );
    assert!(
        bottom - top > view.plot.height() * 0.4,
        "fitted to the plot, not a sliver at the edge: {top}..{bottom} of {:?}",
        view.plot
    );
    // The same plot on the price axis would be off the plot, and the candles a sliver.
    let mut on_axis = Harness::new(ChartCanvas::default());
    let id = on_axis
        .card
        .add_pine_instance(None, "Far overlay", &far_overlay(""));
    wait_for_run(&mut on_axis, id);
    settled(&mut on_axis);
    let mut shared = self::input(&on_axis, false);
    shared.frame.viewport = input.frame.viewport;
    let shared_view = geometry(&on_axis, &shared).view;
    assert!(shared_view.maximum - shared_view.minimum > 10.0 * (view.maximum - view.minimum));
}

#[test]
fn a_scale_none_overlay_follows_the_bars_on_screen() {
    let mut harness = Harness::new(ChartCanvas::default());
    let id = harness.card.add_pine_instance(
        None,
        "Jumping overlay",
        "//@version=6
indicator(\"Jumping scale none\", overlay = true, scale = scale.none)
plot(bar_index < 200 ? close : close * 20)
",
    );
    wait_for_run(&mut harness, id);
    settled(&mut harness);
    let mut input = input(&harness, false);
    input.frame.viewport = Some(scrolled_back());
    let view = geometry(&harness, &input).view;
    let drawn = gather(&painted(&harness, &input));
    let line = longest_path(&drawn);
    let (top, bottom) = line
        .iter()
        .fold((f32::MAX, f32::MIN), |(top, bottom), point| {
            (top.min(point.y), bottom.max(point.y))
        });
    assert!(
        bottom - top > view.plot.height() * 0.2,
        "the hidden scale fits the bars before the jump, not today's values: {top}..{bottom}"
    );
}

#[test]
fn a_pane_label_in_the_right_margin_is_on_the_panes_axis() {
    let mut harness = Harness::new(ChartCanvas::default());
    let id = harness
        .card
        .add_pine_instance(None, "Margin label", MARGIN_LABEL_PANE);
    wait_for_run(&mut harness, id);
    settled(&mut harness);
    let mut input = input(&harness, false);
    input.frame.viewport = Some(showing(
        HorizontalViewport {
            visible: 40,
            right_offset: 0.0,
        },
        None,
    ));
    let view = geometry(&harness, &input).view;
    let axis: Vec<f64> = gather(&painted(&harness, &input))
        .texts
        .iter()
        .filter(|(pos, ..)| pos.x > view.plot.right() && pos.y > view.plot.bottom())
        .filter_map(|(_, text, _)| text.replace(',', "").parse().ok())
        .collect();
    let lowest = axis.iter().copied().fold(f64::MAX, f64::min);
    assert!(
        lowest < 120.0,
        "the pane reaches the label at 99 (the close is above 200): {axis:?}"
    );
}

#[test]
fn a_drawing_far_past_the_last_bar_reserves_the_same_few_slots_at_every_zoom() {
    let mut harness = Harness::new(ChartCanvas::default());
    let id = harness
        .card
        .add_pine_instance(None, "Far drawing", FAR_DRAWING);
    wait_for_run(&mut harness, id);
    settled(&mut harness);
    for visible in [20, 40, 120, 300] {
        let mut input = input(&harness, false);
        input.frame.viewport = Some(showing(
            HorizontalViewport {
                visible,
                right_offset: 0.0,
            },
            None,
        ));
        let view = geometry(&harness, &input).view;
        assert_eq!(view.reserved, 20, "{visible} candles");
    }
}

#[test]
fn selecting_a_portion_of_a_chart_with_a_far_drawing_shows_all_of_it() {
    let mut harness = Harness::new(ChartCanvas::default());
    let id = harness
        .card
        .add_pine_instance(None, "Far drawing", FAR_DRAWING);
    wait_for_run(&mut harness, id);
    settled(&mut harness);
    let mut input = input(&harness, false);
    input.frame.viewport = Some(showing(
        HorizontalViewport {
            visible: 120,
            right_offset: 100.0,
        },
        None,
    ));
    let before = geometry(&harness, &input).view;
    // Seventy slots from the middle of the plot: far fewer than are shown.
    let left = before.plot.left() + before.slot_width * 30.0;
    let selection = Selection::between(
        (left, before.plot.top()),
        (left + before.slot_width * 70.0, before.plot.bottom()),
        (
            before.plot.left(),
            before.plot.top(),
            before.plot.right(),
            before.plot.bottom(),
        ),
    );
    input.frame.viewport = before.viewport_for(&selection);
    let after = geometry(&harness, &input).view;
    let slots = after.plot.width() / after.slot_width;
    assert!((slots - 70.0).abs() <= 1.0, "{slots} slots, 70 selected");
}

/// A line drawn from the last bar to 40 slots ahead, more than the chart reserves.
const LINE_AHEAD: &str = "//@version=6
indicator(\"Line ahead\", overlay = true)
if barstate.islast
    line.new(bar_index, close, bar_index + 40, close)
";

#[test]
fn panning_past_the_newest_bar_keeps_the_candle_width_and_reaches_a_far_drawing() {
    let mut harness = Harness::new(ChartCanvas::default());
    let id = harness
        .card
        .add_pine_instance(None, "Line ahead", LINE_AHEAD);
    wait_for_run(&mut harness, id);
    settled(&mut harness);
    for visible in [40, 100, 200] {
        let at = |right_offset| {
            let mut input = input(&harness, false);
            let horizontal = HorizontalViewport {
                visible,
                right_offset,
            };
            input.frame.viewport = Some(showing(horizontal, None));
            geometry(&harness, &input).view
        };
        let resting = at(0.0);
        let horizontal = HorizontalViewport {
            visible,
            right_offset: 0.0,
        };
        let furthest = horizontal.offset_limits(resting.total).0;
        for step in 1..=4 {
            let panned = at(furthest * f64::from(step) / 4.0);
            assert_eq!(
                (panned.reserved, panned.slot_width),
                (resting.reserved, resting.slot_width),
                "{visible} candles, step {step} of the pan"
            );
        }
        let panned = at(furthest);
        let slot = (panned.total - 1 + 40 - panned.start) as f32 + panned.lead + 0.5;
        let end = panned.plot.left() + panned.slot_width * slot;
        assert!(
            end <= panned.plot.right(),
            "{visible} candles: the line ends at {end}, the plot at {}",
            panned.plot.right()
        );
    }
}

#[test]
fn a_snapshot_may_show_fewer_bars_than_a_chart_ever_does() {
    let harness = settled_harness();
    let mut input = input(&harness, false);
    input.frame.viewport = Some(showing(
        HorizontalViewport {
            visible: 7,
            right_offset: 0.0,
        },
        None,
    ));
    let view = geometry(&harness, &input).view;
    assert_eq!(view.end - view.start, 7);
    assert_eq!(view.end, 300, "the newest bars");
    let slots = (7 + view.reserved) as f32;
    assert!((view.slot_width - view.plot.width() / slots).abs() < 1e-4);
    // Autoscale follows the seven bars it shows.
    let shown = &harness.candles[view.start..view.end];
    let high = shown.iter().map(|bar| bar.high).fold(f64::MIN, f64::max);
    let low = shown.iter().map(|bar| bar.low).fold(f64::MAX, f64::min);
    assert!(view.minimum <= low && view.maximum >= high);
    let all = geometry(&harness, &self::input(&harness, false)).view;
    assert!(view.maximum - view.minimum < all.maximum - all.minimum);
    painted(&harness, &input);
}

#[test]
fn the_geometry_is_where_the_snapshot_paints() {
    let harness = settled_harness();
    let input = input(&harness, false);
    let SnapshotGeometry {
        view,
        axis_width,
        time_axis_height,
    } = geometry(&harness, &input);
    let size = harness.card.snapshot_size();
    assert_eq!(view.plot.min, Pos2::ZERO);
    assert_eq!(view.plot.right() + axis_width, size.x);
    assert_eq!(view.plot.bottom() + time_axis_height, size.y);
    assert_eq!(view.top_inset, 0.0);
    assert_eq!(view.total, 300);
    // The lines that bound the scales are drawn on the plot's edges.
    let drawn = gather(&painted(&harness, &input));
    let line = CanvasStyle::resolve(&harness.card.appearance().canvas).scale_line;
    let bounding: Vec<_> = drawn
        .segments
        .iter()
        .filter(|(_, stroke)| stroke.color == line)
        .collect();
    assert!(
        bounding.iter().any(|(points, _)| points[0].x == points[1].x
            && (points[0].x - view.plot.right()).abs() <= 0.5),
        "the price axis' edge"
    );
    assert!(
        bounding.iter().any(|(points, _)| points[0].y == points[1].y
            && (points[0].y - view.plot.bottom()).abs() <= 0.5),
        "the time axis' edge"
    );
}

#[test]
fn the_geometry_follows_the_scale_the_picture_is_drawn_at() {
    let harness = settled_harness();
    let input = input(&harness, false);
    let size = harness.card.snapshot_size();
    let at = |ppp| harness.card.snapshot_geometry(&input, size, ppp).unwrap();
    let (one, two) = (at(1.0), at(2.0));
    // Text is measured for the pixels it lands on, so the axis may differ by a pixel at most.
    assert!((one.axis_width - two.axis_width).abs() <= 1.0);
    assert_eq!(
        (one.view.start, one.view.end),
        (two.view.start, two.view.end)
    );
}

#[test]
fn a_rectangle_on_the_plot_round_trips_through_a_viewport() {
    let harness = settled_harness();
    for log in [false, true] {
        let mut input = input(&harness, false);
        input.scale_mode = PriceScaleMode::from_log_flag(log);
        let base = geometry(&harness, &input).view;
        let plot = base.plot;
        let selection = Selection::between(
            (plot.left() + 120.0, plot.top() + 60.0),
            (plot.left() + 420.0, plot.top() + 260.0),
            (plot.left(), plot.top(), plot.right(), plot.bottom()),
        );
        let viewport = base.viewport_for(&selection).expect("a viewport");
        assert!(viewport.vertical.manual.is_some());
        input.frame.viewport = Some(viewport);
        let framed = geometry(&harness, &input).view;

        let slot_at = |view: &ViewGeometry, x: f32| {
            f64::from(view.start as f32 - view.lead)
                + f64::from(x - view.plot.left()) / f64::from(view.slot_width)
        };
        let (want_left, want_right) = (
            slot_at(&base, selection.left),
            slot_at(&base, selection.right),
        );
        let (got_left, got_right) = (
            slot_at(&framed, framed.plot.left()),
            slot_at(&framed, framed.plot.right()),
        );
        // The bar count is whole: the edges land within half a bar of the rectangle's.
        assert!((got_left - want_left).abs() <= 0.5 + 1e-3, "{log}");
        assert!((got_right - want_right).abs() <= 0.5 + 1e-3, "{log}");

        let price_at = |y: f32| {
            let fraction = f64::from((y - plot.top()) / plot.height());
            if log {
                (base.maximum.ln() - fraction * (base.maximum.ln() - base.minimum.ln())).exp()
            } else {
                base.maximum - fraction * (base.maximum - base.minimum)
            }
        };
        let close = |a: f64, b: f64| (a - b).abs() <= 1e-6 * b.abs();
        assert!(close(framed.maximum, price_at(selection.top)), "{log}");
        assert!(close(framed.minimum, price_at(selection.bottom)), "{log}");
        assert_eq!(framed.logarithmic, log);
    }
}

#[test]
fn a_rectangle_too_narrow_to_choose_is_no_viewport_and_a_flat_one_keeps_autoscale() {
    let harness = settled_harness();
    let base = geometry(&harness, &input(&harness, false)).view;
    let plot = base.plot;
    let bounds = (plot.left(), plot.top(), plot.right(), plot.bottom());
    let thin = Selection::between((100.0, 50.0), (102.0, 250.0), bounds);
    assert_eq!(base.viewport_for(&thin), None);
    let flat = Selection::between((100.0, 100.0), (400.0, 102.0), bounds);
    let viewport = base.viewport_for(&flat).expect("the bars still");
    assert_eq!(viewport.vertical.manual, None);
}

// MARK: Nothing changes

#[test]
fn drawing_with_overrides_changes_nothing_in_the_card() {
    let (mut harness, id) = busy_harness();
    harness.card.set_visible_candles(80);
    settled(&mut harness);
    let viewport = harness.card.saved_viewport();
    let horizontal = harness.card.horizontal();
    let visible = harness.card.visible_candles();
    let panes = harness.card.pane_ids.clone();
    let extent = format!("{:?}", harness.card.script_extent);
    let near_oldest = harness.card.near_oldest;
    let own = dumped(&painted(&harness, &input(&harness, false)));
    let frozen = harness.card.snapshot_outputs();
    let mut overridden = input(&harness, false);
    overridden.frame = SnapshotFrame {
        viewport: Some(showing(
            HorizontalViewport {
                visible: 9,
                right_offset: -4.0,
            },
            Some((120.0, 180.0)),
        )),
        outputs: Some(&frozen),
        include_indicators: false,
        restyle: Restyle {
            hide_grid: true,
            hide_watermark: true,
            text_scale: 2.0,
        },
    };
    let size = harness.card.snapshot_size();
    for scale in SnapshotScale::ALL {
        let dimensions = SnapshotDimensions::fit(size.x, size.y, scale).expect("a size");
        harness
            .card
            .snapshot_scene(&overridden, dimensions, Color32::BLACK)
            .expect("a scene");
    }
    geometry(&harness, &overridden);
    assert_eq!(harness.card.saved_viewport(), viewport);
    assert_eq!(harness.card.horizontal(), horizontal);
    assert_eq!(harness.card.visible_candles(), visible);
    assert_eq!(harness.card.pane_ids, panes);
    assert_eq!(format!("{:?}", harness.card.script_extent), extent);
    assert_eq!(harness.card.near_oldest, near_oldest);
    assert_eq!(harness.card.pane_count(), 1);
    assert!(harness.card.pine_status(id).is_some());
    // The card's own picture is what it was.
    assert_eq!(
        dumped(&painted(&harness, &self::input(&harness, false))),
        own
    );
}

// MARK: Indicators

#[test]
fn without_indicators_the_chart_is_the_plain_chart_with_the_whole_height() {
    let (busy, _) = busy_harness();
    let plain = settled_harness();
    let with = geometry(&busy, &input(&busy, false));
    let mut without = input(&busy, false);
    without.frame.include_indicators = false;
    let bare = geometry(&busy, &without);
    let size = busy.card.snapshot_size();
    assert!(
        with.view.plot.height() < bare.view.plot.height() * 0.8,
        "the pane took its share: {} vs {}",
        with.view.plot.height(),
        bare.view.plot.height()
    );
    assert_eq!(bare.view.plot.bottom() + bare.time_axis_height, size.y);
    // Nothing of an indicator, a script or a pane is left: it is the plain chart's picture.
    assert_eq!(
        dumped(&painted(&busy, &without)),
        dumped(&painted(&plain, &input(&plain, false)))
    );
    assert_ne!(
        dumped(&painted(&busy, &input(&busy, false))),
        dumped(&painted(&plain, &input(&plain, false))),
        "and with them it is not"
    );
}

#[test]
fn without_indicators_an_overlay_script_reserves_no_slots_and_widens_no_scale() {
    let mut harness = Harness::new(ChartCanvas::default());
    let id = harness
        .card
        .add_pine_instance(None, "Reaching", REACHING_SCRIPT);
    wait_for_run(&mut harness, id);
    settled(&mut harness);
    let plain = settled_harness();
    let with = geometry(&harness, &input(&harness, false)).view;
    let bare = geometry(&plain, &input(&plain, false)).view;
    assert!(
        with.reserved > bare.reserved,
        "the script reaches past the bars"
    );
    assert!(with.maximum > bare.maximum, "and above the prices");
    let mut without = input(&harness, false);
    without.frame.include_indicators = false;
    assert_eq!(
        geometry(&harness, &without),
        geometry(&plain, &input(&plain, false))
    );
}

// MARK: Frozen outputs

#[test]
fn frozen_outputs_are_drawn_whatever_the_scripts_do_afterwards() {
    let (mut harness, id) = busy_harness();
    let frozen = harness.card.snapshot_outputs();
    let before = dumped(&painted(&harness, &input(&harness, false)));
    let tall = geometry(&harness, &input(&harness, false));

    harness.card.remove_pine_instance(id);
    harness.frame();
    settled(&mut harness);
    assert_eq!(harness.card.pane_count(), 0);

    let live = input(&harness, false);
    assert_ne!(
        dumped(&painted(&harness, &live)),
        before,
        "the script is gone"
    );
    let mut kept = input(&harness, false);
    kept.frame.outputs = Some(&frozen);
    assert_eq!(dumped(&painted(&harness, &kept)), before);
    assert_eq!(geometry(&harness, &kept), tall);
}

// MARK: Restyle

fn grid_segments(drawn: &Drawn, style: &CanvasStyle) -> usize {
    let colors = [style.horizontal_grid, style.vertical_grid]
        .map(|line| line.expect("the grid is on").stroke.color);
    drawn
        .segments
        .iter()
        .filter(|(_, stroke)| colors.contains(&stroke.color))
        .count()
}

fn largest_axis_text(drawn: &Drawn, plot_right: f32) -> f32 {
    drawn
        .texts
        .iter()
        .filter(|(pos, ..)| pos.x > plot_right)
        .map(|(_, _, size)| *size)
        .fold(0.0, f32::max)
}

#[test]
fn a_restyle_takes_the_grid_away_and_makes_the_axis_text_larger() {
    let harness = settled_harness();
    let style = CanvasStyle::resolve(&harness.card.appearance().canvas);
    let plain = input(&harness, false);
    let mut restyled = input(&harness, false);
    restyled.frame.restyle = Restyle {
        hide_grid: true,
        hide_watermark: true,
        text_scale: 1.5,
    };
    let (before, after) = (
        gather(&painted(&harness, &plain)),
        gather(&painted(&harness, &restyled)),
    );
    assert!(grid_segments(&before, &style) >= 4);
    assert_eq!(grid_segments(&after, &style), 0);
    let (small, large) = (geometry(&harness, &plain), geometry(&harness, &restyled));
    let (small_text, large_text) = (
        largest_axis_text(&before, small.view.plot.right()),
        largest_axis_text(&after, large.view.plot.right()),
    );
    assert!(small_text > 0.0);
    assert!((large_text - small_text * 1.5).abs() < 1e-3, "{large_text}");
    assert!(
        large.axis_width > small.axis_width,
        "the axis grows with it"
    );
    assert!(large.time_axis_height > small.time_axis_height);
    // The chart's own settings are not what changed.
    assert!(harness.card.appearance().canvas.vertical_grid.visible);
}

#[test]
fn a_restyle_that_changes_nothing_changes_nothing() {
    let harness = settled_harness();
    let plain = input(&harness, false);
    let mut same = input(&harness, false);
    same.frame.restyle = Restyle::default();
    assert_eq!(
        dumped(&painted(&harness, &plain)),
        dumped(&painted(&harness, &same))
    );
    let canvas = &harness.card.appearance().canvas;
    let style = CanvasStyle::resolve(canvas);
    let kept = style.restyled(Restyle::default());
    assert_eq!(
        (kept.price_size, kept.time_size, kept.watermark),
        (style.price_size, style.time_size, style.watermark)
    );
    // A scale that is no number is no scale.
    for scale in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        let odd = style.restyled(Restyle {
            text_scale: scale,
            ..Restyle::default()
        });
        assert_eq!(odd.price_size, style.price_size, "{scale}");
    }
}
