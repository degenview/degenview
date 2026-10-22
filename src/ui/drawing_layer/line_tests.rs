//! The lines, channels and pitchforks projected onto a plot whose numbers are easy to check by
//! hand: 100 px per bar (bar `i` is centred on x = 50 + 100 i), and prices 80 to 120 down a
//! 400 px plot (y = (120 − price) × 10).

use super::shape::{Screen, Tag, TextPlace};
use super::*;
use crate::model::chart_type::PriceSource;
use crate::model::{DrawingStyle, Kline, PitchforkVariant};
use eframe::egui::{Rect, Vec2, pos2};

const PLOT: Rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(1000.0, 400.0));

fn bars(closes: &[f64]) -> Vec<Kline> {
    closes
        .iter()
        .enumerate()
        .map(|(index, close)| Kline {
            open_time: index as i64 * 60_000,
            close_time: index as i64 * 60_000 + 59_999,
            open: *close,
            high: close + 1.0,
            low: close - 1.0,
            close: *close,
            volume: 1.0,
        })
        .collect()
}

fn scale(logarithmic: bool) -> PriceScale {
    PriceScale {
        minimum: 80.0,
        maximum: 120.0,
        top: 0.0,
        height: 400.0,
        logarithmic,
    }
}

fn projection<'a>(data: &'a [Kline], scale: &'a PriceScale) -> Projection<'a> {
    Projection {
        plot: PLOT,
        pixels_per_point: 2.0,
        format: PriceFormat::AUTO,
        scale,
        times: TimeIndex::new(data),
        start: 0,
        slot_width: 100.0,
        lead: 0.0,
    }
}

/// An anchor at bar `bar` and `price`: x = 50 + 100 bar, y = (120 - price) * 10.
fn at(bar: i64, price: f64) -> Anchor {
    Anchor {
        time_ms: bar * 60_000,
        price,
    }
}

fn near(a: Pos2, b: Pos2) -> bool {
    a.distance(b) < 0.05
}

fn built_with(tool: Tool, anchors: &[Anchor], style: DrawingStyle, data: &[Kline]) -> Screen {
    let drawing = Drawing::build(tool, 1, anchors, style).expect("built");
    let scale = scale(false);
    shape::build(&drawing, &projection(data, &scale)).expect("projected")
}

fn built(tool: Tool, anchors: &[Anchor]) -> Screen {
    built_with(tool, anchors, tool.default_style(), &bars(&[100.0; 10]))
}

/// Whether the drawing has nothing on the plot (so no screen shape is built for it).
fn not_drawn(tool: Tool, anchors: &[Anchor]) -> bool {
    let drawing = Drawing::build(tool, 1, anchors, tool.default_style()).expect("built");
    let data = bars(&[100.0; 10]);
    let scale = scale(false);
    shape::build(&drawing, &projection(&data, &scale)).is_none()
}

fn only_line(screen: &Screen) -> [Pos2; 2] {
    assert_eq!(screen.lines.len(), 1, "{:?}", screen.lines);
    [screen.lines[0][0], screen.lines[0][1]]
}

fn cross(a: Vec2, b: Vec2) -> f32 {
    a.x * b.y - a.y * b.x
}

// MARK: Lines

#[test]
fn a_ray_runs_from_its_origin_through_its_second_point_to_the_edge() {
    // (150, 200) → (350, 100): up and to the right, half a pixel up per pixel across.
    let screen = built(Tool::Ray, &[at(1, 100.0), at(3, 110.0)]);
    let [from, to] = only_line(&screen);
    assert!(
        near(from, pos2(150.0, 200.0)),
        "starts at its origin: {from:?}"
    );
    assert!(near(to, pos2(550.0, 0.0)), "leaves by the top: {to:?}");
    assert!(near(screen.handles[0].pos, pos2(150.0, 200.0)));
    assert!(near(screen.handles[1].pos, pos2(350.0, 100.0)));
    // Drawn the other way it runs the other way: the direction is origin to second point.
    let back = built(Tool::Ray, &[at(3, 110.0), at(1, 100.0)]);
    let [from, to] = only_line(&back);
    assert!(near(from, pos2(350.0, 100.0)));
    assert!(
        to.x == 0.0 || to.y == 400.0,
        "runs off the other end: {to:?}"
    );
}

