//! Paints the chart canvas: resolves a chart's [`ChartCanvas`] against the app theme once per
//! frame ([`CanvasStyle`]) and draws the background, grid, separators, crosshair lines,
//! watermark and scale lines from it. The one place a canvas colour is decided, so no painter
//! module resolves a theme colour of its own.
//!
//! Everything here is presentation: it reads no market data and holds no state, so a change of
//! canvas repaints without reaching the constructed bars, the indicators or Pine.

use super::theme;
use crate::model::ChartCanvas;
use crate::model::chart::canvas::{BackgroundMode, ButtonVisibility, LineAppearance};
use crate::model::chart::kind::{LineDash, Rgba};
use eframe::egui::{Color32, FontId, Mesh, Painter, Pos2, Rect, Shape, Stroke};

/// How the chart's backdrop is filled.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fill {
    Solid(Color32),
    /// Vertical, `top` to `bottom`.
    Gradient {
        top: Color32,
        bottom: Color32,
    },
}

/// One kind of line, resolved: its stroke and how it is broken up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineStyle {
    pub stroke: Stroke,
    pub dash: LineDash,
}

/// A chart's canvas against the active theme. Cheap to build (a handful of colours), so it is
/// made each frame rather than cached: a theme switch is picked up at once, and an explicit
/// colour is never replaced by a theme one.
#[derive(Clone, Copy, Debug)]
pub struct CanvasStyle {
    pub fill: Fill,
    /// `None` when the line is switched off.
    pub vertical_grid: Option<LineStyle>,
    pub horizontal_grid: Option<LineStyle>,
    pub separator: Option<LineStyle>,
    pub crosshair: LineStyle,
    pub watermark: Color32,
    pub text_color: Color32,
    /// The price axis' size in points; the time axis reads one smaller.
    pub price_size: f32,
    pub time_size: f32,
    pub scale_line: Color32,
    pub navigation: ButtonVisibility,
    pub pane_buttons: ButtonVisibility,
    /// Whether the crosshair carries its box of values.
    pub ohlc_readout: bool,
}

/// An `Rgba` as the straight-alpha colour egui paints.
pub fn rgba(color: Rgba) -> Color32 {
    Color32::from_rgba_unmultiplied(color[0], color[1], color[2], color[3])
}

fn line(appearance: &LineAppearance, theme_color: Color32) -> Option<LineStyle> {
    appearance.visible.then(|| LineStyle {
        stroke: Stroke::new(
            f32::from(appearance.width),
            appearance.color.map_or(theme_color, rgba),
        ),
        dash: appearance.dash,
    })
}

/// A one-point solid line in `color`.
pub fn solid(color: Color32) -> LineStyle {
    LineStyle {
        stroke: Stroke::new(1.0, color),
        dash: LineDash::Solid,
    }
}

/// `a` moved `t` of the way to `b`.
fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let channel = |from: u8, to: u8| {
        (f32::from(from) + (f32::from(to) - f32::from(from)) * t)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Color32::from_rgb(
        channel(a.r(), b.r()),
        channel(a.g(), b.g()),
        channel(a.b(), b.b()),
    )
}

/// The theme colours a canvas leaves to the theme.
#[derive(Clone, Copy, Debug)]
pub struct ThemeColors {
    pub surface: Color32,
    pub grid: Color32,
    pub text: Color32,
    pub border: Color32,
    pub ink: Color32,
    /// How far the default gradient's bottom leans towards the accent.
    pub gradient_tint: f32,
}

impl ThemeColors {
    pub fn current() -> Self {
        Self {
            surface: theme::surface(),
            grid: theme::grid(),
            text: theme::text_secondary(),
            border: theme::border(),
            ink: theme::ink(22),
            gradient_tint: theme::pick_value(0.16, 0.10),
        }
    }
}

impl CanvasStyle {
    pub fn resolve(canvas: &ChartCanvas) -> Self {
        Self::resolve_with(canvas, ThemeColors::current())
    }

