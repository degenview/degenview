//! What the Studio's tests share: a captured chart from the card's own test harness, the
//! lookup the composer finds its card by, a context to paint on and shapes written out to be
//! compared.

use super::capture::{Capture, ChartCapture, ChartParts};
use crate::model::chart::canvas::BackgroundMode;
use crate::model::chart::price_scale::PriceScaleMode;
use crate::model::chart::snapshot::studio::{Area, AspectRatio, Background, StudioConfig, Style};
use crate::model::{ChartCanvas, DataSource, Timeframe};
use crate::service::snapshot::raster;
use crate::ui::chart::card::studio_fixture::Fixture;
use crate::ui::chart::snapshot_scene::Images;
use crate::ui::theme;
use eframe::egui::epaint::ClippedShape;
use eframe::egui::{self, Color32, ColorImage, Painter, Pos2, Rect, Shape, Vec2};

pub(super) const CHART_ID: u64 = 1;

/// A light canvas, as a user who sets one makes it: the chart's colours are its own.
pub(super) fn light_canvas() -> ChartCanvas {
    let mut canvas = ChartCanvas::default();
    canvas.background.mode = BackgroundMode::Solid;
    canvas.background.solid = Some([250, 251, 252, 255]);
    canvas.scales.text_color = Some([96, 104, 116, 255]);
    canvas.scales.line_color = Some([214, 218, 224, 255]);
    canvas.vertical_grid.color = Some([236, 239, 243, 255]);
    canvas.horizontal_grid.color = Some([236, 239, 243, 255]);
    canvas.pane_separator.color = Some([214, 218, 224, 255]);
    canvas
}

pub(super) fn parts(fixture: &Fixture, chart_id: u64) -> ChartParts {
    ChartParts {
        chart_id,
        symbol: "BTCUSDT".to_owned(),
        source: DataSource::Binance,
        timeframe: Timeframe::Hour,
        title: "BTC/USDT".to_owned(),
        candles: fixture.candles(),
        drawings: fixture.drawings(),
        indicators: fixture.indicators(),
        open_notes: Vec::new(),
        images: Images::new(),
        replay_active: false,
        include_trading: false,
        scale_mode: PriceScaleMode::Regular,
        logo: None,
    }
}

pub(super) fn chart_capture(
    fixture: &Fixture,
    chart_id: u64,
    portion: Option<Rect>,
) -> ChartCapture {
    ChartCapture::new(fixture.card(), parts(fixture, chart_id), 1.0, portion)
        .expect("a chart that can be captured")
}

pub(super) fn single(fixture: &Fixture) -> Capture {
    Capture::Single(chart_capture(fixture, CHART_ID, None))
}

/// The Styled look in `size` on `background`.
pub(super) fn styled(size: AspectRatio, background: Background) -> StudioConfig {
    let mut config = StudioConfig::new(Area::FullChart);
    config.set_style(Style::Styled);
    config.size = size;
    config.background = background;
    config
}

/// The chart as it is on screen: the Chart look, which the Studio no longer opens with.
pub(super) fn plain() -> StudioConfig {
    let mut config = StudioConfig::new(Area::FullChart);
    config.set_style(Style::Chart);
    config
}

/// Paints into a context of its own at `pixels_per_point`, on a screen of `size` points, and
/// gives the shapes. The textures it uploaded are dropped: nothing is drawn from them here.
pub(super) fn paint_shapes(
    size: Vec2,
    pixels_per_point: f32,
    paint: impl FnOnce(&Painter, Rect),
) -> Vec<ClippedShape> {
    let ctx = egui::Context::default();
    theme::install_offscreen(&ctx);
    let rect = Rect::from_min_size(Pos2::ZERO, size);
    let raw = egui::RawInput {
        screen_rect: Some(rect),
        viewports: std::iter::once((
            egui::ViewportId::ROOT,
            egui::ViewportInfo {
                native_pixels_per_point: Some(pixels_per_point),
                ..Default::default()
            },
        ))
        .collect(),
        ..Default::default()
    };
    let mut paint = Some(paint);
    let mut output = ctx.run_ui(raw, |ui| {
        if let Some(paint) = paint.take() {
            paint(&ui.painter_at(rect), rect);
        }
    });
    output.textures_delta.clear();
    std::mem::take(&mut output.shapes)
}

/// A shape reduced to what it draws and where, positions in points, so two paintings at other
/// scales can be compared.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Flat {
    Text {
        text: String,
        pos: Pos2,
        color: Color32,
    },
    Rect {
        rect: Rect,
        fill: Color32,
    },
    Circle {
        center: Pos2,
        radius: f32,
        fill: Color32,
    },
    Line(Vec<Pos2>),
    Mesh {
        vertices: usize,
        bounds: Rect,
    },
    Other(String),
}

