//! The chart patterns, Elliott waves and cycles projected onto the same hand-checkable plot as the
//! lines: 100 px per bar (bar `i` is centred on x = 50 + 100 i), prices 80 to 120 down a 400 px
//! plot (y = (120 - price) x 10). Every expected number below is worked by hand from that.

use super::line_tests::{PLOT, at, bars, near, projection, scale};
use super::shape::{LabelLook, Screen};
use super::*;
use crate::model::DrawingStyle;
use crate::model::elliott::Degree;
use eframe::egui::{pos2, vec2};

fn built_on(tool: Tool, anchors: &[Anchor], style: DrawingStyle, logarithmic: bool) -> Screen {
    let drawing = Drawing::build(tool, 1, anchors, style).expect("built");
    let scale = scale(logarithmic);
    let data = bars(&[100.0; 12]);
    shape::build(&drawing, &projection(&data, &scale)).expect("projected")
}

fn built(tool: Tool, anchors: &[Anchor]) -> Screen {
    built_on(tool, anchors, tool.default_style(), false)
}

fn texts(screen: &Screen) -> Vec<&str> {
    screen
        .labels
        .iter()
        .map(|label| label.text.as_str())
        .collect()
}

/// A Gartley's five pivots on bars 0 to 4: XA = 20, B at 0.618 of it, C half of AB back, and D at
/// the 0.786 retracement of XA.
fn gartley() -> Vec<Anchor> {
    vec![
        at(0, 90.0),
        at(1, 110.0),
        at(2, 97.64),
        at(3, 103.82),
        at(4, 94.28),
    ]
}

// MARK: Chart patterns

#[test]
fn xabcd_draws_its_legs_its_fills_and_its_four_ratios() {
    let screen = built(Tool::XabcdPattern, &gartley());
    // The zigzag through X, A, B, C, D.
    assert_eq!(screen.lines.len(), 1);
    let zigzag = &screen.lines[0];
    let expected = [
        pos2(50.0, 300.0),
        pos2(150.0, 100.0),
        pos2(250.0, 223.6),
        pos2(350.0, 161.8),
        pos2(450.0, 257.2),
    ];
    assert_eq!(zigzag.len(), 5);
    for (got, want) in zigzag.iter().zip(expected) {
        assert!(near(*got, want), "{got:?} vs {want:?}");
    }
    // The two triangles XAB and BCD are filled.
    assert_eq!(screen.bands.len(), 2);
    // Four dashed lines to measure along: X-B, A-C, B-D and X-D.
    assert_eq!(screen.styled.len(), 4);
    assert!(screen.styled.iter().all(
        |line| line.path.len() == 2 && line.dash == crate::model::chart_type::LineDash::Dashed
    ));
    // The ratios first (AB/XA, BC/AB, CD/BC, AD/XA), then the pivots' names.
    assert_eq!(
        texts(&screen),
        ["0.618", "0.5", "1.544", "0.786", "X", "A", "B", "C", "D"]
    );
    assert_eq!(screen.handles.len(), 5);
}

#[test]
fn a_ratio_sits_on_the_middle_of_the_line_it_measures() {
    let screen = built(Tool::XabcdPattern, &gartley());
    // AB/XA is printed on X-B: from (50, 300) to (250, 223.6).
    assert!(near(screen.labels[0].at, pos2(150.0, 261.8)));
    // BC/AB on A-C: from (150, 100) to (350, 161.8).
    assert!(near(screen.labels[1].at, pos2(250.0, 130.9)));
    // CD/BC on B-D: from (250, 223.6) to (450, 257.2).
    assert!(near(screen.labels[2].at, pos2(350.0, 240.4)));
    // AD/XA on X-D: from (50, 300) to (450, 257.2).
    assert!(near(screen.labels[3].at, pos2(250.0, 278.6)));
    // No two ratios share a spot.
    for (i, a) in screen.labels[..4].iter().enumerate() {
        for b in &screen.labels[i + 1..4] {
            assert!(a.at.distance(b.at) > 20.0);
        }
    }
}

