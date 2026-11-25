//! The countdown on the last price's tag, drawn headlessly: where its row sits, that the price
//! stays where it was, and that the axis keeps its width as the digits change.

use super::canvas_tests::{Drawn, Harness};
use crate::model::ChartCanvas;
use crate::model::chart::countdown::Countdown;
use eframe::egui::Pos2;

fn countdown(remaining_ms: i64) -> Countdown {
    Countdown {
        close_ms: 1_000_000 + remaining_ms,
        remaining_ms,
    }
}

fn harness_with(countdown: Option<Countdown>) -> Harness {
    let mut harness = Harness::new(ChartCanvas::default());
    harness.card.set_countdown(countdown);
    harness
}

/// The countdown's text and the price row it hangs under, in the axis column.
fn tag_rows(drawn: &Drawn, text: &str) -> Option<((Pos2, f32), (Pos2, f32))> {
    let (at, _, size) = drawn.texts.iter().find(|(_, shown, _)| shown == text)?;
    let price = drawn.texts.iter().find(|(price_at, shown, _)| {
        shown != text
            && (price_at.x - at.x).abs() < 0.5
            && price_at.y < at.y
            && at.y - price_at.y < 24.0
    })?;
    Some(((*at, *size), (price.0, price.2)))
}

#[test]
fn the_countdown_is_a_smaller_second_row_under_the_price() {
    let (drawn, _) = harness_with(Some(countdown(125_000))).draw();
    let ((count, count_size), (price, price_size)) =
        tag_rows(&drawn, "02:05").expect("the countdown is drawn under a price");
    assert!(count_size < price_size, "{count_size} under {price_size}");
    assert!(count.y > price.y + price_size, "a row of its own");
}

#[test]
fn the_price_stays_on_its_line_with_or_without_the_countdown() {
    let (without, _) = harness_with(None).draw();
    let (with, _) = harness_with(Some(countdown(125_000))).draw();
    let (_, (price, _)) = tag_rows(&with, "02:05").expect("the countdown is drawn");
    assert!(
        without
            .texts
            .iter()
            .any(|(at, _, _)| (at.x - price.x).abs() < 0.5 && (at.y - price.y).abs() < 0.01),
        "the price row is where the price tag was"
    );
}

#[test]
fn the_axis_keeps_its_width_as_the_digits_change() {
    let left_edge = |remaining_ms| {
        let (drawn, _) = harness_with(Some(countdown(remaining_ms))).draw();
        let text = crate::model::chart::countdown::format(remaining_ms);
        tag_rows(&drawn, &text).expect("drawn").0.0.x
    };
    let (none, _) = harness_with(None).draw();
    let widest = none
        .texts
        .iter()
        .map(|(at, _, _)| at.x)
        .fold(f32::MIN, f32::max);
    assert_eq!(left_edge(599_000), left_edge(600_000), "09:59 and 10:00");
    assert_eq!(left_edge(59_000), left_edge(3_599_000), "00:59 and 59:59");
    assert!(
        (left_edge(59_000) - widest).abs() < 40.0,
        "the countdown is in the axis column"
    );
}

#[test]
fn the_axis_is_as_wide_with_no_countdown_as_with_one() {
    let (with, _) = harness_with(Some(countdown(30_000))).draw();
    let (without, _) = harness_with(None).draw();
    let plot_right = |drawn: &Drawn| {
        drawn
            .segments
            .iter()
            .map(|(points, _)| points[1].x)
            .fold(f32::MIN, f32::max)
    };
    assert_eq!(
        plot_right(&with),
        plot_right(&without),
        "a countdown that comes and goes with the connection does not move the plot"
    );
}

#[test]
fn switched_off_the_tag_shows_the_price_alone() {
    let mut harness = harness_with(Some(countdown(125_000)));
    harness.card.appearance_mut().canvas.bar_countdown = false;
    let (drawn, _) = harness.draw();
    assert!(drawn.texts.iter().all(|(_, text, _)| text != "02:05"));
}

#[test]
fn overlay_tags_keep_clear_of_the_two_row_tag_on_both_sides() {
    use super::overlays::{TagBand, spread};
    use eframe::egui::{Color32, Rect};
    let plot = Rect::from_min_max(Pos2::ZERO, Pos2::new(400.0, 400.0));
    let band = TagBand {
        top: 100.0,
        bottom: 131.0,
    };
    let half = 9.0;
    let tags = [95.0, 105.0, 118.0, 130.0, 140.0]
        .into_iter()
        .map(|y| (y, Color32::GRAY, String::new()))
        .collect();
    let placed = spread(tags, half, plot, Some(band));
    assert_eq!(placed.len(), 5);
    for (y, _, _) in &placed {
        assert!(
            y + half <= band.top || y - half >= band.bottom,
            "a tag at {y} overlaps the band"
        );
    }
    for pair in placed.windows(2) {
        assert!(
            pair[1].0 - pair[0].0 >= 2.0 * half + 1.0 - 1e-3,
            "{placed:?}"
        );
    }
    let above = placed.iter().filter(|(y, _, _)| *y < band.top).count();
    assert_eq!(above, 2, "the tags that were above it stay above");
}
