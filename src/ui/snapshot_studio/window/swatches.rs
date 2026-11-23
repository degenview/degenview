//! The Background swatches: six discs, each painted from the same colour definitions the
//! composer paints the backdrop with (`background::swatch`), so what is picked is what is made.

use crate::i18n::tr;
use crate::model::chart::snapshot::studio::Background;
use crate::ui::snapshot_studio::compose::background::{self, SwatchLook};
use crate::ui::theme;
use eframe::egui::{self, Color32, Id, Mesh, Painter, Pos2, Sense, Shape, Stroke, Ui, pos2, vec2};

const DIAMETER: f32 = 22.0;
const GAP: f32 = 8.0;
/// How far outside a disc its ring is drawn.
const RING: f32 = 3.5;
/// The corners of the fan a gradient disc is made of.
const SEGMENTS: usize = 40;

/// The room the row takes.
pub(super) const WIDTH: f32 = 6.0 * DIAMETER + 5.0 * GAP;

/// The row of swatches; returns the one clicked this frame.
pub(super) fn row(ui: &mut Ui, selected: Background) -> Option<Background> {
    let (rect, _) = ui.allocate_exact_size(vec2(WIDTH, DIAMETER), Sense::hover());
    let mut picked = None;
    for (index, background) in Background::ALL.into_iter().enumerate() {
        let cell = egui::Rect::from_min_size(
            pos2(rect.left() + index as f32 * (DIAMETER + GAP), rect.top()),
            vec2(DIAMETER, DIAMETER),
        );
        let response = ui
            .interact(cell, Id::new(("studio-background", index)), Sense::click())
            .on_hover_text(background.label())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let response = crate::ui::accessibility::label(
            response,
            tr!("{name} background", name = background.label()),
        );
        let painter = ui.painter();
        paint(
            painter,
            cell.center(),
            DIAMETER / 2.0,
            background::swatch(background),
        );
        let ring = |color: Color32, width: f32| {
            painter.circle_stroke(
                cell.center(),
                DIAMETER / 2.0 + RING,
                Stroke::new(width, color),
            );
        };
        if background == selected {
            ring(theme::text_primary(), 1.5);
        } else if response.has_focus() {
            ring(theme::text_secondary(), 1.5);
        } else if response.hovered() {
            ring(theme::ink(80), 1.5);
        }
        if response.clicked() {
            picked = Some(background);
        }
    }
    picked
}

/// One disc with its hairline, so a dark one keeps its edge on a dark window.
fn paint(painter: &Painter, centre: Pos2, radius: f32, look: SwatchLook) {
    match look {
        SwatchLook::Slashed => {
            painter.circle_stroke(centre, radius - 0.75, Stroke::new(1.5, theme::ink(90)));
            let reach = radius * std::f32::consts::FRAC_1_SQRT_2;
            painter.line_segment(
                [centre + vec2(-reach, reach), centre + vec2(reach, -reach)],
                Stroke::new(1.5, theme::ink(90)),
            );
            return;
        }
        SwatchLook::Dots { field, dot } => {
            painter.circle_filled(centre, radius, field);
            let step = radius / 2.6;
            for row in -2..=2 {
                for column in -2..=2 {
                    let at = centre + vec2(column as f32, row as f32) * step;
                    if (at - centre).length() <= radius - 2.2 {
                        painter.circle_filled(at, 0.8, dot);
                    }
                }
            }
        }
        SwatchLook::Gradient { corners } => {
            painter.add(Shape::mesh(gradient_disc(centre, radius, corners)));
        }
    }
    painter.circle_stroke(centre, radius, Stroke::new(1.0, theme::ink(60)));
}

/// A fan of triangles from the centre, each corner coloured by where it lies between the four
/// colours of the swatch's corners.
fn gradient_disc(centre: Pos2, radius: f32, corners: [Color32; 4]) -> Mesh {
    let at = |point: Pos2| {
        let u = ((point.x - centre.x) / (2.0 * radius) + 0.5).clamp(0.0, 1.0);
        let v = ((point.y - centre.y) / (2.0 * radius) + 0.5).clamp(0.0, 1.0);
        let top = corners[0].lerp_to_gamma(corners[1], u);
        let bottom = corners[2].lerp_to_gamma(corners[3], u);
        top.lerp_to_gamma(bottom, v)
    };
    let mut mesh = Mesh::default();
    mesh.colored_vertex(centre, at(centre));
    for step in 0..SEGMENTS {
        let angle = step as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
        let point = centre + vec2(angle.cos(), angle.sin()) * radius;
        mesh.colored_vertex(point, at(point));
    }
    for step in 0..SEGMENTS as u32 {
        mesh.add_triangle(0, 1 + step, 1 + (step + 1) % SEGMENTS as u32);
    }
    mesh
}