#[test]
fn a_pivots_name_goes_above_a_high_and_below_a_low() {
    let screen = built(Tool::XabcdPattern, &gartley());
    let pivots = &screen.labels[4..];
    assert!(
        screen.labels[..4]
            .iter()
            .all(|label| label.look == LabelLook::Ratio),
        "a ratio is not dressed as the name of a pivot"
    );
    let pushes: Vec<f32> = pivots.iter().map(|label| label.push.y).collect();
    // X low, A high, B low, C high, D low.
    assert_eq!(pushes, [1.0, -1.0, 1.0, -1.0, 1.0]);
    assert!(pivots.iter().all(|label| label.look == LabelLook::Pill));
    // The name sits at its pivot.
    assert!(near(pivots[1].at, pos2(150.0, 100.0)));
}

#[test]
fn the_ratios_are_the_same_on_a_logarithmic_scale() {
    let linear = built(Tool::XabcdPattern, &gartley());
    let logarithmic = built_on(
        Tool::XabcdPattern,
        &gartley(),
        Tool::XabcdPattern.default_style(),
        true,
    );
    // The pivots move on the screen (log spaces the prices differently) but never the numbers.
    assert_eq!(texts(&linear), texts(&logarithmic));
    // X sits at 90: y = 300 on the linear scale, and higher on the logarithmic one (where
    // 90 is 29 % of the way up from 80 to 120 in logs, not 25 %).
    assert!(near(linear.labels[4].at, pos2(50.0, 300.0)));
    assert!(logarithmic.labels[4].at.y < 290.0);
}

#[test]
fn the_cypher_prints_its_own_three_ratios() {
    // X=90 A=110 (XA = 20); B halfway back at 100; C at 1.272 of XA from X, 115.44 (XC =
    // 25.44); D at 0.786 of XC back from C.
    let d = 115.44 - 0.786 * (115.44 - 90.0);
    let anchors = [
        at(0, 90.0),
        at(1, 110.0),
        at(2, 100.0),
        at(3, 115.44),
        at(4, d),
    ];
    let screen = built(Tool::CypherPattern, &anchors);
    assert_eq!(
        texts(&screen),
        ["0.5", "1.272", "0.786", "X", "A", "B", "C", "D"]
    );
    // Measured along X-B, X-C and X-D.
    assert_eq!(screen.styled.len(), 3);
    for (line, end) in screen.styled.iter().zip([2, 3, 4]) {
        assert!(near(line.path[0], pos2(50.0, 300.0)));
        assert!((line.path[1].x - (50.0 + 100.0 * end as f32)).abs() < 0.05);
    }
}

#[test]
fn abcd_has_two_ratios_and_no_fill() {
    let screen = built(
        Tool::AbcdPattern,
        &[at(0, 90.0), at(1, 110.0), at(2, 97.64), at(3, 117.64)],
    );
    assert_eq!(texts(&screen), ["0.618", "1.618", "A", "B", "C", "D"]);
    assert!(screen.bands.is_empty());
    assert_eq!(screen.handles.len(), 4);
}

#[test]
fn head_and_shoulders_names_its_three_peaks_and_runs_a_neckline_under_them() {
    // Start 100, left shoulder 108, trough 103, head 114, trough 105, right shoulder 110, end 104.
    let anchors = [
        at(0, 100.0),
        at(1, 108.0),
        at(2, 103.0),
        at(3, 114.0),
        at(4, 105.0),
        at(5, 110.0),
        at(6, 104.0),
    ];
    let screen = built(Tool::HeadAndShoulders, &anchors);
    assert_eq!(texts(&screen), ["Left Shoulder", "Head", "Right Shoulder"]);
    // The neckline climbs one price a bar through the troughs (103 on bar 2, 105 on bar 4): it
    // is 101 at the start and 107 at the end, run from the first pivot's time to the last's.
    assert_eq!(screen.lines.len(), 2);
    let neckline = &screen.lines[1];
    assert!(near(neckline[0], pos2(50.0, 190.0)), "{neckline:?}");
    assert!(near(neckline[1], pos2(650.0, 130.0)), "{neckline:?}");
    // A triangle for each shoulder and the head.
    assert_eq!(screen.bands.len(), 3);
}

#[test]
fn an_inverse_head_and_shoulders_is_the_same_tool_upside_down() {
    let upright = [100.0, 108.0, 103.0, 114.0, 105.0, 110.0, 104.0];
    let anchors: Vec<Anchor> = upright
        .iter()
        .enumerate()
        .map(|(bar, price)| at(bar as i64, 220.0 - price))
        .collect();
    let screen = built(Tool::HeadAndShoulders, &anchors);
    assert_eq!(texts(&screen), ["Left Shoulder", "Head", "Right Shoulder"]);
    // The names now sit below their pivots, which are lows.
    assert!(screen.labels.iter().all(|label| label.push.y > 0.0));
}

