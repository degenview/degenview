//! The Fibonacci drawings projected onto a plot whose numbers are easy to check by hand: 100 px per
//! bar (bar `i` is centred on x = 50 + 100 i), and prices 80 to 120 down a 400 px plot
//! (y = (120 − price) × 10). Every expected number below is worked out by hand, not read back from
//! the code under test.

use super::line_tests::{PLOT, at, bars, near, projection, scale};
use super::shape::{LabelLook, Screen};
use super::*;
use crate::model::DrawingStyle;
use crate::model::fib_style::FibStyle;
use eframe::egui::pos2;

fn built_on(tool: Tool, anchors: &[Anchor], style: DrawingStyle, logarithmic: bool) -> Screen {
    let drawing = Drawing::build(tool, 1, anchors, style).expect("built");
    let data = bars(&[100.0; 10]);
    let scale = scale(logarithmic);
    shape::build(&drawing, &projection(&data, &scale)).expect("projected")
}

fn built(tool: Tool, anchors: &[Anchor]) -> Screen {
    built_on(tool, anchors, tool.default_style(), false)
}

/// The y of every drawn level line (a horizontal `styled` path), nearest the top first.
fn level_ys(screen: &Screen) -> Vec<f32> {
    let mut ys: Vec<f32> = screen
        .styled
        .iter()
        .filter(|line| line.path.len() == 2 && line.path[0].y == line.path[1].y)
        .map(|line| line.path[0].y)
        .collect();
    ys.sort_by(f32::total_cmp);
    ys
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.05
}

// MARK: Retracement

#[test]
fn a_retracement_has_zero_on_its_end_and_one_on_its_start() {
    // From 100 (bar 1) to 110 (bar 5): 0 is at 110 (y = 100), 1 at 100 (y = 200).
    let screen = built(Tool::FibRetracement, &[at(1, 100.0), at(5, 110.0)]);
    let ys = level_ys(&screen);
    // 0, 0.236, 0.382, 0.5, 0.618, 0.786, 1, 1.618, 2.618 are on the plot; 3.618 (y 461.8) and
    // 4.236 (y 523.6) are below it.
    let expected = [
        100.0, 123.6, 138.2, 150.0, 161.8, 178.6, 200.0, 261.8, 361.8,
    ];
    assert_eq!(ys.len(), expected.len(), "{ys:?}");
    for (y, want) in ys.iter().zip(expected) {
        assert!(close(*y, want), "{y} against {want}");
    }
    // The lines run between the anchors: bar 1 is x = 150, bar 5 is x = 550.
    for line in &screen.styled {
        assert!(close(line.path[0].x, 150.0) && close(line.path[1].x, 550.0));
    }
    assert!(near(screen.handles[0].pos, pos2(150.0, 200.0)));
    assert!(near(screen.handles[1].pos, pos2(550.0, 100.0)));
}

#[test]
fn a_falling_retracement_mirrors_a_rising_one() {
    // From 110 down to 100: 0 at 100 (y 200), 1 at 110 (y 100); 0.5 stays at 105 (y 150).
    let screen = built(Tool::FibRetracement, &[at(1, 110.0), at(5, 100.0)]);
    let ys = level_ys(&screen);
    // 0.236 sits 2.36 above the end, at 102.36 (y 176.4).
    assert!(ys.iter().any(|y| close(*y, 176.4)), "{ys:?}");
    assert!(ys.iter().any(|y| close(*y, 150.0)));
    // 1.618 is 16.18 above the start, at 126.18: off the plot's top.
    assert!(ys.iter().all(|y| *y >= 0.0));
}

#[test]
fn reverse_swaps_the_ends() {
    let mut style = Tool::FibRetracement.default_style();
    let mut fib = style.fib.expect("a retracement carries its levels");
    fib.reverse = true;
    style.fib = Some(fib);
    let screen = built_on(
        Tool::FibRetracement,
        &[at(1, 100.0), at(5, 110.0)],
        style,
        false,
    );
    let ys = level_ys(&screen);
    // 0 is now at the start (100, y 200) and 1 at the end (110, y 100); 0.236 is at 102.36.
    assert!(ys.iter().any(|y| close(*y, 176.4)), "{ys:?}");
    assert!(!ys.iter().any(|y| close(*y, 123.6)));
}

