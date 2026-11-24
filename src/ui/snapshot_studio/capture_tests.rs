//! What a capture holds and what it says about itself.

use super::capture::cached_logo_image;
use super::fixtures::{CHART_ID, chart_capture, parts, single};
use crate::model::chart::snapshot::framing::Framing;
use crate::model::{ChartCanvas, DataSource};
use crate::ui::chart::card::studio_fixture::Fixture;
use crate::ui::chart::card::{ChartCard, SnapshotFrame};
use crate::ui::snapshot_studio::capture::ChartCapture;
use eframe::egui::{Pos2, Rect, pos2};

fn geometry_of(
    card: &ChartCard,
    capture: &ChartCapture,
    frame: SnapshotFrame,
) -> crate::ui::chart::card::ViewGeometry {
    card.snapshot_geometry(&capture.input(frame), capture.size(), 1.0)
        .expect("measured")
        .view
}

#[test]
fn a_capture_says_what_the_include_toggles_can_switch_off() {
    let plain = Fixture::plain(ChartCanvas::default(), 300);
    assert!(!single(&plain).has_indicators());
    assert!(!single(&plain).has_drawings());

    let with_indicators = Fixture::plain(ChartCanvas::default(), 300).with_indicators();
    assert!(single(&with_indicators).has_indicators());

    let with_drawing = Fixture::plain(ChartCanvas::default(), 300).with_a_drawing();
    assert!(single(&with_drawing).has_drawings());
    assert!(!single(&with_drawing).has_indicators());

    // A script's plots count as indicators too.
    let busy = Fixture::busy(ChartCanvas::default(), 300);
    assert!(single(&busy).has_indicators() && single(&busy).has_drawings());
}

#[test]
fn indicators_hidden_in_the_chart_and_hidden_drawings_do_not_count() {
    let fixture = Fixture::plain(ChartCanvas::default(), 300).with_a_drawing();
    let mut hidden_parts = parts(&fixture, CHART_ID);
    hidden_parts
        .drawings
        .iter_mut()
        .for_each(|drawing| drawing.set_hidden(true));
    hidden_parts.indicators.show_ema = true;
    hidden_parts.indicators.hide_ema = true;
    let capture = ChartCapture::new(fixture.card(), hidden_parts, 1.0, None).expect("captured");
    assert!(!capture.has_drawings(), "a hidden drawing is not drawn");
    assert!(
        !capture.has_indicators(),
        "nor an indicator the chart hides"
    );
}

#[test]
fn a_capture_is_the_size_of_the_chart_and_starts_as_the_chart_was() {
    let fixture = Fixture::plain(ChartCanvas::default(), 300);
    let capture = chart_capture(&fixture, CHART_ID, None);
    assert_eq!(capture.size(), fixture.card().snapshot_size());
    let live = geometry_of(fixture.card(), &capture, SnapshotFrame::default());
    let again = geometry_of(
        fixture.card(),
        &capture,
        SnapshotFrame {
            viewport: Some(Framing::default().viewport(capture.base(), capture.base().plot)),
            ..SnapshotFrame::default()
        },
    );
    assert_eq!((again.start, again.end), (live.start, live.end));
    assert!((again.minimum - live.minimum).abs() < 1e-6);
    assert!((again.maximum - live.maximum).abs() < 1e-6);
    assert_eq!(capture.base().plot, live.plot.size());
}

#[test]
fn a_portion_is_a_capture_of_the_selection_showing_the_bars_it_covered() {
    let fixture = Fixture::plain(ChartCanvas::default(), 300);
    let whole = chart_capture(&fixture, CHART_ID, None);
    let live = geometry_of(fixture.card(), &whole, SnapshotFrame::default());
    let selection = Rect::from_min_max(pos2(200.0, 100.0), pos2(500.0, 300.0));
    let portion = chart_capture(&fixture, CHART_ID, Some(selection));
    assert_eq!(portion.size(), selection.size());

    let shown = geometry_of(
        fixture.card(),
        &portion,
        SnapshotFrame {
            viewport: Some(Framing::default().viewport(portion.base(), portion.base().plot)),
            ..SnapshotFrame::default()
        },
    );
    // The bars across the selection, a slot of the live chart each.
    let across = f64::from(selection.width() / live.slot_width);
    let shown_slots = f64::from(shown.plot.width() / shown.slot_width);
    assert!(
        (shown_slots - across).abs() <= 2.0,
        "{shown_slots} against {across}"
    );
    // The first of them is where the selection's left edge was.
    let slot_at = |view: &crate::ui::chart::card::ViewGeometry, x: f32| {
        view.start as f64 - f64::from(view.lead)
            + f64::from((x - view.plot.left()) / view.slot_width)
    };
    let (first, wanted) = (
        slot_at(&shown, shown.plot.left()),
        slot_at(&live, selection.min.x),
    );
    assert!((first - wanted).abs() <= 1.0, "{first} against {wanted}");
    // And the prices are those between its top and bottom, not the chart's.
    assert!(shown.maximum - shown.minimum < (live.maximum - live.minimum) * 0.7);
}

#[test]
fn a_selection_too_small_to_hold_a_bar_is_no_capture() {
    let fixture = Fixture::plain(ChartCanvas::default(), 300);
    let tiny = Rect::from_min_max(Pos2::new(300.0, 100.0), Pos2::new(300.2, 300.0));
    assert!(ChartCapture::new(fixture.card(), parts(&fixture, CHART_ID), 1.0, Some(tiny)).is_err());
}

#[test]
fn a_capture_is_frozen_from_what_the_card_was_given() {
    let fixture = Fixture::plain(ChartCanvas::default(), 300);
    let mut cut = parts(&fixture, CHART_ID);
    cut.candles.truncate(150);
    cut.replay_active = true;
    let capture = ChartCapture::new(fixture.card(), cut, 1.0, None).expect("captured");
    assert_eq!(capture.base().total_bars, 150, "cut at the replay");
}

#[test]
fn a_market_with_no_logo_of_its_own_has_none_to_freeze() {
    let ctx = eframe::egui::Context::default();
    assert!(cached_logo_image(&ctx, "CMC-TOP", DataSource::CoinMarketCap).is_none());
}