#[test]
fn a_triangle_runs_its_two_sides_to_the_apex() {
    // A=110 B=90 C=105 D=95 on bars 0, 1, 4, 5: the sides A-C and B-D fall and rise by 0.125 px
    // a pixel and meet at (900, 206.25).
    let anchors = [at(0, 110.0), at(1, 90.0), at(4, 105.0), at(5, 95.0)];
    let screen = built(Tool::TrianglePattern, &anchors);
    assert_eq!(screen.lines.len(), 3);
    assert_eq!(screen.lines[0].len(), 4, "the zigzag A-B-C-D");
    let (ac, bd) = (&screen.lines[1], &screen.lines[2]);
    assert!(
        near(ac[0], pos2(50.0, 100.0)) && near(ac[1], pos2(900.0, 206.25)),
        "{ac:?}"
    );
    assert!(
        near(bd[0], pos2(150.0, 300.0)) && near(bd[1], pos2(900.0, 206.25)),
        "{bd:?}"
    );
    assert_eq!(texts(&screen), ["A", "B", "C", "D"]);
    assert_eq!(screen.bands.len(), 1);
}

#[test]
fn an_expanding_triangle_stops_at_its_last_pivot() {
    // The sides fan out, so there is no apex to run to.
    let anchors = [at(0, 105.0), at(1, 95.0), at(3, 110.0), at(4, 90.0)];
    let screen = built(Tool::TrianglePattern, &anchors);
    let (ac, bd) = (&screen.lines[1], &screen.lines[2]);
    assert!((ac[1].x - 450.0).abs() < 0.05 && (bd[1].x - 450.0).abs() < 0.05);
}

#[test]
fn three_drives_print_each_retracement_and_extension() {
    // start 90, drive 1 to 110, A back to 100 (0.5), drive 2 to 115 (1.5 of A), C back to 107.5
    // (0.5), drive 3 to 120 (1.667 of C).
    let anchors = [
        at(0, 90.0),
        at(1, 110.0),
        at(2, 100.0),
        at(3, 115.0),
        at(4, 107.5),
        at(5, 120.0),
    ];
    let screen = built(Tool::ThreeDrivesPattern, &anchors);
    assert_eq!(
        texts(&screen),
        ["0.5", "1.5", "0.5", "1.667", "1", "A", "2", "C", "3"]
    );
    assert_eq!(screen.styled.len(), 4);
    assert_eq!(screen.bands.len(), 1);
}

#[test]
fn the_labels_and_the_ratios_can_each_be_turned_off() {
    let mut style = Tool::XabcdPattern.default_style();
    style.pattern.ratios = false;
    let screen = built_on(Tool::XabcdPattern, &gartley(), style, false);
    assert_eq!(texts(&screen), ["X", "A", "B", "C", "D"]);
    assert!(
        screen.styled.is_empty(),
        "no measuring lines without ratios"
    );
    style.pattern.ratios = true;
    style.pattern.labels = false;
    let screen = built_on(Tool::XabcdPattern, &gartley(), style, false);
    assert_eq!(texts(&screen), ["0.618", "0.5", "1.544", "0.786"]);
    style.pattern.precision = 1;
    let screen = built_on(Tool::XabcdPattern, &gartley(), style, false);
    assert_eq!(texts(&screen), ["0.6", "0.5", "1.5", "0.8"]);
}

#[test]
fn a_degenerate_leg_prints_no_ratio_instead_of_nan() {
    // XA has no length: the two ratios measured against it are left out.
    let anchors = [
        at(0, 100.0),
        at(1, 100.0),
        at(2, 90.0),
        at(3, 95.0),
        at(4, 92.0),
    ];
    let screen = built(Tool::XabcdPattern, &anchors);
    assert_eq!(texts(&screen), ["0.5", "0.6", "X", "A", "B", "C", "D"]);
    assert!(
        screen
            .labels
            .iter()
            .all(|label| !label.text.contains("NaN") && !label.text.contains("inf"))
    );
}

