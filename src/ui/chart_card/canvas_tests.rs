#![allow(clippy::field_reassign_with_default)]
//! The chart drawn headlessly with different canvases: what reaches the painter, where, and that
//! the canvas never touches the data, the bars or a script's run.

use super::*;
use crate::model::canvas::{
    BackgroundMode, ButtonVisibility, ChartMargins, ScaleTextSize, WatermarkMode,
};
use crate::model::chart_type::LineDash;
use crate::model::viewport::MIN_VISIBLE as MIN_VISIBLE_CANDLES;
use crate::model::{ChartCanvas, IndicatorConfig, SmallPrices};
use crate::ui::chart_buttons::NavAction;
use crate::ui::replay_overlay::ReplayChartFrame;
use eframe::egui::epaint::ClippedShape;
use eframe::egui::vec2;

pub(super) const HOUR: i64 = 3_600_000;
pub(super) const SIZE: Vec2 = Vec2::new(760.0, 520.0);

pub(super) fn series(count: usize) -> Vec<Kline> {
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

#[derive(Default)]
pub(super) struct Drawn {
    segments: Vec<([Pos2; 2], Stroke)>,
    texts: Vec<(Pos2, String, f32)>,
    meshes: Vec<Arc<egui::epaint::Mesh>>,
}

fn collect(shape: &Shape, out: &mut Drawn) {
    match shape {
        Shape::LineSegment { points, stroke } => out.segments.push((*points, *stroke)),
        Shape::Text(text) => {
            let size = text
                .galley
                .job
                .sections
                .first()
                .map_or(0.0, |section| section.format.font_id.size);
            // The centre of the text, which is where a label anchored on its middle sits.
            out.texts.push((
                text.pos + text.galley.size() / 2.0,
                text.galley.text().to_owned(),
                size,
            ));
        }
        Shape::Mesh(mesh) => out.meshes.push(Arc::clone(mesh)),
        Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, out)),
        _ => {}
    }
}

fn gather(shapes: &[ClippedShape]) -> Drawn {
    let mut out = Drawn::default();
    for clipped in shapes {
        // Shapes clipped away entirely are never painted.
        if clipped.clip_rect.width() > 0.0 && clipped.clip_rect.height() > 0.0 {
            collect(&clipped.shape, &mut out);
        }
    }
    out
}

/// A chart on a window: the card's own draw, run for a few frames so animations settle.
pub(super) struct Harness {
    pub(super) ctx: egui::Context,
    pub(super) card: ChartCard,
    pub(super) candles: Vec<Kline>,
    pub(super) replay_active: bool,
    pub(super) pointer: Option<Pos2>,
    pub(super) crosshair: Option<CrosshairInfo>,
    pub(super) indicators: IndicatorConfig,
    pub(super) drawings: Vec<crate::model::Drawing>,
    pub(super) error: Option<&'static str>,
    pub(super) scale_mode: PriceScaleMode,
}

impl Harness {
    pub(super) fn new(canvas: ChartCanvas) -> Self {
        let mut card = ChartCard::default();
        card.appearance_mut().canvas = canvas;
        card.set_timeframe(Timeframe::Hour);
        Self {
            ctx: egui::Context::default(),
            card,
            candles: series(300),
            replay_active: false,
            pointer: None,
            crosshair: None,
            indicators: IndicatorConfig::default(),
            drawings: Vec::new(),
            error: None,
            scale_mode: PriceScaleMode::Regular,
        }
    }

    pub(super) fn frame(&mut self) -> (Vec<ClippedShape>, ChartPointer) {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, SIZE)),
            events: self
                .pointer
                .map(egui::Event::PointerMoved)
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let mut pointer = ChartPointer::default();
        let (card, candles, replay_active, crosshair, error, scale_mode) = (
            &mut self.card,
            &self.candles,
            self.replay_active,
            self.crosshair,
            self.error,
            self.scale_mode,
        );
        let (indicators, drawings) = (&self.indicators, &self.drawings);
        let mut output = self.ctx.run_ui(input, |ui| {
            let mut drawing = DrawingFrame::new(drawings, crate::model::Tool::Cursor, None, 1)
                .with_view(Timeframe::Hour, false);
            let mut replay = ReplayChartFrame {
                selecting: false,
                active: replay_active,
                picked_ms: None,
            };
            pointer = card.body(
                ui,
                "BTCUSDT",
                candles,
                false,
                error,
                scale_mode,
                DataSource::Binance,
                indicators,
                crosshair.as_ref(),
                1,
                &mut drawing,
                &mut replay,
            );
        });
        output.textures_delta.clear();
        (std::mem::take(&mut output.shapes), pointer)
    }

    /// Frames until the fades and the first layout have settled; the last frame's drawing.
    pub(super) fn draw(&mut self) -> (Drawn, ChartPointer) {
        let mut last = self.frame();
        for _ in 0..3 {
            last = self.frame();
        }
        (gather(&last.0), last.1)
    }
}

fn horizontal_lines(drawn: &Drawn) -> Vec<f32> {
    drawn
        .segments
        .iter()
        .filter(|(points, _)| points[0].y == points[1].y && (points[1].x - points[0].x) > 100.0)
        .map(|(points, _)| points[0].y)
        .collect()
}

/// The price axis' tick labels: not the last-price tag, which is one more number on the axis.
fn price_labels(drawn: &Drawn) -> Vec<(f32, String)> {
    let last_close = series(300).last().expect("candles").close;
    drawn
        .texts
        .iter()
        .filter(|(pos, ..)| pos.x > SIZE.x - 90.0)
        .filter_map(|(pos, text, _)| Some((pos.y, text.clone(), text.parse::<f64>().ok()?)))
        // The last price's tag is rounded to the chart's places (two for this price).
        .filter(|(_, _, value)| (value - last_close).abs() > 0.01)
        .map(|(y, text, _)| (y, text))
        .collect()
}

/// Every number on the price axis, tags included: what a chart prints at its right edge. A zero
/// count is drawn as its own smaller piece of text, lowered under the line; it reads here as the
/// subscript digits it stands for.
pub(super) fn axis_texts(drawn: &Drawn) -> Vec<String> {
    let mut pieces: Vec<&(Pos2, String, f32)> = drawn
        .texts
        .iter()
        .filter(|(pos, ..)| pos.x > SIZE.x - 90.0)
        .collect();
    pieces.sort_by(|a, b| a.0.y.total_cmp(&b.0.y));
    // Labels are many points apart; the pieces of one are within a few.
    let mut lines: Vec<Vec<&(Pos2, String, f32)>> = Vec::new();
    for piece in pieces {
        match lines.last_mut() {
            Some(line) if (piece.0.y - line[0].0.y).abs() < 6.0 => line.push(piece),
            _ => lines.push(vec![piece]),
        }
    }
    lines
        .into_iter()
        .map(|mut line| {
            line.sort_by(|a, b| a.0.x.total_cmp(&b.0.x));
            let largest = line.iter().map(|piece| piece.2).fold(0.0, f32::max);
            line.iter()
                .map(|(_, text, size)| {
                    if line_has_count(&line) && *size < largest - 0.5 {
                        text.chars()
                            .filter_map(|digit| digit.to_digit(10))
                            .filter_map(|digit| char::from_u32(0x2080 + digit))
                            .collect()
                    } else {
                        text.clone()
                    }
                })
                .collect()
        })
        .collect()
}

