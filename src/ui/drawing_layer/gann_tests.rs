//! The Gann drawings projected onto a plot whose numbers are easy to check by hand: 100 px per bar
//! (bar `i` is centred on x = 50 + 100 i), and prices 80 to 120 down a 400 px plot
//! (y = (120 − price) × 10). Every expected number below is worked out by hand.

use super::edit::Part;
use super::line_tests::{at, bars, near, projection, scale};
use super::shape::{LabelLook, Screen, Styled};
use super::*;
use crate::model::DrawingStyle;
use crate::model::drawing_style::MiddleLine;
use crate::model::fib_style::FibStyle;
use eframe::egui::{pos2, vec2};

fn built_on(tool: Tool, anchors: &[Anchor], style: DrawingStyle, slot_width: f32) -> Screen {
    let drawing = Drawing::build(tool, 1, anchors, style).expect("built");
    let data = bars(&[100.0; 10]);
    let scale = scale(false);
    let mut projection = projection(&data, &scale);
    projection.slot_width = slot_width;
    shape::build(&drawing, &projection).expect("projected")
}

fn built(tool: Tool, anchors: &[Anchor]) -> Screen {
    built_on(tool, anchors, tool.default_style(), 100.0)
}

fn with_fib(tool: Tool, change: impl Fn(&mut FibStyle)) -> DrawingStyle {
    let mut style = tool.default_style();
    let mut fib = style.fib.expect("a Gann tool carries its levels");
    change(&mut fib);
    style.fib = Some(fib);
    style
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.06
}

/// The rays that start at `origin`: where each of them ends.
fn ends(screen: &Screen, origin: Pos2) -> Vec<Pos2> {
    screen
        .styled
        .iter()
        .filter(|line| line.path.len() == 2 && near(line.path[0], origin))
        .map(|line| line.path[1])
        .collect()
}

fn reaches(ends: &[Pos2], want: Pos2) -> bool {
    ends.iter().any(|end| near(*end, want))
}

// MARK: Fan

#[test]
fn a_gann_fans_one_by_one_passes_through_its_second_anchor_and_the_others_scale_its_slope() {
    // From (150, 200) to (350, 100): 200 across and 100 up. Price over time: 1x1 is that line, 2x1
    // twice as steep, 1x2 half.
    let screen = built(Tool::GannFan, &[at(1, 100.0), at(3, 110.0)]);
    let origin = pos2(150.0, 200.0);
    let rays = ends(&screen, origin);
    assert_eq!(rays.len(), 9, "{rays:?}");
    // 1x1: slope -1/2: leaves the top at x = 150 + 400.
    assert!(reaches(&rays, pos2(550.0, 0.0)), "{rays:?}");
    // 2x1: slope -1: x = 350. 3x1: slope -3/2: x = 150 + 200 x 2/3. 8x1: slope -4: x = 200.
    assert!(reaches(&rays, pos2(350.0, 0.0)));
    assert!(reaches(&rays, pos2(150.0 + 200.0 * 2.0 / 3.0, 0.0)));
    assert!(reaches(&rays, pos2(200.0, 0.0)));
    // 1x2: slope -1/4: leaves the top at x = 950. 1x8: slope -1/16: the right edge at 200 - 53.125.
    assert!(reaches(&rays, pos2(950.0, 0.0)));
    assert!(reaches(&rays, pos2(1000.0, 200.0 - 850.0 / 16.0)));
}