#[test]
fn an_extended_line_runs_edge_to_edge_through_both_anchors() {
    let screen = built(Tool::ExtendedLine, &[at(1, 100.0), at(3, 110.0)]);
    let [from, to] = only_line(&screen);
    assert!(near(from, pos2(0.0, 275.0)), "{from:?}");
    assert!(near(to, pos2(550.0, 0.0)), "{to:?}");
    // The handles stay on the anchors, not on the ends of the visible line.
    assert!(near(screen.handles[0].pos, pos2(150.0, 200.0)));
    assert!(near(screen.handles[1].pos, pos2(350.0, 100.0)));
}

#[test]
fn an_extended_line_far_from_its_anchors_is_still_found_and_a_missing_one_is_not_drawn() {
    // Anchors far to the left and below, the line still crossing the plot.
    let screen = built(Tool::ExtendedLine, &[at(-40, 80.0), at(-30, 90.0)]);
    assert_eq!(screen.lines.len(), 1);
    let [from, to] = only_line(&screen);
    assert!(PLOT.expand(0.01).contains(from) && PLOT.expand(0.01).contains(to));
    // Hit-testing follows the visible line, not the far-off anchors.
    assert!(screen.hits_body(from.lerp(to, 0.5), 6.0, false));
    // A line that passes beside the plot is not drawn at all.
    let beside = built(Tool::ExtendedLine, &[at(-40, 80.0), at(-40, 120.0)]);
    assert!(beside.lines.is_empty());
}

#[test]
fn a_horizontal_line_spans_the_plot_at_its_price_whatever_its_time() {
    let screen = built(Tool::HorizontalLine, &[at(5, 100.0)]);
    let [from, to] = only_line(&screen);
    assert!(near(from, pos2(0.0, 200.0)) && near(to, pos2(1000.0, 200.0)));
    assert!(matches!(
        screen.tags[..],
        [Tag::Price { y, price, .. }] if y == 200.0 && price == 100.0
    ));
    // Out of the price range there is nothing on the plot to build, or to tag on the axis.
    assert!(not_drawn(Tool::HorizontalLine, &[at(5, 500.0)]));
}

#[test]
fn a_horizontal_ray_starts_at_its_origin_and_never_runs_left() {
    let screen = built(Tool::HorizontalRay, &[at(3, 100.0)]);
    let [from, to] = only_line(&screen);
    assert!(near(from, pos2(350.0, 200.0)) && near(to, pos2(1000.0, 200.0)));
    assert!(near(screen.handles[0].pos, pos2(350.0, 200.0)));
    // An origin off to the left starts at the plot's edge.
    let left = built(Tool::HorizontalRay, &[at(-5, 100.0)]);
    assert!(near(only_line(&left)[0], pos2(0.0, 200.0)));
    // An origin past the right edge draws nothing.
    assert!(not_drawn(Tool::HorizontalRay, &[at(30, 100.0)]));
}

#[test]
fn a_vertical_line_spans_the_plot_at_its_time() {
    let screen = built(Tool::VerticalLine, &[at(2, 100.0)]);
    let [from, to] = only_line(&screen);
    assert!(near(from, pos2(250.0, 0.0)) && near(to, pos2(250.0, 400.0)));
    assert!(matches!(
        screen.tags[..],
        [Tag::Time { x, time_ms, .. }] if x == 250.0 && time_ms == 120_000
    ));
    assert!(matches!(screen.text, Some(TextPlace::Column { x, .. }) if x == 250.0));
}

#[test]
fn a_crossline_is_two_lines_through_one_handle() {
    let screen = built(Tool::Crossline, &[at(2, 100.0)]);
    assert_eq!(screen.lines.len(), 2);
    assert!(
        near(screen.lines[0][0], pos2(0.0, 200.0)) && near(screen.lines[0][1], pos2(1000.0, 200.0))
    );
    assert!(
        near(screen.lines[1][0], pos2(250.0, 0.0)) && near(screen.lines[1][1], pos2(250.0, 400.0))
    );
    assert_eq!(screen.handles.len(), 1);
    assert!(near(screen.handles[0].pos, pos2(250.0, 200.0)));
    assert_eq!(screen.tags.len(), 2);
    // Both arms can be picked.
    assert!(screen.hits_body(pos2(700.0, 200.0), 6.0, false));
    assert!(screen.hits_body(pos2(250.0, 30.0), 6.0, false));
    assert!(!screen.hits_body(pos2(700.0, 120.0), 6.0, false));
}

