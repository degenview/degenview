//! What surrounds a chart that sits on a backdrop: the backdrop itself, the soft shadow under the
//! chart and the rounding of the chart's corners.
//!
//! The chart is painted square and its corners are then masked: four pieces of the surface
//! (backdrop with the shadow over it) cover what lies outside the rounded corner. A piece is
//! coloured by [`Surround::surface_at`], the same function the backdrop and the shadow are
//! painted from, so it joins them without a seam, and its inner edge is feathered by one pixel so
//! the curve is smooth. The shadow is a mesh with its own falloff for the same reason: the
//! masks need to know how dark it is at any point, which a stock blurred rectangle cannot say.

use super::background;
use crate::model::chart::snapshot::composition::Composition;
use crate::model::chart::snapshot::studio::Background;
use eframe::egui::{Color32, Mesh, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, Vec2, vec2};

/// The radius of the chart's rounded corners, in points.
pub const CORNER_RADIUS: f32 = 10.0;
/// How dark the shadow is at its darkest.
const SHADOW_STRENGTH: f32 = 0.42;
/// The shadow's softness and how far it is moved down, as a share of the margin around the chart.
const SOFTNESS_SHARE: f32 = 0.5;
const OFFSET_SHARE: f32 = 0.22;
/// Segments to a corner of the shadow's outline and to the masked corner's curve.
const ARC_STEPS: usize = 8;
/// Rings the shadow's falloff is cut into.
const RINGS: usize = 10;
/// Steps across a corner mask, from the curve to the square's corner.
const MASK_STEPS: usize = 3;
/// The chart's hairline on a backdrop.
const HAIRLINE: u8 = 30;

/// The shadow under the chart: how dark it is at a point, from the distance to a rounded
/// rectangle (the chart, moved down).
#[derive(Clone, Copy, Debug)]
struct Shadow {
    rect: Rect,
    radius: f32,
    /// The distances from the outline over which it fades from full to nothing: it is `softness`
    /// strong inside the outline and gone `softness` outside it.
    softness: f32,
}

impl Shadow {
    /// How far `point` is outside the outline, negative inside it.
    fn distance(&self, point: Pos2) -> f32 {
        let half = self.rect.size() / 2.0;
        let from_centre = (point - self.rect.center()).abs();
        let inner = vec2(
            from_centre.x - (half.x - self.radius),
            from_centre.y - (half.y - self.radius),
        );
        inner.max(Vec2::ZERO).length() + inner.x.max(inner.y).min(0.0) - self.radius
    }

    fn alpha(&self, point: Pos2) -> f32 {
        let t = (self.distance(point) / self.softness * 0.5 + 0.5).clamp(0.0, 1.0);
        SHADOW_STRENGTH * (1.0 - t * t * (3.0 - 2.0 * t))
    }

    /// The outline `distance` away from the shadow's own, as a loop of points.
    fn outline(&self, distance: f32) -> Vec<Pos2> {
        let radius = self.radius + distance;
        let corners = [
            (
                self.rect.right_top(),
                vec2(-1.0, 1.0),
                -std::f32::consts::FRAC_PI_2,
            ),
            (self.rect.right_bottom(), vec2(-1.0, -1.0), 0.0),
            (
                self.rect.left_bottom(),
                vec2(1.0, -1.0),
                std::f32::consts::FRAC_PI_2,
            ),
            (self.rect.left_top(), vec2(1.0, 1.0), std::f32::consts::PI),
        ];
        corners
            .into_iter()
            .flat_map(|(corner, inward, start)| {
                let centre = corner + inward * self.radius;
                (0..=ARC_STEPS).map(move |step| {
                    let angle =
                        start + std::f32::consts::FRAC_PI_2 * step as f32 / ARC_STEPS as f32;
                    centre + vec2(angle.cos(), angle.sin()) * radius
                })
            })
            .collect()
    }

    fn paint(&self, painter: &Painter) {
        let rings: Vec<(Vec<Pos2>, u8)> = (0..=RINGS)
            .map(|ring| {
                let distance = -self.softness + 2.0 * self.softness * ring as f32 / RINGS as f32;
                let outline = self.outline(distance);
                let alpha = (self.alpha(outline[0]) * 255.0).round() as u8;
                (outline, alpha)
            })
            .collect();
        let mut mesh = Mesh::default();
        for (outline, alpha) in &rings {
            for point in outline {
                mesh.colored_vertex(*point, Color32::from_black_alpha(*alpha));
            }
        }
        let count = rings[0].0.len() as u32;
        for ring in 0..RINGS as u32 {
            for index in 0..count {
                let next = (index + 1) % count;
                let (a, b) = (ring * count + index, ring * count + next);
                let (c, d) = (a + count, b + count);
                mesh.add_triangle(a, b, c);
                mesh.add_triangle(b, d, c);
            }
        }
        // Inside the innermost ring the shadow is flat.
        let centre = mesh.vertices.len() as u32;
        mesh.colored_vertex(self.rect.center(), Color32::from_black_alpha(rings[0].1));
        for index in 0..count {
            mesh.add_triangle(centre, index, (index + 1) % count);
        }
        painter.add(Shape::mesh(mesh));
    }
}