#[test]
fn extending_runs_the_levels_to_the_plots_edges() {
    let mut style = Tool::FibRetracement.default_style();
    style.extend_left = true;
    style.extend_right = true;
    let screen = built_on(
        Tool::FibRetracement,
        &[at(1, 100.0), at(5, 110.0)],
        style,
        false,
    );
    for line in &screen.styled {
        assert!(line.path[0].x <= PLOT.left(), "{:?}", line.path);
        assert!(line.path[1].x >= PLOT.right(), "{:?}", line.path);
        assert!(line.path[0].x >= PLOT.left() - 13.0, "a bounded coordinate");
        assert!(line.path[1].x <= PLOT.right() + 13.0);
    }
    // Only one end.
    let mut style = Tool::FibRetracement.default_style();
    style.extend_right = true;
    let screen = built_on(
        Tool::FibRetracement,
        &[at(1, 100.0), at(5, 110.0)],
        style,
        false,
    );
    assert!(close(screen.styled[0].path[0].x, 150.0));
    assert!(screen.styled[0].path[1].x >= PLOT.right());
}

#[test]
fn the_log_flag_spaces_levels_by_the_ratio_of_the_prices_on_a_log_chart() {
    let anchors = [at(1, 100.0), at(5, 110.0)];
    // The y of a price on the logarithmic axis 80 to 120 over 400 px.
    let y_of = |price: f64| (400.0 * (120f64 / price).ln() / (120f64 / 80f64).ln()) as f32;
    let mut style = Tool::FibRetracement.default_style();
    let mut fib = style.fib.unwrap();
    fib.log_levels = true;
    style.fib = Some(fib);
    let logged = built_on(Tool::FibRetracement, &anchors, style, true);
    // The 0.5 level is the geometric mean of 110 and 100.
    let middle = (110f64 * 100f64).sqrt();
    assert!(
        level_ys(&logged).iter().any(|y| close(*y, y_of(middle))),
        "{:?} against {}",
        level_ys(&logged),
        y_of(middle)
    );
    // Without the flag the same chart takes the plain difference: 105.
    let plain = built_on(
        Tool::FibRetracement,
        &anchors,
        Tool::FibRetracement.default_style(),
        true,
    );
    assert!(level_ys(&plain).iter().any(|y| close(*y, y_of(105.0))));
    // The flag does nothing on a linear chart.
    let mut style = Tool::FibRetracement.default_style();
    style.fib = Some(FibStyle {
        log_levels: true,
        ..style.fib.unwrap()
    });
    let linear = built_on(Tool::FibRetracement, &anchors, style, false);
    assert!(level_ys(&linear).iter().any(|y| close(*y, 150.0)));
}

#[test]
fn a_style_with_no_levels_of_its_own_draws_the_tools_defaults() {
    // A drawing made by code, or by a client that sends no levels, still has its levels.
    let anchors = [at(1, 100.0), at(5, 110.0)];
    let bare = built_on(
        Tool::FibRetracement,
        &anchors,
        DrawingStyle::default(),
        false,
    );
    let own = built(Tool::FibRetracement, &anchors);
    assert_eq!(level_ys(&bare), level_ys(&own));
    assert!(!level_ys(&bare).is_empty());
}

#[test]
fn each_level_is_labelled_with_its_ratio_and_price_in_its_own_colour() {
    let screen = built(Tool::FibRetracement, &[at(1, 100.0), at(5, 110.0)]);
    let label = screen
        .labels
        .iter()
        .find(|label| label.text.starts_with("0.618 "))
        .expect("a 0.618 label");
    // 110 − 0.618 × 10 = 103.82, written as the chart writes prices.
    assert_eq!(label.text, "0.618 (103.82)");
    // The teal of the 0.618 level, hung off the line's left end, above the line.
    assert_eq!(label.look, LabelLook::Ink([8, 153, 129, 255]));
    assert!(
        close(label.at.x, 153.0) && close(label.at.y, 161.8 - 3.0),
        "{label:?}"
    );
    assert_eq!((label.edge.x, label.edge.y), (-1.0, 1.0));
}

