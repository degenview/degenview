//! The values tooltip on long press: when a press becomes it, what it lists, where it goes, and
//! that it leaves the press alone otherwise.

use super::canvas_tests::{Harness, SIZE, gather, wait_for_run};
use super::values_tooltip::Hold;
use super::*;
use crate::model::{ChartCanvas, CursorMode, Drawing, Tool};

const LONG: f64 = 0.6;

/// A chart with the pointer on its plot and the crosshair there, the button not yet down.
fn hovering() -> Harness {
    let mut harness = Harness::new(ChartCanvas::default());
    harness.crosshair = Some(CrosshairInfo {
        frozen: false,
        owner_id: 1,
        x_fraction: 0.5,
        time: None,
        price: 230.0,
    });
    harness.pointer = Some(plot_point());
    harness.draw();
    harness
}

fn plot_point() -> Pos2 {
    Pos2::new(SIZE.x * 0.5, SIZE.y * 0.4)
}

fn press(harness: &mut Harness) {
    harness.button = Some(true);
    harness.frame();
}

fn release(harness: &mut Harness) {
    harness.button = Some(false);
    harness.frame();
}

/// Holds the button down for `seconds` without moving, a frame at a time.
fn wait(harness: &mut Harness, seconds: f64) {
    harness.clock += seconds;
    harness.draw();
}

fn texts(harness: &mut Harness) -> Vec<String> {
    let frame = harness.frame();
    gather(&frame.0)
        .texts
        .into_iter()
        .map(|(_, text, _)| text)
        .collect()
}

#[test]
fn holding_still_shows_the_tooltip_and_letting_go_drops_it() {
    let mut harness = hovering();
    press(&mut harness);
    assert_eq!(
        harness.card.hold,
        Hold::Waiting {
            origin: plot_point(),
            since: harness.clock
        }
    );
    wait(&mut harness, 0.2);
    assert!(!harness.card.shows_values_tooltip(), "not yet");
    wait(&mut harness, LONG);
    assert!(harness.card.shows_values_tooltip());
    assert!(
        harness.card.values_hold_claims_drag(),
        "the press is the tooltip's"
    );
    release(&mut harness);
    assert_eq!(harness.card.hold, Hold::Idle);
    assert!(!harness.card.values_hold_claims_drag());
}

#[test]
fn a_press_that_moves_is_a_drag_and_never_the_tooltip() {
    let mut harness = hovering();
    press(&mut harness);
    harness.pointer = Some(plot_point() + Vec2::new(40.0, 0.0));
    harness.frame();
    assert_eq!(harness.card.hold, Hold::Spent);
    wait(&mut harness, LONG * 3.0);
    assert!(!harness.card.shows_values_tooltip());
    release(&mut harness);
    assert_eq!(harness.card.hold, Hold::Idle);
}

#[test]
fn a_small_tremor_is_still_a_still_hand() {
    let mut harness = hovering();
    press(&mut harness);
    harness.pointer = Some(plot_point() + Vec2::new(3.0, 2.0));
    harness.frame();
    wait(&mut harness, LONG);
    assert!(harness.card.shows_values_tooltip());
}

#[test]
fn with_the_option_off_nothing_shows() {
    let mut harness = hovering();
    harness.card.set_values_tooltip(false);
    press(&mut harness);
    wait(&mut harness, LONG * 2.0);
    assert_eq!(harness.card.hold, Hold::Idle, "never even waits");
    assert!(!harness.card.shows_values_tooltip());
}

#[test]
fn the_modes_that_use_the_press_never_show_it() {
    for mode in [CursorMode::Demonstration, CursorMode::Eraser] {
        let mut harness = hovering();
        harness.card.set_cursor_mode(mode);
        press(&mut harness);
        wait(&mut harness, LONG * 2.0);
        assert!(!harness.card.shows_values_tooltip(), "{mode:?}");
    }
    for mode in [CursorMode::Cross, CursorMode::Dot, CursorMode::Arrow] {
        let mut harness = hovering();
        harness.card.set_cursor_mode(mode);
        press(&mut harness);
        wait(&mut harness, LONG);
        assert!(harness.card.shows_values_tooltip(), "{mode:?}");
    }
}