fn line_has_count(line: &[&(Pos2, String, f32)]) -> bool {
    line.len() == 3
}

/// The sample market scaled to a price of the given size (`1e-7` puts the closes near 0.00001).
fn scaled_series(factor: f64) -> Vec<Kline> {
    series(300)
        .into_iter()
        .map(|bar| Kline {
            open: bar.open * factor,
            high: bar.high * factor,
            low: bar.low * factor,
            close: bar.close * factor,
            ..bar
        })
        .collect()
}

const SUBSCRIPTS: [char; 10] = ['₀', '₁', '₂', '₃', '₄', '₅', '₆', '₇', '₈', '₉'];

#[test]
fn a_tiny_price_counts_its_zeros_on_the_axis_by_default() {
    let mut harness = Harness::new(ChartCanvas::default());
    harness.candles = scaled_series(1e-7);
    let (drawn, _) = harness.draw();
    let labels = axis_texts(&drawn);
    assert!(labels.len() >= 3, "{labels:?}");
    for label in &labels {
        assert!(
            label.starts_with("0.0") && label.contains(SUBSCRIPTS),
            "{label} counts its zeros"
        );
        assert!(!label.contains("0.00"), "{label} writes no run of zeros");
    }
}

#[test]
fn writing_the_zeros_out_gives_plain_decimals() {
    let mut harness = Harness::new(ChartCanvas::default());
    harness.candles = scaled_series(1e-7);
    harness.card.appearance_mut().axis_small_prices = SmallPrices::Decimals;
    let (drawn, _) = harness.draw();
    let labels = axis_texts(&drawn);
    assert!(labels.len() >= 3, "{labels:?}");
    for label in &labels {
        assert!(
            label.starts_with("0.0000") && !label.contains(SUBSCRIPTS),
            "{label} spells the zeros"
        );
    }
}

#[test]
fn pinned_places_reach_every_price_the_chart_writes() {
    let mut harness = Harness::new(ChartCanvas::default());
    harness.card.appearance_mut().axis_decimals = Some(1);
    let (drawn, _) = harness.draw();
    for label in axis_texts(&drawn).iter().filter(|text| text.contains('.')) {
        let places = label.split_once('.').map_or(0, |(_, rest)| rest.len());
        assert_eq!(places, 1, "{label}");
    }
}

#[test]
fn automatic_places_never_repeat_a_label_when_the_ticks_are_close() {
    let card = ChartCard::default();
    let ticks = [67_432.0, 67_432.5, 67_433.0, 67_433.5];
    let step = tick_step(&ticks);
    assert_eq!(step, 0.5);
    let (scale, format) = (
        PriceScale::linear(67_432.0, 67_433.5, 0.0, 100.0),
        card.appearance.price_format(),
    );
    let labels: Vec<String> = ticks
        .iter()
        .map(|tick| scale.tick_label(format, *tick, step))
        .collect();
    assert_eq!(labels, ["67,432.0", "67,432.5", "67,433.0", "67,433.5"]);
    // Ticks a unit apart need none; fewer than two ticks have no step to follow.
    assert_eq!(
        scale.tick_label(format, 67_432.0, tick_step(&[67_432.0, 67_433.0])),
        "67,432"
    );
    assert_eq!(tick_step(&[67_432.0]), 0.0);
    assert_eq!(tick_step(&[]), 0.0);
}

// MARK: Background

#[test]
fn a_solid_background_fills_the_chart_with_the_chosen_colour() {
    let mut canvas = ChartCanvas::default();
    canvas.background.solid = Some([10, 20, 30, 255]);
    let mut harness = Harness::new(canvas);
    let (shapes, _) = harness.frame();
    let filled = shapes.iter().any(|clipped| {
        matches!(&clipped.shape, Shape::Rect(rect)
            if rect.fill == Color32::from_rgb(10, 20, 30) && rect.rect.width() >= SIZE.x - 1.0)
    });
    assert!(filled, "the whole chart gets the colour");
}

#[test]
fn a_gradient_background_is_one_mesh_over_the_whole_chart() {
    let mut canvas = ChartCanvas::default();
    canvas.background.mode = BackgroundMode::Gradient;
    canvas.background.top = Some([1, 2, 3, 255]);
    canvas.background.bottom = Some([200, 100, 50, 255]);
    let mut harness = Harness::new(canvas);
    let (drawn, pointer) = harness.draw();
    let rect = pointer.rect.expect("a chart");
    let gradient: Vec<_> = drawn
        .meshes
        .iter()
        .filter(|mesh| mesh.vertices.len() == 4)
        .filter(|mesh| {
            mesh.vertices
                .iter()
                .any(|vertex| vertex.color == Color32::from_rgb(200, 100, 50))
        })
        .collect();
    assert_eq!(gradient.len(), 1, "one mesh, not a texture per frame");
    let bounds = gradient[0]
        .vertices
        .iter()
        .fold(Rect::NOTHING, |bounds, vertex| {
            bounds.union(Rect::from_min_max(vertex.pos, vertex.pos))
        });
    assert_eq!(bounds, rect, "plot, scales and panes share the backdrop");
    let tops: Vec<_> = gradient[0]
        .vertices
        .iter()
        .filter(|vertex| vertex.pos.y == rect.top())
        .map(|vertex| vertex.color)
        .collect();
    assert!(
        tops.iter()
            .all(|color| *color == Color32::from_rgb(1, 2, 3))
    );
}

// MARK: Grid

#[test]
fn the_horizontal_grid_stands_on_the_price_labels() {
    let mut harness = Harness::new(ChartCanvas::default());
    let (drawn, _) = harness.draw();
    let labels = price_labels(&drawn);
    let lines = horizontal_lines(&drawn);
    assert!(labels.len() >= 3, "{labels:?}");
    for (y, text) in &labels {
        assert!(
            lines.iter().any(|line| (line - y).abs() <= 0.6),
            "no grid line at the label {text} (y {y}); lines {lines:?}"
        );
    }
}

#[test]
fn the_price_labels_are_round_numbers_in_order() {
    let mut harness = Harness::new(ChartCanvas::default());
    let (drawn, _) = harness.draw();
    let mut labels = price_labels(&drawn);
    labels.sort_by(|a, b| a.0.total_cmp(&b.0));
    let values: Vec<f64> = labels
        .iter()
        .map(|(_, text)| text.parse().expect("number"))
        .collect();
    assert!(
        values.windows(2).all(|pair| pair[0] > pair[1]),
        "high at the top: {values:?}"
    );
    let steps: Vec<f64> = values.windows(2).map(|pair| pair[0] - pair[1]).collect();
    assert!(
        steps.iter().all(|step| (step - steps[0]).abs() < 1e-6),
        "evenly spaced prices: {values:?}"
    );
}

