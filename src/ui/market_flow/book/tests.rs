use super::ladder::{lines, stack_figure};
use super::*;
use crate::model::depth::{CENTRE_ROW, Price, Row, View, WINDOW_ROWS};
use crate::ui::market_flow::tests::{feed, live_ladder, waiting_ladder};
use crate::ui::theme;
use eframe::egui;

#[test]
fn sizes_are_short_and_keep_their_magnitude() {
    assert_eq!(size_text(0.0), "0");
    assert_eq!(size_text(0.0004), "<0.001");
    assert_eq!(size_text(0.28354), "0.284");
    assert_eq!(size_text(1.2345), "1.23");
    assert_eq!(size_text(12.34), "12.3");
    assert_eq!(size_text(340.4), "340");
    assert_eq!(size_text(1_234.0), "1.2K");
    assert_eq!(size_text(34_600.0), "35K");
    assert_eq!(size_text(-3.0), "3.00");
}

#[test]
fn the_big_trade_choices_rise() {
    let thresholds: Vec<f64> = BigTrade::ALL.iter().map(|big| big.threshold()).collect();
    assert_eq!(thresholds, [10_000.0, 100_000.0, 1_000_000.0]);
    assert_eq!(BigTrade::default(), BigTrade::HundredThousand);
}

#[test]
fn a_new_market_forgets_the_old_ones_row_size_and_scroll_but_not_the_big_trade_choice() {
    let mut state = BookViewState {
        group: Some(Price::from_units(1_000_000_000)),
        centre: Some(Price::from_units(5_000_000_000_000)),
        big: BigTrade::Million,
        scrolled: 7.0,
    };
    state.forget_market();
    assert_eq!(state.view(), View::default());
    assert_eq!(state.big, BigTrade::Million);
    assert_eq!(state.scrolled, 0.0);
}

#[test]
fn the_ladder_shows_the_rows_around_the_middle_with_the_spread_above_the_best_bid() {
    let ladder = live_ladder(View::default());
    let shown = lines(&ladder, 20);
    assert_eq!(shown.len(), 20);
    let rows: Vec<usize> = shown
        .iter()
        .filter_map(|line| match line {
            ladder::Line::Row(index) => Some(*index),
            ladder::Line::Spread => None,
        })
        .collect();
    assert!(rows.windows(2).all(|pair| pair[1] == pair[0] + 1));
    assert!(rows.contains(&CENTRE_ROW));
    assert_eq!(
        shown
            .iter()
            .filter(|line| matches!(line, ladder::Line::Spread))
            .count(),
        1
    );
    // The line below the spread is the row of the best bid.
    let at = shown
        .iter()
        .position(|line| matches!(line, ladder::Line::Spread))
        .unwrap();
    let ladder::Line::Row(below) = shown[at + 1] else {
        panic!("a spread above a spread");
    };
    assert_eq!(
        Some(ladder.rows[below].price),
        ladder.best_bid.map(|bid| bid.floor_to(ladder.group))
    );
}

#[test]
fn a_tall_panel_shows_every_row_and_no_more() {
    let ladder = live_ladder(View::default());
    let shown = lines(&ladder, 10_000);
    let rows = shown
        .iter()
        .filter(|line| matches!(line, ladder::Line::Row(_)))
        .count();
    assert_eq!(rows, WINDOW_ROWS);
}

#[test]
fn stacked_and_pulled_size_reads_by_sign_and_is_coloured_when_notable() {
    let row = |bid, pull_stack| Row {
        bid,
        pull_stack,
        ..Row::default()
    };
    assert_eq!(stack_figure(&row(10.0, 0.0)), None);
    assert_eq!(stack_figure(&row(10.0, 0.0001)), None);
    let (text, colour) = stack_figure(&row(10.0, 1.0)).unwrap();
    assert_eq!(text, "+1.00");
    assert_eq!(
        colour,
        theme::text_tertiary(),
        "a tenth of what rests is not notable"
    );
    let (text, colour) = stack_figure(&row(10.0, 3.0)).unwrap();
    assert_eq!((text.as_str(), colour), ("+3.00", theme::UP));
    let (text, colour) = stack_figure(&row(0.0, -4.0)).unwrap();
    assert_eq!(
        (text.as_str(), colour),
        ("−4.00", theme::DOWN),
        "all of it pulled"
    );
}

#[test]
fn the_mode_says_what_is_missing_instead_of_an_empty_ladder() {
    let mut state = BookViewState::default();
    for body in [
        BookBody { feed: None },
        BookBody {
            feed: Some(feed(&waiting_ladder(crate::model::depth::Link::Connecting))),
        },
    ] {
        egui::__run_test_ui(|ui| show(ui, &mut state, &body));
    }
    assert_eq!(
        state,
        BookViewState::default(),
        "drawing a state changes nothing"
    );
}

#[test]
fn the_footnote_says_where_the_book_is_from_and_since_when_it_traded() {
    let ladder = live_ladder(View::default());
    let note = footnote_text(feed(&ladder));
    assert!(note.starts_with("BTCUSDT spot, Binance."), "{note}");
    assert!(note.contains("Traded since "), "{note}");
    let quiet = waiting_ladder(crate::model::depth::Link::Live);
    assert!(footnote_text(feed(&quiet)).contains("No trades yet."));
}

/// Draws the mode over a live ladder in a screen of the panel's size, feeding `events` to the
/// the frame after the first and none after, until the smoothed scrolling has settled.
fn scrolled_by(wheel: f32, pointer: egui::Pos2) -> BookViewState {
    use crate::ui::render_tests::Screen;
    let ladder = live_ladder(View::default());
    let body = BookBody {
        feed: Some(feed(&ladder)),
    };
    let mut screen = Screen::new(egui::vec2(340.0, 640.0));
    let mut state = BookViewState::default();
    // The first frame of a screen has no size to lay out in yet.
    screen.run(Vec::new(), |ui| show(ui, &mut state, &body));
    let events = vec![
        egui::Event::PointerMoved(pointer),
        egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, wheel),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        },
    ];
    screen.run(events, |ui| show(ui, &mut state, &body));
    for _ in 0..30 {
        screen.run(Vec::new(), |ui| show(ui, &mut state, &body));
    }
    state
}

#[test]
fn scrolling_over_the_ladder_moves_its_window_a_row_for_each_row_of_scrolling() {
    let ladder = live_ladder(View::default());
    let centre = ladder.centre().unwrap();
    // Scrolling up shows higher prices; three rows' worth of points moves three rows.
    let up = scrolled_by(60.0, egui::pos2(170.0, 150.0));
    assert_eq!(up.centre, Some(centre + ladder.group.times(3)));
    let down = scrolled_by(-40.0, egui::pos2(170.0, 150.0));
    assert_eq!(down.centre, Some(centre + ladder.group.times(-2)));
}

#[test]
fn scrolling_elsewhere_leaves_the_ladder_where_it_is() {
    // Over the tape, below the ladder.
    assert_eq!(scrolled_by(60.0, egui::pos2(170.0, 560.0)).centre, None);
    // Less than a row is remembered, not lost, but it moves nothing yet.
    assert_eq!(scrolled_by(10.0, egui::pos2(170.0, 150.0)).centre, None);
}