#[test]
fn a_pattern_is_not_drawn_with_the_wrong_number_of_pivots() {
    assert!(
        Drawing::build(
            Tool::XabcdPattern,
            1,
            &gartley()[..4],
            Tool::XabcdPattern.default_style()
        )
        .is_none()
    );
    let mut six = gartley();
    six.push(at(5, 100.0));
    assert!(
        Drawing::build(
            Tool::XabcdPattern,
            1,
            &six,
            Tool::XabcdPattern.default_style()
        )
        .is_none()
    );
    // Nor with every pivot on one point.
    assert!(
        Drawing::build(
            Tool::XabcdPattern,
            1,
            &[at(0, 100.0); 5],
            Tool::XabcdPattern.default_style()
        )
        .is_none()
    );
}

// MARK: Elliott waves

fn wave_prices(count: usize) -> Vec<Anchor> {
    [90.0, 100.0, 95.0, 110.0, 102.0, 115.0]
        .into_iter()
        .take(count)
        .enumerate()
        .map(|(bar, price)| at(bar as i64, price))
        .collect()
}

#[test]
fn an_impulse_labels_its_five_waves_in_its_degree() {
    let screen = built(Tool::ElliottImpulse, &wave_prices(6));
    // The default degree is Intermediate.
    assert_eq!(texts(&screen), ["(1)", "(2)", "(3)", "(4)", "(5)"]);
    assert!(
        screen
            .labels
            .iter()
            .all(|label| label.look == LabelLook::Plain)
    );
    assert_eq!(screen.lines.len(), 1);
    assert_eq!(screen.lines[0].len(), 6);
    assert_eq!(screen.handles.len(), 6);
    // Wave 1 ends on a high: its label is above, wave 2's below.
    assert!(screen.labels[0].push.y < 0.0 && screen.labels[1].push.y > 0.0);
    // And each label sits on its pivot, not on the wave's start.
    assert!(near(screen.labels[0].at, pos2(150.0, 200.0)));
    assert!(near(screen.labels[4].at, pos2(550.0, 50.0)));
}

#[test]
fn changing_the_degree_rewrites_every_label() {
    for (degree, want) in [
        (Degree::Minor, ["1", "2", "3", "4", "5"]),
        (
            Degree::Primary,
            ["((1))", "((2))", "((3))", "((4))", "((5))"],
        ),
        (Degree::Cycle, ["I", "II", "III", "IV", "V"]),
        (
            Degree::Minute,
            ["((i))", "((ii))", "((iii))", "((iv))", "((v))"],
        ),
    ] {
        let mut style = Tool::ElliottImpulse.default_style();
        style.pattern.degree = degree;
        let screen = built_on(Tool::ElliottImpulse, &wave_prices(6), style, false);
        assert_eq!(texts(&screen), want, "{degree:?}");
    }
}

#[test]
fn each_wave_has_its_own_sequence_of_labels() {
    let cases = [
        (Tool::ElliottCorrection, 4, vec!["A", "B", "C"]),
        (Tool::ElliottTriangle, 6, vec!["A", "B", "C", "D", "E"]),
        (Tool::ElliottDoubleCombo, 4, vec!["W", "X", "Y"]),
        (Tool::ElliottTripleCombo, 6, vec!["W", "X", "Y", "X", "Z"]),
    ];
    for (tool, count, want) in cases {
        let mut style = tool.default_style();
        style.pattern.degree = Degree::Minor;
        let screen = built_on(tool, &wave_prices(count), style, false);
        assert_eq!(texts(&screen), want, "{tool:?}");
        assert_eq!(screen.handles.len(), count, "{tool:?}");
    }
}

#[test]
fn a_wave_with_its_labels_off_draws_only_its_line() {
    let mut style = Tool::ElliottImpulse.default_style();
    style.pattern.labels = false;
    let screen = built_on(Tool::ElliottImpulse, &wave_prices(6), style, false);
    assert!(screen.labels.is_empty());
    assert_eq!(screen.lines.len(), 1);
}

#[test]
fn an_impulse_that_breaks_the_textbook_rules_is_drawn_as_placed() {
    // Wave 2 retraces past the start of wave 1 and wave 4 overlaps wave 1: a manual drawing
    // does not reject or move them.
    let anchors = [
        at(0, 100.0),
        at(1, 105.0),
        at(2, 95.0),
        at(3, 110.0),
        at(4, 103.0),
        at(5, 108.0),
    ];
    let screen = built(Tool::ElliottImpulse, &anchors);
    assert!(
        near(screen.lines[0][2], pos2(250.0, 250.0)),
        "wave 2 stays where it was put"
    );
    assert!(near(screen.lines[0][4], pos2(450.0, 170.0)));
}

