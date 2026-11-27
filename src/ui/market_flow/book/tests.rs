use super::ladder::{lines, rows_in, stack_figure};
use super::*;
use crate::model::DataSource;
use crate::model::depth::{DepthMarket, Price, Row, View, market_of};
use crate::ui::market_flow::tests::{feed, first_view, live_depth, waiting_depth};
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
    let labels: Vec<&str> = BigTrade::ALL.iter().map(|big| big.label()).collect();
    assert_eq!(labels, ["≥ $10K", "≥ $100K", "≥ $1M"]);
    crate::i18n::with_locale(crate::i18n::Locale::PSEUDO, || {
        assert!(BigTrade::ALL.iter().all(|big| big.label().starts_with('[')));
    });
}

fn market(symbol: &str) -> Option<DepthMarket> {
    market_of(DataSource::Binance, symbol)
}

#[test]
fn a_new_market_forgets_the_old_ones_row_size_and_scroll_but_not_the_big_trade_choice() {
    let mut state = BookViewState::default();
    state.group = Some(Price::from_units(1_000_000_000));
    state.centre = Some(Price::from_units(5_000_000_000_000));
    state.big = BigTrade::Million;
    state.rows = 30;
    state.scrolled = 7.0;
    state.follow(market("BTCUSDT").as_ref());
    assert_eq!(state.group, None);
    assert_eq!(state.centre, None);
    assert_eq!(state.scrolled, 0.0);
    assert_eq!(state.big, BigTrade::Million);
    assert_eq!(
        state.rows, 30,
        "the room for rows is the panel's, not the market's"
    );
}

#[test]
fn the_book_starts_over_on_another_market_and_keeps_its_place_on_the_same_one() {
    let mut state = BookViewState::default();
    let (btc, eth) = (market("BTCUSDT"), market("ETHUSDT"));
    state.follow(btc.as_ref());
    state.group = Some(Price::from_units(1_000_000_000));
    state.follow(btc.as_ref());
    assert!(state.group.is_some(), "the same market");
    state.follow(eth.as_ref());
    assert_eq!(state.group, None, "another market");
    // A chart with no book in between is another market too.
    state.group = Some(Price::from_units(1_000_000_000));
    state.follow(None);
    state.follow(eth.as_ref());
    assert_eq!(state.group, None);
}

#[test]
fn the_ladder_is_asked_for_the_rows_the_panel_last_had_room_for() {
    let mut state = BookViewState::default();
    assert_eq!(state.view().rows, state.rows);
    state.rows = 31;
    state.group = Some(Price::from_units(5_000_000));
    assert_eq!(
        state.view(),
        View {
            group: Some(Price::from_units(5_000_000)),
            centre: None,
            rows: 31
        }
    );
}

fn spreads(shown: &[ladder::Line]) -> Vec<usize> {
    (0..shown.len())
        .filter(|at| matches!(shown[*at], ladder::Line::Spread))
        .collect()
}

#[test]
fn the_ladder_shows_every_row_with_the_spread_above_the_best_bid() {
    let ladder = live_depth(first_view()).ladder;
    let rows = ladder.rows.len();
    // Room for the rows and the spread line.
    let shown = lines(&ladder, rows + 1);
    assert_eq!(shown.len(), rows + 1);
    let indices: Vec<usize> = shown
        .iter()
        .filter_map(|line| match line {
            ladder::Line::Row(index) => Some(*index),
            ladder::Line::Spread => None,
        })
        .collect();
    assert_eq!(indices, (0..rows).collect::<Vec<_>>());
    assert!(indices.contains(&(rows / 2)), "the middle row is on screen");
    let at = spreads(&shown);
    assert_eq!(at.len(), 1);
    // The line below the spread is the row of the best bid.
    let ladder::Line::Row(below) = shown[at[0] + 1] else {
        panic!("a spread above a spread");
    };
    assert_eq!(Some(below), ladder.best_bid_row());
    // With room for the rows only, the spread takes the place of the last one.
    let fitted = lines(&ladder, rows);
    assert_eq!(fitted.len(), rows);
    assert_eq!(spreads(&fitted).len(), 1);
    assert!(matches!(fitted[rows - 1], ladder::Line::Row(index) if index == rows - 2));
}

