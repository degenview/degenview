//! The backdrops a Styled chart can sit on. Each is a pure function of where in the image a point
//! is ([`color_at`]), so the corner masks that round the chart's corners ask the very function the
//! backdrop was painted from and match it exactly.
//!
//! The colours are the picture's content, not the app's chrome: the palettes are DegenView's own
//! (a deep teal with a cyan glow, warm pink to orange, purple to teal through magenta, deep blue
//! with a bright glow, charcoal with dots) and are kept deep enough for a light or dark chart
//! card to stand out. A gradient is a vertex-coloured grid, dense enough to look smooth at 8K;
//! the dots are circles on a fixed grid in points, so they stay sharp at any scale.

use crate::model::chart::snapshot::studio::Background;
use eframe::egui::{Color32, Mesh, Painter, Pos2, Rect, Shape, Vec2, pos2, vec2};

/// The grid a gradient is made of. Smooth at 8K: a cell is a few dozen pixels, and the colour
/// changes slowly enough over one that linear shading between its corners cannot be told from
/// the function itself.
const COLUMNS: usize = 96;
const ROWS: usize = 64;

/// The dots of [`Background::Dots`]: their pitch and radius, in points.
const DOT_SPACING: f32 = 14.0;
const DOT_RADIUS: f32 = 1.2;

type Rgb = [f32; 3];

const fn rgb(hex: u32) -> Rgb {
    [
        ((hex >> 16) & 0xff) as f32,
        ((hex >> 8) & 0xff) as f32,
        (hex & 0xff) as f32,
    ]
}

const CHARCOAL: u32 = 0x16181c;
const DOT: u32 = 0x2c2f36;

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

fn color(rgb: Rgb) -> Color32 {
    let channel = |value: f32| value.round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(channel(rgb[0]), channel(rgb[1]), channel(rgb[2]))
}

/// A colour through `stops` (position, colour), evenly blended between neighbours.
fn through(stops: &[(f32, Rgb)], at: f32) -> Rgb {
    let first = stops[0];
    let last = stops[stops.len() - 1];
    if at <= first.0 {
        return first.1;
    }
    for pair in stops.windows(2) {
        let ((from, a), (to, b)) = (pair[0], pair[1]);
        if at <= to {
            return mix(a, b, (at - from) / (to - from));
        }
    }
    last.1
}

/// 1 at `centre`, falling smoothly to 0 `radius` away (per axis, in the image's own proportions).
fn glow(uv: Vec2, centre: Vec2, radius: Vec2) -> f32 {
    let offset = vec2((uv.x - centre.x) / radius.x, (uv.y - centre.y) / radius.y);
    let t = (1.0 - offset.length()).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The colour of `background` at `uv`, `(0, 0)` the image's top-left corner and `(1, 1)` its
/// bottom-right. Always opaque. [`Background::None`] has no backdrop and answers black.
pub fn color_at(background: Background, uv: Vec2) -> Color32 {
    let diagonal = (uv.x * 0.5 + uv.y * 0.5).clamp(0.0, 1.0);
    let tinted = |base: Rgb, glows: &[(Vec2, Vec2, Rgb, f32)]| {
        glows
            .iter()
            .fold(base, |under, &(centre, radius, over, strength)| {
                mix(under, over, strength * glow(uv, centre, radius))
            })
    };
    let rgb = match background {
        Background::None => [0.0; 3],
        Background::Dots => rgb(CHARCOAL),
        Background::Luminous => tinted(
            through(
                &[
                    (0.0, rgb(0x0b4249)),
                    (0.55, rgb(0x06292f)),
                    (1.0, rgb(0x031a1f)),
                ],
                uv.x * 0.3 + uv.y * 0.7,
            ),
            &[
                (vec2(0.1, 0.05), vec2(0.85, 0.9), rgb(0x2fd6c6), 0.8),
                (vec2(0.95, 1.0), vec2(0.6, 0.7), rgb(0x6beadc), 0.45),
                (vec2(0.06, 0.02), vec2(0.35, 0.4), rgb(0xe9fffb), 0.3),
            ],
        ),
        Background::Radiant => tinted(
            through(
                &[
                    (0.0, rgb(0xff3d86)),
                    (0.55, rgb(0xff6a5a)),
                    (1.0, rgb(0xffa43a)),
                ],
                diagonal,
            ),
            &[(vec2(0.9, 0.05), vec2(0.7, 0.7), rgb(0xffe1d2), 0.35)],
        ),
        Background::Spectrum => tinted(
            through(
                &[
                    (0.0, rgb(0x4a2bd8)),
                    (0.5, rgb(0xc42fb0)),
                    (1.0, rgb(0x12b8a4)),
                ],
                diagonal,
            ),
            &[(vec2(0.95, 0.0), vec2(0.6, 0.6), rgb(0x8f5cff), 0.3)],
        ),
        Background::Illuminated => tinted(
            through(
                &[(0.0, rgb(0x050f2c)), (1.0, rgb(0x0a2166))],
                uv.y * 0.8 + uv.x * 0.2,
            ),
            &[
                (vec2(0.5, 1.05), vec2(1.0, 0.95), rgb(0x2f7cf6), 0.9),
                (vec2(0.5, 1.02), vec2(0.45, 0.4), rgb(0xa9ccff), 0.55),
                (vec2(0.0, 0.0), vec2(0.6, 0.6), rgb(0x1d4fd0), 0.4),
            ],
        ),
    };
    color(rgb)
}

/// The position of `point` in `rect` as `(0..1, 0..1)`.
pub fn uv_in(rect: Rect, point: Pos2) -> Vec2 {
    vec2(
        (point.x - rect.min.x) / rect.width().max(f32::EPSILON),
        (point.y - rect.min.y) / rect.height().max(f32::EPSILON),
    )
}

/// What a swatch for a backdrop shows, drawn from the same definitions as the backdrop itself.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SwatchLook {
    /// No backdrop: a slashed circle.
    Slashed,
    /// A field with dots on it.
    Dots { field: Color32, dot: Color32 },
    /// The backdrop at the four corners of the swatch (top-left, top-right, bottom-left,
    /// bottom-right), blended across it.
    Gradient { corners: [Color32; 4] },
}

