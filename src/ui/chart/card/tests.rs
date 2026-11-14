use super::crosshair::flow_readout;
use super::header::change_text;
use super::*;

#[test]
fn contained_center_keeps_an_item_inside_the_bounds() {
    use super::paint::contained_center;
    assert_eq!(contained_center(50.0, 0.0, 100.0, 20.0), Some(50.0));
    assert_eq!(contained_center(-5.0, 0.0, 100.0, 20.0), Some(10.0));
    assert_eq!(contained_center(500.0, 0.0, 100.0, 20.0), Some(90.0));
    assert_eq!(contained_center(50.0, 0.0, 10.0, 20.0), None);
}

#[test]
fn the_refresh_warning_names_rate_limits() {
    use super::{DataSource, ProviderError, refresh_warning_text};
    use crate::service::net::http;
    let source = DataSource::CoinMarketCap;
    let limited = ProviderError::CoinMarketCap(http::Error::Status(429));
    assert_eq!(
        refresh_warning_text(&limited, source, true),
        "CoinMarketCap rate limit reached · showing the last result"
    );
    let timeout = ProviderError::other("timeout");
    assert!(refresh_warning_text(&timeout, source, true).ends_with("(timeout)"));
    assert!(!refresh_warning_text(&timeout, source, false).contains("timeout"));
}

#[test]
fn preview_inputs_set_before_the_source_still_apply() {
    let mut card = ChartCard::default();
    let mut inputs = HashMap::new();
    inputs.insert("len".to_owned(), InputValue::Int(30));
    card.set_pine_inputs(inputs.clone());
    card.set_pine_source(Some("indicator(\"x\")".into()));
    assert_eq!(card.instances[0].input_values(), inputs);
    // Clearing the script keeps the overrides for the next source.
    card.set_pine_source(None);
    card.set_pine_source(Some("indicator(\"x\")".into()));
    assert_eq!(card.instances[0].input_values(), inputs);
}

#[test]
fn readout_flows_onto_rows_without_exceeding_the_width() {
    let widths = [40.0, 40.0, 40.0, 40.0, 50.0];
    // One row when it all fits.
    assert!(
        flow_readout(&widths, 10.0, 1000.0)
            .iter()
            .all(|(row, _)| *row == 0)
    );
    // Narrow: breaks between items, every row within the limit.
    let placed = flow_readout(&widths, 10.0, 100.0);
    assert_eq!(placed[0], (0, 0.0));
    assert_eq!(placed[1], (0, 50.0));
    assert_eq!(placed[2], (1, 0.0));
    for ((row, x), width) in placed.iter().zip(widths) {
        assert!(*row <= 2 && x + width <= 100.0);
    }
    // An item wider than the limit gets its own row rather than looping.
    assert_eq!(
        flow_readout(&[200.0, 20.0], 10.0, 100.0),
        [(0, 0.0), (1, 0.0)]
    );
    assert!(flow_readout(&[], 10.0, 100.0).is_empty());
}

fn candle(open: f64, close: f64) -> Kline {
    Kline {
        open_time: 0,
        close_time: 1,
        open,
        high: open.max(close),
        low: open.min(close),
        close,
        volume: 1.0,
    }
}

fn point(open_time: i64, close: f64) -> Kline {
    Kline {
        open_time,
        close_time: open_time + 1,
        open: close,
        high: close,
        low: close,
        close,
        volume: 0.0,
    }
}

fn choice(token_id: &str, label: &str) -> PmSeries {
    PmSeries {
        token_id: token_id.into(),
        label: label.into(),
    }
}

fn two_lines() -> Vec<SeriesLine> {
    SeriesLine::build(
        &[choice("a", "Cut"), choice("b", "Hold")],
        vec![
            ("b".into(), vec![point(0, 0.5), point(1, 0.9)]),
            ("a".into(), vec![point(0, 0.1), point(1, 0.2)]),
        ],
    )
}