#[test]
fn labels_follow_the_styles_switches() {
    let anchors = [at(1, 100.0), at(5, 110.0)];
    let with = |change: fn(&mut FibStyle)| {
        let mut style = Tool::FibRetracement.default_style();
        let mut fib = style.fib.unwrap();
        change(&mut fib);
        style.fib = Some(fib);
        built_on(Tool::FibRetracement, &anchors, style, false)
    };
    let texts = |screen: &Screen| -> Vec<String> {
        screen
            .labels
            .iter()
            .map(|label| label.text.clone())
            .collect()
    };
    let percent = with(|fib| fib.as_percent = true);
    assert!(texts(&percent).contains(&"61.8% (103.82)".to_owned()));
    let ratios_only = with(|fib| fib.show_prices = false);
    assert!(texts(&ratios_only).contains(&"0.618".to_owned()));
    let prices_only = with(|fib| fib.show_labels = false);
    assert!(texts(&prices_only).contains(&"103.82".to_owned()));
    let none = with(|fib| {
        fib.show_labels = false;
        fib.show_prices = false;
    });
    assert!(none.labels.is_empty());
    // Centred, under the line.
    let placed = with(|fib| {
        fib.label_h = crate::model::drawing_style::HorizontalAlign::Center;
        fib.label_v = crate::model::drawing_style::VerticalAlign::Top;
    });
    let label = placed
        .labels
        .iter()
        .find(|label| label.text.starts_with("0.5 "))
        .unwrap();
    assert!(
        close(label.at.x, 350.0) && close(label.at.y, 150.0 + 3.0),
        "{label:?}"
    );
    assert_eq!((label.edge.x, label.edge.y), (0.0, -1.0));
}

#[test]
fn the_background_is_the_farther_levels_colour_at_the_fills_opacity() {
    let screen = built(Tool::FibRetracement, &[at(1, 100.0), at(5, 110.0)]);
    // Between 0 (y 100) and 0.236 (y 123.6): the red of 0.236, at the 20 % the library draws.
    let first = &screen.bands[0];
    assert_eq!(first.color, [242, 54, 69, 51]);
    assert!(
        first
            .polygon
            .iter()
            .all(|p| (100.0 - 0.05..=123.6 + 0.05).contains(&p.y))
    );
    // The last pair on the plot ends at the bottom edge.
    assert!(
        screen
            .bands
            .last()
            .unwrap()
            .polygon
            .iter()
            .all(|p| p.y <= 400.0)
    );
    // Disabled levels are neither lines nor band edges.
    let mut style = Tool::FibRetracement.default_style();
    let mut fib = style.fib.unwrap();
    for level in fib.levels.as_mut_slice() {
        level.enabled = level.ratio == 0.0 || level.ratio == 1.0;
    }
    style.fib = Some(fib);
    let two = built_on(
        Tool::FibRetracement,
        &[at(1, 100.0), at(5, 110.0)],
        style,
        false,
    );
    assert_eq!(level_ys(&two).len(), 2);
    assert_eq!(two.bands.len(), 1);
}

#[test]
fn one_colour_overrides_every_levels() {
    let mut style = Tool::FibRetracement.default_style();
    style.use_one_color = true;
    let screen = built_on(
        Tool::FibRetracement,
        &[at(1, 100.0), at(5, 110.0)],
        style,
        false,
    );
    assert!(
        screen
            .styled
            .iter()
            .all(|line| line.color == style.stroke.color)
    );
    assert!(
        screen
            .labels
            .iter()
            .all(|l| l.look == LabelLook::Ink(style.stroke.color))
    );
}

