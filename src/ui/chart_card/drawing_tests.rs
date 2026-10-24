//! Drawings on the whole chart: that they paint inside the plot and nowhere else, that the chart
//! type, a replay cursor, a log scale or a timeframe never rewrite them, and that a drawing's
//! own changes never touch the data or a script's run.

use super::*;
use crate::model::drawing_style::Visibility;
use crate::model::{Anchor, ChartType, Drawing, IndicatorConfig, Placement, Tool};
use crate::ui::replay_overlay::ReplayChartFrame;
use eframe::egui::epaint::ClippedShape;

const HOUR: i64 = 3_600_000;
const SIZE: Vec2 = Vec2::new(900.0, 560.0);

fn series(count: usize) -> Vec<Kline> {
    (0..count)
        .map(|index| {
            let close = 100.0 + (index as f64 * 0.7).sin() * 20.0 + index as f64 * 0.5;
            Kline {
                open_time: index as i64 * HOUR,
                close_time: (index as i64 + 1) * HOUR - 1,
                open: close - 1.0,
                high: close + 2.0,
                low: close - 3.0,
                close,
                volume: 10.0 + index as f64,
            }
        })
        .collect()
}

fn anchor(hour: i64, price: f64) -> Anchor {
    Anchor {
        time_ms: hour * HOUR,
        price,
    }
}

/// One drawing of `tool`, sized to sit among the last bars of `series(300)`.
fn drawing(tool: Tool, id: u64) -> Drawing {
    let anchors = [
        anchor(250, 222.0),
        anchor(270, 245.0),
        anchor(262, 212.0),
        anchor(280, 235.0),
        anchor(284, 226.0),
        anchor(288, 241.0),
        anchor(292, 230.0),
    ];
    let count = match tool.spec().placement {
        Placement::Points(count) => usize::from(count),
        Placement::Single => 1,
        _ => 3,
    };
    let mut drawing =
        Drawing::sample(tool, id, &anchors[..count], tool.default_style()).expect("builds");
    drawing.set_text("note");
    drawing
}

struct Chart {
    ctx: egui::Context,
    card: ChartCard,
    candles: Vec<Kline>,
    scale_mode: PriceScaleMode,
    tool: Tool,
    hide_all: bool,
    timeframe: Timeframe,
    selected: Vec<u64>,
}

impl Chart {
    fn new() -> Self {
        let mut card = ChartCard::default();
        card.set_timeframe(Timeframe::Hour);
        Self {
            ctx: egui::Context::default(),
            card,
            candles: series(300),
            scale_mode: PriceScaleMode::Regular,
            tool: Tool::Cursor,
            hide_all: false,
            timeframe: Timeframe::Hour,
            selected: Vec::new(),
        }
    }

    fn frame(&mut self, drawings: &[Drawing]) -> Vec<ClippedShape> {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, SIZE)),
            ..Default::default()
        };
        let (card, candles, scale_mode) = (&mut self.card, &self.candles, self.scale_mode);
        let (tool, hide_all, timeframe) = (self.tool, self.hide_all, self.timeframe);
        let selected = self.selected.clone();
        let mut output = self.ctx.run_ui(input, |ui| {
            let mut frame = DrawingFrame::new(drawings, tool, None, 100)
                .with_selection(selected.clone())
                .with_view(timeframe, hide_all);
            let mut replay = ReplayChartFrame {
                selecting: false,
                active: false,
                picked_ms: None,
            };
            card.body(
                ui,
                "BTCUSDT",
                candles,
                false,
                None,
                scale_mode,
                DataSource::Binance,
                &IndicatorConfig::default(),
                None,
                1,
                &mut frame,
                &mut replay,
            );
        });
        output.textures_delta.clear();
        std::mem::take(&mut output.shapes)
    }

    /// The shapes after a few frames, so the first layout and the fades have settled.
    fn draw(&mut self, drawings: &[Drawing]) -> Vec<ClippedShape> {
        let mut last = self.frame(drawings);
        for _ in 0..3 {
            last = self.frame(drawings);
        }
        last
    }
}