#[test]
fn a_trend_angle_is_read_from_its_left_end() {
    let screen = built(Tool::TrendAngle, &[at(3, 110.0), at(1, 100.0)]);
    let mark = screen.angle.expect("an angle mark");
    assert!(
        near(mark.origin, pos2(150.0, 200.0)),
        "left end: {:?}",
        mark.origin
    );
    assert!(near(mark.toward, pos2(350.0, 100.0)));
}

#[test]
fn a_texted_line_puts_its_text_along_the_visible_stretch() {
    let mut drawing = Drawing::build(
        Tool::TrendLine,
        1,
        &[at(1, 100.0), at(3, 110.0)],
        Tool::TrendLine.default_style(),
    )
    .unwrap();
    drawing.set_text("support");
    let scale = scale(false);
    let data = bars(&[100.0; 10]);
    let screen = shape::build(&drawing, &projection(&data, &scale)).unwrap();
    let Some(TextPlace::Line { from, to }) = screen.text else {
        panic!("text along the line");
    };
    assert!(near(from, pos2(150.0, 200.0)) && near(to, pos2(350.0, 100.0)));
}

// MARK: Channels

#[test]
fn a_parallel_channel_keeps_its_second_edge_parallel_at_a_price_offset() {
    // Edge one (150, 200) → (350, 100). c at (250, 250) is 100 px below the edge at x = 250.
    let screen = built(
        Tool::ParallelChannel,
        &[at(1, 100.0), at(3, 110.0), at(2, 95.0)],
    );
    assert_eq!(screen.lines.len(), 2);
    assert!(
        near(screen.lines[1][0], pos2(150.0, 300.0)),
        "{:?}",
        screen.lines[1]
    );
    assert!(near(screen.lines[1][1], pos2(350.0, 200.0)));
    let first = screen.lines[0][1] - screen.lines[0][0];
    let second = screen.lines[1][1] - screen.lines[1][0];
    assert!(cross(first, second).abs() < 1e-3, "parallel");
    // The middle line is halfway between them, and the fill is the region between.
    assert_eq!(screen.styled.len(), 1);
    assert!(near(screen.styled[0].path[0], pos2(150.0, 250.0)));
    assert_eq!(screen.fill.len(), 4);
    // The handle on the second edge sits at its middle.
    assert!(near(screen.handles[2].pos, pos2(250.0, 250.0)));
}

#[test]
fn a_parallel_channel_runs_on_with_its_extensions_and_stays_inside_the_plot() {
    let mut style = Tool::ParallelChannel.default_style();
    style.extend_right = true;
    style.extend_left = true;
    let screen = built_with(
        Tool::ParallelChannel,
        &[at(1, 100.0), at(3, 110.0), at(2, 95.0)],
        style,
        &bars(&[100.0; 10]),
    );
    for line in &screen.lines {
        assert!(PLOT.expand(0.01).contains(line[0]) && PLOT.expand(0.01).contains(line[1]));
    }
    assert!(
        screen
            .fill
            .iter()
            .all(|point| PLOT.expand(0.01).contains(*point))
    );
    // Reversed and falling edges keep their offsets too.
    let falling = built(
        Tool::ParallelChannel,
        &[at(3, 100.0), at(1, 110.0), at(2, 90.0)],
    );
    let first = falling.lines[0][1] - falling.lines[0][0];
    let second = falling.lines[1][1] - falling.lines[1][0];
    assert!(cross(first, second).abs() < 1e-3);
}

#[test]
fn a_flat_channel_has_a_level_edge_over_the_same_times() {
    let screen = built(
        Tool::FlatChannel,
        &[at(1, 100.0), at(3, 110.0), at(2, 95.0)],
    );
    assert_eq!(screen.lines.len(), 2);
    assert!(
        near(screen.lines[1][0], pos2(150.0, 250.0))
            && near(screen.lines[1][1], pos2(350.0, 250.0))
    );
    assert!(screen.lines[1][0].y == screen.lines[1][1].y, "level");
    assert_eq!(screen.fill.len(), 4);
    assert!(near(screen.handles[2].pos, pos2(250.0, 250.0)));
}