#[test]
fn a_level_can_be_picked_on_its_line_and_the_trend_line_hides() {
    let screen = built(Tool::FibRetracement, &[at(1, 100.0), at(5, 110.0)]);
    assert!(screen.hits_body(pos2(350.0, 150.0), 6.0, false, false));
    assert!(
        !screen.hits_body(pos2(350.0, 144.1), 3.0, false, false),
        "between lines"
    );
    assert_eq!(screen.lines.len(), 1, "the trend line");
    let mut style = Tool::FibRetracement.default_style();
    let mut fib = style.fib.unwrap();
    fib.trend_line = false;
    style.fib = Some(fib);
    let hidden = built_on(
        Tool::FibRetracement,
        &[at(1, 100.0), at(5, 110.0)],
        style,
        false,
    );
    assert!(hidden.lines.is_empty());
}

#[test]
fn a_retracement_off_the_plot_builds_nothing_visible_and_stays_finite() {
    // Both ends far above the plot: no line is on it, so none is built.
    let drawing = Drawing::build(
        Tool::FibRetracement,
        1,
        &[at(1, 1.0e9), at(5, 1.1e9)],
        Tool::FibRetracement.default_style(),
    )
    .unwrap();
    let data = bars(&[100.0; 10]);
    let scale = scale(false);
    let screen = shape::build(&drawing, &projection(&data, &scale));
    if let Some(screen) = screen {
        assert!(screen.styled.is_empty());
        assert!(screen.labels.is_empty());
    }
}

// MARK: Extension

/// The x of every vertical level line, left to right.
fn level_xs(screen: &Screen) -> Vec<f32> {
    let mut xs: Vec<f32> = screen
        .styled
        .iter()
        .filter(|line| line.path.len() == 2 && line.path[0].x == line.path[1].x)
        .map(|line| line.path[0].x)
        .collect();
    xs.sort_by(f32::total_cmp);
    xs
}

#[test]
fn an_extension_projects_the_first_move_from_the_third_anchor() {
    // 100 (bar 1) up to 110 (bar 3), back to 104 (bar 5): level r is at 104 + 10 r.
    let anchors = [at(1, 100.0), at(3, 110.0), at(5, 104.0)];
    let screen = built(Tool::FibExtension, &anchors);
    let ys = level_ys(&screen);
    // 0 at 104 (y 160), 0.236 at 106.36 (y 136.4), 0.5 at 109 (y 110), 1 at 114 (y 60), and
    // 1.618 at 120.18, a hair above the plot's top edge (y -1.8) where it is still built.
    for want in [160.0, 136.4, 110.0, 60.0, -1.8] {
        assert!(ys.iter().any(|y| close(*y, want)), "{want} in {ys:?}");
    }
    // The lines span the three anchors: bars 1 to 5 are x = 150 to 550.
    for line in &screen.styled {
        assert!(close(line.path[0].x, 150.0) && close(line.path[1].x, 550.0));
    }
    // The trend line runs through all three.
    assert_eq!(screen.lines.len(), 1);
    assert!(near(screen.lines[0][1], pos2(350.0, 100.0)));
    assert!(near(screen.lines[0][2], pos2(550.0, 160.0)));
    assert_eq!(screen.handles.len(), 3);
}

#[test]
fn a_reversed_extension_projects_the_move_the_other_way() {
    let anchors = [at(1, 100.0), at(3, 110.0), at(5, 104.0)];
    let mut style = Tool::FibExtension.default_style();
    let mut fib = style.fib.unwrap();
    fib.reverse = true;
    style.fib = Some(fib);
    let screen = built_on(Tool::FibExtension, &anchors, style, false);
    // 104 - 10 x 0.5 = 99 (y 210); 104 - 10 x 1 = 94 (y 260).
    let ys = level_ys(&screen);
    assert!(ys.iter().any(|y| close(*y, 210.0)), "{ys:?}");
    assert!(ys.iter().any(|y| close(*y, 260.0)));
}

#[test]
fn a_falling_first_move_extends_downward() {
    let screen = built(
        Tool::FibExtension,
        &[at(1, 110.0), at(3, 100.0), at(5, 106.0)],
    );
    // From 106, level 1 is another 10 down: 96 (y 240).
    assert!(level_ys(&screen).iter().any(|y| close(*y, 240.0)));
}

