#![allow(clippy::field_reassign_with_default)]
use super::*;
use crate::model::ChartAppearance;

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
}

// MARK: Defaults and persistence

#[test]
fn the_default_canvas_follows_the_theme_everywhere() {
    let canvas = ChartCanvas::default();
    assert_eq!(canvas.background.mode, BackgroundMode::Solid);
    assert_eq!(canvas.background.solid, None);
    assert_eq!(canvas.vertical_grid.color, None);
    assert_eq!(canvas.horizontal_grid.color, None);
    assert_eq!(canvas.scales.text_color, None);
    assert_eq!(canvas.scales.line_color, None);
    assert_eq!(canvas.watermark.mode, WatermarkMode::Hidden);
    assert_eq!(canvas.crosshair.dash, LineDash::Dashed);
    assert_eq!(canvas.buttons.navigation, ButtonVisibility::OnHover);
    assert!(canvas.ohlc_readout);
    assert!(canvas.bar_countdown);
    assert_eq!(
        canvas.margins,
        ChartMargins {
            top: 10,
            bottom: 8,
            right: 10
        }
    );
}

#[test]
fn the_canvas_round_trips_through_json() {
    let mut canvas = ChartCanvas::default();
    canvas.background.mode = BackgroundMode::Gradient;
    canvas.background.top = Some([1, 2, 3, 255]);
    canvas.background.bottom = Some([4, 5, 6, 128]);
    canvas.vertical_grid.visible = false;
    canvas.horizontal_grid.dash = LineDash::Dotted;
    canvas.crosshair.width = 3;
    canvas.watermark.mode = WatermarkMode::Replay;
    canvas.scales.text_size = ScaleTextSize::Pt14;
    canvas.buttons.pane = ButtonVisibility::Never;
    canvas.margins.right = 42;
    canvas.ohlc_readout = false;
    canvas.bar_countdown = false;
    let encoded = serde_json::to_string(&canvas).expect("serialize");
    let decoded: ChartCanvas = serde_json::from_str(&encoded).expect("deserialize");
    assert_eq!(decoded, canvas);
}

#[test]
fn an_old_appearance_without_a_canvas_keeps_its_palette_and_gets_the_default_canvas() {
    let mut appearance = ChartAppearance::default();
    appearance.bullish = [1, 2, 3];
    appearance.axis_decimals = Some(4);
    let mut row = serde_json::to_value(appearance).expect("serialize");
    row.as_object_mut().expect("object").remove("canvas");
    let restored: ChartAppearance = serde_json::from_value(row).expect("decode");
    assert_eq!(restored.bullish, [1, 2, 3]);
    assert_eq!(restored.axis_decimals, Some(4));
    assert_eq!(restored.canvas, ChartCanvas::default());
}

#[test]
fn a_canvas_saved_before_the_readout_setting_shows_the_readout() {
    let mut row = serde_json::to_value(ChartCanvas::default()).expect("serialize");
    row.as_object_mut()
        .expect("object")
        .remove("ohlc_readout")
        .expect("the key is written");
    let restored: ChartCanvas = serde_json::from_value(row).expect("decode");
    assert!(restored.ohlc_readout);
}

#[test]
fn a_canvas_saved_before_the_countdown_setting_shows_the_countdown() {
    let mut row = serde_json::to_value(ChartCanvas::default()).expect("serialize");
    row.as_object_mut().expect("object").remove("bar_countdown");
    let restored: ChartCanvas = serde_json::from_value(row).expect("decode");
    assert!(restored.bar_countdown);
}

#[test]
fn the_countdown_is_written_only_when_it_is_not_the_default() {
    let key = |canvas: ChartCanvas| {
        serde_json::to_value(canvas)
            .expect("serialize")
            .get("bar_countdown")
            .cloned()
    };
    // A build that predates the setting reads and writes the same row back until it is turned off.
    assert_eq!(key(ChartCanvas::default()), None);
    let mut off = ChartCanvas::default();
    off.bar_countdown = false;
    assert_eq!(key(off), Some(serde_json::Value::Bool(false)));
}