#[test]
fn a_press_that_starts_off_the_plot_or_on_a_drawing_is_not_ours() {
    // Off the plot: on the price axis.
    let mut harness = hovering();
    harness.pointer = Some(Pos2::new(SIZE.x - 20.0, SIZE.y * 0.4));
    harness.draw();
    press(&mut harness);
    wait(&mut harness, LONG * 2.0);
    assert!(!harness.card.shows_values_tooltip(), "the axis");
    release(&mut harness);

    // On a drawing: a rectangle over the whole plot.
    let mut harness = hovering();
    let covering = Drawing::build(
        Tool::Rectangle,
        1,
        &[
            crate::model::Anchor {
                time_ms: 0,
                price: 0.0,
            },
            crate::model::Anchor {
                time_ms: 400 * super::canvas_tests::HOUR,
                price: 10_000.0,
            },
        ],
        Tool::Rectangle.default_style(),
    )
    .expect("a rectangle");
    harness.drawings.push(covering);
    harness.draw();
    press(&mut harness);
    wait(&mut harness, LONG * 2.0);
    assert!(
        !harness.card.shows_values_tooltip(),
        "a drawing's press is the drawing's"
    );
}

#[test]
fn the_tooltip_names_the_bar_with_the_values_the_frame_holds() {
    let mut harness = hovering();
    harness.indicators.show_ema = true;
    harness.indicators.show_bollinger = true;
    harness.indicators.show_rsi = true;
    harness.draw();
    let quiet = texts(&mut harness);
    assert!(
        !quiet.contains(&"Chg".to_owned()),
        "no tooltip before the hold"
    );
    press(&mut harness);
    wait(&mut harness, LONG);
    let shown = texts(&mut harness);
    for label in [
        "O", "H", "L", "C", "Vol", "Chg", "RSI", "BB upper", "BB basis", "BB lower",
    ] {
        assert!(
            shown.iter().any(|text| text == label),
            "{label} is listed: {shown:?}"
        );
    }
    assert!(
        shown.iter().any(|text| text.starts_with("EMA ")),
        "{shown:?}"
    );
    release(&mut harness);
    assert!(
        !texts(&mut harness).contains(&"Chg".to_owned()),
        "gone with the button"
    );
}

#[test]
fn the_tooltip_stays_inside_the_chart_wherever_the_pointer_is() {
    for at in [
        Pos2::new(SIZE.x * 0.5, SIZE.y * 0.4),
        Pos2::new(SIZE.x - 90.0, SIZE.y - 90.0),
        Pos2::new(40.0, 120.0),
    ] {
        let mut harness = hovering();
        harness.indicators.show_ema = true;
        harness.pointer = Some(at);
        harness.draw();
        press(&mut harness);
        wait(&mut harness, LONG);
        let frame = harness.frame();
        let shown = gather(&frame.0);
        let chart = Rect::from_min_size(Pos2::ZERO, SIZE);
        for (pos, text, _) in &shown.texts {
            if text == "Chg" {
                assert!(
                    chart.contains(*pos),
                    "{text} at {pos:?} for the pointer at {at:?}"
                );
            }
        }
        assert!(
            shown.texts.iter().any(|(_, text, _)| text == "Chg"),
            "{at:?}"
        );
    }
}

#[test]
fn scripts_plots_on_the_chart_and_in_their_panes_are_listed() {
    let mut harness = hovering();
    let overlay = harness.card.add_pine_instance(
        None,
        "Overlay",
        "//@version=6\nindicator(\"Overlay\", overlay = true)\nplot(close, title = \"Close line\")\n",
    );
    let pane = harness.card.add_pine_instance(
        None,
        "Pane",
        "//@version=6\nindicator(\"Pane\")\nplot(open, title = \"Open pane\")\n",
    );
    wait_for_run(&mut harness, overlay);
    wait_for_run(&mut harness, pane);
    harness.draw();
    press(&mut harness);
    wait(&mut harness, LONG);
    let shown = texts(&mut harness);
    for name in ["Close line", "Open pane"] {
        assert!(shown.iter().any(|text| text == name), "{name}: {shown:?}");
    }
}