#[test]
fn the_extension_log_flag_repeats_the_first_moves_percentage() {
    let anchors = [at(1, 100.0), at(3, 110.0), at(5, 104.0)];
    let y_of = |price: f64| (400.0 * (120f64 / price).ln() / (120f64 / 80f64).ln()) as f32;
    let mut style = Tool::FibExtension.default_style();
    let mut fib = style.fib.unwrap();
    fib.log_levels = true;
    style.fib = Some(fib);
    let screen = built_on(Tool::FibExtension, &anchors, style, true);
    // A 10 % rise from 104 is 114.4.
    assert!(level_ys(&screen).iter().any(|y| close(*y, y_of(114.4))));
}

// MARK: Channel

fn slope(line: &super::shape::Styled) -> f32 {
    (line.path[1].y - line.path[0].y) / (line.path[1].x - line.path[0].x)
}

fn height_at(line: &super::shape::Styled, x: f32) -> f32 {
    line.path[0].y + slope(line) * (x - line.path[0].x)
}

#[test]
fn a_fib_channels_levels_are_parallel_and_spaced_by_the_third_anchors_offset() {
    // Base line 100 (bar 1) to 110 (bar 5); the third anchor at bar 3 is at 108, 3 above the
    // base line there (105): an offset of 30 px upward.
    let anchors = [at(1, 100.0), at(5, 110.0), at(3, 108.0)];
    let screen = built(Tool::FibChannel, &anchors);
    // The base line rises 100 px over 400: -0.25 px per px, and so does every level.
    for line in &screen.styled {
        assert!((slope(line) + 0.25).abs() < 1e-4, "{:?}", line.path);
    }
    // Each line is found by its label, which sits on it (the label is middle aligned).
    let find = |ratio: f64| {
        let text = format!("{} ", crate::model::fib::level_label(ratio, false));
        let label = screen
            .labels
            .iter()
            .find(|label| label.text.starts_with(&text))
            .unwrap_or_else(|| panic!("a label for {ratio}"));
        screen
            .styled
            .iter()
            .find(|line| (height_at(line, label.at.x) - label.at.y).abs() < 0.1)
            .unwrap_or_else(|| panic!("a line for {ratio}"))
    };
    // Level 0 is the base line (y 150 at x 350); level 1 passes through the third anchor
    // (x 350, y 120); level 0.5 is halfway between them, 15 px above the base line.
    assert!(close(height_at(find(0.0), 350.0), 150.0));
    assert!(close(height_at(find(1.0), 350.0), 120.0));
    assert!(close(height_at(find(0.5), 350.0), 135.0));
    // 2.618 is as far beyond the third anchor as 1.618 is from it: 150 - 2.618 x 30.
    assert!(close(height_at(find(2.618), 350.0), 150.0 - 2.618 * 30.0));
    assert!(near(screen.handles[2].pos, pos2(350.0, 120.0)));
}

#[test]
fn a_channel_extends_to_the_plot_whichever_way_it_was_drawn() {
    let mut style = Tool::FibChannel.default_style();
    style.extend_right = true;
    // Drawn right to left, so "right" is still the right of the screen.
    let backwards = built_on(
        Tool::FibChannel,
        &[at(5, 102.0), at(1, 100.0), at(3, 103.0)],
        style,
        false,
    );
    let forwards = built_on(
        Tool::FibChannel,
        &[at(1, 100.0), at(5, 102.0), at(3, 103.0)],
        style,
        false,
    );
    for screen in [&backwards, &forwards] {
        let xs: Vec<f32> = screen
            .styled
            .iter()
            .flat_map(|line| line.path.iter())
            .map(|p| p.x)
            .collect();
        let right = xs.iter().copied().fold(0.0, f32::max);
        let left = xs.iter().copied().fold(f32::MAX, f32::min);
        assert!(close(right, 1000.0), "{right}");
        assert!(close(left, 150.0), "{left}");
    }
}