pub fn swatch(background: Background) -> SwatchLook {
    match background {
        Background::None => SwatchLook::Slashed,
        Background::Dots => SwatchLook::Dots {
            field: color(rgb(CHARCOAL)),
            dot: color(rgb(DOT)),
        },
        gradient => {
            // A little inside the corners: a swatch is small and the glows sit at the edges.
            let at = |x: f32, y: f32| color_at(gradient, vec2(x, y));
            SwatchLook::Gradient {
                corners: [at(0.1, 0.1), at(0.9, 0.1), at(0.1, 0.9), at(0.9, 0.9)],
            }
        }
    }
}

/// Paints the backdrop over `rect`, leaving out what `covered` (the chart, drawn next) hides whole:
/// cells of the grid that lie inside it, and the dots of [`Background::Dots`] that do.
pub fn paint(painter: &Painter, rect: Rect, background: Background, covered: Rect) {
    if !background.has_frame() || !rect.is_positive() {
        return;
    }
    let mut mesh = Mesh::default();
    mesh.vertices.reserve((COLUMNS + 1) * (ROWS + 1));
    for row in 0..=ROWS {
        for column in 0..=COLUMNS {
            let uv = vec2(column as f32 / COLUMNS as f32, row as f32 / ROWS as f32);
            mesh.colored_vertex(
                pos2(
                    rect.min.x + rect.width() * uv.x,
                    rect.min.y + rect.height() * uv.y,
                ),
                color_at(background, uv),
            );
        }
    }
    let stride = (COLUMNS + 1) as u32;
    for row in 0..ROWS as u32 {
        for column in 0..COLUMNS as u32 {
            let top_left = row * stride + column;
            let cell = Rect::from_min_max(
                mesh.vertices[top_left as usize].pos,
                mesh.vertices[(top_left + stride + 1) as usize].pos,
            );
            if covered.contains_rect(cell) {
                continue;
            }
            mesh.add_triangle(top_left, top_left + 1, top_left + stride);
            mesh.add_triangle(top_left + 1, top_left + stride + 1, top_left + stride);
        }
    }
    painter.add(Shape::mesh(mesh));
    if background == Background::Dots {
        paint_dots(painter, rect, covered);
    }
}

fn paint_dots(painter: &Painter, rect: Rect, covered: Rect) {
    let dot = color(rgb(DOT));
    let hidden = covered.shrink(DOT_RADIUS);
    let columns = (rect.width() / DOT_SPACING).floor() as usize;
    let rows = (rect.height() / DOT_SPACING).floor() as usize;
    // Centred on the image, so both margins read the same.
    let origin = pos2(
        rect.min.x + (rect.width() - (columns.saturating_sub(1)) as f32 * DOT_SPACING) / 2.0,
        rect.min.y + (rect.height() - (rows.saturating_sub(1)) as f32 * DOT_SPACING) / 2.0,
    );
    let dots = (0..rows)
        .flat_map(|row| (0..columns).map(move |column| (row, column)))
        .map(|(row, column)| origin + vec2(column as f32 * DOT_SPACING, row as f32 * DOT_SPACING))
        .filter(|center| !hidden.contains(*center))
        .map(|center| Shape::circle_filled(center, DOT_RADIUS, dot))
        .collect::<Vec<_>>();
    painter.extend(dots);
}