#[test]
fn a_disjoint_channel_keeps_its_edges_parallel_with_their_own_times() {
    // Edge one (150, 200) → (350, 100); c at (550, 300); d at bar 6 → x = 650.
    let screen = built(
        Tool::DisjointChannel,
        &[at(1, 100.0), at(3, 110.0), at(5, 90.0)],
    );
    assert_eq!(screen.lines.len(), 2);
    assert!(
        near(screen.lines[0][0], pos2(150.0, 200.0))
            && near(screen.lines[0][1], pos2(350.0, 100.0))
    );
    // The second edge starts where c was clicked and is as long as the first.
    assert!(
        near(screen.lines[1][0], pos2(550.0, 300.0)),
        "{:?}",
        screen.lines[1]
    );
    assert!(
        near(screen.lines[1][1], pos2(750.0, 200.0)),
        "{:?}",
        screen.lines[1]
    );
    let first = screen.lines[0][1] - screen.lines[0][0];
    let second = screen.lines[1][1] - screen.lines[1][0];
    assert!(cross(first, second).abs() < 1e-3, "parallel");
    assert_eq!(screen.handles.len(), 4);
    // Moving the first edge's ends leaves the second where it is: that is the degree of freedom
    // a parallel channel does not have.
    assert_ne!(screen.lines[0][0].x, screen.lines[1][0].x);
}

// MARK: Pitchforks

fn fork_points() -> [Anchor; 3] {
    // (150, 200), (350, 100), (550, 200).
    [at(1, 100.0), at(3, 110.0), at(5, 100.0)]
}

#[test]
fn a_pitchfork_draws_its_median_and_enabled_levels_on_both_sides() {
    let screen = built(Tool::Pitchfork, &fork_points());
    // Origin P1; the handle runs to the middle of P2–P3, (450, 150), and on to two handles.
    let [from, to] = only_line(&screen);
    assert!(near(from, pos2(150.0, 200.0)), "{from:?}");
    assert!(near(to, pos2(750.0, 100.0)), "{to:?}");
    // Levels 0.5 and 1 on, each side: four lines, all parallel to the median.
    assert_eq!(screen.styled.len(), 4);
    let median = to - from;
    for styled in &screen.styled {
        let line = styled.path[1] - styled.path[0];
        assert!(cross(median, line).abs() < 1e-2, "parallel to the median");
    }
    // Level 1 starts on P2 and P3.
    let starts: Vec<Pos2> = screen.styled.iter().map(|s| s.path[0]).collect();
    assert!(starts.iter().any(|start| near(*start, pos2(350.0, 100.0))));
    assert!(starts.iter().any(|start| near(*start, pos2(550.0, 200.0))));
    // Five lines make four bands between them.
    assert_eq!(screen.bands.len(), 4);
    assert!(screen.bands.iter().all(|band| band.polygon.len() >= 3));
    assert_eq!(screen.handles.len(), 3);
}

#[test]
fn the_four_pitchforks_start_and_end_differently() {
    let medians: Vec<[Pos2; 2]> = PitchforkVariant::ALL
        .iter()
        .map(|variant| only_line(&built(variant.tool(), &fork_points())))
        .collect();
    // Standard from P1; Schiff from P1's time at the price halfway to P2; Modified Schiff from
    // the middle of P1 and P2; Inside from there to P3.
    assert!(near(medians[0][0], pos2(150.0, 200.0)));
    assert!(near(medians[1][0], pos2(150.0, 150.0)));
    assert!(near(medians[2][0], pos2(250.0, 150.0)));
    assert!(near(medians[3][0], pos2(250.0, 150.0)));
    for (index, one) in medians.iter().enumerate() {
        for other in &medians[index + 1..] {
            assert!(
                !(near(one[0], other[0]) && near(one[1], other[1])),
                "{one:?} {other:?}"
            );
        }
    }
}

#[test]
fn a_pitchfork_with_levels_off_is_just_its_median_and_extends_when_asked() {
    let mut style = Tool::Pitchfork.default_style();
    for level in &mut style.pitchfork_levels {
        level.line.enabled = false;
    }
    let screen = built_with(Tool::Pitchfork, &fork_points(), style, &bars(&[100.0; 10]));
    assert_eq!(screen.lines.len(), 1);
    assert!(screen.styled.is_empty() && screen.bands.is_empty());
    style.extend_right = true;
    let long = built_with(Tool::Pitchfork, &fork_points(), style, &bars(&[100.0; 10]));
    let [_, to] = only_line(&long);
    assert!(
        to.x == 1000.0 || to.y == 0.0 || to.y == 400.0,
        "runs to the plot's edge: {to:?}"
    );
}

