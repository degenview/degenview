//! The positions, forecast, bars pattern, ghost feed, sector, measurers, VWAP and volume profiles
//! projected onto the hand-checkable plot of the line tests: 100 px per bar (bar `i` is centred on
//! x = 50 + 100 i), prices 80 to 120 down a 400 px plot (y = (120 - price) x 10), twelve bars of
//! volume 1 each. Every expected number below is worked by hand from that.

use super::line_tests::{PLOT, at, bars, near, projection, scale};
use super::shape::{CandleLook, LabelLook, Screen, Tag};
use super::*;
use crate::model::bars_pattern::{BarsMode, BarsPattern};
use crate::model::chart_type::{LineDash, Rgba};
use crate::model::forecast_style::ProfileVolume;
use crate::model::{DrawingStyle, Kline};
use eframe::egui::pos2;

fn built_on(
    tool: Tool,
    anchors: &[Anchor],
    style: DrawingStyle,
    data: &[Kline],
    logarithmic: bool,
) -> Screen {
    let drawing = Drawing::build(tool, 1, anchors, style).expect("built");
    let scale = scale(logarithmic);
    shape::build(&drawing, &projection(data, &scale)).expect("projected")
}

fn flat() -> Vec<Kline> {
    bars(&[100.0; 12])
}

fn built(tool: Tool, anchors: &[Anchor]) -> Screen {
    built_on(tool, anchors, tool.default_style(), &flat(), false)
}

fn texts(screen: &Screen) -> Vec<&str> {
    screen
        .labels
        .iter()
        .map(|label| label.text.as_str())
        .collect()
}

fn long() -> [Anchor; 3] {
    [at(2, 100.0), at(5, 110.0), at(5, 95.0)]
}

// MARK: Positions

#[test]
fn a_long_position_is_a_profit_zone_over_a_stop_zone_between_three_levels() {
    let screen = built(Tool::LongPosition, &long());
    // The entry (250, 200), the target (550, 100) and the stop (550, 250).
    assert_eq!(screen.boxes.len(), 2);
    let options = Tool::LongPosition.default_style().position;
    assert_eq!(
        screen.boxes[0].rect,
        Rect::from_min_max(pos2(250.0, 100.0), pos2(550.0, 200.0))
    );
    assert_eq!(screen.boxes[0].color, options.profit);
    assert_eq!(
        screen.boxes[1].rect,
        Rect::from_min_max(pos2(250.0, 200.0), pos2(550.0, 250.0))
    );
    assert_eq!(screen.boxes[1].color, options.stop);
    // The three levels run across the zones.
    let ys: Vec<f32> = screen.lines.iter().map(|line| line[0].y).collect();
    assert_eq!(ys, [200.0, 100.0, 250.0]);
    assert!(
        screen
            .lines
            .iter()
            .all(|line| line[0].x == 250.0 && line[1].x == 550.0)
    );
    // A handle on each anchor.
    assert!(near(screen.handles[0].pos, pos2(250.0, 200.0)));
    assert!(near(screen.handles[1].pos, pos2(550.0, 100.0)));
    assert!(near(screen.handles[2].pos, pos2(550.0, 250.0)));
}

#[test]
fn a_short_position_has_its_profit_below_the_entry() {
    let screen = built(
        Tool::ShortPosition,
        &[at(2, 100.0), at(5, 90.0), at(5, 105.0)],
    );
    // The target is the lower level (y = 300), the stop the upper (y = 150).
    assert_eq!(
        screen.boxes[0].rect,
        Rect::from_min_max(pos2(250.0, 200.0), pos2(550.0, 300.0))
    );
    assert_eq!(
        screen.boxes[1].rect,
        Rect::from_min_max(pos2(250.0, 150.0), pos2(550.0, 200.0))
    );
    assert!(
        screen.labels[1].text.starts_with("Stop: 5.00"),
        "{}",
        screen.labels[1].text
    );
    assert!(
        screen
            .labels
            .last()
            .unwrap()
            .text
            .starts_with("Short (hypothetical)"),
        "{:?}",
        texts(&screen)
    );
}

#[test]
fn the_stats_say_what_the_formulas_say() {
    let screen = built(Tool::LongPosition, &long());
    // Entry 100, stop 95, target 110; account 1,000 risking 25 % = 250: a risk quantity of 50
    // and a leverage quantity of 10, so 10 lots: 100 to win, 50 to lose.
    assert_eq!(
        texts(&screen),
        [
            "Target: 10.00 (10.00%) 1,000, Amount: 1,100",
            "Stop: 5.00 (5.00%) 500, Amount: 950.00",
            "Long (hypothetical)\nOpen P&L: 0.00, Qty: 10\nRisk/Reward Ratio: 2.00",
        ]
    );
}