#[test]
fn the_vertical_grid_stands_on_the_time_labels() {
    let mut harness = Harness::new(ChartCanvas::default());
    let (drawn, pointer) = harness.draw();
    let rect = pointer.rect.expect("chart");
    let labels: Vec<f32> = drawn
        .texts
        .iter()
        .filter(|(pos, text, _)| pos.y > rect.bottom() - 30.0 && text.contains(':'))
        .map(|(pos, ..)| pos.x)
        .collect();
    assert!(labels.len() >= 3, "{labels:?}");
    let verticals: Vec<f32> = drawn
        .segments
        .iter()
        .filter(|(points, _)| {
            points[0].x == points[1].x && (points[1].y - points[0].y).abs() > 100.0
        })
        .map(|(points, _)| points[0].x)
        .collect();
    // The label is centred on the bar; its text box is centred, so compare centres.
    let centres = labels.clone();
    for centre in centres {
        assert!(
            verticals.iter().any(|x| (x - centre).abs() < 6.0),
            "no grid line near the label at {centre}; lines {verticals:?}"
        );
    }
}

#[test]
fn switched_off_grids_draw_no_lines() {
    let mut canvas = ChartCanvas::default();
    canvas.vertical_grid.visible = false;
    canvas.horizontal_grid.visible = false;
    let mut harness = Harness::new(canvas);
    let (drawn, pointer) = harness.draw();
    let rect = pointer.rect.expect("chart");
    let grid_colour = theme::grid();
    let faint = |stroke: &Stroke| {
        stroke.color == grid_colour || stroke.color == grid_colour.gamma_multiply(0.65)
    };
    assert!(
        drawn.segments.iter().all(|(_, stroke)| !faint(stroke)),
        "no grid-coloured lines remain"
    );
    // The labels are still there, and the scale lines.
    assert!(price_labels(&drawn).len() >= 3);
    assert!(
        drawn
            .segments
            .iter()
            .any(|(points, _)| points[0].x == points[1].x && points[0].x > rect.right() - 120.0)
    );
}

#[test]
fn a_dashed_grid_breaks_into_pieces() {
    let mut solid = Harness::new(ChartCanvas::default());
    let mut dashed_canvas = ChartCanvas::default();
    dashed_canvas.horizontal_grid.dash = LineDash::Dashed;
    let mut dashed = Harness::new(dashed_canvas);
    let (solid_drawn, _) = solid.draw();
    let (dashed_drawn, _) = dashed.draw();
    assert!(dashed_drawn.segments.len() > solid_drawn.segments.len() + 20);
}

#[test]
fn a_custom_grid_colour_is_used_as_picked_including_its_opacity() {
    let mut canvas = ChartCanvas::default();
    canvas.horizontal_grid.color = Some([255, 0, 0, 100]);
    let mut harness = Harness::new(canvas);
    let (drawn, _) = harness.draw();
    let expected = Color32::from_rgba_unmultiplied(255, 0, 0, 100);
    assert!(
        drawn
            .segments
            .iter()
            .filter(|(_, stroke)| stroke.color == expected)
            .count()
            >= 3
    );
}

// MARK: Scales

#[test]
fn the_scale_text_size_reaches_both_axes_and_widens_the_axis() {
    let mut small = Harness::new(ChartCanvas::default());
    let mut canvas = ChartCanvas::default();
    canvas.scales.text_size = ScaleTextSize::Pt16;
    canvas.scales.text_color = Some([9, 8, 7, 255]);
    let mut large = Harness::new(canvas);
    let (small_drawn, small_pointer) = small.draw();
    let (large_drawn, large_pointer) = large.draw();
    let size_of = |drawn: &Drawn, price: bool| {
        drawn
            .texts
            .iter()
            .find(|(pos, text, _)| {
                let rect_bottom = SIZE.y;
                if price {
                    pos.x > SIZE.x - 90.0 && text.parse::<f64>().is_ok()
                } else {
                    pos.y > rect_bottom - 40.0 && text.contains(':')
                }
            })
            .map(|(_, _, size)| *size)
    };
    assert_eq!(size_of(&small_drawn, true), Some(10.0));
    assert_eq!(size_of(&small_drawn, false), Some(9.0));
    assert_eq!(size_of(&large_drawn, true), Some(16.0));
    assert_eq!(size_of(&large_drawn, false), Some(15.0));
    let _ = (small_pointer, large_pointer);
    // The labels are in the colour picked, and only the scale texts are.
    let recoloured = large_drawn
        .texts
        .iter()
        .filter(|(_, text, _)| text.parse::<f64>().is_ok())
        .count();
    assert!(recoloured >= 3);
}

#[test]
fn a_larger_scale_text_makes_the_time_axis_taller_and_the_plot_shorter() {
    let plot_bottom = |size: ScaleTextSize| {
        let mut canvas = ChartCanvas::default();
        canvas.scales.text_size = size;
        let mut harness = Harness::new(canvas);
        let (drawn, _) = harness.draw();
        // The scale line along the time axis' top edge.
        drawn
            .segments
            .iter()
            .filter(|(points, _)| points[0].y == points[1].y && points[1].x - points[0].x > 100.0)
            .map(|(points, _)| points[0].y)
            .fold(f32::MIN, f32::max)
    };
    assert!(plot_bottom(ScaleTextSize::Pt18) < plot_bottom(ScaleTextSize::Pt10));
}

#[test]
fn scale_lines_use_their_own_colour() {
    let mut canvas = ChartCanvas::default();
    canvas.scales.line_color = Some([1, 200, 3, 255]);
    let mut harness = Harness::new(canvas);
    let (drawn, _) = harness.draw();
    let lines = drawn
        .segments
        .iter()
        .filter(|(_, stroke)| stroke.color == Color32::from_rgb(1, 200, 3))
        .count();
    assert!(lines >= 2, "the price axis' edge and the time axis' edge");
}

// MARK: Margins

fn plot_of(card: &ChartCard, candles: &[Kline]) -> (Rect, View) {
    let plot = Rect::from_min_size(Pos2::ZERO, vec2(600.0, 400.0));
    let view = card
        .view(candles, plot, PriceScaleMode::Regular, ChartType::Candles)
        .expect("a view");
    (plot, view)
}

#[test]
fn the_top_and_bottom_margins_place_the_data_by_their_share_of_the_plot() {
    let mut canvas = ChartCanvas::default();
    canvas.margins = ChartMargins {
        top: 20,
        bottom: 10,
        right: 0,
    };
    let mut card = ChartCard::default();
    card.appearance_mut().canvas = canvas;
    let candles = series(100);
    let (plot, view) = plot_of(&card, &candles);
    let shown = &candles[view.start..view.end];
    let high = shown.iter().map(|bar| bar.high).fold(f64::MIN, f64::max);
    let low = shown.iter().map(|bar| bar.low).fold(f64::MAX, f64::min);
    let top_gap = view.scale.y(high) - plot.top();
    let bottom_gap = plot.bottom() - view.scale.y(low);
    assert!((top_gap - 80.0).abs() < 0.5, "20% of 400: {top_gap}");
    assert!((bottom_gap - 40.0).abs() < 0.5, "10% of 400: {bottom_gap}");
}