    pub fn resolve_with(canvas: &ChartCanvas, colors: ThemeColors) -> Self {
        let background = &canvas.background;
        let surface = colors.surface;
        let fill = match background.mode {
            BackgroundMode::Solid => Fill::Solid(background.solid.map_or(surface, rgba)),
            BackgroundMode::Gradient => Fill::Gradient {
                top: background.top.map_or(surface, rgba),
                // The surface leaning towards the accent: a visible but quiet gradient.
                bottom: background.bottom.map_or_else(
                    || mix(surface, theme::SYSTEM_ACCENT, colors.gradient_tint),
                    rgba,
                ),
            },
        };
        let grid = colors.grid;
        let size = canvas.scales.text_size;
        Self {
            fill,
            vertical_grid: line(&canvas.vertical_grid, grid.gamma_multiply(0.65)),
            horizontal_grid: line(&canvas.horizontal_grid, grid),
            separator: line(&canvas.pane_separator, grid),
            crosshair: line(
                &LineAppearance {
                    visible: true,
                    ..canvas.crosshair
                },
                Color32::from_gray(135).gamma_multiply(0.6),
            )
            .expect("a crosshair is always drawn"),
            watermark: canvas.watermark.color.map_or(colors.ink, rgba),
            text_color: canvas.scales.text_color.map_or(colors.text, rgba),
            price_size: size.points(),
            time_size: size.time_points(),
            scale_line: canvas.scales.line_color.map_or(colors.border, rgba),
            navigation: canvas.buttons.navigation,
            pane_buttons: canvas.buttons.pane,
            ohlc_readout: canvas.ohlc_readout,
        }
    }

    pub fn price_font(&self) -> FontId {
        FontId::monospace(self.price_size)
    }

    pub fn time_font(&self) -> FontId {
        FontId::monospace(self.time_size)
    }

    /// Half the height of a price tag on the axis: the label with a margin, 9 points at the
    /// default size.
    pub fn tag_half_height(&self) -> f32 {
        (self.price_size + 8.0) / 2.0
    }

    /// The time axis: the label with room around it, 24 points at the default size.
    pub fn time_axis_height(&self) -> f32 {
        (self.time_size * 2.0 + 6.0).max(24.0)
    }

    /// The least the price axis is wide, before the labels it holds ask for more.
    pub fn min_axis_width(&self, base: f32) -> f32 {
        base * (self.price_size / 10.0).max(1.0)
    }

    /// The label spacing in a script pane, which is short: closer than the price chart's.
    pub fn pane_tick_spacing(&self) -> f32 {
        (self.price_size * 2.8).max(28.0)
    }

    /// The label spacing the horizontal grid and the price axis share.
    pub fn price_tick_spacing(&self) -> f32 {
        (self.price_size * 5.6).max(52.0)
    }
}

// MARK: Geometry helpers

/// `coordinate` moved to the pixel centre a `width`-point line needs to stay one crisp,
/// full-intensity row or column of pixels at `pixels_per_point`.
pub fn crisp(coordinate: f32, width: f32, pixels_per_point: f32) -> f32 {
    let scale = pixels_per_point.max(0.5);
    let pixels = coordinate * scale;
    let thick = (width * scale).round().max(1.0);
    let snapped = if thick as u32 % 2 == 1 {
        pixels.floor() + 0.5
    } else {
        pixels.round()
    };
    snapped / scale
}

/// The dash and gap of a style in points, `None` for a solid line. Lengths are logical points,
/// so a line looks the same on every display.
pub fn dash_pattern(dash: LineDash, width: f32) -> Option<(f32, f32)> {
    match dash {
        LineDash::Solid => None,
        LineDash::Dashed => Some((6.0 + width, 4.0)),
        LineDash::Dotted => Some((width.max(1.0), 2.0 + width)),
    }
}