#[test]
fn the_tags_sit_beyond_the_levels_and_the_stats_wait_for_a_selection() {
    let screen = built(Tool::LongPosition, &long());
    // Target above, stop below, the middle on the entry line; each at the middle of the width.
    assert!(near(screen.labels[0].at, pos2(400.0, 100.0)));
    assert_eq!(screen.labels[0].push.y, -1.0);
    assert!(near(screen.labels[1].at, pos2(400.0, 250.0)));
    assert_eq!(screen.labels[1].push.y, 1.0);
    assert!(near(screen.labels[2].at, pos2(400.0, 200.0)));
    // Unselected, a position shows its zones and levels and no stats...
    assert!(screen.labels.iter().all(|label| label.on_select));
    // ...unless it is set to always show them.
    let mut style = Tool::LongPosition.default_style();
    style.position.always_stats = true;
    let always = built_on(Tool::LongPosition, &long(), style, &flat(), false);
    assert!(always.labels.iter().all(|label| !label.on_select));
}

#[test]
fn a_compact_position_says_less() {
    let mut style = Tool::LongPosition.default_style();
    style.position.compact = true;
    let screen = built_on(Tool::LongPosition, &long(), style, &flat(), false);
    assert_eq!(screen.labels[0].text, "10.00 (10.00%)");
    assert_eq!(screen.labels[1].text, "5.00 (5.00%)");
    assert_eq!(screen.labels[2].text, "Long (hypothetical)\nOpen P&L: 0.00");
}