#[test]
fn the_margins_follow_the_chart_whatever_its_type_or_scale() {
    for chart_type in [ChartType::Candles, ChartType::Line, ChartType::Area] {
        for log in [false, true] {
            let mut card = ChartCard::default();
            card.appearance_mut().canvas.margins = ChartMargins {
                top: 25,
                bottom: 15,
                right: 0,
            };
            card.set_chart_type(chart_type);
            let candles = series(100);
            let plot = Rect::from_min_size(Pos2::ZERO, vec2(600.0, 400.0));
            let view = card
                .view(
                    &candles,
                    plot,
                    PriceScaleMode::from_log_flag(log),
                    chart_type,
                )
                .expect("a view");
            let (low, high) = card.value_range(&candles[view.start..view.end], chart_type);
            let (top, bottom) = (view.scale.y(high), view.scale.y(low));
            assert!(
                (top - 100.0).abs() < 0.6 && (bottom - 340.0).abs() < 0.6,
                "{chart_type:?} log {log}: {top} {bottom}"
            );
        }
    }
}

#[test]
fn margins_never_change_what_is_drawn_or_scanned() {
    // The prices the scale spans differ; the candles do not.
    let candles = series(100);
    let mut tight = ChartCard::default();
    tight.appearance_mut().canvas.margins = ChartMargins {
        top: 0,
        bottom: 0,
        right: 0,
    };
    let mut roomy = ChartCard::default();
    roomy.appearance_mut().canvas.margins = ChartMargins {
        top: 40,
        bottom: 40,
        right: 0,
    };
    let (_, a) = plot_of(&tight, &candles);
    let (_, b) = plot_of(&roomy, &candles);
    assert!(b.scale.maximum > a.scale.maximum && b.scale.minimum < a.scale.minimum);
    assert_eq!((a.start, a.end), (b.start, b.end));
}

#[test]
fn the_right_margin_keeps_free_slots_after_the_newest_bar_at_the_current_spacing() {
    let candles = series(300);
    let mut card = ChartCard::default();
    card.appearance_mut().canvas.margins.right = 10;
    let (plot, view) = plot_of(&card, &candles);
    assert!(
        (view.slot_width - 600.0 / 130.0).abs() < 1e-4,
        "{}",
        view.slot_width
    );
    let newest_x = plot.left() + view.slot_width * (299.0 - view.start as f32 + 0.5 + view.lead);
    assert!((plot.right() - newest_x - 10.5 * view.slot_width).abs() < 0.01);
    // No margin: the newest bar sits at the edge, and the spacing is wider.
    card.appearance_mut().canvas.margins.right = 0;
    let (_, flush) = plot_of(&card, &candles);
    assert!(flush.slot_width > view.slot_width);
}

#[test]
fn the_right_margin_holds_while_scrolling_and_zooming() {
    let candles = series(300);
    let mut card = ChartCard::default();
    card.appearance_mut().canvas.margins.right = 10;
    let (_, rest) = plot_of(&card, &candles);
    card.viewport.horizontal.right_offset = 4.0;
    let (_, scrolled) = plot_of(&card, &candles);
    assert_eq!(
        rest.slot_width, scrolled.slot_width,
        "spacing is steady while scrolling"
    );
    assert_eq!(scrolled.end, 300, "the margin still shows empty slots");
    card.viewport.horizontal.right_offset = 50.0;
    let (_, back) = plot_of(&card, &candles);
    assert_eq!(back.end, 260);
    assert_eq!(rest.slot_width, back.slot_width);
    card.viewport.horizontal.visible = 60;
    card.viewport.horizontal.right_offset = 0.0;
    let (_, zoomed) = plot_of(&card, &candles);
    assert!((zoomed.slot_width - 600.0 / 70.0).abs() < 1e-4);
}

#[test]
fn the_right_margin_never_changes_the_data_a_script_reads() {
    let candles = series(300);
    let mut card = ChartCard::default();
    card.appearance_mut().canvas.margins.right = 0;
    let before = run_revision(&candles, None, 0);
    card.appearance_mut().canvas.margins.right = 80;
    assert_eq!(before, run_revision(&candles, None, 0));
}

#[test]
fn a_margin_with_few_candles_and_a_flat_series_still_draws() {
    for count in [0usize, 1, 2, 5] {
        let mut harness = Harness::new(ChartCanvas::default());
        harness.candles = series(count);
        let (drawn, _) = harness.draw();
        assert!(
            drawn
                .segments
                .iter()
                .all(|(points, _)| points.iter().all(|p| p.is_finite()))
        );
    }
    let mut harness = Harness::new(ChartCanvas::default());
    harness.candles = (0..50)
        .map(|index| Kline {
            open_time: index * HOUR,
            close_time: (index + 1) * HOUR - 1,
            open: 5.0,
            high: 5.0,
            low: 5.0,
            close: 5.0,
            volume: 1.0,
        })
        .collect();
    let (drawn, _) = harness.draw();
    assert!(
        drawn
            .segments
            .iter()
            .all(|(points, _)| points.iter().all(|p| p.is_finite()))
    );
    assert!(drawn.texts.iter().all(|(pos, ..)| pos.is_finite()));
}

// MARK: Watermark

fn watermarks(drawn: &Drawn) -> Vec<&(Pos2, String, f32)> {
    drawn
        .texts
        .iter()
        .filter(|(_, text, size)| *size >= 16.0 && text.contains("BTC"))
        .collect()
}

#[test]
fn the_watermark_follows_its_mode_and_the_replay() {
    for (mode, replaying, shown) in [
        (WatermarkMode::Hidden, false, false),
        (WatermarkMode::Hidden, true, false),
        (WatermarkMode::Symbol, false, true),
        (WatermarkMode::Symbol, true, true),
        (WatermarkMode::Replay, false, false),
        (WatermarkMode::Replay, true, true),
    ] {
        let mut canvas = ChartCanvas::default();
        canvas.watermark.mode = mode;
        canvas.watermark.color = Some([255, 255, 255, 40]);
        let mut harness = Harness::new(canvas);
        harness.replay_active = replaying;
        let (drawn, _) = harness.draw();
        assert_eq!(
            !watermarks(&drawn).is_empty(),
            shown,
            "{mode:?} replay {replaying}"
        );
    }
}

#[test]
fn the_watermark_names_the_market_timeframe_and_source_and_sits_behind_the_candles() {
    let mut canvas = ChartCanvas::default();
    canvas.watermark.mode = WatermarkMode::Symbol;
    let mut harness = Harness::new(canvas);
    let (shapes, _) = harness.frame();
    let (shapes, _) = {
        let _ = shapes;
        harness.frame()
    };
    let order: Vec<(usize, String)> = shapes
        .iter()
        .enumerate()
        .flat_map(|(index, clipped)| {
            let mut one = Drawn::default();
            collect(&clipped.shape, &mut one);
            one.texts.into_iter().map(move |(_, text, _)| (index, text))
        })
        .collect();
    let drawn = gather(&shapes);
    let title = watermarks(&drawn);
    assert_eq!(title.len(), 1);
    assert!(
        drawn
            .texts
            .iter()
            .any(|(_, text, _)| text.contains("1H") && text.contains("Binance")),
        "{:?}",
        drawn
            .texts
            .iter()
            .map(|(_, text, _)| text)
            .collect::<Vec<_>>()
    );
    // It is painted before the first candle, i.e. behind the series.
    let watermark_at = order
        .iter()
        .position(|(_, text)| text.contains("BTC"))
        .expect("drawn");
    let candle_at = shapes
        .iter()
        .position(|clipped| {
            matches!(&clipped.shape, Shape::Rect(rect) if rect.fill == Color32::from_rgb(48, 209, 88)
                || rect.fill == Color32::from_rgb(255, 69, 58))
        })
        .expect("candles are drawn");
    let watermark_shape = order[watermark_at].0;
    assert!(
        watermark_shape < candle_at,
        "{watermark_shape} vs {candle_at}"
    );
}