// MARK: Cycles

fn vertical_xs(screen: &Screen) -> Vec<f32> {
    screen
        .lines
        .iter()
        .map(|line| {
            assert_eq!(line.len(), 2);
            assert!((line[0].x - line[1].x).abs() < 1e-3, "vertical");
            assert_eq!((line[0].y, line[1].y), (PLOT.top(), PLOT.bottom()));
            line[0].x
        })
        .collect()
}

#[test]
fn cyclic_lines_repeat_at_the_distance_between_the_anchors() {
    // Bars 1 and 3: a period of two bars, 200 px.
    let screen = built(Tool::CyclicLines, &[at(1, 100.0), at(3, 100.0)]);
    assert_eq!(vertical_xs(&screen), [150.0, 350.0, 550.0, 750.0, 950.0]);
    // The Charting Library's dashed grey line joins the two anchors.
    assert_eq!(screen.styled.len(), 1);
    assert_eq!(screen.styled[0].path.len(), 2);
    assert_eq!(screen.handles.len(), 2);
}

#[test]
fn cyclic_lines_drawn_right_to_left_repeat_to_the_left() {
    let screen = built(Tool::CyclicLines, &[at(3, 100.0), at(1, 100.0)]);
    assert_eq!(vertical_xs(&screen), [350.0, 150.0]);
}

#[test]
fn dragging_an_anchor_changes_every_repeat() {
    let wide = built(Tool::CyclicLines, &[at(1, 100.0), at(4, 100.0)]);
    assert_eq!(vertical_xs(&wide), [150.0, 450.0, 750.0]);
}

#[test]
fn cyclic_lines_too_close_to_tell_apart_are_not_drawn() {
    // A second of a minute-long bar: under two pixels apart.
    let close = Anchor {
        time_ms: 61_000,
        price: 100.0,
    };
    let screen = built(Tool::CyclicLines, &[at(1, 100.0), close]);
    assert!(screen.lines.is_empty());
    assert_eq!(screen.handles.len(), 2, "the anchors can still be grabbed");
}

#[test]
fn cycles_need_two_different_times() {
    for tool in [Tool::CyclicLines, Tool::TimeCycles, Tool::SineLine] {
        assert!(
            Drawing::build(tool, 1, &[at(2, 100.0), at(2, 105.0)], tool.default_style()).is_none(),
            "{tool:?}"
        );
    }
}

#[test]
fn time_cycles_are_half_circles_over_each_stretch_of_the_period() {
    // Bars 1 and 3 at 100 and 105: arcs 200 px wide on the baseline at the first price (y = 200),
    // bulging up because the second anchor is higher.
    let screen = built(Tool::TimeCycles, &[at(1, 100.0), at(3, 105.0)]);
    assert_eq!(screen.lines.len(), 5, "starts at bars 1, 3, 5, 7 and 9");
    let first = &screen.lines[0];
    assert!(near(first[0], pos2(150.0, 200.0)));
    assert!(near(*first.last().unwrap(), pos2(350.0, 200.0)));
    let crown = first.iter().map(|p| p.y).fold(f32::MAX, f32::min);
    // A radius above the baseline, give or take the half-pixel the sampling is allowed.
    assert!((crown - 100.0).abs() < 0.25, "{crown}");
    for point in first {
        assert!((point.distance(pos2(250.0, 200.0)) - 100.0).abs() < 0.05);
    }
    // Each arc is filled.
    assert_eq!(screen.bands.len(), 5);
}

#[test]
fn time_cycles_bulge_down_when_the_second_anchor_is_lower() {
    let screen = built(Tool::TimeCycles, &[at(1, 100.0), at(3, 95.0)]);
    let trough = screen.lines[0].iter().map(|p| p.y).fold(f32::MIN, f32::max);
    assert!((trough - 300.0).abs() < 0.25, "{trough}");
}

#[test]
fn time_cycles_too_narrow_to_read_are_not_drawn() {
    let narrow = Anchor {
        time_ms: 63_000,
        price: 100.0,
    };
    let screen = built(Tool::TimeCycles, &[at(1, 100.0), narrow]);
    assert!(screen.lines.is_empty() && screen.bands.is_empty());
}