#[test]
fn the_angles_follow_the_charts_scales_and_never_a_fixed_45_degrees() {
    // The same two anchors on a plot zoomed to half the bar spacing: the 1x1 still goes through
    // the second anchor, wherever that now is, and so does it on a different price scale.
    let anchors = [at(1, 100.0), at(3, 110.0)];
    for slot_width in [100.0, 50.0, 30.0] {
        let screen = built_on(
            Tool::GannFan,
            &anchors,
            Tool::GannFan.default_style(),
            slot_width,
        );
        let first_x = slot_width * 1.5;
        let second_x = slot_width * 3.5;
        let origin = pos2(first_x, 200.0);
        let second = pos2(second_x, 100.0);
        // The ray whose slope matches the line to the second anchor is the 1x1: it passes through.
        let slope = (second.y - origin.y) / (second.x - origin.x);
        let one_by_one = screen
            .styled
            .iter()
            .find(|line| {
                let d = line.path[1] - line.path[0];
                d.x.abs() > 1e-3 && ((d.y / d.x) - slope).abs() < 1e-3
            })
            .unwrap_or_else(|| panic!("a ray of slope {slope} at {slot_width}"));
        let across = crate::ui::drawing_geometry::distance_to_segment(
            second,
            one_by_one.path[0],
            one_by_one.path[1],
        );
        assert!(across < 0.05, "{slot_width}: {across}");
    }
    // On a different price scale the anchors are elsewhere on the screen, and the 1x1 follows.
    let data = bars(&[100.0; 10]);
    let tall = PriceScale::linear(90.0, 120.0, 0.0, 400.0);
    let drawing = Drawing::build(
        Tool::GannFan,
        1,
        &[at(1, 100.0), at(3, 110.0)],
        Tool::GannFan.default_style(),
    )
    .unwrap();
    let projection = projection(&data, &tall);
    let screen = shape::build(&drawing, &projection).unwrap();
    let origin = projection.point(at(1, 100.0));
    let second = projection.point(at(3, 110.0));
    let through = screen.styled.iter().any(|line| {
        near(line.path[0], origin)
            && crate::ui::drawing_geometry::distance_to_segment(second, line.path[0], line.path[1])
                < 0.05
    });
    assert!(through, "the 1x1 passes through the second anchor");
}

#[test]
fn a_fan_drawn_leftward_or_downward_runs_the_way_it_was_drawn() {
    // From (550, 100) down and left to (350, 200): the rays go on that way.
    let screen = built(Tool::GannFan, &[at(5, 110.0), at(3, 100.0)]);
    let rays = ends(&screen, pos2(550.0, 100.0));
    // 1x1: 100 down per 200 left: leaves the left edge at y = 100 + 275 = 375.
    assert!(reaches(&rays, pos2(0.0, 375.0)), "{rays:?}");
}

#[test]
fn a_fans_labels_name_the_angles_at_the_ends_of_its_rays() {
    let screen = built(Tool::GannFan, &[at(1, 100.0), at(3, 110.0)]);
    let mut texts: Vec<&str> = screen.labels.iter().map(|l| l.text.as_str()).collect();
    texts.sort_unstable();
    let mut want = [
        "1x8", "1x4", "1x3", "1x2", "1x1", "2x1", "3x1", "4x1", "8x1",
    ];
    want.sort_unstable();
    assert_eq!(texts, want);
    // And every ray is in its own level's colour, the label too.
    for label in &screen.labels {
        let LabelLook::Ink(color) = label.look else {
            panic!("{label:?}");
        };
        assert!(screen.styled.iter().any(|line| line.color == color));
    }
    let off = built_on(
        Tool::GannFan,
        &[at(1, 100.0), at(3, 110.0)],
        with_fib(Tool::GannFan, |fib| fib.show_labels = false),
        100.0,
    );
    assert!(off.labels.is_empty());
    // The angles are filled between neighbours: nine rays, eight bands.
    assert_eq!(screen.bands.len(), 8);
}

#[test]
fn a_fan_with_both_anchors_at_one_time_does_not_break() {
    let screen = built(Tool::GannFan, &[at(3, 100.0), at(3, 110.0)]);
    assert_eq!(screen.handles.len(), 2);
}

// MARK: Box