#[test]
fn a_canvas_with_only_some_fields_fills_the_rest_with_defaults() {
    let decoded: ChartCanvas =
        serde_json::from_str(r#"{"margins":{"top":20},"crosshair":{"width":2}}"#).expect("decode");
    assert_eq!(decoded.margins.top, 20);
    assert_eq!(decoded.margins.bottom, 8);
    assert_eq!(decoded.crosshair.width, 2);
    assert_eq!(decoded.crosshair.dash, LineDash::Solid);
    assert!(decoded.crosshair.visible);
    assert_eq!(decoded.vertical_grid, LineAppearance::default());
}

// MARK: Enums

#[test]
fn background_modes_and_line_styles_have_labels() {
    assert_eq!(
        BackgroundMode::ALL.map(BackgroundMode::label),
        ["Solid", "Gradient"]
    );
    assert_eq!(
        LineDash::ALL.map(LineDash::label),
        ["Solid", "Dashed", "Dotted"]
    );
}

#[test]
fn a_watermark_shows_by_mode_and_replay_state() {
    assert!(!WatermarkMode::Hidden.shows(true));
    assert!(!WatermarkMode::Hidden.shows(false));
    assert!(WatermarkMode::Symbol.shows(false));
    assert!(WatermarkMode::Symbol.shows(true));
    // Replay mode with no replay running falls back to nothing, never to the symbol.
    assert!(!WatermarkMode::Replay.shows(false));
    assert!(WatermarkMode::Replay.shows(true));
}

#[test]
fn button_visibility_decides_by_hover() {
    assert!(ButtonVisibility::Always.wanted(false));
    assert!(ButtonVisibility::OnHover.wanted(true));
    assert!(!ButtonVisibility::OnHover.wanted(false));
    assert!(!ButtonVisibility::Never.wanted(true));
}

#[test]
fn scale_text_sizes_ascend_and_the_time_axis_is_one_point_smaller() {
    let points = ScaleTextSize::ALL.map(ScaleTextSize::points);
    assert!(points.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(ScaleTextSize::default().points(), 10.0);
    assert_eq!(ScaleTextSize::Pt14.time_points(), 13.0);
    assert_eq!(ScaleTextSize::Pt12.label(), "12 pt");
}

// MARK: Margins

#[test]
fn margins_are_forced_into_their_bounds() {
    let wild = ChartMargins {
        top: 200,
        bottom: 99,
        right: 60_000,
    }
    .sanitized();
    assert_eq!(wild.top + wild.bottom, ChartMargins::MAX_COMBINED);
    assert!(wild.top <= ChartMargins::MAX_VERTICAL);
    assert!(wild.bottom <= ChartMargins::MAX_VERTICAL);
    assert_eq!(wild.right, ChartMargins::MAX_RIGHT);
    let fine = ChartMargins {
        top: 30,
        bottom: 20,
        right: 5,
    };
    assert_eq!(fine.sanitized(), fine);
}

#[test]
fn a_canvas_with_bad_values_is_healed_as_a_whole() {
    let mut canvas = ChartCanvas::default();
    canvas.crosshair.width = 99;
    canvas.vertical_grid.width = 0;
    canvas.margins.top = 255;
    let healed = canvas.sanitized();
    assert_eq!(healed.crosshair.width, 4);
    assert_eq!(healed.vertical_grid.width, 1);
    assert_eq!(healed.margins.top, ChartMargins::MAX_VERTICAL);
}

#[test]
fn margins_place_the_data_by_their_share_of_the_height() {
    let margins = ChartMargins {
        top: 10,
        bottom: 20,
        right: 0,
    };
    let (minimum, maximum) = price_range(100.0, 200.0, margins, false);
    let total = maximum - minimum;
    // The data spans 70% of the plot, 20% sits below it and 10% above.
    assert!(close((100.0 - minimum) / total, 0.20));
    assert!(close((maximum - 200.0) / total, 0.10));
    assert!(close(100.0 / total, 100.0 / total));
}

#[test]
fn zero_margins_fit_the_data_edge_to_edge() {
    let margins = ChartMargins {
        top: 0,
        bottom: 0,
        right: 0,
    };
    assert_eq!(price_range(5.0, 9.0, margins, false), (5.0, 9.0));
}

#[test]
fn logarithmic_margins_are_the_same_share_of_the_height() {
    let margins = ChartMargins {
        top: 10,
        bottom: 10,
        right: 0,
    };
    let (minimum, maximum) = price_range(10.0, 1000.0, margins, true);
    let (low, high) = (minimum.ln(), maximum.ln());
    assert!(close((10f64.ln() - low) / (high - low), 0.10));
    assert!(close((high - 1000f64.ln()) / (high - low), 0.10));
    assert!(minimum > 0.0);
}

#[test]
fn a_flat_series_still_gets_a_scale_around_it() {
    for logarithmic in [false, true] {
        let (minimum, maximum) = price_range(42.0, 42.0, ChartMargins::default(), logarithmic);
        assert!(minimum < 42.0 && 42.0 < maximum, "{logarithmic}");
    }
    let (minimum, maximum) = price_range(0.0, 0.0, ChartMargins::default(), false);
    assert!(minimum < 0.0 && 0.0 < maximum);
}

#[test]
fn the_price_range_stays_finite_for_extreme_and_bad_input() {
    let cases = [
        (1e-12, 3e-12),
        (1e15, 2e15),
        (-500.0, -100.0),
        (f64::NAN, 1.0),
        (f64::INFINITY, 1.0),
        (9.0, 3.0),
        (0.0, 1e300),
    ];
    for (low, high) in cases {
        for logarithmic in [false, true] {
            let (minimum, maximum) = price_range(low, high, ChartMargins::default(), logarithmic);
            assert!(
                minimum.is_finite() && maximum.is_finite() && minimum < maximum,
                "{low} {high} {logarithmic}: {minimum} {maximum}"
            );
        }
    }
}

// MARK: Ticks

#[test]
fn linear_ticks_are_round_multiples_inside_the_range() {
    let ticks = price_ticks(98.0, 207.0, false, 6);
    assert!(!ticks.is_empty() && ticks.len() <= 6, "{ticks:?}");
    assert!(ticks.iter().all(|tick| (98.0..=207.0).contains(tick)));
    let step = ticks[1] - ticks[0];
    assert!(
        close(step, 20.0) || close(step, 25.0) || close(step, 50.0),
        "{step}"
    );
    assert!(ticks.windows(2).all(|pair| close(pair[1] - pair[0], step)));
    assert!(
        ticks
            .iter()
            .all(|tick| close(tick / step, (tick / step).round()))
    );
}

#[test]
fn ticks_cope_with_tiny_and_huge_prices() {
    let tiny = price_ticks(0.000_001_2, 0.000_001_9, false, 5);
    assert!(!tiny.is_empty());
    assert!(
        tiny.iter()
            .all(|tick| *tick >= 0.000_001_2 && *tick <= 0.000_001_9)
    );
    let huge = price_ticks(1.0e9, 3.5e9, false, 5);
    assert!(!huge.is_empty());
    assert!(huge.iter().all(|tick| tick.is_finite()));
}

#[test]
fn ticks_cross_zero_for_negative_values() {
    let ticks = price_ticks(-30.0, 40.0, false, 7);
    assert!(ticks.contains(&0.0), "{ticks:?}");
    assert!(ticks.iter().any(|tick| *tick < 0.0));
}

#[test]
fn logarithmic_ticks_are_decades_and_thin_to_fit() {
    let roomy = price_ticks(1.0, 1000.0, true, 12);
    assert!(roomy.contains(&1.0) && roomy.contains(&10.0) && roomy.contains(&100.0));
    assert!(roomy.contains(&1000.0));
    assert!(roomy.len() > 4, "{roomy:?}");
    let tight = price_ticks(1.0, 1000.0, true, 4);
    assert_eq!(tight, vec![1.0, 10.0, 100.0, 1000.0]);
    let tighter = price_ticks(1e-3, 1e9, true, 4);
    assert!(!tighter.is_empty() && tighter.len() <= 4, "{tighter:?}");
}

#[test]
fn a_narrow_logarithmic_range_falls_back_to_round_prices() {
    let ticks = price_ticks(100.0, 140.0, true, 6);
    assert!(ticks.len() >= 2);
    assert!(ticks.iter().all(|tick| (100.0..=140.0).contains(tick)));
}

#[test]
fn ticks_are_empty_for_a_degenerate_scale() {
    assert!(price_ticks(5.0, 5.0, false, 6).is_empty());
    assert!(price_ticks(f64::NAN, 5.0, false, 6).is_empty());
    assert!(price_ticks(1.0, 5.0, false, 0).is_empty());
    assert!(price_ticks(9.0, 1.0, false, 4).is_empty());
}

#[test]
fn the_axis_holds_its_widest_label_and_a_margin() {
    // 40 points of text and 17 of margin, rounded up to a 4 point step.
    assert_eq!(axis_width(40.0, 44.0, 148.0), 60.0);
    assert_eq!(axis_width(43.0, 44.0, 148.0), 60.0);
    assert_eq!(axis_width(44.0, 44.0, 148.0), 64.0);
}

#[test]
fn the_axis_stays_between_its_floor_and_ceiling() {
    assert_eq!(axis_width(6.0, 44.0, 148.0), 44.0);
    assert_eq!(axis_width(0.0, 44.0, 148.0), 44.0);
    assert_eq!(axis_width(f32::NAN, 44.0, 148.0), 44.0);
    assert_eq!(axis_width(500.0, 44.0, 148.0), 148.0);
    assert_eq!(axis_width(10.0, 60.0, 40.0), 60.0);
}

#[test]
fn a_longer_label_never_narrows_the_axis() {
    let mut last = 0.0;
    for tenths in 0..2000 {
        let width = axis_width(tenths as f32 / 10.0, 44.0, 148.0);
        assert!(width >= last);
        assert_eq!(width % 4.0, 0.0);
        last = width;
    }
}

#[test]
fn the_tick_budget_follows_the_height() {
    assert_eq!(tick_budget(560.0, 56.0), 10);
    assert_eq!(tick_budget(10.0, 56.0), 1);
    assert_eq!(tick_budget(0.0, 56.0), 0);
    assert_eq!(tick_budget(f32::NAN, 56.0), 0);
}

#[test]
fn time_ticks_stay_on_their_bars_while_the_chart_pans() {
    // 20-point slots: a label every fifth bar of the series.
    let at = |first| {
        time_ticks(first, 30, 20.0, 60.0)
            .into_iter()
            .map(|index| first + index)
            .collect::<Vec<_>>()
    };
    let before = at(100);
    let panned = at(103);
    for bar in &before {
        assert!(bar % 5 == 0);
        if (103..130).contains(bar) {
            assert!(panned.contains(bar), "{bar} moved");
        }
    }
    assert_eq!(time_tick_step(20.0, 60.0), 5);
    assert_eq!(time_tick_step(500.0, 60.0), 1);
    assert_eq!(time_tick_step(f32::NAN, 60.0), 100);
}

#[test]
fn wide_time_labels_are_spaced_so_they_never_touch() {
    // A 13-point axis font drawn 1.35 times larger: labels about 116 points wide.
    let (slot, label) = (8.0, 116.0);
    let step = time_tick_step(slot, label);
    assert!(step as f32 * slot >= label + 16.0, "{step}");
    // Narrow labels keep the usual spacing.
    assert_eq!(time_tick_step(slot, 60.0), 13);
    assert_eq!(time_tick_step(slot, f32::NAN), 13);
}

// MARK: Window

#[test]
fn the_window_without_a_margin_is_the_last_visible_bars() {
    let window = visible_window(500, 120, 0.0, 0);
    assert_eq!((window.start, window.end), (380, 500));
    assert_eq!((window.lead, window.slots), (0.0, 120));
    assert_eq!(window.count(), 120);
}

#[test]
fn a_right_margin_reserves_empty_slots_past_the_newest_bar() {
    let window = visible_window(500, 120, 0.0, 10);
    assert_eq!((window.start, window.end), (380, 500));
    assert_eq!(window.slots, 130, "the margin is part of the plot");
    // The newest bar sits ten slots in from the right edge.
    let newest_slot = window.lead as usize + window.count() - 1;
    assert_eq!(window.slots - 1 - newest_slot, 10);
}

#[test]
fn scrolling_back_fills_the_margin_with_later_bars_at_the_same_spacing() {
    let at_rest = visible_window(500, 120, 0.0, 10);
    let scrolled = visible_window(500, 120, 4.0, 10);
    assert_eq!(scrolled.slots, at_rest.slots, "bar spacing does not change");
    assert_eq!(scrolled.end, 500, "still inside the margin");
    assert_eq!(scrolled.start, 376);
    let far = visible_window(500, 120, 30.0, 10);
    assert_eq!((far.start, far.end), (350, 480));
    assert_eq!(far.slots, 130);
    assert_eq!(far.lead, 0.0);
    assert_eq!(far.count(), 130, "the margin's slots show real bars now");
}

#[test]
fn a_short_series_sits_at_the_right_of_the_window() {
    let window = visible_window(30, 120, 0.0, 0);
    assert_eq!((window.start, window.end), (0, 30));
    assert_eq!(window.lead, 90.0);
    let with_margin = visible_window(30, 120, 0.0, 10);
    assert_eq!(
        with_margin.lead, 90.0,
        "the margin is at the right, not the left"
    );
    assert_eq!(with_margin.slots, 130);
}

#[test]
fn an_empty_or_overscrolled_series_has_an_empty_window() {
    assert_eq!(visible_window(0, 120, 0.0, 10).count(), 0);
    assert_eq!(visible_window(10, 120, 500.0, 10).count(), 0);
    let window = visible_window(10, 0, 0.0, 0);
    assert_eq!(window.slots, 1);
}

#[test]
fn a_fractional_offset_cuts_the_bar_at_each_edge() {
    let window = visible_window(500, 120, 0.25, 0);
    // 119.75 bars are whole; the first bar's slot lies a quarter outside the plot.
    assert_eq!((window.start, window.end), (379, 500));
    assert!((window.lead + 0.75).abs() < 1e-6, "{}", window.lead);
    let other = visible_window(500, 120, 3.5, 0);
    assert_eq!((other.start, other.end), (376, 497));
    assert!((other.lead + 0.5).abs() < 1e-6, "{}", other.lead);
}

#[test]
fn a_whole_offset_cuts_no_bar() {
    let window = visible_window(500, 120, 7.0000000001, 0);
    assert_eq!((window.start, window.end), (373, 493));
    assert_eq!(window.lead, 0.0);
}

#[test]
fn a_negative_offset_leaves_empty_slots_after_the_newest_bar() {
    let window = visible_window(500, 120, -10.0, 0);
    assert_eq!((window.start, window.end), (390, 500));
    assert_eq!(window.lead, 0.0);
    assert_eq!(window.slots, 120, "the spacing does not change");
    // The newest bar's slot is 11 from the right edge: ten empty slots follow it.
    let newest_slot = window.lead + (window.count() - 1) as f32;
    assert_eq!(window.slots as f32 - 1.0 - newest_slot, 10.0);
}

#[test]
fn a_fractional_whitespace_offset_shows_the_newest_bar_between_slots() {
    let window = visible_window(500, 120, -2.5, 0);
    assert_eq!(window.end, 500);
    let newest_slot = window.lead + (window.count() - 1) as f32;
    assert!((window.slots as f32 - 1.0 - newest_slot - 2.5).abs() < 1e-5);
}

#[test]
fn a_non_finite_offset_is_the_newest_bars() {
    assert_eq!(
        visible_window(500, 120, f64::NAN, 0),
        visible_window(500, 120, 0.0, 0)
    );
}

// MARK: Panes

const AXIS: f32 = 24.0;

fn spec(id: u64, collapsed: bool) -> PaneSpec {
    PaneSpec {
        id,
        collapsed,
        share: None,
    }
}

#[test]
fn without_panes_the_main_chart_has_everything() {
    let split = split_panes(600.0, &[], None, AXIS);
    assert_eq!(split.main, 600.0);
    assert!(split.panes.is_empty() && !split.main_collapsed);
}

#[test]
fn expanded_panes_share_thirty_percent_and_fill_the_area_exactly() {
    let split = split_panes(600.0, &[spec(1, false), spec(2, false)], None, AXIS);
    assert_eq!(split.panes, vec![90.0, 90.0]);
    assert_eq!(split.main, 420.0);
    let odd = split_panes(
        601.0,
        &[spec(1, false), spec(2, false), spec(3, false)],
        None,
        AXIS,
    );
    assert_eq!(odd.main + odd.panes.iter().sum::<f32>(), 601.0);
}

#[test]
fn a_collapsed_pane_is_its_strip_and_returns_nothing_to_the_others() {
    let split = split_panes(600.0, &[spec(1, true), spec(2, false)], None, AXIS);
    assert_eq!(split.panes[0], COLLAPSED_PANE_HEIGHT);
    assert_eq!(split.panes[1], 180.0);
    assert_eq!(split.main, 600.0 - 180.0 - COLLAPSED_PANE_HEIGHT);
    let all = split_panes(600.0, &[spec(1, true), spec(2, true)], None, AXIS);
    assert_eq!(all.panes, vec![COLLAPSED_PANE_HEIGHT; 2]);
    assert_eq!(all.main, 600.0 - 2.0 * COLLAPSED_PANE_HEIGHT);
}

#[test]
fn a_maximized_pane_takes_everything_but_the_strips_and_the_time_axis() {
    let panes = [spec(1, false), spec(2, false), spec(3, true)];
    let split = split_panes(600.0, &panes, Some(2), AXIS);
    assert!(split.main_collapsed);
    assert_eq!(split.main, AXIS);
    assert_eq!(split.panes[0], COLLAPSED_PANE_HEIGHT);
    assert_eq!(split.panes[2], COLLAPSED_PANE_HEIGHT);
    assert_eq!(split.panes[1], 600.0 - AXIS - 2.0 * COLLAPSED_PANE_HEIGHT);
    assert_eq!(split.main + split.panes.iter().sum::<f32>(), 600.0);
}

#[test]
fn maximizing_a_pane_that_is_gone_or_too_tall_for_the_chart_changes_nothing() {
    let panes = [spec(1, false)];
    let gone = split_panes(600.0, &panes, Some(9), AXIS);
    assert!(!gone.main_collapsed);
    let tiny = split_panes(70.0, &panes, Some(1), AXIS);
    assert!(!tiny.main_collapsed, "no room to maximize into");
}

#[test]
fn a_short_chart_keeps_its_main_area_before_its_panes() {
    let split = split_panes(150.0, &[spec(1, false), spec(2, false)], None, AXIS);
    assert!(split.main >= 60.0, "{split:?}");
    assert!(split.panes.iter().all(|height| *height >= 0.0));
    assert!((split.main + split.panes.iter().sum::<f32>() - 150.0).abs() < 0.001);
    let zero = split_panes(0.0, &[spec(1, false)], None, AXIS);
    assert!(zero.main >= 0.0 && zero.panes.iter().all(|height| *height >= 0.0));
    let nan = split_panes(f32::NAN, &[spec(1, false)], None, AXIS);
    assert!(nan.main.is_finite());
}

fn sized(id: u64, share: f32) -> PaneSpec {
    PaneSpec {
        id,
        collapsed: false,
        share: Some(share),
    }
}

#[test]
fn a_pane_keeps_the_height_it_was_dragged_to_and_the_others_keep_the_default() {
    let panes = [sized(1, 0.25), spec(2, false)];
    let split = split_panes(600.0, &panes, None, AXIS);
    assert_eq!(split.panes[0], 150.0);
    assert_eq!(split.panes[1], 90.0, "the default share over two panes");
    assert_eq!(split.main, 360.0);
    assert_eq!(split.main + split.panes.iter().sum::<f32>(), 600.0);
}

#[test]
fn dragged_panes_give_way_to_the_main_chart_and_never_shrink_to_nothing() {
    let greedy = split_panes(600.0, &[sized(1, 0.6), sized(2, 0.6)], None, AXIS);
    assert!(greedy.main >= 90.0, "{greedy:?}");
    assert_eq!(greedy.main + greedy.panes.iter().sum::<f32>(), 600.0);
    assert!(
        (greedy.panes[0] - greedy.panes[1]).abs() <= 1.0,
        "{greedy:?}"
    );
    let thin = split_panes(600.0, &[sized(1, 0.0)], None, AXIS);
    assert_eq!(thin.panes[0], MIN_PANE_HEIGHT);
}

#[test]
fn a_collapsed_pane_ignores_its_share() {
    let mut pane = sized(1, 0.4);
    pane.collapsed = true;
    let split = split_panes(600.0, &[pane], None, AXIS);
    assert_eq!(split.panes, vec![COLLAPSED_PANE_HEIGHT]);
}

fn drag(total: f32, panes: &[PaneSpec], index: usize, up: f32) -> Vec<(u64, f32)> {
    let start = split_panes(total, panes, None, AXIS);
    drag_divider(total, &start, panes, index, up)
}

#[test]
fn dragging_the_line_under_the_main_chart_trades_height_with_it() {
    let panes = [spec(1, false), spec(2, false)];
    // 600 splits into main 420 and two panes of 90.
    let up = drag(600.0, &panes, 0, 30.0);
    assert_eq!(up, vec![(1, 120.0 / 600.0), (2, 90.0 / 600.0)]);
    let down = drag(600.0, &panes, 0, -30.0);
    assert_eq!(down, vec![(1, 60.0 / 600.0), (2, 90.0 / 600.0)]);
}

#[test]
fn dragging_the_line_between_two_panes_leaves_the_main_chart_alone() {
    let panes = [spec(1, false), spec(2, false)];
    let moved = drag(600.0, &panes, 1, 20.0);
    assert_eq!(moved, vec![(1, 70.0 / 600.0), (2, 110.0 / 600.0)]);
    let total: f32 = moved.iter().map(|(_, share)| share * 600.0).sum();
    assert!((total - 180.0).abs() < 0.01, "the panes keep their sum");
}

#[test]
fn a_drag_stops_where_a_side_reaches_its_minimum() {
    let panes = [spec(1, false), spec(2, false)];
    let far_up = drag(600.0, &panes, 1, 1000.0);
    assert_eq!(far_up[0].1 * 600.0, MIN_PANE_HEIGHT);
    assert_eq!(far_up[1].1 * 600.0, 180.0 - MIN_PANE_HEIGHT);
    let far_down = drag(600.0, &panes, 1, -1000.0);
    assert_eq!(far_down[1].1 * 600.0, MIN_PANE_HEIGHT);
    let into_main = drag(600.0, &panes, 0, 1000.0);
    assert_eq!(
        into_main[0].1 * 600.0,
        90.0 + (420.0 - 90.0),
        "the main chart keeps its minimum"
    );
}

#[test]
fn a_line_skips_collapsed_panes_and_a_collapsed_pane_has_none() {
    let panes = [spec(1, false), spec(2, true), spec(3, false)];
    let moved = drag(600.0, &panes, 2, 10.0);
    let ids: Vec<u64> = moved.iter().map(|(id, _)| *id).collect();
    assert_eq!(ids, vec![1, 3], "the strip between them is not resized");
    assert!(
        moved[0].1 < moved[1].1,
        "the pane above gave height to the one below"
    );
    assert!(drag(600.0, &panes, 1, 10.0).is_empty());
    assert!(drag(600.0, &panes, 7, 10.0).is_empty());
    assert!(drag(0.0, &panes, 0, 10.0).is_empty());
    assert!(drag(600.0, &panes, 0, f32::NAN).is_empty());
}

#[test]
fn default_panes_keep_a_usable_height_while_the_chart_has_room() {
    // 30 % of 240 would give three panes 24 pt each: no plot (it needs 16 pt of margins).
    let split = split_panes(
        240.0,
        &[spec(1, false), spec(2, false), spec(3, false)],
        None,
        AXIS,
    );
    assert_eq!(split.panes, vec![MIN_PANE_HEIGHT; 3]);
    assert_eq!(split.main, 240.0 - 3.0 * MIN_PANE_HEIGHT);
    assert!(split.main >= 90.0, "the main chart keeps its minimum too");
}

#[test]
fn a_chart_too_short_for_the_floors_gives_each_pane_what_it_can_but_keeps_its_buttons() {
    let many: Vec<PaneSpec> = (1..=4).map(|id| spec(id, false)).collect();
    let split = split_panes(240.0, &many, None, AXIS);
    // 150 pt of room for four panes: under 48 pt each, over the 24 pt their buttons need.
    assert!(
        split.panes.iter().all(|h| (36.0..48.0).contains(h)),
        "{split:?}"
    );
    assert_eq!(split.main, 90.0);
    let crowded: Vec<PaneSpec> = (1..=8).map(|id| spec(id, false)).collect();
    let tight = split_panes(240.0, &crowded, None, AXIS);
    assert!(
        tight.panes.iter().all(|h| *h >= COLLAPSED_PANE_HEIGHT),
        "{tight:?}"
    );
    assert!(tight.main >= 0.0);
    assert!((tight.main + tight.panes.iter().sum::<f32>() - 240.0).abs() < 0.001);
}

#[test]
fn more_than_three_panes_share_the_same_thirty_percent() {
    let panes: Vec<PaneSpec> = (1..=5).map(|id| spec(id, false)).collect();
    let split = split_panes(1000.0, &panes, None, AXIS);
    assert_eq!(split.panes, vec![60.0; 5]);
    assert_eq!(split.main, 700.0);
}

#[test]
fn a_dragged_pane_is_cut_back_towards_its_floor_not_below_it() {
    let split = split_panes(600.0, &[sized(1, 0.9), sized(2, 0.0)], None, AXIS);
    assert_eq!(split.panes[1], MIN_PANE_HEIGHT);
    assert_eq!(
        split.main, 90.0,
        "the big pane stops where the main chart's minimum is"
    );
    assert_eq!(split.panes[0], 600.0 - 90.0 - MIN_PANE_HEIGHT);
}

/// `panes` with the shares a drag returned.
fn with_shares(panes: &[PaneSpec], shares: &[(u64, f32)]) -> Vec<PaneSpec> {
    panes
        .iter()
        .map(|pane| PaneSpec {
            share: shares
                .iter()
                .find(|(id, _)| *id == pane.id)
                .map(|(_, s)| *s),
            ..*pane
        })
        .collect()
}

#[test]
fn the_split_keeps_every_height_a_drag_asked_for() {
    // Short charts have a smaller main minimum and fewer points to spare, so their limits differ
    // from a tall chart's; a collapsed pane sits between the main chart and the dragged one.
    for total in [100.0_f32, 240.0, 600.0] {
        let panes = [
            spec(1, false),
            spec(2, true),
            spec(3, false),
            spec(4, false),
        ];
        let start = split_panes(total, &panes, None, AXIS);
        for index in [0, 2, 3] {
            for up in [-1000.0, -12.5, 12.5, 1000.0] {
                let shares = drag_divider(total, &start, &panes, index, up);
                let dragged = split_panes(total, &with_shares(&panes, &shares), None, AXIS);
                for (at, pane) in panes.iter().enumerate().filter(|(_, p)| !p.collapsed) {
                    let share = shares
                        .iter()
                        .find(|(id, _)| *id == pane.id)
                        .expect("share")
                        .1;
                    assert!(
                        (dragged.panes[at] - share * total).abs() <= 1.0,
                        "total {total}, line {index}, up {up}: {dragged:?} for {shares:?}"
                    );
                }
                // The main chart yields only to the panes' buttons: with room for those it keeps its
                // minimum however the line was dragged.
                if total - 4.0 * COLLAPSED_PANE_HEIGHT >= main_floor(total) {
                    assert!(dragged.main >= main_floor(total) - 1.0, "{dragged:?}");
                }
                let (floor, _) = pane_limits(total, 3, COLLAPSED_PANE_HEIGHT);
                assert!(
                    panes
                        .iter()
                        .enumerate()
                        .all(|(at, pane)| pane.collapsed || dragged.panes[at] >= floor.floor()),
                    "{dragged:?}"
                );
            }
        }
    }
}

#[test]
fn a_pane_the_drag_does_not_touch_never_moves() {
    let panes = [spec(1, false), spec(2, false), spec(3, false)];
    let start = split_panes(601.0, &panes, None, AXIS);
    for half_points in 0..40 {
        let up = half_points as f32 * 0.5;
        let shares = drag_divider(601.0, &start, &panes, 1, up);
        let dragged = split_panes(601.0, &with_shares(&panes, &shares), None, AXIS);
        assert_eq!(dragged.panes[2], start.panes[2], "up {up}: {dragged:?}");
        assert_eq!(dragged.main, start.main, "up {up}: {dragged:?}");
    }
}

#[test]
fn a_start_that_is_not_the_split_of_these_panes_has_no_line_to_drag() {
    let two = [spec(1, false), spec(2, false)];
    let start = split_panes(600.0, &two, None, AXIS);
    let three = [spec(1, false), spec(2, false), spec(3, false)];
    assert!(drag_divider(600.0, &start, &three, 1, 10.0).is_empty());
    assert!(drag_divider(600.0, &start, &two[..1], 0, 10.0).is_empty());
}

#[test]
fn a_scale_with_room_for_two_labels_always_gets_at_least_two() {
    // 14..86 at two labels: the step 50 alone would leave only 50.
    let ticks = price_ticks(14.0, 86.0, false, 2);
    assert!(ticks.len() >= 2, "{ticks:?}");
    assert!(ticks.iter().all(|tick| (14.0..=86.0).contains(tick)));
    // One label's worth of room stays one.
    assert!(price_ticks(14.0, 86.0, false, 1).len() <= 1);
}