#[test]
fn a_pitchfork_is_unchanged_by_how_far_the_chart_is_zoomed() {
    // Squeeze the price axis to half: every construction keeps its ratios.
    let data = bars(&[100.0; 10]);
    let tall = scale(false);
    let squeezed = PriceScale {
        minimum: 40.0,
        maximum: 160.0,
        top: 0.0,
        height: 400.0,
        logarithmic: false,
    };
    let drawing = Drawing::build(
        Tool::Pitchfork,
        1,
        &fork_points(),
        Tool::Pitchfork.default_style(),
    )
    .unwrap();
    let a = shape::build(&drawing, &projection(&data, &tall)).unwrap();
    let b = shape::build(&drawing, &projection(&data, &squeezed)).unwrap();
    let fork = |screen: &Screen| {
        let handle = screen.lines[0][1] - screen.lines[0][0];
        let level = screen.styled[0].path[1] - screen.styled[0].path[0];
        cross(handle, level) / handle.length_sq()
    };
    assert!(
        fork(&a).abs() < 1e-3 && fork(&b).abs() < 1e-3,
        "levels stay parallel"
    );
}

// MARK: Regression

fn rising() -> Vec<Kline> {
    bars(&[100.0, 102.0, 104.0, 106.0, 108.0])
}

fn regression(extend: bool, data: &[Kline]) -> Screen {
    let mut style = Tool::RegressionTrend.default_style();
    style.extend_right = extend;
    built_with(
        Tool::RegressionTrend,
        &[at(0, 100.0), at(4, 108.0)],
        style,
        data,
    )
}

#[test]
fn a_regression_is_fitted_to_the_bars_between_its_anchors() {
    let screen = regression(false, &rising());
    // A perfect line: slope 2 a bar. Base from (50, 200) to (450, 120), and no spread.
    let base = &screen.styled[0].path;
    assert!(near(base[0], pos2(50.0, 200.0)), "{base:?}");
    assert!(near(*base.last().unwrap(), pos2(450.0, 120.0)));
    let upper = &screen.styled[1].path;
    assert!(
        near(upper[0], base[0]),
        "no deviation, so the bands lie on the line"
    );
    let readout = screen.readout.as_ref().expect("Pearson's R");
    assert_eq!(readout.text, "Pearson's R: 1.00");
    // The handles are on the line at the anchors' bars.
    assert!(near(screen.handles[0].pos, pos2(50.0, 200.0)));
    assert!(near(screen.handles[1].pos, pos2(450.0, 120.0)));
}

#[test]
fn a_regression_extends_to_the_plots_right_edge_when_asked() {
    let screen = regression(true, &rising());
    let base = &screen.styled[0].path;
    let end = *base.last().unwrap();
    assert!(end.x >= 1000.0, "{end:?}");
    // Still the same line: 2 a bar, 20 px up for each 100 px across.
    assert!((base[0].y - end.y - (end.x - base[0].x) * 0.2).abs() < 0.05);
}

#[test]
fn a_regression_has_bands_that_follow_the_deviation() {
    // Closes with spread: 100, 104, 100, 104, 100 → a flat fit with sigma sqrt(3.84) ≈ 1.96.
    let data = bars(&[100.0, 104.0, 100.0, 104.0, 100.0]);
    let mut style = Tool::RegressionTrend.default_style();
    style.regression.upper_dev = 1.0;
    style.regression.lower_dev = -3.0;
    let screen = built_with(
        Tool::RegressionTrend,
        &[at(0, 100.0), at(4, 100.0)],
        style,
        &data,
    );
    let base = screen.styled[0].path[0].y;
    let upper = screen.styled[1].path[0].y;
    let lower = screen.styled[2].path[0].y;
    // Upper is one sigma above (smaller y), lower three sigmas below.
    let sigma = 3.84f32.sqrt() * 10.0;
    assert!((base - upper - sigma).abs() < 0.2, "{base} {upper}");
    assert!((lower - base - 3.0 * sigma).abs() < 0.5);
    assert_eq!(screen.bands.len(), 2);
}

#[test]
fn a_regression_over_one_bar_has_nothing_to_fit() {
    let screen = built_with(
        Tool::RegressionTrend,
        // Two times within the one bar.
        &[
            Anchor {
                time_ms: 125_000,
                price: 100.0,
            },
            Anchor {
                time_ms: 150_000,
                price: 101.0,
            },
        ],
        Tool::RegressionTrend.default_style(),
        &rising(),
    );
    assert!(screen.styled.is_empty() && screen.readout.is_none());
    assert_eq!(screen.handles.len(), 2, "still there to be moved");
}