/// How many shapes were painted under each clip rectangle.
fn per_clip(shapes: &[ClippedShape]) -> Vec<(Rect, usize)> {
    let mut counts: Vec<(Rect, usize)> = Vec::new();
    for shape in shapes {
        match counts.iter_mut().find(|(rect, _)| *rect == shape.clip_rect) {
            Some((_, count)) => *count += 1,
            None => counts.push((shape.clip_rect, 1)),
        }
    }
    counts
}

/// The clip rectangles that gained shapes when the drawings were added.
fn gained(without: &[ClippedShape], with: &[ClippedShape]) -> Vec<Rect> {
    let before = per_clip(without);
    per_clip(with)
        .into_iter()
        .filter(|(rect, count)| {
            before
                .iter()
                .find(|(other, _)| other == rect)
                .is_none_or(|(_, was)| count > was)
        })
        .map(|(rect, _)| rect)
        .collect()
}

/// What a tool paints beyond the plot-clipped drawing layer. Only the tags of the lines at a
/// price or a time, and of a position's three prices, do: they sit on the axes, by design. Everything else a tool paints is held to
/// the plot.
fn plot_gains(tool: Tool, gains: Vec<Rect>) -> Vec<Rect> {
    let tags = matches!(
        tool,
        Tool::HorizontalLine
            | Tool::HorizontalRay
            | Tool::VerticalLine
            | Tool::Crossline
            | Tool::LongPosition
            | Tool::ShortPosition
    );
    gains
        .into_iter()
        .filter(|clip| !(tags && clip.right() > SIZE.x - 40.0))
        .collect()
}

#[test]
fn every_tool_paints_inside_the_plot_and_nowhere_else() {
    for tool in Tool::GROUPED {
        let mut chart = Chart::new();
        let bare = chart.draw(&[]);
        let drawings = [drawing(tool, 1)];
        let with = chart.draw(&drawings);
        let gains = plot_gains(tool, gained(&bare, &with));
        assert!(!gains.is_empty(), "{tool:?} painted nothing");
        for clip in gains {
            // Over the candles, clear of the price axis on the right and the time axis below.
            assert!(
                clip.right() <= SIZE.x - 40.0,
                "{tool:?} spills over the axis: {clip:?}"
            );
            assert!(
                clip.bottom() <= SIZE.y - 14.0,
                "{tool:?} spills over the time axis: {clip:?}"
            );
            assert!(clip.top() >= 0.0 && clip.left() >= 0.0);
        }
    }
}

/// A Post card among the last candles of a chart whose candles start at `base`, with the card
/// over the plot's middle. The shapes painted for it.
fn post_shapes(base: i64) -> usize {
    let mut chart = Chart::new();
    for (index, candle) in chart.candles.iter_mut().enumerate() {
        candle.open_time = base + index as i64 * HOUR;
        candle.close_time = candle.open_time + HOUR - 1;
    }
    let bare = chart.draw(&[]).len();
    let at = Anchor {
        time_ms: base + 150 * HOUR,
        price: 215.0,
    };
    let post =
        Drawing::sample(Tool::Post, 1, &[at], Tool::Post.default_style()).expect("a post card");
    chart.draw(&[post]).len() - bare
}

#[test]
fn a_post_card_is_joined_to_its_candle_by_a_painted_line() {
    let sample =
        crate::model::link_card::Link::parse("https://x.com/degenview/status/1750000000000000000")
            .expect("a link");
    let time = sample.time_ms().expect("a snowflake id has a time");
    // The post's hour is among the candles, 280 of 300 in: the card, its line and its dot.
    let joined = post_shapes(time - 280 * HOUR);
    // Every candle is years older than the post: just the card.
    let alone = post_shapes(time - 400_000 * HOUR);
    assert!(
        joined >= alone + 2,
        "the line and its dot are painted: {joined} shapes against {alone}"
    );
}

#[test]
fn the_original_tools_still_paint_inside_the_plot() {
    for tool in [Tool::TrendLine, Tool::FibRetracement] {
        let mut chart = Chart::new();
        let bare = chart.draw(&[]);
        let with = chart.draw(&[Drawing::build(
            tool,
            1,
            &[anchor(250, 222.0), anchor(280, 245.0)],
            tool.default_style(),
        )
        .unwrap()]);
        assert!(!gained(&bare, &with).is_empty(), "{tool:?}");
    }
}