#[test]
fn a_gann_box_cuts_its_price_range_and_its_time_range_at_the_levels() {
    // (150, 200) to (550, 100): price levels at y = 200 - 100 c, time levels at x = 150 + 400 c.
    let screen = built(Tool::GannBox, &[at(1, 100.0), at(5, 110.0)]);
    let ratios = [0.0, 0.25, 0.382, 0.5, 0.618, 0.75, 1.0];
    let mut ys: Vec<f32> = screen
        .styled
        .iter()
        .filter(|line| line.path[0].y == line.path[1].y)
        .map(|line| line.path[0].y)
        .collect();
    ys.sort_by(f32::total_cmp);
    let mut want_y: Vec<f32> = ratios.iter().map(|c| 200.0 - 100.0 * c).collect();
    want_y.sort_by(f32::total_cmp);
    assert_eq!(ys.len(), 7);
    for (y, want) in ys.iter().zip(&want_y) {
        assert!(close(*y, *want), "{y} against {want}");
    }
    let mut xs: Vec<f32> = screen
        .styled
        .iter()
        .filter(|line| line.path[0].x == line.path[1].x)
        .map(|line| line.path[0].x)
        .collect();
    xs.sort_by(f32::total_cmp);
    for (x, c) in xs.iter().zip(ratios) {
        assert!(close(*x, 150.0 + 400.0 * c), "{x} against {c}");
    }
    // Each line stays between the box's sides.
    for line in &screen.styled {
        let [from, to] = [line.path[0], line.path[1]];
        assert!(from.x >= 150.0 - 0.1 && to.x <= 550.0 + 0.1);
        assert!(from.y.min(to.y) >= 100.0 - 0.1 && from.y.max(to.y) <= 200.0 + 0.1);
    }
}

#[test]
fn a_gann_boxs_two_fills_are_separate_and_the_angles_start_off() {
    let anchors = [at(1, 100.0), at(5, 110.0)];
    let screen = built(Tool::GannBox, &anchors);
    // Six strips between its seven price levels, six between its seven time levels.
    assert_eq!(screen.bands.len(), 6);
    assert_eq!(screen.shades.len(), 6);
    let no_second = built_on(
        Tool::GannBox,
        &anchors,
        with_fib(Tool::GannBox, |fib| fib.second_fill = false),
        100.0,
    );
    assert_eq!(no_second.bands.len(), 6);
    assert!(no_second.shades.is_empty());
    // Angles are off until asked for, then ten rays: two diagonals and two from each corner.
    let grid = Tool::GannBox.default_style().fib.unwrap().grid;
    fn angled<'a>(screen: &'a Screen, grid: &MiddleLine) -> Vec<&'a Styled> {
        screen
            .styled
            .iter()
            .filter(|line| line.color == grid.color && line.width == grid.width)
            .collect()
    }
    assert!(angled(&screen, &grid).is_empty());
    let on = built_on(
        Tool::GannBox,
        &anchors,
        with_fib(Tool::GannBox, |fib| fib.grid.enabled = true),
        100.0,
    );
    let rays = angled(&on, &grid);
    assert_eq!(rays.len(), 10);
    // One of the diagonals runs corner to corner.
    assert!(rays.iter().any(|line| {
        (near(line.path[0], pos2(150.0, 200.0)) && near(line.path[1], pos2(550.0, 100.0)))
            || (near(line.path[1], pos2(150.0, 200.0)) && near(line.path[0], pos2(550.0, 100.0)))
    }));
}

#[test]
fn reversing_a_box_counts_its_ratios_from_the_other_corner() {
    let anchors = [at(1, 100.0), at(5, 110.0)];
    let reversed = built_on(
        Tool::GannBox,
        &anchors,
        with_fib(Tool::GannBox, |fib| fib.reverse = true),
        100.0,
    );
    // The orange 0.25 price level, which was a quarter of the way up (y 175), is now a quarter of
    // the way down from the top (y 125).
    let orange = Tool::GannBox.default_style().fib.unwrap().levels.as_slice()[1].color;
    let y = reversed
        .styled
        .iter()
        .find(|line| line.color == orange && line.path[0].y == line.path[1].y)
        .map(|line| line.path[0].y)
        .expect("the orange level");
    assert!(close(y, 125.0), "{y}");
}