#[test]
fn no_spread_line_leads_the_ladder_and_none_pushes_the_lines_past_the_room() {
    // One row: the best bid is in the first (only) row, so there is nothing above it.
    let one = live_depth(View {
        rows: 1,
        ..first_view()
    })
    .ladder;
    assert_eq!(one.best_bid_row(), Some(0));
    assert!(spreads(&lines(&one, 5)).is_empty());
    // A ladder drawn in less room than it has lines loses its last.
    let ladder = live_depth(first_view()).ladder;
    let rows = ladder.rows.len();
    let cut = lines(&ladder, rows - 2);
    assert_eq!(cut.len(), rows - 2);
    assert_eq!(spreads(&cut).len(), 1);
    // Scrolled so far that the best bid is no longer among the rows: no spread.
    let away = live_depth(View {
        centre: Some(ladder.centre().unwrap() + ladder.group.times(1_000)),
        ..first_view()
    })
    .ladder;
    assert_eq!(away.best_bid_row(), None);
    assert!(spreads(&lines(&away, 100)).is_empty());
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
            feed: Some(feed(&waiting_depth(crate::model::depth::Link::Connecting))),
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
    let depth = live_depth(first_view());
    let note = footnote_text(feed(&depth));
    assert!(note.starts_with("BTCUSDT, Binance spot."), "{note}");
    assert!(note.contains("Traded since "), "{note}");
    let quiet = waiting_depth(crate::model::depth::Link::Live);
    assert!(footnote_text(feed(&quiet)).contains("No trades yet."));
}

/// Draws the mode over a live ladder in a screen of the panel's size, feeding `events` to the
/// the frame after the first and none after, until the smoothed scrolling has settled.
fn scrolled_by(wheel: f32, pointer: egui::Pos2) -> BookViewState {
    use crate::ui::render_tests::Screen;
    let depth = live_depth(first_view());
    let body = BookBody {
        feed: Some(feed(&depth)),
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
fn scrolling_over_the_ladder_moves_its_rows_a_row_for_each_row_of_scrolling() {
    let ladder = live_depth(first_view()).ladder;
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

/// Draws the mode over live depth in a screen of `height` for a few frames, giving the depth the
/// rows the state asks for each frame, as the host does.
fn drawn_in(height: f32) -> BookViewState {
    use crate::ui::render_tests::Screen;
    let mut screen = Screen::new(egui::vec2(340.0, height));
    let mut state = BookViewState::default();
    for _ in 0..4 {
        let depth = live_depth(state.view());
        let body = BookBody {
            feed: Some(feed(&depth)),
        };
        screen.run(Vec::new(), |ui| show(ui, &mut state, &body));
    }
    state
}

#[test]
fn the_ladder_is_asked_for_a_row_for_each_line_the_panel_has_room_for() {
    let short = drawn_in(640.0);
    let tall = drawn_in(1_000.0);
    assert!(short.rows >= 1);
    assert!(tall.rows > short.rows, "{} and {}", tall.rows, short.rows);
    // The depth drawn for those rows fills the ladder's lines, the spread's among them.
    let depth = live_depth(tall.view());
    assert_eq!(depth.ladder.rows.len(), tall.rows);
    assert_eq!(lines(&depth.ladder, tall.rows).len(), tall.rows);
}

#[test]
fn a_ladder_asks_for_a_row_for_each_line_and_never_for_none() {
    assert_eq!(rows_in(20), 20);
    assert_eq!(rows_in(1), 1);
    assert_eq!(rows_in(0), 1);
}