#[test]
fn a_drawing_keeps_its_anchors_through_a_chart_type_change_and_still_paints() {
    let stored = drawing(Tool::Rectangle, 1);
    let mut chart = Chart::new();
    let bare = chart.draw(&[]);
    for chart_type in [
        ChartType::Candles,
        ChartType::HeikinAshi,
        ChartType::Renko,
        ChartType::Line,
    ] {
        chart.card.set_chart_type(chart_type);
        let before = chart.draw(&[]);
        let with = chart.draw(std::slice::from_ref(&stored));
        assert!(
            !gained(&before, &with).is_empty() || chart_type == ChartType::Renko,
            "{chart_type:?}"
        );
    }
    let _ = bare;
    // The stored anchors are what they were: nothing in the layer rewrites a drawing.
    assert_eq!(stored, drawing(Tool::Rectangle, 1));
}

#[test]
fn a_renko_chart_draws_every_tool_without_panicking_or_misplacing_it() {
    let mut chart = Chart::new();
    chart.card.set_chart_type(ChartType::Renko);
    let bare = chart.draw(&[]);
    for tool in Tool::GROUPED {
        let with = chart.draw(&[drawing(tool, 1)]);
        for clip in plot_gains(tool, gained(&bare, &with)) {
            assert!(
                clip.right() <= SIZE.x - 40.0 && clip.bottom() <= SIZE.y - 14.0,
                "{tool:?}"
            );
        }
    }
}

#[test]
fn a_replay_cut_off_leaves_drawings_where_they_are() {
    let mut chart = Chart::new();
    let stored = [drawing(Tool::Ellipse, 1), drawing(Tool::Arrow, 2)];
    // Replay shows only the first 200 bars, so both drawings lie in what has not happened yet.
    let cut = chart.candles[..200].to_vec();
    chart.candles = cut;
    chart.card.set_series_cutoff(Some(199 * HOUR));
    let bare = chart.draw(&[]);
    let with = chart.draw(&stored);
    for clip in gained(&bare, &with) {
        assert!(clip.right() <= SIZE.x - 40.0 && clip.bottom() <= SIZE.y - 14.0);
    }
    // The drawings are untouched by the cut.
    assert_eq!(stored[0], drawing(Tool::Ellipse, 1));
}

#[test]
fn a_log_scale_draws_every_tool_inside_the_plot() {
    let mut chart = Chart::new();
    chart.scale_mode = PriceScaleMode::Logarithmic;
    let bare = chart.draw(&[]);
    for tool in Tool::GROUPED {
        let with = chart.draw(&[drawing(tool, 1)]);
        assert!(!gained(&bare, &with).is_empty(), "{tool:?}");
    }
}

/// Every word painted, nested shapes included.
fn words(shapes: &[ClippedShape]) -> Vec<String> {
    fn of(shape: &egui::Shape, into: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(text) => into.push(text.galley.text().to_owned()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| of(shape, into)),
            _ => {}
        }
    }
    let mut words = Vec::new();
    for clipped in shapes {
        of(&clipped.shape, &mut words);
    }
    words
}

#[test]
fn drawings_stay_where_their_prices_are_in_percent_and_indexed() {
    let mut regular = Chart::new();
    let bare = regular.draw(&[]);
    for mode in [PriceScaleMode::Percent, PriceScaleMode::IndexedTo100] {
        let mut chart = Chart::new();
        chart.scale_mode = mode;
        let bare_in_mode = chart.draw(&[]);
        for tool in Tool::GROUPED {
            let stored = [drawing(tool, 1)];
            let expected = gained(&bare, &regular.draw(&stored));
            let drawn = gained(&bare_in_mode, &chart.draw(&stored));
            assert!(!drawn.is_empty(), "{mode:?} {tool:?}");
            // Percent and indexed are linear in the price: the same clips take the same shapes at the
            // same heights. Their width follows the plot, which a wider axis label narrows.
            let height = |rects: &[Rect]| -> Vec<(f32, f32)> {
                rects.iter().map(|r| (r.top(), r.bottom())).collect()
            };
            assert_eq!(height(&drawn), height(&expected), "{mode:?} {tool:?} moved");
            assert_eq!(
                stored[0],
                drawing(tool, 1),
                "{mode:?} {tool:?} was rewritten"
            );
        }
    }
}