/// Draws `from → to` in `style`.
pub fn stroke_segment(painter: &Painter, from: Pos2, to: Pos2, style: LineStyle) {
    if !(from.is_finite() && to.is_finite()) {
        return;
    }
    match dash_pattern(style.dash, style.stroke.width) {
        None => {
            painter.line_segment([from, to], style.stroke);
        }
        Some((dash, gap)) => {
            painter.extend(Shape::dashed_line(&[from, to], style.stroke, dash, gap));
        }
    }
}

/// A horizontal line across `x_range` at `y`, pixel-aligned.
pub fn horizontal(painter: &Painter, x_range: (f32, f32), y: f32, style: LineStyle) {
    let y = crisp(y, style.stroke.width, painter.ctx().pixels_per_point());
    stroke_segment(
        painter,
        Pos2::new(x_range.0, y),
        Pos2::new(x_range.1, y),
        style,
    );
}

/// A vertical line down `y_range` at `x`, pixel-aligned.
pub fn vertical(painter: &Painter, x: f32, y_range: (f32, f32), style: LineStyle) {
    let x = crisp(x, style.stroke.width, painter.ctx().pixels_per_point());
    stroke_segment(
        painter,
        Pos2::new(x, y_range.0),
        Pos2::new(x, y_range.1),
        style,
    );
}

// MARK: Painters

/// Fills `rect` with the chart's backdrop. A gradient is one four-vertex mesh the GPU
/// interpolates, so it costs the same at any size, has no texture to recreate and follows every
/// resize and display scale exactly.
pub fn paint_background(painter: &Painter, rect: Rect, fill: Fill) {
    if !(rect.is_finite() && rect.width() > 0.0 && rect.height() > 0.0) {
        return;
    }
    match fill {
        Fill::Solid(color) => {
            painter.rect_filled(rect, 0.0, color);
        }
        Fill::Gradient { top, bottom } => {
            let mut mesh = Mesh::default();
            mesh.colored_vertex(rect.left_top(), top);
            mesh.colored_vertex(rect.right_top(), top);
            mesh.colored_vertex(rect.right_bottom(), bottom);
            mesh.colored_vertex(rect.left_bottom(), bottom);
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(0, 2, 3);
            painter.add(Shape::mesh(mesh));
        }
    }
}

/// The lines that bound the scales: the price axis' left edge, down to `bottom`, and the time
/// axis' top edge, out to `right`.
pub fn paint_scale_lines(painter: &Painter, plot: Rect, right: f32, bottom: f32, color: Color32) {
    let style = solid(color);
    vertical(painter, plot.right(), (plot.top(), bottom), style);
    horizontal(painter, (plot.left(), right), plot.bottom(), style);
}

/// The most space a watermark may take of the plot, as a share of its width and height.
const WATERMARK_WIDTH_SHARE: f32 = 0.7;
const WATERMARK_HEIGHT_SHARE: f32 = 0.24;
/// A watermark smaller than this reads as noise: it is left out.
const WATERMARK_MIN_SIZE: f32 = 16.0;
const WATERMARK_MAX_SIZE: f32 = 72.0;

/// The font size of a watermark title that is `width_at_max × height_at_max` points at the
/// largest size, fitted into a `plot_width × plot_height` plot; `None` when it would not be
/// legible.
pub fn watermark_size(
    plot_width: f32,
    plot_height: f32,
    width_at_max: f32,
    height_at_max: f32,
) -> Option<f32> {
    if ![plot_width, plot_height, width_at_max, height_at_max]
        .iter()
        .all(|value| value.is_finite() && *value > 0.0)
    {
        return None;
    }
    let fit = (plot_width * WATERMARK_WIDTH_SHARE / width_at_max)
        .min(plot_height * WATERMARK_HEIGHT_SHARE / height_at_max)
        .min(1.0);
    let size = WATERMARK_MAX_SIZE * fit;
    (size >= WATERMARK_MIN_SIZE).then_some(size)
}