// MARK: Crosshair

#[test]
fn the_crosshair_takes_its_style_and_stays_above_the_series() {
    let mut canvas = ChartCanvas::default();
    canvas.crosshair.color = Some([255, 128, 0, 200]);
    canvas.crosshair.width = 3;
    canvas.crosshair.dash = LineDash::Solid;
    let mut harness = Harness::new(canvas);
    harness.crosshair = Some(CrosshairInfo {
        owner_id: 1,
        x_fraction: 0.5,
        price: 230.0,
    });
    let (drawn, _) = harness.draw();
    let colour = Color32::from_rgba_unmultiplied(255, 128, 0, 200);
    let lines: Vec<_> = drawn
        .segments
        .iter()
        .filter(|(_, stroke)| stroke.color == colour)
        .collect();
    assert_eq!(lines.len(), 2, "one vertical and one horizontal line");
    assert!(lines.iter().all(|(_, stroke)| stroke.width == 3.0));
    // Only another chart's column carries just the vertical line.
    harness.crosshair = Some(CrosshairInfo {
        owner_id: 99,
        x_fraction: 0.5,
        price: 230.0,
    });
    let (other, _) = harness.draw();
    assert_eq!(
        other
            .segments
            .iter()
            .filter(|(_, stroke)| stroke.color == colour)
            .count(),
        1
    );
}

#[test]
fn a_dashed_crosshair_is_the_default_and_a_style_change_keeps_the_position() {
    let mut harness = Harness::new(ChartCanvas::default());
    harness.crosshair = Some(CrosshairInfo {
        owner_id: 1,
        x_fraction: 0.25,
        price: 230.0,
    });
    let (dashed, _) = harness.draw();
    harness.card.appearance_mut().canvas.crosshair.dash = LineDash::Solid;
    let (solid, _) = harness.draw();
    let verticals = |drawn: &Drawn| -> Vec<f32> {
        drawn
            .segments
            .iter()
            .filter(|(points, stroke)| {
                points[0].x == points[1].x
                    && stroke.color == Color32::from_gray(135).gamma_multiply(0.6)
            })
            .map(|(points, _)| points[0].x)
            .collect()
    };
    assert!(verticals(&dashed).len() > 5, "dashes");
    let x = verticals(&solid);
    assert_eq!(x.len(), 1, "a solid line is one segment");
    let again: Vec<f32> = verticals(&dashed);
    assert!(
        again.iter().all(|v| (v - x[0]).abs() < 0.6),
        "same column either way"
    );
}

#[test]
fn the_ohlc_readout_hides_without_touching_the_crosshair() {
    let mut harness = Harness::new(ChartCanvas::default());
    harness.crosshair = Some(CrosshairInfo {
        owner_id: 1,
        x_fraction: 0.5,
        price: 230.0,
    });
    let crosshair_lines = |drawn: &Drawn| {
        let colour = Color32::from_gray(135).gamma_multiply(0.6);
        drawn
            .segments
            .iter()
            .filter(|(_, stroke)| stroke.color == colour)
            .count()
    };
    let readout = |drawn: &Drawn| {
        ["O ", "H ", "L ", "C "]
            .into_iter()
            .filter(|label| {
                drawn
                    .texts
                    .iter()
                    .any(|(_, text, _)| text.starts_with(label))
            })
            .count()
    };
    let (shown, _) = harness.draw();
    assert_eq!(readout(&shown), 4, "open, high, low and close are drawn");

    harness.card.appearance_mut().canvas.ohlc_readout = false;
    let (hidden, _) = harness.draw();
    assert_eq!(readout(&hidden), 0, "the values box is gone");
    assert_eq!(
        crosshair_lines(&hidden),
        crosshair_lines(&shown),
        "the crosshair is unchanged"
    );
    assert!(
        hidden.texts.iter().any(|(_, text, _)| !text.is_empty()),
        "the axis tags still draw"
    );
}

// MARK: Presentation only

#[test]
fn changing_the_canvas_never_rebuilds_the_bars_or_reruns_scripts() {
    let mut harness = Harness::new(ChartCanvas::default());
    harness.card.set_chart_type(ChartType::Renko);
    harness.draw();
    let built = harness.card.constructed.clone().expect("constructed bars");
    let revision = run_revision(&harness.candles, None, 0);
    let changes: [fn(&mut ChartCanvas); 6] = [
        |canvas| canvas.background.mode = BackgroundMode::Gradient,
        |canvas| canvas.vertical_grid.visible = false,
        |canvas| canvas.scales.text_size = ScaleTextSize::Pt18,
        |canvas| canvas.margins.right = 50,
        |canvas| canvas.margins.top = 30,
        |canvas| canvas.watermark.mode = WatermarkMode::Symbol,
    ];
    for change in changes {
        change(&mut harness.card.appearance_mut().canvas);
        harness.draw();
        let now = harness.card.constructed.clone().expect("still constructed");
        assert!(Arc::ptr_eq(&built, &now), "the Renko bars were rebuilt");
    }
    assert_eq!(revision, run_revision(&harness.candles, None, 0));
}

#[test]
fn every_canvas_draws_finite_geometry_on_every_chart_type_and_size() {
    let mut canvas = ChartCanvas::default();
    canvas.background.mode = BackgroundMode::Gradient;
    canvas.watermark.mode = WatermarkMode::Symbol;
    canvas.scales.text_size = ScaleTextSize::Pt18;
    canvas.margins = ChartMargins {
        top: 45,
        bottom: 35,
        right: 200,
    };
    for chart_type in ChartType::ALL {
        let mut harness = Harness::new(canvas);
        harness.card.set_chart_type(chart_type);
        let (drawn, _) = harness.draw();
        for (points, stroke) in &drawn.segments {
            assert!(
                points.iter().all(|p| p.is_finite()) && stroke.width.is_finite(),
                "{chart_type:?}"
            );
        }
        assert!(
            drawn.texts.iter().all(|(pos, ..)| pos.is_finite()),
            "{chart_type:?}"
        );
    }
}

// MARK: Buttons

