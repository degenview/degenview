//! The countdown on the last price's tag, drawn headlessly: where its row sits, that the price
//! stays where it was, and that the axis keeps its width as the digits change.

use super::canvas_tests::{Drawn, Harness};
use crate::model::chart::countdown::Countdown;
use crate::model::{ChartCanvas, Timeframe};
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

/// How far in from the card's edge the plot ends: the price axis' width.
fn plot_right_of(harness: &mut Harness) -> f32 {
    super::overlay_tests::plot_right(&harness.draw().0)
}

/// A chart of a day timeframe whose prices are `scale` times the fixture's.
fn daily(scale: f64, countdown: bool) -> Harness {
    let mut harness = harness_with(None);
    harness.card.set_timeframe(Timeframe::DAY_1);
    harness.card.appearance_mut().canvas.bar_countdown = countdown;
    for candle in &mut harness.candles {
        candle.open *= scale;
        candle.high *= scale;
        candle.low *= scale;
        candle.close *= scale;
    }
    harness
}

#[test]
fn the_axis_reserves_room_for_the_countdown_text_of_the_timeframe() {
    let with = plot_right_of(&mut daily(1.0, true));
    let without = plot_right_of(&mut daily(1.0, false));
    assert!(
        with < without,
        "a short price leaves the countdown (23:59:59 on a daily bar) wider than the labels: {with} vs {without}"
    );
}

#[test]
fn price_labels_wider_than_the_reservation_still_widen_the_axis() {
    let short = plot_right_of(&mut daily(1.0, true));
    let long_with = plot_right_of(&mut daily(10_000.0, true));
    let long_without = plot_right_of(&mut daily(10_000.0, false));
    assert!(long_with < short, "{long_with} vs {short}");
    assert_eq!(
        long_with, long_without,
        "where the labels are wider the countdown adds nothing"
    );
}

#[test]
fn a_bar_of_an_hour_reserves_less_than_a_day() {
    let mut hourly = daily(1.0, true);
    hourly.card.set_timeframe(Timeframe::HOUR_1);
    assert!(
        plot_right_of(&mut hourly) > plot_right_of(&mut daily(1.0, true)),
        "an hour never reads `23:59:59`"
    );
}

mod placement {
    use super::super::overlays::{TagBand, spread};
    use eframe::egui::{Color32, Pos2, Rect};

    const HALF: f32 = 9.0;

    fn plot(height: f32) -> Rect {
        Rect::from_min_max(Pos2::ZERO, Pos2::new(400.0, height))
    }

    fn place(ys: &[f32], plot: Rect, band: Option<TagBand>) -> Vec<f32> {
        let tags = ys
            .iter()
            .map(|y| (*y, Color32::GRAY, String::new()))
            .collect();
        spread(tags, HALF, plot, band)
            .into_iter()
            .map(|tag| tag.0)
            .collect()
    }

    fn clear_of(placed: &[f32], band: TagBand) {
        for y in placed {
            assert!(
                y + HALF <= band.top || y - HALF >= band.bottom,
                "a tag at {y} overlaps the band {band:?}"
            );
        }
    }

    fn apart(placed: &[f32]) {
        let mut sorted = placed.to_vec();
        sorted.sort_by(f32::total_cmp);
        for pair in sorted.windows(2) {
            assert!(pair[1] - pair[0] >= 2.0 * HALF + 1.0 - 1e-3, "{sorted:?}");
        }
    }

    fn inside(placed: &[f32], plot: Rect) {
        for y in placed {
            assert!(*y >= plot.top() + HALF && *y <= plot.bottom() - HALF, "{y}");
        }
    }

    #[test]
    fn tags_keep_clear_of_the_two_row_tag_on_both_sides() {
        let band = TagBand {
            top: 100.0,
            bottom: 131.0,
        };
        let placed = place(&[95.0, 105.0, 118.0, 130.0, 140.0], plot(400.0), Some(band));
        clear_of(&placed, band);
        apart(&placed);
        assert_eq!(placed.iter().filter(|y| **y < band.top).count(), 2);
    }

    #[test]
    fn a_tag_the_bottom_edge_leaves_no_room_for_goes_above_instead_of_into_the_tag() {
        // The last price is near the bottom: its countdown row went above the price, so the
        // tag ends at the plot's bottom and a tag that was below it has nowhere to stand there.
        let plot = plot(200.0);
        let band = TagBand {
            top: 150.0,
            bottom: 181.0,
        };
        for ys in [&[185.0][..], &[185.0, 190.0, 195.0][..]] {
            let placed = place(ys, plot, Some(band));
            assert_eq!(placed.len(), ys.len());
            clear_of(&placed, band);
            apart(&placed);
            inside(&placed, plot);
        }
    }

    #[test]
    fn a_tag_the_top_edge_leaves_no_room_for_goes_below() {
        let plot = plot(200.0);
        let band = TagBand {
            top: 10.0,
            bottom: 41.0,
        };
        let placed = place(&[5.0, 12.0, 30.0], plot, Some(band));
        clear_of(&placed, band);
        apart(&placed);
        inside(&placed, plot);
    }

    #[test]
    fn more_tags_than_the_axis_holds_overlap_inside_the_plot_without_panicking() {
        let plot = plot(60.0);
        let band = TagBand {
            top: 20.0,
            bottom: 50.0,
        };
        let placed = place(&[10.0, 30.0, 55.0], plot, Some(band));
        assert_eq!(placed.len(), 3);
        for y in &placed {
            assert!(*y >= plot.top() && *y <= plot.bottom());
        }
    }

    #[test]
    fn tags_with_nothing_to_avoid_are_left_where_they_are_and_keep_their_spacing_at_an_edge() {
        assert_eq!(place(&[50.0, 100.0], plot(400.0), None), [50.0, 100.0]);
        let piled = place(&[395.0, 396.0, 397.0], plot(400.0), None);
        apart(&piled);
        inside(&piled, plot(400.0));
    }
}