/// A faint title and subtitle centred in `plot`, behind everything the chart draws.
pub fn paint_watermark(painter: &Painter, plot: Rect, title: &str, subtitle: &str, color: Color32) {
    if color.a() == 0 || title.is_empty() {
        return;
    }
    let reference = painter.layout_no_wrap(
        title.to_owned(),
        FontId::proportional(WATERMARK_MAX_SIZE),
        color,
    );
    let Some(size) = watermark_size(
        plot.width(),
        plot.height(),
        reference.size().x,
        reference.size().y,
    ) else {
        return;
    };
    let title = painter.layout_no_wrap(title.to_owned(), FontId::proportional(size), color);
    let subtitle = (!subtitle.is_empty()).then(|| {
        painter.layout_no_wrap(
            subtitle.to_owned(),
            FontId::proportional((size * 0.38).max(11.0)),
            color,
        )
    });
    let gap = size * 0.08;
    let block = title.size().y + subtitle.as_ref().map_or(0.0, |text| gap + text.size().y);
    let top = plot.center().y - block / 2.0;
    let title_pos = Pos2::new(plot.center().x - title.size().x / 2.0, top);
    let below = title.size().y + gap;
    painter.galley(title_pos, title, color);
    if let Some(subtitle) = subtitle {
        let at = Pos2::new(plot.center().x - subtitle.size().x / 2.0, top + below);
        painter.galley(at, subtitle, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::chart::canvas::WatermarkMode;
    use eframe::egui::{self, Vec2};

    fn ctx_with<R>(run: impl FnOnce(&Painter) -> R) -> (R, Vec<egui::epaint::ClippedShape>) {
        let ctx = egui::Context::default();
        let mut result = None;
        let mut run = Some(run);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let painter = ui.painter().clone();
            if let Some(run) = run.take() {
                result = Some(run(&painter));
            }
        });
        output.textures_delta.clear();
        (result.expect("ran"), std::mem::take(&mut output.shapes))
    }

    fn dark() -> ThemeColors {
        ThemeColors {
            surface: Color32::from_rgb(20, 26, 28),
            grid: Color32::from_rgb(36, 44, 48),
            text: Color32::from_rgb(152, 152, 157),
            border: Color32::from_rgb(44, 53, 57),
            ink: Color32::from_white_alpha(22),
            gradient_tint: 0.16,
        }
    }

    fn light() -> ThemeColors {
        ThemeColors {
            surface: Color32::WHITE,
            grid: Color32::from_rgb(236, 237, 240),
            text: Color32::from_rgb(110, 110, 115),
            border: Color32::from_rgb(208, 209, 213),
            ink: Color32::from_black_alpha(22),
            gradient_tint: 0.10,
        }
    }

    #[test]
    fn the_default_canvas_resolves_to_the_theme() {
        for colors in [dark(), light()] {
            let style = CanvasStyle::resolve_with(&ChartCanvas::default(), colors);
            assert_eq!(style.fill, Fill::Solid(colors.surface));
            assert_eq!(style.horizontal_grid.expect("on").stroke.color, colors.grid);
            assert_eq!(
                style.vertical_grid.expect("on").stroke.color,
                colors.grid.gamma_multiply(0.65)
            );
            assert_eq!(style.text_color, colors.text);
            assert_eq!(style.scale_line, colors.border);
            assert_eq!(style.watermark, colors.ink);
            assert_eq!(style.price_size, 10.0);
            assert_eq!(style.time_size, 9.0);
            assert_eq!(style.crosshair.dash, LineDash::Dashed);
        }
    }

    #[test]
    fn explicit_colours_win_over_the_theme_and_survive_a_switch() {
        let mut canvas = ChartCanvas::default();
        canvas.background.solid = Some([10, 20, 30, 255]);
        canvas.horizontal_grid.color = Some([200, 0, 0, 80]);
        canvas.scales.text_color = Some([1, 2, 3, 255]);
        let before = CanvasStyle::resolve_with(&canvas, dark());
        let after = CanvasStyle::resolve_with(&canvas, light());
        for style in [before, after] {
            assert_eq!(style.fill, Fill::Solid(Color32::from_rgb(10, 20, 30)));
            assert_eq!(
                style.horizontal_grid.expect("on").stroke.color,
                Color32::from_rgba_unmultiplied(200, 0, 0, 80)
            );
            assert_eq!(style.text_color, Color32::from_rgb(1, 2, 3));
        }
        // The colours left to the theme did follow it.
        assert_ne!(before.scale_line, after.scale_line);
        assert_ne!(before.vertical_grid, after.vertical_grid);
    }

    #[test]
    fn a_gradient_resolves_both_stops_and_keeps_the_solid_colour_aside() {
        let mut canvas = ChartCanvas::default();
        canvas.background.mode = BackgroundMode::Gradient;
        canvas.background.solid = Some([9, 9, 9, 255]);
        canvas.background.top = Some([1, 1, 1, 255]);
        canvas.background.bottom = Some([2, 2, 2, 255]);
        let style = CanvasStyle::resolve(&canvas);
        assert_eq!(
            style.fill,
            Fill::Gradient {
                top: Color32::from_rgb(1, 1, 1),
                bottom: Color32::from_rgb(2, 2, 2)
            }
        );
        canvas.background.mode = BackgroundMode::Solid;
        assert_eq!(
            CanvasStyle::resolve(&canvas).fill,
            Fill::Solid(Color32::from_rgb(9, 9, 9))
        );
        // Unset stops default to the surface and a tint of it.
        canvas.background = crate::model::chart::canvas::BackgroundAppearance {
            mode: BackgroundMode::Gradient,
            ..Default::default()
        };
        let Fill::Gradient { top, bottom } = CanvasStyle::resolve(&canvas).fill else {
            panic!("a gradient");
        };
        assert_eq!(top, theme::surface());
        assert_ne!(bottom, top);
    }

    #[test]
    fn a_switched_off_line_resolves_to_nothing_but_the_crosshair_stays() {
        let mut canvas = ChartCanvas::default();
        canvas.vertical_grid.visible = false;
        canvas.horizontal_grid.visible = false;
        canvas.pane_separator.visible = false;
        canvas.crosshair.visible = false;
        let style = CanvasStyle::resolve(&canvas);
        assert!(style.vertical_grid.is_none());
        assert!(style.horizontal_grid.is_none());
        assert!(style.separator.is_none());
        assert_eq!(style.crosshair.stroke.width, 1.0);
    }

    #[test]
    fn the_scale_size_grows_the_axis_geometry() {
        let mut canvas = ChartCanvas::default();
        let base = CanvasStyle::resolve(&canvas);
        assert_eq!(base.tag_half_height(), 9.0);
        assert_eq!(base.time_axis_height(), 24.0);
        assert_eq!(base.min_axis_width(82.0), 82.0);
        canvas.scales.text_size = crate::model::chart::canvas::ScaleTextSize::Pt18;
        let large = CanvasStyle::resolve(&canvas);
        assert!(large.tag_half_height() > base.tag_half_height());
        assert!(large.time_axis_height() > base.time_axis_height());
        assert!(large.min_axis_width(82.0) > 82.0);
        assert!(large.price_tick_spacing() > base.price_tick_spacing());
    }

    #[test]
    fn crisp_lines_land_on_pixel_centres_at_every_scale() {
        // One point is one pixel at 1×: the centre of the pixel.
        assert_eq!(crisp(10.2, 1.0, 1.0), 10.5);
        assert_eq!(crisp(10.9, 1.0, 1.0), 10.5);
        // At 2× a one-point line is two pixels wide: on a pixel boundary.
        assert_eq!(crisp(10.2, 1.0, 2.0), 10.0);
        assert_eq!(crisp(10.3, 1.0, 2.0), 10.5);
        // A three-pixel line wants a pixel centre again.
        assert_eq!(crisp(10.2, 3.0, 1.0), 10.5);
        assert!(crisp(0.0, 1.0, 0.0).is_finite());
    }

    #[test]
    fn dashes_are_logical_lengths_and_solid_has_none() {
        assert_eq!(dash_pattern(LineDash::Solid, 1.0), None);
        let (dash, gap) = dash_pattern(LineDash::Dashed, 1.0).expect("dashed");
        assert!(dash > gap && gap > 0.0);
        let (dot, space) = dash_pattern(LineDash::Dotted, 1.0).expect("dotted");
        assert!(dot <= 1.0 && space > dot);
        // A thicker dotted line keeps round dots.
        let (thick_dot, _) = dash_pattern(LineDash::Dotted, 3.0).expect("dotted");
        assert_eq!(thick_dot, 3.0);
    }

    #[test]
    fn a_gradient_is_one_mesh_covering_its_rectangle() {
        let rect = Rect::from_min_size(Pos2::new(10.0, 20.0), Vec2::new(300.0, 200.0));
        let top = Color32::from_rgb(10, 0, 0);
        let bottom = Color32::from_rgb(0, 0, 10);
        let (_, shapes) = ctx_with(|painter| {
            paint_background(painter, rect, Fill::Gradient { top, bottom });
        });
        let meshes: Vec<&Mesh> = shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                Shape::Mesh(mesh) => Some(&**mesh),
                _ => None,
            })
            .collect();
        assert_eq!(meshes.len(), 1);
        let mesh = meshes[0];
        assert_eq!(mesh.vertices.len(), 4);
        let bounds = mesh.vertices.iter().fold(Rect::NOTHING, |bounds, vertex| {
            bounds.union(Rect::from_min_max(vertex.pos, vertex.pos))
        });
        assert_eq!(bounds, rect);
        for vertex in &mesh.vertices {
            let expected = if vertex.pos.y == rect.top() {
                top
            } else {
                bottom
            };
            assert_eq!(vertex.color, expected);
        }
    }

    #[test]
    fn an_empty_or_unbounded_rectangle_paints_nothing() {
        let (_, shapes) = ctx_with(|painter| {
            paint_background(
                painter,
                Rect::from_min_size(Pos2::ZERO, Vec2::ZERO),
                Fill::Solid(Color32::RED),
            );
            paint_background(painter, Rect::EVERYTHING, Fill::Solid(Color32::RED));
            paint_background(painter, Rect::NOTHING, Fill::Solid(Color32::RED));
        });
        assert!(shapes.is_empty());
    }

    #[test]
    fn horizontal_and_vertical_lines_are_pixel_aligned_segments() {
        let style = LineStyle {
            stroke: Stroke::new(1.0, Color32::WHITE),
            dash: LineDash::Solid,
        };
        let (_, shapes) = ctx_with(|painter| {
            horizontal(painter, (0.0, 100.0), 33.2, style);
            vertical(painter, 12.7, (5.0, 50.0), style);
        });
        let segments: Vec<[Pos2; 2]> = shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                Shape::LineSegment { points, .. } => Some(*points),
                _ => None,
            })
            .collect();
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0][0].y, segments[0][1].y);
        assert_eq!(segments[0][0].y, 33.5);
        assert_eq!(segments[1][0].x, 12.5);
        assert_eq!((segments[1][0].y, segments[1][1].y), (5.0, 50.0));
    }

    #[test]
    fn dashed_lines_break_into_several_pieces_within_their_extent() {
        let style = LineStyle {
            stroke: Stroke::new(1.0, Color32::WHITE),
            dash: LineDash::Dashed,
        };
        let (_, shapes) = ctx_with(|painter| {
            horizontal(painter, (0.0, 100.0), 20.0, style);
        });
        let pieces: Vec<[Pos2; 2]> = shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                Shape::LineSegment { points, .. } => Some(*points),
                _ => None,
            })
            .collect();
        assert!(pieces.len() > 5, "{} pieces", pieces.len());
        for piece in pieces {
            assert!(piece.iter().all(|point| (0.0..=100.001).contains(&point.x)));
        }
    }

    #[test]
    fn non_finite_lines_are_not_drawn() {
        let style = LineStyle {
            stroke: Stroke::new(1.0, Color32::WHITE),
            dash: LineDash::Dashed,
        };
        let (_, shapes) = ctx_with(|painter| {
            stroke_segment(
                painter,
                Pos2::new(f32::NAN, 0.0),
                Pos2::new(1.0, 1.0),
                style,
            );
            horizontal(
                painter,
                (0.0, f32::INFINITY),
                5.0,
                LineStyle {
                    dash: LineDash::Solid,
                    ..style
                },
            );
        });
        assert!(shapes.is_empty());
    }

    #[test]
    fn the_watermark_fits_the_plot_and_vanishes_when_unreadable() {
        // A title 400 wide and 80 tall at the largest size, in a roomy plot: the largest size.
        assert_eq!(watermark_size(2000.0, 1000.0, 400.0, 80.0), Some(72.0));
        // A narrow plot shrinks it to fit the width share.
        let narrow = watermark_size(300.0, 1000.0, 400.0, 80.0).expect("fits");
        assert!((narrow - 72.0 * 300.0 * 0.7 / 400.0).abs() < 1e-3);
        // A short plot shrinks it to the height share.
        let short = watermark_size(2000.0, 150.0, 400.0, 80.0).expect("fits");
        assert!((short - 72.0 * 150.0 * 0.24 / 80.0).abs() < 1e-3);
        // Too small to read, and nonsense input, draw nothing.
        assert_eq!(watermark_size(100.0, 60.0, 400.0, 80.0), None);
        assert_eq!(watermark_size(0.0, 100.0, 400.0, 80.0), None);
        assert_eq!(watermark_size(f32::NAN, 100.0, 400.0, 80.0), None);
        assert_eq!(watermark_size(100.0, 100.0, 0.0, 80.0), None);
    }

    #[test]
    fn the_watermark_is_centred_in_the_plot() {
        let plot = Rect::from_min_size(Pos2::new(40.0, 10.0), Vec2::new(800.0, 500.0));
        let (_, shapes) = ctx_with(|painter| {
            paint_watermark(
                painter,
                plot,
                "BTCUSDT",
                "1H · Binance",
                Color32::from_white_alpha(30),
            );
        });
        let texts: Vec<Rect> = shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                Shape::Text(text) => Some(text.visual_bounding_rect()),
                _ => None,
            })
            .collect();
        assert_eq!(texts.len(), 2, "a title and a subtitle");
        let block = texts[0].union(texts[1]);
        assert!(plot.contains_rect(block));
        assert!(
            // Glyph side bearings shift the ink a few points off the layout box.
            (block.center().x - plot.center().x).abs() < 6.0,
            "{block:?} in {plot:?}"
        );
        assert!((block.center().y - plot.center().y).abs() < block.height() * 0.2);
        assert!(texts[0].width() > texts[1].width());
    }

    #[test]
    fn a_transparent_or_empty_watermark_draws_nothing() {
        let plot = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 500.0));
        let (_, shapes) = ctx_with(|painter| {
            paint_watermark(painter, plot, "BTC", "1H", Color32::TRANSPARENT);
            paint_watermark(painter, plot, "", "1H", Color32::WHITE);
            paint_watermark(
                painter,
                Rect::from_min_size(Pos2::ZERO, Vec2::new(60.0, 40.0)),
                "BTC",
                "",
                Color32::WHITE,
            );
        });
        assert!(shapes.is_empty());
    }

    #[test]
    fn watermark_modes_decide_by_replay() {
        assert!(WatermarkMode::Symbol.shows(false));
        assert!(!WatermarkMode::Replay.shows(false));
    }
}