#[test]
fn the_navigation_buttons_follow_their_visibility() {
    let button_texts = |visibility: ButtonVisibility, pointer: Option<Pos2>| -> bool {
        let mut canvas = ChartCanvas::default();
        canvas.buttons.navigation = visibility;
        let mut harness = Harness::new(canvas);
        harness.pointer = pointer;
        let (shapes, _) = harness.frame();
        let _ = shapes;
        for _ in 0..4 {
            harness.frame();
        }
        let (shapes, _) = harness.frame();
        // The group is a rounded pill with a border, bottom-left of the plot.
        shapes.iter().any(|clipped| {
            matches!(&clipped.shape, Shape::Rect(rect)
                if rect.rect.width() > 100.0 && rect.rect.width() < 140.0
                    && rect.rect.height() > 20.0 && rect.rect.height() < 30.0
                    && rect.rect.left() < 40.0)
        })
    };
    let over = Some(Pos2::new(300.0, 200.0));
    assert!(button_texts(ButtonVisibility::Always, None));
    assert!(button_texts(ButtonVisibility::OnHover, over));
    assert!(!button_texts(ButtonVisibility::OnHover, None));
    assert!(!button_texts(ButtonVisibility::Never, over));
}

#[test]
fn navigation_buttons_drive_the_one_viewport() {
    let mut card = ChartCard::default();
    let total = 500;
    card.apply_navigation(NavAction::ZoomIn, total);
    assert!(card.viewport.horizontal.visible < 120);
    let zoomed_in = card.viewport.horizontal.visible;
    card.apply_navigation(NavAction::ZoomOut, total);
    assert!(card.viewport.horizontal.visible > zoomed_in);
    card.apply_navigation(NavAction::ScrollLeft, total);
    assert!(card.viewport.horizontal.right_offset > 0.0);
    let scrolled = card.viewport.horizontal.right_offset;
    card.apply_navigation(NavAction::ScrollRight, total);
    assert!(card.viewport.horizontal.right_offset < scrolled);
    card.viewport.vertical.zoom = 3.0;
    card.apply_navigation(NavAction::Reset, total);
    assert_eq!(
        (
            card.viewport.horizontal.visible,
            card.viewport.horizontal.right_offset,
            card.viewport.vertical.zoom
        ),
        (120, 0.0, 1.0)
    );
}

#[test]
fn navigation_stays_inside_the_data_and_the_zoom_limits() {
    let mut card = ChartCard::default();
    for _ in 0..40 {
        card.apply_navigation(NavAction::ZoomIn, 500);
    }
    assert_eq!(card.viewport.horizontal.visible, MIN_VISIBLE_CANDLES);
    for _ in 0..80 {
        card.apply_navigation(NavAction::ZoomOut, 500);
    }
    assert_eq!(card.viewport.horizontal.visible, 500);
    for _ in 0..80 {
        card.apply_navigation(NavAction::ScrollLeft, 500);
    }
    assert_eq!(
        card.viewport.horizontal.right_offset,
        (500 - MIN_VISIBLE_CANDLES) as f64
    );
    for _ in 0..200 {
        card.apply_navigation(NavAction::ScrollRight, 500);
    }
    assert_eq!(card.viewport.horizontal.right_offset, 0.0);
    // Too little data to scroll at all, and none of it makes the bars wider: the chart keeps the
    // bar width a deep market has instead of jumping to fit what it holds.
    let mut tiny = ChartCard::default();
    tiny.apply_navigation(NavAction::ZoomOut, 5);
    tiny.apply_navigation(NavAction::ScrollLeft, 5);
    assert_eq!(
        (
            tiny.viewport.horizontal.visible,
            tiny.viewport.horizontal.right_offset
        ),
        (crate::model::viewport::DEFAULT_VISIBLE, 0.0)
    );
}

// MARK: Panes

const PANE_SCRIPT: &str = "//@version=6\nindicator(\"Pane\")\nplot(close)\n";
const PANE_SCRIPT_B: &str = "//@version=6\nindicator(\"Second\")\nplot(open)\n";
const OVERLAY_SCRIPT: &str = "//@version=6\nindicator(\"Overlay\", overlay = true)\nplot(close)\n";