#[test]
fn a_sine_line_runs_through_both_anchors() {
    // A crest at bar 1 (150, 100) and the trough after it at bar 3 (350, 300).
    let screen = built(Tool::SineLine, &[at(1, 110.0), at(3, 90.0)]);
    assert_eq!(screen.lines.len(), 1);
    let wave = &screen.lines[0];
    assert!(wave.iter().any(|p| near(*p, pos2(150.0, 100.0))));
    assert!(wave.iter().any(|p| near(*p, pos2(350.0, 300.0))));
    // Every point is on the sine y = 200 - 100 cos(pi (x - 150) / 200).
    for point in wave {
        let truth = 200.0 - 100.0 * (std::f32::consts::PI * (point.x - 150.0) / 200.0).cos();
        assert!((point.y - truth).abs() < 0.01, "{point:?}");
    }
    // It covers the plot, and so continues after the second anchor and before the first.
    assert!(wave.first().unwrap().x <= 0.0 && wave.last().unwrap().x >= 1000.0);
}

#[test]
fn a_sine_line_with_a_period_too_short_to_see_keeps_only_its_handles() {
    let near_anchor = Anchor {
        time_ms: 60_500,
        price: 90.0,
    };
    let screen = built(Tool::SineLine, &[at(1, 110.0), near_anchor]);
    assert!(screen.lines.is_empty());
    assert_eq!(screen.handles.len(), 2);
}

// MARK: Previews and bounds

#[test]
fn a_preview_is_the_whole_shape_with_the_pointer_standing_in_for_the_pivots_to_come() {
    let first = at(0, 90.0);
    let pointer = at(2, 100.0);
    let Some(Geometry::Pattern { kind, points }) =
        Geometry::preview(Tool::XabcdPattern, &[first, pointer])
    else {
        panic!("a pattern");
    };
    assert_eq!(kind, crate::model::pattern::PatternKind::Xabcd);
    assert_eq!(points.len(), 5);
    assert_eq!(points[0], first);
    assert!(points[1..].iter().all(|point| *point == pointer));
    let Some(Geometry::Wave { points, .. }) =
        Geometry::preview(Tool::ElliottTripleCombo, &[first, pointer])
    else {
        panic!("a wave");
    };
    assert_eq!(points.len(), 6);
    let Some(Geometry::Cycle { a, b, .. }) = Geometry::preview(Tool::SineLine, &[first, pointer])
    else {
        panic!("a cycle");
    };
    assert_eq!((a, b), (first, pointer));
}

#[test]
fn the_preview_projects_through_the_same_code_as_the_finished_drawing() {
    let placed = [at(0, 90.0), at(1, 110.0), at(2, 97.64), at(3, 103.82)];
    let pointer = at(4, 94.28);
    let mut all = placed.to_vec();
    all.push(pointer);
    let preview = Geometry::preview(Tool::XabcdPattern, &all).expect("a preview");
    let finished = Geometry::from_anchors(Tool::XabcdPattern, &all).expect("a pattern");
    assert_eq!(preview, finished);
}

#[test]
fn a_pattern_wholly_off_the_plot_is_culled_by_its_bounds() {
    // Bars far to the right of a chart that ends at bar 11.
    let anchors: Vec<Anchor> = gartley()
        .into_iter()
        .map(|a| Anchor {
            time_ms: a.time_ms + 100 * 60_000,
            price: a.price,
        })
        .collect();
    let drawing = Drawing::build(Tool::XabcdPattern, 1, &anchors, DrawingStyle::default()).unwrap();
    let scale = scale(false);
    let data = bars(&[100.0; 12]);
    // A shape is still built (the layer culls by its bounds), and its bounds are far off.
    let screen = shape::build(&drawing, &projection(&data, &scale)).unwrap();
    assert!(!screen.bounds.intersects(PLOT));
    assert!(
        screen.bands.is_empty(),
        "nothing of a fill is left once it is cut to the plot"
    );
}

#[test]
fn the_bounds_reach_the_labels_beside_a_pivot() {
    let screen = built(Tool::XabcdPattern, &gartley());
    let apex = pos2(150.0, 100.0);
    // A's name is above it; the shape's bounds have room for the plate.
    assert!(screen.bounds.contains(apex - vec2(0.0, 30.0)));
}