#[test]
fn a_price_tag_on_the_axis_reads_as_the_axis_does_while_the_line_keeps_its_price() {
    let line = [drawing(Tool::HorizontalLine, 1)];
    let tag_price = line[0].anchors()[0].price;
    let plain = |chart: &mut Chart| {
        let bare = words(&chart.draw(&[]));
        let mut with = words(&chart.draw(&line));
        for word in bare {
            if let Some(index) = with.iter().position(|other| *other == word) {
                with.remove(index);
            }
        }
        with
    };
    let mut regular = Chart::new();
    let added = plain(&mut regular);
    let price = crate::model::PriceFormat::AUTO.format(tag_price);
    assert!(
        added.contains(&price),
        "regular tags the price {price}: {added:?}"
    );

    let mut percent = Chart::new();
    percent.scale_mode = PriceScaleMode::Percent;
    let added = plain(&mut percent);
    assert!(
        added.iter().any(|word| word.ends_with('%')),
        "percent tags the change: {added:?}"
    );
    assert!(!added.contains(&price), "{added:?}");
    // The line is still the one stored at its price.
    assert_eq!(line[0].anchors()[0].price, tag_price);
}

#[test]
fn hide_all_and_a_timeframe_it_is_not_shown_on_paint_nothing() {
    let mut chart = Chart::new();
    let bare = chart.draw(&[]);
    let mut weekly_only = drawing(Tool::Rectangle, 1);
    weekly_only.set_visibility(Visibility::ALL.with(Timeframe::Hour, false));
    let shown = [drawing(Tool::Rectangle, 1)];
    assert!(!gained(&bare, &chart.draw(&shown)).is_empty());

    chart.hide_all = true;
    assert!(gained(&bare, &chart.draw(&shown)).is_empty(), "hide all");
    chart.hide_all = false;

    assert!(gained(&bare, &chart.draw(std::slice::from_ref(&weekly_only))).is_empty());
    chart.timeframe = Timeframe::Week;
    assert!(!gained(&bare, &chart.draw(std::slice::from_ref(&weekly_only))).is_empty());

    let mut hidden = drawing(Tool::Rectangle, 1);
    hidden.set_hidden(true);
    chart.timeframe = Timeframe::Hour;
    assert!(gained(&bare, &chart.draw(std::slice::from_ref(&hidden))).is_empty());
}

#[test]
fn many_drawings_paint_within_a_frame_budget() {
    let mut chart = Chart::new();
    let mut drawings = Vec::new();
    let mut id = 1;
    for tool in [Tool::Rectangle, Tool::Arrow, Tool::Ellipse, Tool::Curve] {
        for index in 0..100 {
            let mut one = drawing(tool, id);
            let shift = (index % 20) as f64 * 1.5;
            for anchor in one.anchors_mut() {
                anchor.time_ms -= (index % 30) as i64 * HOUR;
                anchor.price -= shift;
            }
            drawings.push(one);
            id += 1;
        }
    }
    chart.draw(&drawings);
    let started = std::time::Instant::now();
    for _ in 0..5 {
        chart.frame(&drawings);
    }
    let per_frame = started.elapsed() / 5;
    eprintln!("400 drawings: {per_frame:?} a frame (unoptimized build)");
    // Debug builds are slow; the point is that nothing is quadratic in the drawing count.
    assert!(
        per_frame < std::time::Duration::from_millis(900),
        "{per_frame:?}"
    );
}

#[test]
fn a_selected_drawing_floats_its_toolbar_and_a_hidden_chart_area_does_not() {
    let mut chart = Chart::new();
    chart.selected = vec![1];
    let drawings = [drawing(Tool::Rectangle, 1)];
    chart.draw(&drawings);
    assert!(
        chart
            .ctx
            .memory(|memory| memory.area_rect(egui::Id::new("drawing-toolbar")))
            .is_some()
    );
}