#[test]
fn a_gann_boxs_labels_sit_on_the_sides_it_is_set_to_show() {
    let anchors = [at(1, 100.0), at(5, 110.0)];
    // Seven price levels on two sides and seven time levels on two sides.
    assert_eq!(built(Tool::GannBox, &anchors).labels.len(), 28);
    let some = built_on(
        Tool::GannBox,
        &anchors,
        with_fib(Tool::GannBox, |fib| {
            fib.sides.left = false;
            fib.sides.top = false;
        }),
        100.0,
    );
    assert_eq!(some.labels.len(), 14);
}

#[test]
fn a_gann_boxs_price_levels_follow_the_price_not_the_pixel_on_a_log_chart() {
    let data = bars(&[100.0; 10]);
    let scale = scale(true);
    let drawing = Drawing::build(
        Tool::GannBox,
        1,
        &[at(1, 100.0), at(5, 110.0)],
        Tool::GannBox.default_style(),
    )
    .unwrap();
    let screen = shape::build(&drawing, &projection(&data, &scale)).unwrap();
    // The 0.5 price level is 105, whose y on the log axis is not halfway between the anchors'.
    let y_of = |price: f64| (400.0 * (120f64 / price).ln() / (120f64 / 80f64).ln()) as f32;
    let halfway = (y_of(100.0) + y_of(110.0)) / 2.0;
    let level = y_of(105.0);
    assert!(
        (level - halfway).abs() > 0.1,
        "the log scale curves the middle"
    );
    assert!(
        screen
            .styled
            .iter()
            .any(|line| line.path[0].y == line.path[1].y && close(line.path[0].y, level)),
        "a level at the price"
    );
}

// MARK: Squares

#[test]
fn a_gann_square_has_a_grid_fan_lines_and_arcs_inside_its_box() {
    // (150, 200) to (550, 120): 400 across, 80 up.
    let screen = built(Tool::GannSquare, &[at(1, 100.0), at(5, 108.0)]);
    let origin = pos2(150.0, 200.0);
    let rays = ends(&screen, origin);
    // 1x1 ends at the far corner; 2x1 (160 up per 400 across) meets the top halfway across; 1x2
    // meets the right side at 40 up.
    assert!(reaches(&rays, pos2(550.0, 120.0)), "{rays:?}");
    assert!(reaches(&rays, pos2(350.0, 120.0)), "{rays:?}");
    assert!(reaches(&rays, pos2(550.0, 160.0)), "{rays:?}");
    // Grid: the level at 0.5 is a line across at y = 160 and one down at x = 350.
    assert!(screen.styled.iter().any(|line| {
        near(line.path[0], pos2(150.0, 160.0)) && near(line.path[1], pos2(550.0, 160.0))
    }));
    assert!(screen.styled.iter().any(|line| {
        near(line.path[0], pos2(350.0, 200.0)) && near(line.path[1], pos2(350.0, 120.0))
            || near(line.path[1], pos2(350.0, 200.0)) && near(line.path[0], pos2(350.0, 120.0))
    }));
    // Everything drawn is inside the box.
    for point in screen.styled.iter().flat_map(|line| line.path.iter()) {
        assert!(
            point.x >= 149.0 && point.x <= 551.0 && point.y >= 119.0 && point.y <= 201.0,
            "{point:?}"
        );
    }
    // The arcs are quarter ellipses: about the first corner (x 1) one box wide and tall.
    let arc = screen
        .styled
        .iter()
        .find(|line| {
            line.path.len() > 8
                && ((line.path[0].x - 150.0) / 400.0).powi(2)
                    + ((line.path[0].y - 200.0) / 80.0).powi(2)
                    < 1.001
                && ((line.path[0].x - 150.0) / 400.0).powi(2)
                    + ((line.path[0].y - 200.0) / 80.0).powi(2)
                    > 0.999
        })
        .expect("the 1x0 arc");
    for point in &arc.path {
        let value = ((point.x - 150.0) / 400.0).powi(2) + ((point.y - 200.0) / 80.0).powi(2);
        assert!((value - 1.0).abs() < 2e-3, "{point:?}");
    }
}