#[test]
fn a_regression_in_replay_uses_only_the_bars_shown() {
    // Five bars rising, then a crash the replay has not reached.
    let all = bars(&[100.0, 102.0, 104.0, 106.0, 108.0, 60.0, 60.0]);
    let shown = &all[..5];
    let with_future = built_with(
        Tool::RegressionTrend,
        &[at(0, 100.0), at(6, 100.0)],
        Tool::RegressionTrend.default_style(),
        shown,
    );
    let to_the_end = built_with(
        Tool::RegressionTrend,
        &[at(0, 100.0), at(4, 100.0)],
        Tool::RegressionTrend.default_style(),
        shown,
    );
    // An anchor after the cursor reads the last bar shown, so both fit the same five bars.
    let fit = |screen: &Screen| screen.styled[0].path.clone();
    assert!(near(fit(&with_future)[0], fit(&to_the_end)[0]));
    assert!(near(
        *fit(&with_future).last().unwrap(),
        *fit(&to_the_end).last().unwrap()
    ));
}

#[test]
fn a_regression_follows_its_source_and_a_log_scale_bends_it() {
    let data = rising();
    let mut style = Tool::RegressionTrend.default_style();
    style.regression.source = PriceSource::High;
    let high = built_with(
        Tool::RegressionTrend,
        &[at(0, 100.0), at(4, 108.0)],
        style,
        &data,
    );
    // The high is one above the close: ten pixels higher.
    assert!((high.styled[0].path[0].y - 190.0).abs() < 0.05);
    // On a log scale it is fitted in logs and drawn as a curve of several points.
    let log = scale(true);
    let drawing = Drawing::build(
        Tool::RegressionTrend,
        1,
        &[at(0, 100.0), at(4, 108.0)],
        Tool::RegressionTrend.default_style(),
    )
    .unwrap();
    let screen = shape::build(&drawing, &projection(&data, &log)).unwrap();
    assert!(screen.styled[0].path.len() > 2);
}

#[test]
fn the_screen_cache_notices_a_live_bar_changing() {
    let mut data = rising();
    let scale = scale(false);
    let before = projection(&data, &scale).key();
    let last = data.len() - 1;
    data[last].close += 1.0;
    let after = projection(&data, &scale).key();
    assert!(before != after, "a regression must be fitted again");
}

// MARK: Editing

fn edit_with(
    original: &Drawing,
    part: edit::Part,
    press: Pos2,
    now: Pos2,
    proj: &Projection,
) -> Drawing {
    edit::edited(original, part, press, now, proj.anchor(now), proj)
}

fn channel_width(drawing: &Drawing, proj: &Projection) -> f32 {
    let points: Vec<Pos2> = drawing.anchors().iter().map(|a| proj.point(*a)).collect();
    crate::ui::line_geometry::channel_offset(points[0], points[1], points[2]).y
}

#[test]
fn dragging_a_channel_end_keeps_the_price_distance_between_its_edges() {
    let data = bars(&[100.0; 10]);
    let scale = scale(false);
    let proj = projection(&data, &scale);
    let channel = Drawing::build(
        Tool::ParallelChannel,
        1,
        &[at(1, 100.0), at(3, 110.0), at(2, 95.0)],
        Tool::ParallelChannel.default_style(),
    )
    .unwrap();
    let width = channel_width(&channel, &proj);
    assert!((width - 100.0).abs() < 0.1, "{width}");
    // Drag the first end 50 px down: the edge swings, the distance stays.
    let moved = edit_with(
        &channel,
        edit::Part::Anchor(0),
        pos2(150.0, 200.0),
        pos2(150.0, 250.0),
        &proj,
    );
    assert!((channel_width(&moved, &proj) - width).abs() < 0.1);
    assert_eq!(moved.anchors()[2].time_ms, channel.anchors()[2].time_ms);
    assert_eq!(moved.anchors()[1], channel.anchors()[1]);
    // So does dragging the other end, and moving the whole channel.
    let other = edit_with(
        &channel,
        edit::Part::Anchor(1),
        pos2(350.0, 100.0),
        pos2(420.0, 40.0),
        &proj,
    );
    assert!((channel_width(&other, &proj) - width).abs() < 0.1);
    let whole = edit_with(
        &channel,
        edit::Part::Body,
        pos2(250.0, 150.0),
        pos2(300.0, 180.0),
        &proj,
    );
    assert!((channel_width(&whole, &proj) - width).abs() < 0.1);
}

