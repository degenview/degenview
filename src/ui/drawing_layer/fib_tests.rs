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