#[test]
fn a_gann_squares_labels_name_its_fan_lines() {
    let screen = built(Tool::GannSquare, &[at(1, 100.0), at(5, 108.0)]);
    let mut texts: Vec<&str> = screen.labels.iter().map(|l| l.text.as_str()).collect();
    texts.sort_unstable();
    assert_eq!(texts, ["1x1", "1x2", "2x1"]);
    // The fixed one prints none.
    let fixed = built(Tool::GannSquareFixed, &[at(1, 100.0), at(5, 108.0)]);
    assert!(fixed.labels.is_empty());
}

#[test]
fn a_set_price_per_bar_sets_how_tall_the_square_is() {
    // Four bars across at a price of 1 a bar is 4 up: the far corner is at 104 (y 160), whatever
    // the second anchor's price says.
    let screen = built_on(
        Tool::GannSquare,
        &[at(1, 100.0), at(5, 108.0)],
        with_fib(Tool::GannSquare, |fib| fib.scale = 1.0),
        100.0,
    );
    assert!(near(screen.handles[0].pos, pos2(150.0, 200.0)));
    assert!(
        near(screen.handles[1].pos, pos2(550.0, 160.0)),
        "{:?}",
        screen.handles[1].pos
    );
    // With none set, it is the second anchor.
    let free = built(Tool::GannSquare, &[at(1, 100.0), at(5, 108.0)]);
    assert!(near(free.handles[1].pos, pos2(550.0, 120.0)));
    // The fixed square has no such setting.
    assert!(
        !crate::model::fib::features_of(Tool::GannSquareFixed)
            .unwrap()
            .scale
    );
}

#[test]
fn a_gann_squares_readout_gives_its_ranges_and_ratio() {
    let screen = built_on(
        Tool::GannSquare,
        &[at(1, 100.0), at(5, 108.0)],
        with_fib(Tool::GannSquare, |fib| fib.readout = true),
        100.0,
    );
    let texts: Vec<&str> = screen.labels.iter().map(|l| l.text.as_str()).collect();
    // 4 bars across, 8 of price up, 2 a bar.
    assert!(texts.contains(&"4 bars"), "{texts:?}");
    assert!(texts.contains(&"8.00"), "{texts:?}");
    assert!(texts.contains(&"1 : 2"), "{texts:?}");
}

#[test]
fn reversing_a_square_fans_it_from_the_other_corner() {
    let screen = built_on(
        Tool::GannSquare,
        &[at(1, 100.0), at(5, 108.0)],
        with_fib(Tool::GannSquare, |fib| fib.reverse = true),
        100.0,
    );
    // The fan lines now start at the second corner (550, 120), the 1x1 ending at (150, 200).
    let rays = ends(&screen, pos2(550.0, 120.0));
    assert!(reaches(&rays, pos2(150.0, 200.0)), "{rays:?}");
}

#[test]
fn a_degenerate_square_keeps_its_handles_and_draws_nothing_inside() {
    for tool in [Tool::GannSquare, Tool::GannSquareFixed, Tool::GannBox] {
        let screen = built(tool, &[at(3, 100.0), at(3, 110.0)]);
        assert_eq!(screen.handles.len(), 2, "{tool:?}");
    }
}

// MARK: Placement and editing