/// A chart on a backdrop: where everything is and how to paint what surrounds the chart.
pub struct Surround {
    canvas: Rect,
    chart: Rect,
    background: Background,
    shadow: Shadow,
}

impl Surround {
    /// For `composition` laid out with its top-left corner at `canvas.min`.
    pub fn new(composition: &Composition, canvas: Rect, background: Background) -> Self {
        let chart = composition.chart.translate(canvas.min.to_vec2());
        let softness = (composition.padding * SOFTNESS_SHARE).max(1.0);
        let offset = composition.padding * OFFSET_SHARE;
        let limit = chart.width().min(chart.height()) / 2.0;
        Self {
            canvas,
            chart,
            background,
            shadow: Shadow {
                rect: chart.translate(vec2(0.0, offset)),
                radius: CORNER_RADIUS.max(softness + 1.0).min(limit),
                softness,
            },
        }
    }

    /// The backdrop and the shadow at `point`.
    pub fn surface_at(&self, point: Pos2) -> Color32 {
        let base = background::color_at(self.background, background::uv_in(self.canvas, point));
        let through = 1.0 - self.shadow.alpha(point);
        let dim = |channel: u8| (f32::from(channel) * through).round() as u8;
        Color32::from_rgb(dim(base.r()), dim(base.g()), dim(base.b()))
    }

    /// The backdrop, then the shadow, under the chart.
    pub fn paint_behind(&self, painter: &Painter) {
        // The chart covers all of itself but its rounded corners, which the masks redraw.
        let hidden = self.chart.shrink(CORNER_RADIUS);
        background::paint(painter, self.canvas, self.background, hidden);
        self.shadow.paint(painter);
    }

    /// Rounds the chart's corners, and draws its hairline. After the chart.
    pub fn paint_over(&self, painter: &Painter) {
        let feather = 1.0 / painter.ctx().pixels_per_point();
        for (outer, inward) in [
            (self.chart.left_top(), vec2(1.0, 1.0)),
            (self.chart.right_top(), vec2(-1.0, 1.0)),
            (self.chart.left_bottom(), vec2(1.0, -1.0)),
            (self.chart.right_bottom(), vec2(-1.0, -1.0)),
        ] {
            self.paint_corner(painter, outer, inward, feather);
        }
        painter.rect_stroke(
            self.chart,
            CORNER_RADIUS,
            Stroke::new(1.0, Color32::from_white_alpha(HAIRLINE)),
            StrokeKind::Inside,
        );
    }

    /// One corner of the chart at `outer`, `inward` pointing into the chart: what lies between
    /// the square's corner and the curve, from the curve outwards to the corner in steps, and a
    /// transparent row `feather` inside the curve for the soft edge.
    fn paint_corner(&self, painter: &Painter, outer: Pos2, inward: Vec2, feather: f32) {
        let radius = CORNER_RADIUS;
        let centre = outer + inward * radius;
        let mut mesh = Mesh::default();
        let rows = MASK_STEPS + 2;
        for step in 0..=ARC_STEPS {
            let angle = std::f32::consts::FRAC_PI_2 * step as f32 / ARC_STEPS as f32;
            let direction = vec2(-inward.x * angle.cos(), -inward.y * angle.sin());
            let curve = centre + direction * radius;
            // The row inside the curve, then the curve, then on to the corner.
            mesh.colored_vertex(
                centre + direction * (radius - feather),
                Color32::TRANSPARENT,
            );
            for part in 0..=MASK_STEPS {
                let point = curve + (outer - curve) * (part as f32 / MASK_STEPS as f32);
                mesh.colored_vertex(point, self.surface_at(point));
            }
        }
        for step in 0..ARC_STEPS as u32 {
            for row in 0..(rows - 1) as u32 {
                let a = step * rows as u32 + row;
                let b = a + rows as u32;
                mesh.add_triangle(a, a + 1, b);
                mesh.add_triangle(a + 1, b + 1, b);
            }
        }
        painter.add(Shape::mesh(mesh));
    }
}