#[test]
fn a_channel_on_a_vertical_base_line_has_no_width_to_measure() {
    let screen = built(
        Tool::FibChannel,
        &[at(1, 100.0), at(1, 110.0), at(3, 108.0)],
    );
    assert!(screen.styled.is_empty());
    assert_eq!(screen.handles.len(), 3, "still reshapeable");
}

#[test]
fn a_channels_labels_carry_the_price_where_they_sit() {
    let screen = built(
        Tool::FibChannel,
        &[at(1, 100.0), at(5, 110.0), at(3, 108.0)],
    );
    // Level 0 starts at (150, 200) = 100 and its label hangs off the left end at x = 153, where
    // the line is 0.75 px higher: 100.075, written as the chart writes prices.
    let label = screen
        .labels
        .iter()
        .find(|label| label.text.starts_with("0 "))
        .unwrap();
    assert_eq!(label.text, "0 (100.08)");
    assert_eq!((label.edge.x, label.edge.y), (-1.0, 0.0), "middle aligned");
}

// MARK: Time zone and trend-based time

#[test]
fn a_time_zone_puts_lines_at_fibonacci_multiples_of_the_base_interval_in_bars() {
    // Bars 1 and 2 are x = 150 and 250: an interval of one bar, so the lines are at bars
    // 1, 2, 3, 4, 6, 9, 14 ...: x = 150, 250, 350, 450, 650, 950, and the rest are past the plot.
    let screen = built(Tool::FibTimeZone, &[at(1, 100.0), at(2, 100.0)]);
    let xs = level_xs(&screen);
    let expected = [150.0, 250.0, 350.0, 450.0, 650.0, 950.0];
    assert_eq!(xs.len(), expected.len(), "{xs:?}");
    for (x, want) in xs.iter().zip(expected) {
        assert!(close(*x, want), "{x} against {want}");
    }
    // Each line crosses the plot.
    for line in &screen.styled {
        assert_eq!((line.path[0].y, line.path[1].y), (0.0, 400.0));
    }
}

#[test]
fn a_time_zone_counts_bars_not_calendar_time() {
    // A weekend between the fourth bar and the fifth: bar 1 to the bar after the gap is three
    // bars, however many hours that is.
    let times = [0, 1, 2, 3, 1000, 1001, 1002, 1003, 1004, 1005];
    let data: Vec<crate::model::Kline> = bars(&[100.0; 10])
        .into_iter()
        .zip(times)
        .map(|(mut bar, minute)| {
            bar.open_time = minute * 60_000;
            bar.close_time = minute * 60_000 + 59_999;
            bar
        })
        .collect();
    let scale = scale(false);
    let drawing = Drawing::build(
        Tool::FibTimeZone,
        1,
        &[
            Anchor {
                time_ms: 60_000,
                price: 100.0,
            },
            Anchor {
                time_ms: 1000 * 60_000,
                price: 100.0,
            },
        ],
        Tool::FibTimeZone.default_style(),
    )
    .unwrap();
    let screen = shape::build(&drawing, &projection(&data, &scale)).unwrap();
    // Interval 3 bars = 300 px: lines at x = 150, 450, 750; the fourth (1050) is past the plot.
    let xs = level_xs(&screen);
    assert_eq!(xs.len(), 3, "{xs:?}");
    for (x, want) in xs.iter().zip([150.0, 450.0, 750.0]) {
        assert!(close(*x, want), "{x} against {want}");
    }
}

#[test]
fn a_time_zone_reaches_into_the_future_and_runs_backwards_as_drawn() {
    // The last bar is bar 9 (x 950); the bar after it is x 1050, off this plot, so a zone from
    // there shows nothing of itself but its trend line.
    let forward = built(Tool::FibTimeZone, &[at(7, 100.0), at(8, 100.0)]);
    let xs = level_xs(&forward);
    for want in [750.0, 850.0, 950.0] {
        assert!(xs.iter().any(|x| close(*x, want)), "{want} in {xs:?}");
    }
    // Drawn right to left the zones run leftward: bars 5, 4, 3, 2, then 0 and below.
    let backward = built(Tool::FibTimeZone, &[at(5, 100.0), at(4, 100.0)]);
    let xs = level_xs(&backward);
    for want in [550.0, 450.0, 350.0, 250.0, 50.0] {
        assert!(xs.iter().any(|x| close(*x, want)), "{want} in {xs:?}");
    }
}