#[test]
fn the_handle_on_the_second_edge_changes_only_the_distance() {
    let data = bars(&[100.0; 10]);
    let scale = scale(false);
    let proj = projection(&data, &scale);
    let channel = Drawing::build(
        Tool::ParallelChannel,
        1,
        &[at(1, 100.0), at(3, 110.0), at(2, 95.0)],
        Tool::ParallelChannel.default_style(),
    )
    .unwrap();
    // Pull it 30 px up while the pointer wanders sideways: the distance shrinks by 30 and
    // the first edge, and where the second edge is anchored, do not move.
    let moved = edit_with(
        &channel,
        edit::Part::Anchor(2),
        pos2(250.0, 250.0),
        pos2(400.0, 220.0),
        &proj,
    );
    assert!((channel_width(&moved, &proj) - 70.0).abs() < 0.1);
    assert_eq!(&moved.anchors()[..2], &channel.anchors()[..2]);
    assert_eq!(moved.anchors()[2].time_ms, channel.anchors()[2].time_ms);
}

#[test]
fn a_flat_channels_level_edge_moves_in_price_only() {
    let data = bars(&[100.0; 10]);
    let scale = scale(false);
    let proj = projection(&data, &scale);
    let channel = Drawing::build(
        Tool::FlatChannel,
        1,
        &[at(1, 100.0), at(3, 110.0), at(2, 95.0)],
        Tool::FlatChannel.default_style(),
    )
    .unwrap();
    let moved = edit_with(
        &channel,
        edit::Part::Anchor(2),
        pos2(250.0, 250.0),
        pos2(700.0, 300.0),
        &proj,
    );
    let level = moved.anchors()[2];
    assert!((level.price - 90.0).abs() < 1e-3, "{level:?}");
    assert_eq!(
        level.time_ms, 120_000,
        "stays at the middle of the sloped edge"
    );
}

#[test]
fn a_horizontal_line_slides_along_its_price_and_a_vertical_one_along_its_time() {
    let data = bars(&[100.0; 10]);
    let scale = scale(false);
    let proj = projection(&data, &scale);
    let horizontal = Drawing::build(
        Tool::HorizontalLine,
        1,
        &[at(4, 100.0)],
        Tool::HorizontalLine.default_style(),
    )
    .unwrap();
    let moved = edit_with(
        &horizontal,
        edit::Part::Anchor(0),
        pos2(500.0, 200.0),
        pos2(800.0, 250.0),
        &proj,
    );
    assert_eq!(moved.anchors()[0].time_ms, horizontal.anchors()[0].time_ms);
    assert!((moved.anchors()[0].price - 95.0).abs() < 1e-3);
    let vertical = Drawing::build(
        Tool::VerticalLine,
        1,
        &[at(4, 100.0)],
        Tool::VerticalLine.default_style(),
    )
    .unwrap();
    let moved = edit_with(
        &vertical,
        edit::Part::Anchor(0),
        pos2(450.0, 200.0),
        pos2(650.0, 50.0),
        &proj,
    );
    assert_eq!(moved.anchors()[0].price, 100.0);
    assert_eq!(moved.anchors()[0].time_ms, 360_000);
}

#[test]
fn a_regression_handle_moves_the_bar_and_never_the_fit() {
    let data = rising();
    let scale = scale(false);
    let proj = projection(&data, &scale);
    let regression = Drawing::build(
        Tool::RegressionTrend,
        1,
        &[at(0, 100.0), at(4, 108.0)],
        Tool::RegressionTrend.default_style(),
    )
    .unwrap();
    // Drag the last handle left to the third bar, and far above the line.
    let moved = edit_with(
        &regression,
        edit::Part::Anchor(1),
        pos2(450.0, 120.0),
        pos2(250.0, 20.0),
        &proj,
    );
    let screen = shape::build(&moved, &projection(&data, &scale)).unwrap();
    // Fitted to the first three bars now; the pointer's height changed nothing about the line.
    assert!(near(screen.styled[0].path[0], pos2(50.0, 200.0)));
    assert!(near(
        *screen.styled[0].path.last().unwrap(),
        pos2(250.0, 160.0)
    ));
}