fn flatten_shape(shape: &Shape, out: &mut Vec<Flat>) {
    match shape {
        Shape::Vec(shapes) => shapes.iter().for_each(|shape| flatten_shape(shape, out)),
        Shape::Noop => {}
        Shape::Text(text) => out.push(Flat::Text {
            text: text.galley.text().to_owned(),
            pos: text.pos,
            color: text
                .galley
                .job
                .sections
                .first()
                .map_or(Color32::PLACEHOLDER, |section| section.format.color),
        }),
        Shape::Rect(rect) => out.push(Flat::Rect {
            rect: rect.rect,
            fill: rect.fill,
        }),
        Shape::Circle(circle) => out.push(Flat::Circle {
            center: circle.center,
            radius: circle.radius,
            fill: circle.fill,
        }),
        Shape::LineSegment { points, .. } => out.push(Flat::Line(points.to_vec())),
        Shape::Path(path) => out.push(Flat::Line(path.points.clone())),
        Shape::Mesh(mesh) => out.push(Flat::Mesh {
            vertices: mesh.vertices.len(),
            bounds: Rect::from_points(&mesh.vertices.iter().map(|v| v.pos).collect::<Vec<_>>()),
        }),
        other => out.push(Flat::Other(format!("{other:?}").chars().take(60).collect())),
    }
}

pub(super) fn flatten(shapes: &[ClippedShape]) -> Vec<Flat> {
    let mut out = Vec::new();
    for clipped in shapes {
        flatten_shape(&clipped.shape, &mut out);
    }
    out
}

/// Every word the shapes write.
pub(super) fn texts(shapes: &[ClippedShape]) -> Vec<String> {
    flatten(shapes)
        .into_iter()
        .filter_map(|flat| match flat {
            Flat::Text { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

fn near(a: Pos2, b: Pos2, tolerance: f32) -> bool {
    (a - b).abs().max_elem() <= tolerance
}

fn rects_near(a: Rect, b: Rect, tolerance: f32) -> bool {
    near(a.min, b.min, tolerance) && near(a.max, b.max, tolerance)
}

/// Whether two shapes draw the same thing, their positions within `tolerance` points.
fn same(a: &Flat, b: &Flat, tolerance: f32) -> bool {
    match (a, b) {
        (
            Flat::Text { text, pos, color },
            Flat::Text {
                text: other,
                pos: at,
                color: tint,
            },
        ) => text == other && color == tint && near(*pos, *at, tolerance),
        (
            Flat::Rect { rect, fill },
            Flat::Rect {
                rect: other,
                fill: tint,
            },
        ) => fill == tint && rects_near(*rect, *other, tolerance),
        (
            Flat::Circle {
                center,
                radius,
                fill,
            },
            Flat::Circle {
                center: at,
                radius: other,
                fill: tint,
            },
        ) => fill == tint && (radius - other).abs() <= tolerance && near(*center, *at, tolerance),
        (Flat::Line(a), Flat::Line(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| near(*a, *b, tolerance))
        }
        (
            Flat::Mesh { vertices, bounds },
            Flat::Mesh {
                vertices: other,
                bounds: around,
            },
        ) => vertices == other && rects_near(*bounds, *around, tolerance),
        (Flat::Other(a), Flat::Other(b)) => a == b,
        _ => false,
    }
}

/// The first place two paintings differ, if they do.
pub(super) fn first_difference(a: &[Flat], b: &[Flat], tolerance: f32) -> Option<String> {
    if a.len() != b.len() {
        return Some(format!("{} shapes against {}", a.len(), b.len()));
    }
    a.iter()
        .zip(b)
        .enumerate()
        .find(|(_, (a, b))| !same(a, b, tolerance))
        .map(|(index, (a, b))| format!("shape {index}: {a:?} against {b:?}"))
}

pub(super) fn rasterized(scene: &raster::Scene) -> ColorImage {
    raster::rasterize(scene).expect("a scene to draw")
}

/// The pixel at (`x`, `y`) of `image`.
pub(super) fn pixel(image: &ColorImage, x: usize, y: usize) -> Color32 {
    image.pixels[y * image.size[0] + x]
}

/// Whether two colours are within `tolerance` on every channel.
pub(super) fn close(a: Color32, b: Color32, tolerance: u8) -> bool {
    a.to_array()
        .iter()
        .zip(b.to_array())
        .all(|(a, b)| a.abs_diff(b) <= tolerance)
}