#[test]
fn the_axis_carries_the_three_prices_in_their_own_colours() {
    let screen = built(Tool::LongPosition, &long());
    let options = Tool::LongPosition.default_style().position;
    let tags: Vec<(f32, f64, Rgba)> = screen
        .tags
        .iter()
        .map(|tag| match tag {
            Tag::Price { y, price, color } => (*y, *price, *color),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(tags.len(), 3);
    assert_eq!((tags[0].0, tags[0].1), (200.0, 100.0));
    assert_eq!((tags[1].0, tags[1].1), (100.0, 110.0));
    assert_eq!(
        tags[1].2,
        [options.profit[0], options.profit[1], options.profit[2], 255]
    );
    assert_eq!((tags[2].0, tags[2].1), (250.0, 95.0));
    // Off with the setting.
    let mut style = Tool::LongPosition.default_style();
    style.price_labels = false;
    assert!(
        built_on(Tool::LongPosition, &long(), style, &flat(), false)
            .tags
            .is_empty()
    );
}

#[test]
fn a_position_says_how_it_ended_once_the_bars_have_reached_a_level() {
    let options = Tool::LongPosition.default_style().position;
    // A bar after the entry reaches the target (110): closed in profit, in the success colour.
    let mut data = flat();
    data[3].high = 111.0;
    let screen = built_on(
        Tool::LongPosition,
        &long(),
        Tool::LongPosition.default_style(),
        &data,
        false,
    );
    let middle = &screen.labels[2];
    assert!(
        middle.text.contains("Closed P&L: +100.00"),
        "{}",
        middle.text
    );
    assert!(matches!(middle.look, LabelLook::Plate { fill, .. } if fill == options.success));
    // Or the stop (95): closed at a loss, in the failure colour.
    let mut data = flat();
    data[4].low = 94.0;
    let screen = built_on(
        Tool::LongPosition,
        &long(),
        Tool::LongPosition.default_style(),
        &data,
        false,
    );
    let middle = &screen.labels[2];
    assert!(
        middle.text.contains("Closed P&L: -50.00"),
        "{}",
        middle.text
    );
    assert!(matches!(middle.look, LabelLook::Plate { fill, .. } if fill == options.failure));
    // The entry bar itself is not examined, nor a bar that is not there yet.
    let mut data = flat();
    data[2].high = 111.0;
    data[8].high = 111.0;
    let ignored = built_on(
        Tool::LongPosition,
        &[at(2, 100.0), at(5, 110.0), at(5, 95.0)],
        Tool::LongPosition.default_style(),
        &data[..3],
        false,
    );
    assert!(
        ignored.labels[2].text.contains("Open P&L"),
        "{}",
        ignored.labels[2].text
    );
}

#[test]
fn a_position_with_its_stop_on_the_wrong_side_has_no_size_to_state() {
    // A long whose stop is above its entry: no risk to size, so no quantity or reward over risk.
    let screen = built(
        Tool::LongPosition,
        &[at(2, 100.0), at(5, 110.0), at(5, 105.0)],
    );
    let middle = &screen.labels[2].text;
    assert!(middle.contains("P&L: —"), "{middle}");
    assert!(middle.contains("Risk/Reward Ratio: —"), "{middle}");
    assert!(!screen.labels.iter().any(|label| label.text.contains("NaN")));
}

#[test]
fn a_position_is_picked_by_its_zones() {
    let screen = built(Tool::LongPosition, &long());
    // Inside the profit zone, nothing painted but the fill colour: still the drawing.
    assert!(screen.hits_body(pos2(400.0, 150.0), 4.0, false, false));
    assert!(screen.hits_body(pos2(400.0, 230.0), 4.0, false, false));
    assert!(!screen.hits_body(pos2(100.0, 150.0), 4.0, false, false));
    assert!(!screen.hits_body(pos2(400.0, 300.0), 4.0, false, false));
}

#[test]
fn the_stop_stays_under_the_target_and_the_position_can_run_either_way() {
    // Drawn right to left (the target before the entry in time) it is the same box.
    let screen = built(
        Tool::LongPosition,
        &[at(5, 100.0), at(2, 110.0), at(2, 95.0)],
    );
    assert_eq!(
        screen.boxes[0].rect,
        Rect::from_min_max(pos2(250.0, 100.0), pos2(550.0, 200.0))
    );
}

#[test]
fn a_new_position_is_sized_from_the_view() {
    let data = flat();
    let scale = scale(false);
    let proj = projection(&data, &scale);
    // A click at (250, 200): the target a tenth of the plot above, the stop half that below, the
    // right edge a fifth of its width on: x = 450 (bar 4), y = 160 (104) and 220 (98).
    let sized = forecast_shapes::sized_position(Tool::LongPosition, at(2, 100.0), &proj).unwrap();
    assert_eq!(sized[0], at(2, 100.0));
    assert_eq!(sized[1].time_ms, 4 * 60_000);
    assert_eq!(sized[2].time_ms, 4 * 60_000);
    assert!((sized[1].price - 104.0).abs() < 1e-6);
    assert!((sized[2].price - 98.0).abs() < 1e-6);
    // A short is the other way up.
    let short = forecast_shapes::sized_position(Tool::ShortPosition, at(2, 100.0), &proj).unwrap();
    assert!((short[1].price - 96.0).abs() < 1e-6 && (short[2].price - 102.0).abs() < 1e-6);
    // And it makes a drawing.
    assert!(Drawing::build(Tool::LongPosition, 1, &sized, DrawingStyle::default()).is_some());
    assert!(forecast_shapes::sized_position(Tool::Rectangle, at(2, 100.0), &proj).is_none());
}

#[test]
fn a_position_works_on_a_logarithmic_scale_too() {
    // The numbers come from the prices; only where the levels sit moves.
    let linear = built(Tool::LongPosition, &long());
    let log = built_on(
        Tool::LongPosition,
        &long(),
        Tool::LongPosition.default_style(),
        &flat(),
        true,
    );
    assert_eq!(texts(&linear), texts(&log));
    assert_ne!(linear.boxes[0].rect, log.boxes[0].rect);
}

// MARK: Position forecast

#[test]
fn a_forecast_is_an_arrow_with_its_source_and_target_and_how_it_turned_out() {
    let screen = built(Tool::PositionForecast, &[at(2, 100.0), at(6, 110.0)]);
    // From (250, 200) to (650, 100), headed.
    assert!(near(screen.lines[0][0], pos2(250.0, 200.0)));
    assert_eq!(screen.solids.len(), 1);
    assert_eq!(
        texts(&screen),
        // The source price; the move (+10 or +10 %) and the time; and the bars never got there.
        ["100.00", "+10.00 (+10.00%)\n5 bars · 4m", "Failure"]
    );
    assert_eq!(screen.handles.len(), 2);
}

#[test]
fn a_forecast_succeeds_when_a_bar_reaches_the_target() {
    let mut data = flat();
    data[4].high = 111.0;
    let screen = built_on(
        Tool::PositionForecast,
        &[at(2, 100.0), at(6, 110.0)],
        Tool::PositionForecast.default_style(),
        &data,
        false,
    );
    assert_eq!(screen.labels.last().unwrap().text, "Success");
    // And says nothing until the time is up: the target bar not yet there.
    let early = built_on(
        Tool::PositionForecast,
        &[at(2, 100.0), at(6, 110.0)],
        Tool::PositionForecast.default_style(),
        &flat()[..5],
        false,
    );
    assert_eq!(early.labels.len(), 2, "no result yet");
}

// MARK: Bars pattern

fn pattern(mode: BarsMode) -> Screen {
    let data = flat();
    let mut copy = BarsPattern::capture(&data, 0, 2, at(5, 100.0)).unwrap();
    copy.mode = mode;
    let drawing =
        Drawing::from_geometry(1, Geometry::Bars(copy), Tool::BarsPattern.default_style()).unwrap();
    let scale = scale(false);
    shape::build(&drawing, &projection(&data, &scale)).expect("projected")
}

#[test]
fn a_bars_pattern_sets_the_copied_bars_down_from_its_anchor() {
    let screen = pattern(BarsMode::Bars);
    // Three bars on the bars after bar 5's slot: x = 550, 650, 750, each from 101 to 99 around a
    // close at the anchor's price (y = 200).
    assert_eq!(screen.candles.len(), 3);
    for (candle, x) in screen.candles.iter().zip([550.0, 650.0, 750.0]) {
        assert!((candle.x - x).abs() < 0.05);
        assert!((candle.high - 190.0).abs() < 0.05 && (candle.low - 210.0).abs() < 0.05);
        assert!((candle.open - 200.0).abs() < 0.05 && (candle.close - 200.0).abs() < 0.05);
        assert!(matches!(candle.look, CandleLook::Stick { .. }));
    }
    assert_eq!(screen.handles.len(), 1);
    assert!(near(screen.handles[0].pos, pos2(550.0, 200.0)));
    // Each bar can be picked.
    assert_eq!(screen.hit.len(), 3);
    assert!(screen.hits_body(pos2(650.0, 200.0), 4.0, false, false));
}

#[test]
fn a_bars_pattern_can_be_candles_or_a_line() {
    let candles = pattern(BarsMode::Candles);
    assert!(matches!(candles.candles[0].look, CandleLook::Body { .. }));
    let line = pattern(BarsMode::LineClose);
    assert!(line.candles.is_empty());
    assert_eq!(line.lines.len(), 1);
    assert_eq!(line.lines[0].len(), 3);
    assert!(near(line.lines[0][1], pos2(650.0, 200.0)));
    // The high follows the highs: 101 is y = 190.
    let high = pattern(BarsMode::LineHigh);
    assert!(near(high.lines[0][0], pos2(550.0, 190.0)));
}

#[test]
fn a_bars_pattern_is_its_own_copy_and_follows_the_options() {
    let data = flat();
    // Changing the chart's bars afterwards changes nothing about the drawing.
    let copy = BarsPattern::capture(&data, 0, 2, at(5, 100.0)).unwrap();
    let before = copy.shown();
    let mut changed = data.clone();
    for bar in &mut changed {
        bar.high += 50.0;
    }
    assert_eq!(copy.shown(), before);
    // Mirrored, the bars hang the other way: the high (101) becomes 99 and the low 101.
    let mut mirrored = copy.clone();
    mirrored.mirrored = true;
    let drawing = Drawing::from_geometry(
        1,
        Geometry::Bars(mirrored),
        Tool::BarsPattern.default_style(),
    )
    .unwrap();
    let scale = scale(false);
    let screen = shape::build(&drawing, &projection(&data, &scale)).unwrap();
    assert!(
        (screen.candles[0].high - 190.0).abs() < 0.05,
        "high and low swap, the bar is the same"
    );
}

#[test]
fn three_clicks_copy_the_bars_between_the_first_two() {
    let data = flat();
    let scale = scale(false);
    let proj = projection(&data, &scale);
    // Bars 1 to 4 copied, set down at bar 7: four bars.
    let geometry =
        forecast_shapes::captured_pattern(&[at(1, 100.0), at(4, 100.0), at(7, 100.0)], &proj)
            .unwrap();
    let Geometry::Bars(copy) = geometry else {
        panic!("a bars pattern");
    };
    assert_eq!(copy.bars.len(), 4);
    assert_eq!(copy.at, at(7, 100.0));
    assert_eq!((copy.from_ms, copy.to_ms), (60_000, 4 * 60_000));
    // Two clicks only choose the range, shown as a date range; fewer say nothing.
    assert!(matches!(
        forecast_shapes::bars_preview(&[at(1, 100.0), at(4, 100.0)], &proj),
        Some(Geometry::Measure { .. })
    ));
    assert!(forecast_shapes::bars_preview(&[at(1, 100.0)], &proj).is_none());
    assert!(forecast_shapes::captured_pattern(&[at(1, 100.0), at(4, 100.0)], &proj).is_none());
}

// MARK: Ghost feed

#[test]
fn a_ghost_feed_is_candles_from_the_first_price_to_the_second() {
    let screen = built(Tool::GhostFeed, &[at(2, 100.0), at(6, 110.0)]);
    // Four candles, on bars 3 to 6: x = 350, 450, 550, 650; the first opens at the first price
    // (y = 200) and the last closes on the second (y = 100).
    assert_eq!(screen.candles.len(), 4);
    for (candle, x) in screen.candles.iter().zip([350.0, 450.0, 550.0, 650.0]) {
        assert!((candle.x - x).abs() < 0.05);
        assert!(matches!(candle.look, CandleLook::Body { .. }));
    }
    assert!((screen.candles[0].open - 200.0).abs() < 0.05);
    assert!((screen.candles[3].close - 100.0).abs() < 0.05);
    // The same every time it is drawn.
    let again = built(Tool::GhostFeed, &[at(2, 100.0), at(6, 110.0)]);
    for (a, b) in screen.candles.iter().zip(&again.candles) {
        assert_eq!((a.high, a.low, a.close), (b.high, b.low, b.close));
    }
    assert_eq!(screen.handles.len(), 2);
}

#[test]
fn a_ghost_feed_can_be_drawn_backwards_and_nothing_else_is_made_real() {
    // From the later anchor to the earlier: the same candles, running from the earlier price.
    let forwards = built(Tool::GhostFeed, &[at(2, 100.0), at(6, 110.0)]);
    let backwards = built(Tool::GhostFeed, &[at(6, 110.0), at(2, 100.0)]);
    assert_eq!(forwards.candles.len(), backwards.candles.len());
    assert!((backwards.candles[0].open - 200.0).abs() < 0.05);
    // A run to a price on the same bar is one candle; the data is never touched.
    let one = built(Tool::GhostFeed, &[at(2, 100.0), at(2, 104.0)]);
    assert_eq!(one.candles.len(), 1);
}

#[test]
fn a_ghost_feeds_candles_are_as_big_as_the_bars_before_them_unless_told() {
    // Bars of 2 from high to low: the automatic average is 2 (20 px at 10 px a price); a variance
    // of nothing keeps every candle at exactly that.
    let mut style = Tool::GhostFeed.default_style();
    style.ghost.variance = 0.0;
    let auto = built_on(
        Tool::GhostFeed,
        &[at(2, 100.0), at(6, 108.0)],
        style,
        &flat(),
        false,
    );
    for candle in &auto.candles {
        let range = (candle.low - candle.high).abs();
        assert!((range - 20.0).abs() < 0.05, "{range}");
    }
    // Asked for 5 % of 100: a range of 5, 50 px.
    style.ghost.average = 5.0;
    let asked = built_on(
        Tool::GhostFeed,
        &[at(2, 100.0), at(6, 108.0)],
        style,
        &flat(),
        false,
    );
    for candle in &asked.candles {
        let range = (candle.low - candle.high).abs();
        assert!(range >= 49.9, "{range}");
    }
}

// MARK: Sector

#[test]
fn a_sector_is_a_wedge_of_the_circle_from_its_centre_between_two_rays() {
    // Centre (250, 200); the radius point (550, 200) sets a radius of 300 and the first ray; the
    // tip (450, 100) the second, up and to the right (an angle of -0.4636 rad on the screen).
    let screen = built(Tool::Sector, &[at(2, 100.0), at(5, 100.0), at(4, 110.0)]);
    assert_eq!(screen.outlines.len(), 1);
    let wedge = &screen.outlines[0];
    assert!(near(wedge[0], pos2(250.0, 200.0)), "from the centre");
    assert!(near(wedge[1], pos2(550.0, 200.0)), "along the first ray");
    let end = *wedge.last().unwrap();
    assert!(
        (end.distance(pos2(250.0, 200.0)) - 300.0).abs() < 0.2,
        "{end:?}"
    );
    assert!(
        (end.x - 518.3).abs() < 0.5 && (end.y - 65.8).abs() < 0.5,
        "{end:?}"
    );
    // Every point on the arc is a radius from the centre.
    for point in &wedge[1..] {
        assert!((point.distance(pos2(250.0, 200.0)) - 300.0).abs() < 0.2);
    }
    assert_eq!(screen.handles.len(), 3);
    assert_eq!(
        screen.guides.len(),
        1,
        "the line to the tip, while selected"
    );
}

#[test]
fn a_sector_takes_the_short_way_round() {
    // The tip just below the first ray sweeps the other way, not nearly all the way round.
    let screen = built(Tool::Sector, &[at(2, 100.0), at(5, 100.0), at(4, 90.0)]);
    let end = *screen.outlines[0].last().unwrap();
    assert!(end.y > 200.0, "swept downwards: {end:?}");
    // A zero radius is just its handles.
    let none = built(Tool::Sector, &[at(2, 100.0), at(2, 100.5), at(4, 110.0)]);
    assert!(none.outlines.len() <= 1);
}

// MARK: Measurers

#[test]
fn a_price_range_is_a_box_between_two_prices_with_an_arrow_and_its_numbers() {
    let screen = built(Tool::PriceRange, &[at(2, 100.0), at(5, 112.0)]);
    // From (250, 200) to (550, 80): the box over the drag's width.
    assert_eq!(screen.outlines.len(), 1);
    assert!(near(screen.outlines[0][0], pos2(250.0, 80.0)));
    assert!(near(screen.outlines[0][2], pos2(550.0, 200.0)));
    // An arrow from the first price to the second down the middle.
    assert!(near(screen.lines[0][0], pos2(400.0, 200.0)));
    assert_eq!(screen.solids.len(), 1);
    // +12 on 100: 12 %, and 1,200 steps of a cent.
    assert_eq!(texts(&screen), ["+12.00 (+12.00%) +1,200"]);
    // The label sits beyond the box, on the side the price went.
    assert!(near(screen.labels[0].at, pos2(400.0, 80.0)));
    assert_eq!(screen.labels[0].push.y, -1.0);
    assert!(matches!(
        screen.labels[0].look,
        LabelLook::Plate { fill, .. } if fill == DrawingStyle::default().measure.label
    ));
}

#[test]
fn a_price_range_downwards_reads_negative_and_hangs_below() {
    let screen = built(Tool::PriceRange, &[at(2, 100.0), at(5, 90.0)]);
    assert_eq!(texts(&screen), ["-10.00 (-10.00%) -1,000"]);
    assert_eq!(screen.labels[0].push.y, 1.0);
}

#[test]
fn a_date_range_is_a_box_between_two_times_over_the_height_of_the_plot() {
    let screen = built(Tool::DateRange, &[at(2, 100.0), at(6, 100.0)]);
    assert!(near(screen.outlines[0][0], pos2(250.0, PLOT.top())));
    assert!(near(screen.outlines[0][2], pos2(650.0, PLOT.bottom())));
    // Five bars (both ends count), four minutes, and the volume of those five bars.
    assert_eq!(texts(&screen), ["5 bars, 4m\nVolume: 5"]);
    // An arrow along the time at the height of the anchors.
    assert!(near(screen.lines[0][0], pos2(250.0, 200.0)));
}

#[test]
fn a_date_and_price_range_says_both() {
    let screen = built(Tool::DateAndPriceRange, &[at(2, 100.0), at(6, 108.0)]);
    assert_eq!(
        texts(&screen),
        ["+8.00 (+8.00%) +800\n5 bars, 4m\nVolume: 5"]
    );
    assert_eq!(screen.lines.len(), 2, "an arrow each way");
}

#[test]
fn the_measurers_print_only_what_they_are_set_to_and_never_a_volume_that_is_not_there() {
    let mut style = Tool::DateAndPriceRange.default_style();
    style.measure.percent = false;
    style.measure.ticks = false;
    style.measure.bars = false;
    style.measure.volume = false;
    let screen = built_on(
        Tool::DateAndPriceRange,
        &[at(2, 100.0), at(6, 108.0)],
        style,
        &flat(),
        false,
    );
    assert_eq!(texts(&screen), ["+8.00\n4m"]);
    // A market with no volume has none to report.
    let mut silent = flat();
    for bar in &mut silent {
        bar.volume = 0.0;
    }
    let screen = built_on(
        Tool::DateRange,
        &[at(2, 100.0), at(6, 100.0)],
        Tool::DateRange.default_style(),
        &silent,
        false,
    );
    assert_eq!(texts(&screen), ["5 bars, 4m"]);
    // Nothing on at all: no label.
    let mut none = Tool::PriceRange.default_style();
    none.measure.price = false;
    none.measure.percent = false;
    none.measure.ticks = false;
    assert!(
        built_on(
            Tool::PriceRange,
            &[at(2, 100.0), at(5, 112.0)],
            none,
            &flat(),
            false
        )
        .labels
        .is_empty()
    );
}

#[test]
fn a_measurement_is_the_same_on_every_price_scale() {
    let linear = built(Tool::PriceRange, &[at(2, 100.0), at(5, 112.0)]);
    let log = built_on(
        Tool::PriceRange,
        &[at(2, 100.0), at(5, 112.0)],
        Tool::PriceRange.default_style(),
        &flat(),
        true,
    );
    assert_eq!(texts(&linear), texts(&log));
}

// MARK: Anchored VWAP

/// Twelve bars whose close (and so HLC3) steps by 4 from 100, each with the volume of 1.
fn rising() -> Vec<Kline> {
    let closes: Vec<f64> = (0..12).map(|index| 100.0 + 4.0 * index as f64).collect();
    bars(&closes)
}

#[test]
fn a_vwap_is_the_running_volume_weighted_mean_drawn_from_the_anchor() {
    let screen = built_on(
        Tool::AnchoredVwap,
        &[at(0, 100.0)],
        Tool::AnchoredVwap.default_style(),
        &rising(),
        false,
    );
    let line = &screen.lines[0];
    // Means of 100, 104, 108, 112: 100, 102, 104, 106 (y = 200, 180, 160, 140) at x = 50, 150, ...
    for (index, vwap) in [100.0, 102.0, 104.0, 106.0].into_iter().enumerate() {
        let want = pos2(50.0 + 100.0 * index as f32, (120.0 - vwap) as f32 * 10.0);
        assert!(
            near(line[index], want),
            "{index}: {:?} vs {want:?}",
            line[index]
        );
    }
    // The handle is on the line at the anchor's bar.
    assert!(near(screen.handles[0].pos, pos2(50.0, 200.0)));
}

#[test]
fn the_vwaps_first_band_is_the_volume_weighted_deviation() {
    let screen = built_on(
        Tool::AnchoredVwap,
        &[at(0, 100.0)],
        Tool::AnchoredVwap.default_style(),
        &rising(),
        false,
    );
    // Only the first band is on: an upper and a lower line. At the second bar the prices 100 and
    // 104 have a mean of 102 and a deviation of 2: the band is 104 and 100 (y = 160 and 200).
    assert_eq!(screen.styled.len(), 2);
    assert!(near(screen.styled[0].path[1], pos2(150.0, 160.0)));
    assert!(near(screen.styled[1].path[1], pos2(150.0, 200.0)));
    // And the area between is shaded.
    assert_eq!(screen.shades.len(), 1);
    // A band turned on and one turned off.
    let mut style = Tool::AnchoredVwap.default_style();
    style.vwap.bands[1].enabled = true;
    style.vwap.bands[0].enabled = false;
    style.vwap.fill = false;
    let second = built_on(Tool::AnchoredVwap, &[at(0, 100.0)], style, &rising(), false);
    assert_eq!(second.styled.len(), 2);
    // Two deviations at the second bar: 106 and 98 (y = 140 and 220).
    assert!(near(second.styled[0].path[1], pos2(150.0, 140.0)));
    assert!(second.shades.is_empty());
}

#[test]
fn a_vwap_anchored_later_forgets_what_came_before() {
    let screen = built_on(
        Tool::AnchoredVwap,
        &[at(2, 100.0)],
        Tool::AnchoredVwap.default_style(),
        &rising(),
        false,
    );
    // From bar 2 (price 108): the first point is bar 2's own HLC3, y = 120 - 108 = 120.
    assert!(near(screen.lines[0][0], pos2(250.0, 120.0)));
}

#[test]
fn a_vwap_with_no_volume_says_so_instead_of_inventing_a_line() {
    let mut silent = rising();
    for bar in &mut silent {
        bar.volume = 0.0;
    }
    let screen = built_on(
        Tool::AnchoredVwap,
        &[at(0, 100.0)],
        Tool::AnchoredVwap.default_style(),
        &silent,
        false,
    );
    assert!(screen.lines.is_empty() && screen.styled.is_empty());
    assert_eq!(texts(&screen), ["No volume data"]);
}

// MARK: Volume profile

/// Bars flat on 100 (twice) and 108, each of volume 1 and 1 high on either side.
fn clustered() -> Vec<Kline> {
    bars(&[100.0, 100.0, 108.0])
}

fn profile_style(rows: u32) -> DrawingStyle {
    let mut style = Tool::FixedRangeVolumeProfile.default_style();
    style.profile.rows = rows;
    style
}

#[test]
fn a_profile_is_a_histogram_of_each_rows_volume_beside_the_range() {
    // Bars 0 to 2 span x = 0 to 300 and prices 99 to 109 in four rows of 2.5: the first two bars
    // (99 to 101) are all in the lowest row (volume 2), the third (107 to 109) in the top (1).
    let screen = built_on(
        Tool::FixedRangeVolumeProfile,
        &[at(0, 100.0), at(2, 100.0)],
        profile_style(4),
        &clustered(),
        false,
    );
    let options = profile_style(4).profile;
    assert_eq!(screen.boxes.len(), 2, "the two rows with volume");
    // The longest bar is 70 % of 300 wide, from the range's left edge: the bottom row (99 to 101.5,
    // y = 210 to 185) is the longest, the top (106.5 to 109, y = 135 to 110) half of it. All
    // of it is down volume (the bars close where they open), inside the value area.
    let same = |rect: Rect, left: f32, top: f32, right: f32, bottom: f32| {
        near(rect.left_top(), pos2(left, top)) && near(rect.right_bottom(), pos2(right, bottom))
    };
    assert!(
        same(screen.boxes[0].rect, 0.0, 185.0, 210.0, 210.0),
        "{:?}",
        screen.boxes[0].rect
    );
    assert_eq!(screen.boxes[0].color, options.down);
    assert!(
        same(screen.boxes[1].rect, 0.0, 110.0, 105.0, 135.0),
        "{:?}",
        screen.boxes[1].rect
    );
}

#[test]
fn the_point_of_control_and_the_value_area_are_drawn_across_the_range() {
    let screen = built_on(
        Tool::FixedRangeVolumeProfile,
        &[at(0, 100.0), at(2, 100.0)],
        profile_style(4),
        &clustered(),
        false,
    );
    let options = profile_style(4).profile;
    // The point of control is the middle of the lowest row: 100.25, y = 197.5.
    let poc = &screen.styled[0];
    assert_eq!(poc.color, options.poc_color);
    assert!(near(poc.path[0], pos2(0.0, 197.5)) && near(poc.path[1], pos2(300.0, 197.5)));
    // The value area (70 % of 3 = 2.1: the lowest row has 2, so it grows by the pair above it,
    // which holds the top row's 1) spans every row: its edges are 109 and 99.
    assert_eq!(screen.styled.len(), 3);
    let edges: Vec<f32> = screen.styled[1..]
        .iter()
        .map(|line| line.path[0].y)
        .collect();
    assert!(near(pos2(0.0, edges[0]), pos2(0.0, 110.0)));
    assert!(near(pos2(0.0, edges[1]), pos2(0.0, 210.0)));
    assert!(
        screen.styled[1..]
            .iter()
            .all(|line| line.dash == LineDash::Dashed)
    );
}

#[test]
fn rows_outside_the_value_area_are_dimmer() {
    // Nearly all the volume in the lowest row: the value area is that row alone, and the faint
    // volume at the top is outside it.
    let mut data = bars(&[
        100.0, 100.0, 100.0, 100.0, 100.0, 100.0, 100.0, 100.0, 100.0, 100.0, 100.0, 108.0,
    ]);
    data[11].volume = 0.1;
    let screen = built_on(
        Tool::FixedRangeVolumeProfile,
        &[at(0, 100.0), at(11, 100.0)],
        profile_style(4),
        &data,
        false,
    );
    let options = profile_style(4).profile;
    assert_eq!(screen.boxes[0].color, options.down);
    let faint = screen.boxes.last().unwrap().color;
    assert!(faint[3] < options.down[3], "{faint:?}");
    assert_eq!(faint[0..3], options.down[0..3]);
}

#[test]
fn the_histogram_can_show_total_up_down_or_delta_and_grow_from_either_side() {
    let mut data = clustered();
    data[0].close = 101.0; // an up bar
    let mut style = profile_style(4);
    style.profile.volume = ProfileVolume::UpDown;
    let screen = built_on(
        Tool::FixedRangeVolumeProfile,
        &[at(0, 100.0), at(2, 100.0)],
        style,
        &data,
        false,
    );
    // The lowest row holds one up and one down volume: stacked, up first (a half each of 210).
    assert_eq!(screen.boxes[0].color, style.profile.up);
    assert_eq!(screen.boxes[1].color, style.profile.down);
    assert!((screen.boxes[0].rect.width() - 105.0).abs() < 0.05);
    assert!((screen.boxes[1].rect.left() - 105.0).abs() < 0.05);
    // Total is one colour at the full length.
    style.profile.volume = ProfileVolume::Total;
    let total = built_on(
        Tool::FixedRangeVolumeProfile,
        &[at(0, 100.0), at(2, 100.0)],
        style,
        &data,
        false,
    );
    assert!((total.boxes[0].rect.width() - 210.0).abs() < 0.05);
    assert_eq!(total.boxes[0].color, style.profile.up);
    // Placed on the right, the bars grow leftwards from the range's right edge (x = 300).
    style.profile.right = true;
    let right = built_on(
        Tool::FixedRangeVolumeProfile,
        &[at(0, 100.0), at(2, 100.0)],
        style,
        &data,
        false,
    );
    assert!((right.boxes[0].rect.right() - 300.0).abs() < 0.05);
    assert!((right.boxes[0].rect.left() - 90.0).abs() < 0.05);
}

#[test]
fn an_anchored_profile_runs_from_its_bar_to_the_last() {
    let screen = built_on(
        Tool::AnchoredVolumeProfile,
        &[at(1, 100.0)],
        Tool::AnchoredVolumeProfile.default_style(),
        &flat(),
        false,
    );
    // Bars 1 to 11: x = 100 to 1200, the range's right edge past the plot.
    let poc = &screen.styled[0];
    assert!((poc.path[0].x - 100.0).abs() < 0.05);
    assert!((poc.path[1].x - 1_150.0).abs() < 0.05 + 50.0);
    assert!(!screen.boxes.is_empty());
}

#[test]
fn a_profile_with_no_volume_says_so() {
    let mut silent = flat();
    for bar in &mut silent {
        bar.volume = 0.0;
    }
    let screen = built_on(
        Tool::FixedRangeVolumeProfile,
        &[at(0, 100.0), at(5, 100.0)],
        Tool::FixedRangeVolumeProfile.default_style(),
        &silent,
        false,
    );
    assert!(screen.boxes.is_empty());
    assert_eq!(texts(&screen), ["No volume data"]);
}

#[test]
fn a_profile_can_be_picked_by_a_bar_and_not_by_the_gap_beside_it() {
    let screen = built_on(
        Tool::FixedRangeVolumeProfile,
        &[at(0, 100.0), at(2, 100.0)],
        profile_style(4),
        &clustered(),
        false,
    );
    assert!(screen.hits_body(pos2(100.0, 200.0), 4.0, false, false));
    // Beyond the longest bar, and between the two rows: nothing to pick.
    assert!(!screen.hits_body(pos2(250.0, 190.0), 4.0, false, false));
    assert!(!screen.hits_body(pos2(50.0, 160.0), 4.0, false, false));
}

// MARK: The cache

#[test]
fn a_change_in_the_newest_bars_volume_rebuilds_a_vwap_but_not_a_pan() {
    let mut data = flat();
    let scale = scale(false);
    let before = projection(&data, &scale).key();
    data[11].volume += 5.0;
    let after = projection(&data, &scale).key();
    assert!(
        before != after,
        "a live volume tick must refresh a VWAP or a profile"
    );
    // The same bars again: the same key.
    data[11].volume -= 5.0;
    assert!(before == projection(&data, &scale).key());
}