#[test]
fn lines_follow_the_order_of_the_choices_and_skip_empty_histories() {
    let lines = SeriesLine::build(
        &[choice("a", "A"), choice("gone", "Gone"), choice("c", "C")],
        vec![
            ("c".into(), vec![point(0, 0.3)]),
            ("a".into(), vec![point(0, 0.1)]),
            ("gone".into(), Vec::new()),
        ],
    );
    let labels: Vec<&str> = lines.iter().map(|line| line.label.as_str()).collect();
    assert_eq!(labels, ["A", "C"]);
    // The color is the choice's place among those ticked, even when an earlier one is empty.
    assert_eq!(lines[0].color, SERIES_PALETTE[0]);
    assert_eq!(lines[1].color, SERIES_PALETTE[2]);
}

#[test]
fn slots_an_overlay_script_reserves_past_the_last_bar_narrow_every_slot_and_extend_the_scale() {
    let mut card = ChartCard::default();
    card.set_visible_candles(100);
    card.appearance_mut().canvas.margins.right = 0;
    let candles: Vec<Kline> = (0..100).map(|i| point(i, 1.0)).collect();
    let plot = Rect::from_min_size(Pos2::ZERO, Vec2::new(550.0, 200.0));
    let alone = card
        .view(
            &candles,
            (plot, 0.0),
            PriceScaleMode::Regular,
            ChartType::Candles,
        )
        .expect("view");
    assert_eq!(alone.slot_width, 5.5);
    card.script_extent = OverlayExtent {
        future_bars: 10,
        values: vec![9.0],
    };
    let with_script = card
        .view(
            &candles,
            (plot, 0.0),
            PriceScaleMode::Regular,
            ChartType::Candles,
        )
        .expect("view");
    assert_eq!(with_script.slot_width, 5.0, "550 px over 100 + 10 slots");
    assert!(
        with_script.scale.maximum > 9.0,
        "a drawn price fits the scale"
    );
}

#[test]
fn the_price_scale_starts_below_the_header_while_the_plot_reaches_the_top() {
    let card = ChartCard::default();
    let candles: Vec<Kline> = (0..50).map(|i| point(i, 1.0 + i as f64 * 0.01)).collect();
    let plot = Rect::from_min_size(Pos2::new(0.0, 10.0), Vec2::new(500.0, 300.0));
    let bare = card
        .view(
            &candles,
            (plot, 0.0),
            PriceScaleMode::Regular,
            ChartType::Candles,
        )
        .expect("view");
    let covered = card
        .view(
            &candles,
            (plot, HEADER_HEIGHT),
            PriceScaleMode::Regular,
            ChartType::Candles,
        )
        .expect("view");
    assert_eq!(bare.scale.top, plot.top());
    assert_eq!(covered.scale.top, plot.top() + HEADER_HEIGHT);
    assert_eq!(covered.scale.height, plot.height() - HEADER_HEIGHT);
    assert_eq!(covered.scale.maximum, bare.scale.maximum, "the same prices");
    // The highest price lands at the same distance below the scale's top, so the bars clear
    // the header by default.
    let top = covered.scale.y(covered.scale.maximum);
    assert!(top >= plot.top() + HEADER_HEIGHT - 0.01, "{top}");
    // The part of the card under the header is still the plot: a snapshot loses it.
    let mut card = ChartCard::default();
    card.set_top_inset(HEADER_HEIGHT);
    card.set_snapshot_size_for_tests(Vec2::new(500.0, 345.0));
    assert_eq!(card.snapshot_size(), Vec2::new(500.0, 300.0));
    assert_eq!(card.below_header(plot).top(), plot.top() + HEADER_HEIGHT);
}

#[test]
fn the_price_range_covers_every_line_not_only_the_candles() {
    let mut card = ChartCard::default();
    let candles = [point(0, 0.2), point(1, 0.3)];
    let plot = Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 200.0));
    let alone = card
        .view(
            &candles,
            (plot, 0.0),
            PriceScaleMode::Regular,
            ChartType::Candles,
        )
        .expect("view")
        .scale
        .maximum;
    card.set_series(two_lines());
    let with_lines = card
        .view(
            &candles,
            (plot, 0.0),
            PriceScaleMode::Regular,
            ChartType::Candles,
        )
        .expect("view")
        .scale
        .maximum;
    assert!(with_lines > 0.9 && with_lines > alone);
}