#[test]
fn a_fixed_gann_square_is_placed_and_edited_as_a_square_on_the_screen() {
    // Placing: the second corner is held on the diagonal from the first, without Shift.
    let first = pos2(100.0, 100.0);
    assert_eq!(
        constrained(
            Tool::GannSquareFixed,
            Some(first),
            pos2(300.0, 160.0),
            false
        ),
        pos2(300.0, 300.0)
    );
    // The standard square is free.
    assert_eq!(
        constrained(Tool::GannSquare, Some(first), pos2(300.0, 160.0), false),
        pos2(300.0, 160.0)
    );
    // Editing: dragging a corner keeps it square against the other.
    let data = bars(&[100.0; 10]);
    let scale = scale(false);
    let projection = projection(&data, &scale);
    let drawing = Drawing::build(
        Tool::GannSquareFixed,
        1,
        &[at(1, 100.0), at(3, 100.0 - 20.0)],
        Tool::GannSquareFixed.default_style(),
    )
    .unwrap();
    let now = pos2(450.0, 150.0);
    let edited = edit::edited(
        &drawing,
        Part::Anchor(1),
        now,
        now,
        projection.anchor(now),
        &projection,
    );
    let anchors = edited.anchors();
    let (a, b) = (projection.point(anchors[0]), projection.point(anchors[1]));
    assert!(
        ((b.x - a.x).abs() - (b.y - a.y).abs()).abs() < 0.2,
        "{a:?} {b:?}"
    );
    // The first corner did not move.
    assert!(near(a, pos2(150.0, 200.0)));
    // Moving the whole square keeps it as it is.
    let moved = edit::edited(
        &drawing,
        Part::Body,
        now,
        now + vec2(40.0, 25.0),
        projection.anchor(now),
        &projection,
    );
    let (ma, mb) = (
        projection.point(moved.anchors()[0]),
        projection.point(moved.anchors()[1]),
    );
    assert!(((mb.x - ma.x).abs() - (mb.y - ma.y).abs()).abs() < 0.2);
}

#[test]
fn shift_holds_a_gann_squares_ratio_while_a_corner_moves() {
    let data = bars(&[100.0; 10]);
    let scale = scale(false);
    let projection = projection(&data, &scale);
    // 4 bars across and 8 of price: 2 a bar.
    let drawing = Drawing::build(
        Tool::GannSquare,
        1,
        &[at(1, 100.0), at(5, 108.0)],
        Tool::GannSquare.default_style(),
    )
    .unwrap();
    // The second corner is dragged to bar 6 (x 650) and a price of 100 (y 200).
    let now = pos2(650.0, 200.0);
    let target = projection.anchor(now);
    let plain = edit::edited(&drawing, Part::Anchor(1), now, now, target, &projection);
    let kept = edit::keep_ratio(&drawing, plain.clone(), Part::Anchor(1), &projection);
    // Five bars across at 2 a bar is 10 of price up from 100.
    assert!(
        (kept.anchors()[1].price - 110.0).abs() < 1e-6,
        "{}",
        kept.anchors()[1].price
    );
    assert_eq!(kept.anchors()[1].time_ms, plain.anchors()[1].time_ms);
    // The other tools, a body drag and a box with no ratio are left as they were.
    let trend = Drawing::build(
        Tool::TrendLine,
        2,
        &[at(1, 100.0), at(5, 108.0)],
        Tool::TrendLine.default_style(),
    )
    .unwrap();
    assert_eq!(keep_ratio_of(&trend), trend);
    assert_eq!(
        edit::keep_ratio(&drawing, plain.clone(), Part::Body, &projection),
        plain
    );
    let flat = Drawing::build(
        Tool::GannSquare,
        3,
        &[at(1, 100.0), at(1, 108.0)],
        Tool::GannSquare.default_style(),
    )
    .unwrap();
    let flat_edit = edit::edited(&flat, Part::Anchor(1), now, now, target, &projection);
    assert_eq!(
        edit::keep_ratio(&flat, flat_edit.clone(), Part::Anchor(1), &projection),
        flat_edit
    );
}

fn keep_ratio_of(drawing: &Drawing) -> Drawing {
    let data = bars(&[100.0; 10]);
    let scale = scale(false);
    edit::keep_ratio(
        drawing,
        drawing.clone(),
        Part::Anchor(1),
        &projection(&data, &scale),
    )
}