pub(super) fn wait_for_run(harness: &mut Harness, id: u64) {
    for _ in 0..200 {
        harness.frame();
        if harness.card.pine_status(id).is_some() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("Pine script {id} did not finish");
}

#[test]
fn a_new_overlay_script_never_claims_a_pane_while_it_evaluates() {
    let mut harness = Harness::new(ChartCanvas::default());
    let id = harness
        .card
        .add_pine_instance(None, "Overlay", OVERLAY_SCRIPT);

    harness.frame();
    assert!(
        harness.card.pine_status(id).is_none(),
        "the first frame only schedules the run"
    );
    assert_eq!(
        harness.card.pane_count(),
        0,
        "unknown placement must not default to an own pane"
    );

    wait_for_run(&mut harness, id);
    assert!(harness.card.pine_status(id).unwrap().output.overlay);
    assert_eq!(harness.card.pane_count(), 0, "the result stays overlaid");
}

#[test]
fn the_script_manager_preview_also_waits_for_real_output() {
    let mut harness = Harness::new(ChartCanvas::default());
    harness
        .card
        .set_pine_source(Some(OVERLAY_SCRIPT.to_owned()));

    harness.frame();
    assert!(harness.card.pine_visual().is_none());
    assert_eq!(harness.card.pane_count(), 0);

    wait_for_run(&mut harness, PREVIEW_INSTANCE);
    assert!(harness.card.pine_visual().unwrap().overlay);
    assert_eq!(harness.card.pane_count(), 0);
}

#[test]
fn an_own_pane_appears_only_after_the_script_confirms_it() {
    let mut harness = Harness::new(ChartCanvas::default());
    let id = harness.card.add_pine_instance(None, "Pane", PANE_SCRIPT);

    harness.frame();
    assert!(harness.card.pine_status(id).is_none());
    assert_eq!(harness.card.pane_count(), 0);

    wait_for_run(&mut harness, id);
    assert!(!harness.card.pine_status(id).unwrap().output.overlay);
    assert_eq!(harness.card.pane_count(), 1);
}

#[test]
fn a_script_that_cannot_compile_reports_the_error_without_claiming_a_pane() {
    let mut harness = Harness::new(ChartCanvas::default());
    let id = harness
        .card
        .add_pine_instance(None, "Broken", "indicator(\"Broken\")\nplot(");

    harness.frame();
    assert!(harness.card.pine_status(id).is_none());
    wait_for_run(&mut harness, id);
    assert!(harness.card.pine_status(id).unwrap().error.is_some());
    assert_eq!(harness.card.pane_count(), 0);
}

#[test]
fn a_reconfigured_script_keeps_its_last_valid_placement_until_the_new_run_arrives() {
    let mut harness = Harness::new(ChartCanvas::default());
    harness.card.set_pine_source(Some(PANE_SCRIPT.to_owned()));
    harness.frame();
    wait_for_run(&mut harness, PREVIEW_INSTANCE);
    assert_eq!(harness.card.pane_count(), 1);

    harness
        .card
        .set_pine_source(Some(OVERLAY_SCRIPT.to_owned()));
    harness.frame();
    assert_eq!(
        harness.card.pane_count(),
        1,
        "the last valid output remains visible while the replacement evaluates"
    );

    for _ in 0..200 {
        harness.frame();
        if harness
            .card
            .pine_status(PREVIEW_INSTANCE)
            .is_some_and(|status| status.output.overlay)
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(harness.card.pine_visual().unwrap().overlay);
    assert_eq!(harness.card.pane_count(), 0);
}

const LOGGING_SCRIPT: &str =
    "//@version=6\nindicator(\"Logging\", overlay = true)\nlog.info(\"bar {0}\", bar_index)\n";

#[test]
fn a_runs_log_messages_reach_the_card() {
    let mut harness = Harness::new(ChartCanvas::default());
    let id = harness
        .card
        .add_pine_instance(None, "Logging", LOGGING_SCRIPT);
    wait_for_run(&mut harness, id);
    let logs = harness.card.pine_logs(id).expect("a run");
    assert_eq!(logs.history.len(), 300);
    assert_eq!(logs.history.buffer().get(299).unwrap().message, "bar 299");
    assert!(!logs.history.gap);
}

#[test]
fn a_script_without_log_calls_has_an_empty_history() {
    let mut harness = Harness::new(ChartCanvas::default());
    let id = harness.card.add_pine_instance(None, "Pane", PANE_SCRIPT);
    wait_for_run(&mut harness, id);
    assert!(harness.card.pine_logs(id).unwrap().history.is_empty());
}

#[test]
fn a_restarted_run_gets_a_new_epoch_and_forgets_the_old_messages() {
    let mut harness = Harness::new(ChartCanvas::default());
    harness
        .card
        .set_pine_source(Some(LOGGING_SCRIPT.to_owned()));
    wait_for_run(&mut harness, PREVIEW_INSTANCE);
    let first = harness.card.pine_logs(PREVIEW_INSTANCE).unwrap().epoch;

    harness.card.set_pine_source(Some(
        "//@version=6\nindicator(\"Other\", overlay = true)\nlog.warning(\"other\")\n".to_owned(),
    ));
    for _ in 0..200 {
        harness.frame();
        let logs = harness.card.pine_logs(PREVIEW_INSTANCE).unwrap();
        if logs.epoch != first && !logs.history.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let logs = harness.card.pine_logs(PREVIEW_INSTANCE).unwrap();
    assert_ne!(logs.epoch, first);
    assert!(
        logs.history
            .buffer()
            .iter()
            .all(|event| event.message == "other")
    );
}

#[test]
fn logging_can_be_paused_per_script_without_losing_what_was_kept() {
    let mut harness = Harness::new(ChartCanvas::default());
    let id = harness
        .card
        .add_pine_instance(None, "Logging", LOGGING_SCRIPT);
    wait_for_run(&mut harness, id);
    let kept = harness.card.pine_logs(id).unwrap().history.len();
    assert!(harness.card.pine_logging(id));
    harness.card.set_pine_logging(id, false);
    assert!(!harness.card.pine_logging(id));
    // Pausing never touches the history, and the script keeps running.
    assert_eq!(harness.card.pine_logs(id).unwrap().history.len(), kept);
    harness.card.set_pine_logging(id, true);
    assert!(harness.card.pine_logging(id));
}

#[test]
fn removing_a_script_drops_its_messages() {
    let mut harness = Harness::new(ChartCanvas::default());
    let id = harness
        .card
        .add_pine_instance(None, "Logging", LOGGING_SCRIPT);
    wait_for_run(&mut harness, id);
    harness.card.remove_pine_instance(id);
    harness.frame();
    assert!(harness.card.pine_logs(id).is_none());
}

/// A chart with two script panes, drawn until their runs have arrived.
pub(super) fn with_panes(canvas: ChartCanvas) -> Harness {
    let mut harness = Harness::new(canvas);
    harness.card.add_pine_instance(None, "Pane", PANE_SCRIPT);
    harness
        .card
        .add_pine_instance(None, "Second", PANE_SCRIPT_B);
    for _ in 0..200 {
        harness.frame();
        if harness.card.pane_count() == 2 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(harness.card.pane_count(), 2, "the scripts ran");
    harness
}

#[test]
fn panes_get_their_separators_and_the_canvas_grid() {
    let mut harness = with_panes(ChartCanvas::default());
    let (drawn, _) = harness.draw();
    let separators = drawn
        .segments
        .iter()
        .filter(|(points, stroke)| {
            points[0].y == points[1].y
                && points[1].x - points[0].x > 400.0
                && stroke.color == theme::grid()
        })
        .count();
    assert!(separators >= 2, "one above each pane plus the main grid");
    // Without separators none of those full-width pane lines remain at the pane tops.
    harness.card.appearance_mut().canvas.pane_separator.visible = false;
    let (without, _) = harness.draw();
    let tops = |drawn: &Drawn| {
        drawn
            .segments
            .iter()
            .filter(|(points, _)| points[0].y == points[1].y && points[1].x - points[0].x > 400.0)
            .filter(|(points, _)| points[0].y > SIZE.y * 0.65)
            .count()
    };
    assert!(
        tops(&without) < tops(&drawn),
        "{} vs {}",
        tops(&without),
        tops(&drawn)
    );
}

#[test]
fn pane_buttons_move_collapse_maximize_and_remove_the_right_script() {
    let mut harness = with_panes(ChartCanvas::default());
    let ids = harness.card.pane_ids.clone();
    assert_eq!(ids.len(), 2);
    let names = |card: &ChartCard| -> Vec<String> {
        card.pine_instances()
            .iter()
            .map(|i| i.name.clone())
            .collect()
    };
    assert_eq!(names(&harness.card), ["Pane", "Second"]);
    harness.card.apply_pane_action(ids[0], PaneAction::MoveDown);
    assert_eq!(
        names(&harness.card),
        ["Second", "Pane"],
        "the order is the scripts' order"
    );
    harness.card.apply_pane_action(ids[0], PaneAction::MoveUp);
    assert_eq!(names(&harness.card), ["Pane", "Second"]);
    // Moving the first pane up, or the last down, changes nothing.
    harness.card.apply_pane_action(ids[0], PaneAction::MoveUp);
    harness.card.apply_pane_action(ids[1], PaneAction::MoveDown);
    assert_eq!(names(&harness.card), ["Pane", "Second"]);

    harness
        .card
        .apply_pane_action(ids[1], PaneAction::ToggleCollapse);
    assert!(harness.card.pine_instances()[1].pane_collapsed);
    assert!(!harness.card.pine_instances()[0].pane_collapsed);
    assert_eq!(
        harness.card.pine_instances().len(),
        2,
        "collapsing removes nothing"
    );
    harness.draw();
    assert_eq!(
        harness.card.pane_count(),
        2,
        "a collapsed script still runs and keeps its pane"
    );
    harness
        .card
        .apply_pane_action(ids[1], PaneAction::ToggleMaximize);
    assert_eq!(harness.card.maximized_pane, Some(ids[1]));
    assert!(
        !harness.card.pine_instances()[1].pane_collapsed,
        "maximizing expands"
    );
    harness.draw();
    harness
        .card
        .apply_pane_action(ids[1], PaneAction::ToggleMaximize);
    assert_eq!(harness.card.maximized_pane, None);

    harness.card.apply_pane_action(ids[0], PaneAction::Remove);
    assert_eq!(
        names(&harness.card),
        ["Second"],
        "only the intended script went"
    );
}

#[test]
fn a_maximized_pane_leaves_the_time_axis_and_a_vanished_one_restores_the_chart() {
    let mut harness = with_panes(ChartCanvas::default());
    let ids = harness.card.pane_ids.clone();
    harness
        .card
        .apply_pane_action(ids[0], PaneAction::ToggleMaximize);
    let (drawn, pointer) = harness.draw();
    let rect = pointer.rect.expect("chart");
    assert!(
        drawn.texts.iter().any(|(pos, text, _)| pos.y > rect.top()
            && pos.y < rect.top() + 40.0
            && text.contains(':')),
        "the time axis stays, at the top"
    );
    assert!(
        price_labels(&drawn)
            .iter()
            .all(|(y, _)| *y > rect.top() + 24.0),
        "no price chart"
    );
    harness.card.apply_pane_action(ids[0], PaneAction::Remove);
    harness.draw();
    assert_eq!(
        harness.card.maximized_pane, None,
        "nothing is left maximized"
    );
    let (restored, _) = harness.draw();
    assert!(price_labels(&restored).len() >= 3);
}

// MARK: Templates

#[test]
fn applying_a_template_restyles_the_chart_and_keeps_everything_else() {
    let mut harness = Harness::new(ChartCanvas::default());
    harness.card.add_pine_instance(None, "Pane", PANE_SCRIPT);
    harness.card.set_y_zoom(2.0);
    harness.card.set_visible_candles(80);
    harness.card.set_series_cutoff(Some(5));
    harness.card.set_title(Some("Title".into()));
    harness.card.appearance_mut().axis_decimals = Some(3);
    let mut template = ChartAppearance::default();
    template.bullish = [1, 2, 3];
    template.canvas.margins.right = 33;
    template.canvas.background.mode = BackgroundMode::Gradient;
    template.style.chart_type = ChartType::Renko;
    template.axis_decimals = Some(6);
    harness.card.viewport.horizontal.right_offset = 7.0;
    harness.card.apply_template(
        &template,
        crate::model::chart_template::ApplyScope::AppearanceAndCanvas,
    );
    let after = harness.card.appearance();
    assert_eq!(after.bullish, [1, 2, 3]);
    assert_eq!(after.canvas, template.canvas);
    assert_eq!(after.style.chart_type, ChartType::Renko);
    assert_eq!(after.axis_decimals, Some(3), "the market's precision stays");
    assert_eq!(harness.card.pine_instances().len(), 1);
    assert_eq!(harness.card.y_zoom(), 2.0);
    assert_eq!(harness.card.visible_candles(), 80);
    assert_eq!(harness.card.title(), Some("Title"));
    assert_eq!(
        harness.card.viewport.horizontal.right_offset, 0.0,
        "a price-based type starts from its newest bar"
    );
}

#[test]
fn colours_only_keeps_the_type_and_the_scroll() {
    let mut harness = Harness::new(ChartCanvas::default());
    harness.card.set_chart_type(ChartType::Kagi);
    harness.card.viewport.horizontal.right_offset = 12.0;
    let mut template = ChartAppearance::default();
    template.style.chart_type = ChartType::Candles;
    template.canvas.horizontal_grid.visible = false;
    harness.card.apply_template(
        &template,
        crate::model::chart_template::ApplyScope::ColorsAndCanvas,
    );
    assert_eq!(harness.card.appearance().style.chart_type, ChartType::Kagi);
    assert!(!harness.card.appearance().canvas.horizontal_grid.visible);
    assert_eq!(harness.card.viewport.horizontal.right_offset, 12.0);
}

#[test]
fn a_restored_card_heals_a_canvas_with_out_of_range_values() {
    let mut card = ChartCard::default();
    let mut appearance = ChartAppearance::default();
    appearance.canvas.margins = ChartMargins {
        top: 250,
        bottom: 250,
        right: 60_000,
    };
    appearance.canvas.crosshair.width = 99;
    card.set_appearance(appearance);
    let healed = card.appearance().canvas;
    assert_eq!(healed, healed.sanitized());
    assert!(healed.margins.top + healed.margins.bottom <= ChartMargins::MAX_COMBINED);
}

#[test]
fn every_pane_labels_its_own_scale_with_at_least_two_prices() {
    let mut harness = with_panes(ChartCanvas::default());
    let (drawn, pointer) = harness.draw();
    let rect = pointer.rect.expect("chart");
    let pane_top = rect.top() + rect.height() * 0.68;
    let labels = |from: f32, to: f32| {
        drawn
            .texts
            .iter()
            .filter(|(pos, text, _)| {
                pos.x > SIZE.x - 90.0
                    && pos.y > from
                    && pos.y < to
                    && (text.parse::<f64>().is_ok() || text == "0")
            })
            .count()
    };
    let middle = pane_top + (rect.bottom() - pane_top) / 2.0;
    assert!(labels(pane_top, middle) >= 2, "first pane");
    assert!(labels(middle, rect.bottom()) >= 2, "second pane");
}

#[test]
fn a_failed_load_with_no_candles_shows_only_the_message() {
    let mut harness = Harness::new(ChartCanvas::default());
    harness.candles.clear();
    harness.error = Some("Binance is down");
    let (drawn, pointer) = harness.draw();
    let texts: Vec<&str> = drawn
        .texts
        .iter()
        .map(|(_, text, _)| text.as_str())
        .collect();
    assert_eq!(texts, ["Binance is down"]);
    assert!(drawn.segments.is_empty(), "no grid or axis without data");
    let rect = pointer.rect.expect("the chart still reports its rectangle");
    let (centre, _, _) = &drawn.texts[0];
    assert!((centre.x - rect.center().x).abs() < 1.0 && (centre.y - rect.center().y).abs() < 1.0);
    assert!(pointer.hover_price.is_none() && pointer.context.is_none());
}

#[test]
fn no_candles_and_no_error_draws_an_empty_chart_without_panicking() {
    let mut harness = Harness::new(ChartCanvas::default());
    harness.candles.clear();
    harness.pointer = Some(Pos2::new(300.0, 200.0));
    let (drawn, pointer) = harness.draw();
    assert!(drawn.texts.is_empty() && drawn.segments.is_empty());
    assert!(pointer.rect.is_some());
    assert!(pointer.hover_price.is_none() && pointer.hover_anchor.is_none());
}

#[test]
fn a_failed_refresh_keeps_the_last_candles_and_adds_a_warning() {
    let mut harness = Harness::new(ChartCanvas::default());
    harness.error = Some("timeout");
    let (drawn, pointer) = harness.draw();
    assert!(
        price_labels(&drawn).len() >= 2,
        "the chart is still drawn with its axis"
    );
    assert!(
        drawn
            .texts
            .iter()
            .any(|(_, text, _)| text.contains("last result")),
        "{:?}",
        drawn.texts
    );
    assert!(pointer.rect.is_some());
}