#[test]
fn replay_cuts_lines_at_the_cutoff_and_the_leader_follows() {
    let mut card = ChartCard::default();
    card.set_series(two_lines());
    assert_eq!(card.leading_choice(), Some(("Hold", 0.9)));
    card.set_series_cutoff(Some(0));
    assert_eq!(card.leading_choice(), Some(("Hold", 0.5)));
    assert_eq!(card.series_points(&card.series[0]).count(), 1);
}

#[test]
fn long_labels_are_cut_with_an_ellipsis() {
    assert_eq!(truncated("Yes", 5), "Yes");
    assert_eq!(truncated("abcdefgh", 5), "abcd…");
}

#[test]
fn a_title_replaces_the_market_id_in_the_header() {
    let mut card = ChartCard::default();
    assert_eq!(card.display_title("12345", DataSource::Polymarket), "12345");
    card.set_title(Some("Fed decision".into()));
    assert_eq!(
        card.display_title("12345", DataSource::Polymarket),
        "Fed decision"
    );
}

#[test]
fn header_change_uses_visible_closes_after_pan_and_zoom() {
    let candles = [
        candle(80.0, 100.0),
        candle(105.0, 110.0),
        candle(95.0, 90.0),
        candle(115.0, 120.0),
    ];
    let percent =
        |count, offset| visible_price_move(&candles, count, offset, 0).map(|(_, percent)| percent);
    assert_eq!(percent(2, 0.0), Some(30.0 / 90.0 * 100.0));
    assert_eq!(percent(2, 1.0), Some(-20.0 / 110.0 * 100.0));
    assert_eq!(percent(1, 0.0), None);
    assert_eq!(visible_price_move(&[], 2, 0.0, 0), None);
}

#[test]
fn the_header_change_reads_in_price_and_percent() {
    let candles = [
        candle(80.0, 100.0),
        candle(105.0, 110.0),
        candle(95.0, 90.0),
    ];
    assert_eq!(
        visible_price_move(&candles, 2, 0.0, 0),
        Some((-20.0, -20.0 / 110.0 * 100.0))
    );
    assert_eq!(change_text(Some(67.125), 0.1), "+$67.125 (+0.10%)");
    assert_eq!(change_text(Some(-20.0), -18.18), "-$20.00 (-18.18%)");
    assert_eq!(change_text(None, 0.0), "+0.00%");
}

fn hourly(closes: &[f64]) -> Vec<Kline> {
    closes
        .iter()
        .enumerate()
        .map(|(index, close)| Kline {
            open_time: index as i64 * 3_600_000,
            close_time: (index as i64 + 1) * 3_600_000 - 1,
            open: *close,
            high: *close,
            low: *close,
            close: *close,
            volume: 1.0,
        })
        .collect()
}

fn renko_card(amount: f64) -> ChartCard {
    let mut card = ChartCard::default();
    let style = card.chart_style_mut();
    style.chart_type = ChartType::Renko;
    style.renko.size = crate::model::chart::kind::SizeRule {
        method: crate::model::chart::kind::SizeMethod::Traditional,
        amount,
        ..Default::default()
    };
    card
}