#[test]
fn a_time_zone_with_no_interval_draws_no_lines_and_keeps_its_handles() {
    let screen = built(Tool::FibTimeZone, &[at(3, 100.0), at(3, 110.0)]);
    assert!(screen.styled.is_empty());
    assert_eq!(screen.handles.len(), 2);
}

#[test]
fn a_time_zones_lines_are_labelled_with_their_multiple_beside_them() {
    let screen = built(Tool::FibTimeZone, &[at(1, 100.0), at(2, 100.0)]);
    let texts: Vec<&str> = screen.labels.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(texts, ["0", "1", "2", "3", "5", "8"]);
    // To the right of each line, at the bottom of the plot.
    let first = &screen.labels[0];
    assert!(
        close(first.at.x, 153.0) && close(first.at.y, 397.0),
        "{first:?}"
    );
    assert_eq!((first.edge.x, first.edge.y), (-1.0, 1.0));
}

#[test]
fn a_time_zones_background_is_off_until_asked_for() {
    let anchors = [at(1, 100.0), at(2, 100.0)];
    let plain = built(Tool::FibTimeZone, &anchors);
    assert!(plain.band_meshes.is_empty());
    let mut style = Tool::FibTimeZone.default_style();
    style.fill.enabled = true;
    let filled = built_on(Tool::FibTimeZone, &anchors, style, false);
    assert!(!filled.band_meshes.is_empty());
}

#[test]
fn a_trend_based_time_projects_the_first_moves_length_in_bars_from_the_third_anchor() {
    // Bars 1 to 2 are one bar; the third anchor is bar 4 (x 450): level r is at x = 450 + 100 r.
    let screen = built(
        Tool::FibTrendTime,
        &[at(1, 100.0), at(2, 105.0), at(4, 102.0)],
    );
    let xs = level_xs(&screen);
    let expected = [
        450.0, 488.2, 511.8, 550.0, 588.2, 611.8, 650.0, 688.2, 711.8, 750.0,
    ];
    assert_eq!(xs.len(), expected.len(), "the 0.5 is off: {xs:?}");
    for (x, want) in xs.iter().zip(expected) {
        assert!(close(*x, want), "{x} against {want}");
    }
    // The trend lines join the three anchors.
    assert_eq!(screen.lines[0].len(), 3);
    // A third anchor elsewhere moves the whole projection: it is not a time zone.
    let moved = built(
        Tool::FibTrendTime,
        &[at(1, 100.0), at(2, 105.0), at(6, 102.0)],
    );
    assert!(level_xs(&moved).iter().any(|x| close(*x, 650.0)));
    // The first move's length sets the spacing: three bars apart triples it.
    let longer = built(
        Tool::FibTrendTime,
        &[at(1, 100.0), at(4, 105.0), at(4, 102.0)],
    );
    assert!(
        level_xs(&longer)
            .iter()
            .any(|x| close(*x, 450.0 + 300.0 * 0.618))
    );
}

#[test]
fn every_fib_tool_without_a_style_of_its_own_draws_its_default_levels() {
    let anchors = [at(1, 100.0), at(5, 110.0), at(3, 104.0)];
    for tool in Tool::fib_tools() {
        let Placement::Points(count) = tool.spec().placement else {
            panic!("{tool:?}");
        };
        let anchors = &anchors[..usize::from(count)];
        let bare = built_on(tool, anchors, DrawingStyle::default(), false);
        let own = built(tool, anchors);
        assert!(!bare.styled.is_empty(), "{tool:?} draws levels");
        assert_eq!(bare.styled.len(), own.styled.len(), "{tool:?}");
        assert_eq!(bare.labels.len(), own.labels.len(), "{tool:?}");
    }
}
