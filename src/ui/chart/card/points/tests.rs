use super::super::canvas_tests::Harness;
use super::super::*;
use super::{PendingScript, PickView, SetupEvent, SetupView};
use crate::model::chart::canvas::ChartCanvas;
use crate::pine::inputs::points::PointAxes;

fn asking(pane: Option<u64>) -> SetupView {
    SetupView {
        pending: Vec::new(),
        pick: Some(PickView {
            axes: PointAxes::Both {
                time: "t".into(),
                price: "p".into(),
            },
            pane,
            picked: Vec::new(),
            prompt: Some("Pick".into()),
        }),
    }
}

fn picking() -> Harness {
    let mut harness = Harness::new(ChartCanvas::default());
    harness.card.set_setup_view(asking(None));
    harness.draw();
    harness
}

/// A press and release at `at`.
fn click(harness: &mut Harness, at: Pos2) {
    harness.pointer = Some(at);
    harness.frame();
    harness.button = Some(true);
    harness.frame();
    harness.button = Some(false);
    harness.frame();
}

fn picked(harness: &mut Harness) -> Vec<(i64, f64)> {
    harness
        .card
        .take_setup_events()
        .into_iter()
        .filter_map(|event| match event {
            SetupEvent::Picked { time_ms, price } => Some((time_ms, price)),
            SetupEvent::PromptClosed => None,
        })
        .collect()
}

#[test]
fn a_click_gives_the_bars_open_time_and_the_price_under_it() {
    let mut harness = picking();
    let probe = harness.card.probe.expect("a plot");
    let at = probe.plot.center();
    click(&mut harness, at);
    let events = picked(&mut harness);
    let [(time_ms, price)] = events[..] else {
        panic!("{events:?}");
    };
    assert!(
        harness
            .candles
            .iter()
            .any(|candle| candle.open_time == time_ms),
        "a bar's open: {time_ms}"
    );
    let fraction = f64::from((probe.plot.bottom() - at.y) / probe.plot.height());
    let guess = probe.minimum + fraction * (probe.maximum - probe.minimum);
    assert!(
        (price - guess).abs() < (probe.maximum - probe.minimum) * 0.1,
        "{price} against {guess}"
    );
}

#[test]
fn a_click_past_the_newest_bar_gives_a_time_still_to_come() {
    let mut harness = Harness::new(ChartCanvas::default());
    let mut canvas = ChartCanvas::default();
    canvas.margins.right = 20;
    harness.card.appearance_mut().canvas = canvas;
    harness.card.set_setup_view(asking(None));
    harness.draw();
    let probe = harness.card.probe.expect("a plot");
    click(
        &mut harness,
        Pos2::new(probe.plot.right() - probe.slot_width, probe.plot.center().y),
    );
    let newest = harness.candles.last().expect("candles").open_time;
    let events = picked(&mut harness);
    let [(time_ms, _)] = events[..] else {
        panic!("{events:?}");
    };
    assert!(time_ms > newest);
    assert_eq!((time_ms - newest) % 3_600_000, 0, "on the hourly grid");
}

#[test]
fn a_drag_pans_and_picks_nothing() {
    let mut harness = picking();
    let before = harness.card.probe.expect("a plot");
    let from = before.plot.center();
    harness.pointer = Some(from);
    harness.frame();
    harness.button = Some(true);
    harness.frame();
    for step in 1..=5 {
        harness.pointer = Some(from + Vec2::new(30.0 * step as f32, 0.0));
        harness.frame();
    }
    harness.button = Some(false);
    harness.frame();
    harness.frame();
    assert!(picked(&mut harness).is_empty());
    let after = harness.card.probe.expect("a plot");
    assert!(after.start < before.start, "the chart moved");
}

#[test]
fn nothing_is_picked_while_no_point_is_asked() {
    let mut harness = Harness::new(ChartCanvas::default());
    harness.draw();
    let at = harness.card.probe.expect("a plot").plot.center();
    click(&mut harness, at);
    assert!(picked(&mut harness).is_empty());
}

#[test]
fn a_waiting_pane_script_has_an_empty_pane_that_takes_no_clicks() {
    let mut harness = Harness::new(ChartCanvas::default());
    let mut view = asking(None);
    view.pending.push(PendingScript {
        id: 7,
        name: "Waiting".into(),
        overlay: false,
    });
    harness.card.set_setup_view(view);
    harness.draw();
    assert_eq!(harness.card.pane_count(), 1);
    let plot = harness.card.probe.expect("a plot").plot;
    click(
        &mut harness,
        Pos2::new(plot.center().x, plot.bottom() + 40.0),
    );
    assert!(picked(&mut harness).is_empty());
    click(&mut harness, plot.center());
    assert_eq!(picked(&mut harness).len(), 1);
}