#[test]
fn constructed_bars_are_kept_until_the_candles_or_the_construction_change() {
    let candles = hourly(&[10.0, 12.5, 11.0, 9.0]);
    let mut card = renko_card(1.0);
    let first = card
        .constructed_series(&candles, ChartType::Renko)
        .expect("renko builds bars");
    // Source candles and bricks are different series.
    assert_eq!(first.klines.len(), 4);
    let again = card.constructed_series(&candles, ChartType::Renko).unwrap();
    assert!(Arc::ptr_eq(&first, &again), "a repaint does not rebuild");

    // Colours and visibility are the chart's looks: the bars stay.
    card.chart_style_mut().renko.candle.up_body = Some([1, 2, 3, 255]);
    card.chart_style_mut().renko.candle.wick = false;
    let restyled = card.constructed_series(&candles, ChartType::Renko).unwrap();
    assert!(
        Arc::ptr_eq(&first, &restyled),
        "appearance-only changes keep the bars"
    );

    card.chart_style_mut().renko.size.amount = 2.0;
    let resized = card.constructed_series(&candles, ChartType::Renko).unwrap();
    assert!(!Arc::ptr_eq(&first, &resized));
    assert_ne!(resized.klines.len(), first.klines.len());

    let mut more = candles.clone();
    more.extend(hourly(&[4.0]).into_iter().map(|mut bar| {
        bar.open_time = 4 * 3_600_000;
        bar.close_time = 5 * 3_600_000 - 1;
        bar
    }));
    let grown = card.constructed_series(&more, ChartType::Renko).unwrap();
    assert!(!Arc::ptr_eq(&resized, &grown), "new candles rebuild");

    // Standard types draw the candles as they are and drop the constructed bars.
    assert!(card.constructed_series(&candles, ChartType::Line).is_none());
    assert!(card.constructed.is_none());
}

#[test]
fn a_replay_prefix_builds_the_prefix_of_the_bars_without_the_future() {
    let candles = hourly(&[10.0, 12.5, 11.0, 9.0, 14.0, 6.0]);
    let mut card = renko_card(1.0);
    let full = card.constructed_series(&candles, ChartType::Renko).unwrap();
    let replayed = card
        .constructed_series(&candles[..4], ChartType::Renko)
        .unwrap();
    assert!(replayed.klines.len() < full.klines.len());
    assert_eq!(full.klines[..replayed.klines.len()], replayed.klines[..]);
    // The prefix's bars know nothing past it.
    assert!(replayed.series.bars.iter().all(|bar| bar.last_source < 4));
}

#[test]
fn switching_between_standard_and_price_based_types_resets_the_scroll() {
    let mut card = ChartCard::default();
    card.viewport.horizontal.right_offset = 30.0;
    card.set_chart_type(ChartType::Line);
    let offset = |card: &ChartCard| card.viewport.horizontal.right_offset;
    assert_eq!(offset(&card), 30.0, "a line follows the same candles");
    card.set_chart_type(ChartType::Renko);
    assert_eq!(offset(&card), 0.0);
    card.viewport.horizontal.right_offset = 5.0;
    card.set_chart_type(ChartType::Kagi);
    assert_eq!(offset(&card), 0.0);
}

#[test]
fn a_source_that_cannot_show_a_type_falls_back_to_candles() {
    let mut card = ChartCard::default();
    card.set_chart_type(ChartType::VolumeCandles);
    assert_eq!(
        card.effective_chart_type(DataSource::Binance),
        ChartType::VolumeCandles
    );
    assert_eq!(
        card.effective_chart_type(DataSource::CoinGecko),
        ChartType::Candles
    );
    assert_eq!(
        card.appearance().style.chart_type,
        ChartType::VolumeCandles,
        "the choice is kept for when the market allows it"
    );
}

#[test]
fn shared_colours_follow_the_drawn_types_colours_or_the_saved_palette() {
    let mut card = ChartCard::default();
    let saved = (card.bullish_color(), card.bearish_color());
    assert_eq!(saved.0, Color32::from_rgb(48, 209, 88));
    card.set_chart_type(ChartType::Kagi);
    card.chart_style_mut().kagi.up = Some([10, 20, 30, 255]);
    // Not drawn as Kagi yet: the colours follow what was last drawn.
    assert_eq!(card.bullish_color(), saved.0);
    card.drawn_type = ChartType::Kagi;
    assert_eq!(card.bullish_color(), Color32::from_rgb(10, 20, 30));
    assert_eq!(
        card.bearish_color(),
        saved.1,
        "an unset side keeps the saved palette"
    );
    card.appearance_mut().bearish = [1, 2, 3];
    assert_eq!(card.bearish_color(), Color32::from_rgb(1, 2, 3));
}
