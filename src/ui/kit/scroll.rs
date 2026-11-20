//! Cues that a scrolling list has more to show: a scroll bar that stays in view, and a fade over
//! the edge the list runs on past. A list whose last visible line looks like its end is the one
//! that hides content.

use crate::ui::theme;
use eframe::egui::{
    Color32, CornerRadius, Mesh, Painter, Rect, Shape, Ui, epaint::Vertex, pos2, style,
};

/// How far a fade reaches into the list.
const FADE_HEIGHT: f32 = 28.0;
/// Rows the fade's curve is cut into; more than two so it eases in instead of showing a band.
const FADE_STEPS: usize = 6;
/// Vertices of one fade's mesh, for tests that look for it among the shapes a frame painted.
#[cfg(test)]
pub const FADE_VERTICES: usize = (FADE_STEPS + 1) * 2;
/// Offsets within this many points of an end count as being at it.
const END_TOLERANCE: f32 = 1.0;

/// Which edges of a vertical list have content past them.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Beyond {
    pub above: bool,
    pub below: bool,
}

impl Beyond {
    /// For a list scrolled by `offset` whose `content` is `viewport` tall on screen: nothing when
    /// the content fits, and the end the offset has not reached otherwise.
    pub fn of(offset: f32, content: f32, viewport: f32) -> Self {
        if content <= viewport + END_TOLERANCE {
            return Self::default();
        }
        Self {
            above: offset > END_TOLERANCE,
            below: offset < content - viewport - END_TOLERANCE,
        }
    }
}

/// Makes the scroll areas inside `ui` show a bar whenever their content overflows, in the room the
/// bar reserves, not one that floats over the text and appears on hover. A solid bar takes its
/// colours from the widget visuals, so the handle is set here to a gray that shows on the sheet in
/// both themes, and the track is left clear.
pub fn keep_bars_visible(ui: &mut Ui) {
    let bars = &mut ui.spacing_mut().scroll;
    *bars = style::ScrollStyle::solid();
    bars.bar_outer_margin = 6.0;
    let visuals = ui.visuals_mut();
    visuals.extreme_bg_color = Color32::TRANSPARENT;
    let handle = theme::text_secondary();
    for (state, strength) in [
        (&mut visuals.widgets.inactive, 0.5),
        (&mut visuals.widgets.hovered, 0.75),
        (&mut visuals.widgets.active, 1.0),
    ] {
        state.bg_fill = handle.gamma_multiply(strength);
        state.corner_radius = CornerRadius::same(3);
    }
}

/// Paints `color` over the edges of `viewport` named by `beyond`, solid at the edge and gone
/// `FADE_HEIGHT` inside it. Call it after the list, so it lies on top. `color` is what the list
/// sits on, so the text seems to dissolve into the background.
pub fn fade_edges(painter: &Painter, viewport: Rect, color: Color32, beyond: Beyond) {
    let height = FADE_HEIGHT.min(viewport.height() / 2.0);
    if beyond.above {
        painter.add(Shape::mesh(fade(
            viewport.left(),
            viewport.right(),
            viewport.top(),
            height,
            color,
        )));
    }
    if beyond.below {
        painter.add(Shape::mesh(fade(
            viewport.left(),
            viewport.right(),
            viewport.bottom(),
            -height,
            color,
        )));
    }
}

/// A band from `edge` to `edge + reach` (`reach` is negative to run upwards), solid at the edge.
fn fade(left: f32, right: f32, edge: f32, reach: f32, color: Color32) -> Mesh {
    let mut mesh = Mesh::default();
    for step in 0..=FADE_STEPS {
        let along = step as f32 / FADE_STEPS as f32;
        // Smoothstep from solid to clear: the fade eases out at both ends.
        let alpha = 1.0 - along * along * (3.0 - 2.0 * along);
        let tint = color.gamma_multiply(alpha);
        let y = edge + reach * along;
        for x in [left, right] {
            mesh.vertices.push(Vertex {
                pos: pos2(x, y),
                uv: eframe::egui::epaint::WHITE_UV,
                color: tint,
            });
        }
    }
    for row in 0..FADE_STEPS as u32 {
        let at = row * 2;
        mesh.add_triangle(at, at + 1, at + 2);
        mesh.add_triangle(at + 1, at + 3, at + 2);
    }
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_list_that_fits_has_nothing_beyond_it() {
        assert_eq!(Beyond::of(0.0, 200.0, 300.0), Beyond::default());
        assert_eq!(
            Beyond::of(0.0, 300.5, 300.0),
            Beyond::default(),
            "within a point"
        );
    }

    #[test]
    fn the_fade_shows_only_at_an_end_with_content_past_it() {
        let at = |offset| Beyond::of(offset, 500.0, 300.0);
        assert_eq!(
            at(0.0),
            Beyond {
                above: false,
                below: true
            }
        );
        assert_eq!(
            at(0.5),
            Beyond {
                above: false,
                below: true
            }
        );
        assert_eq!(
            at(100.0),
            Beyond {
                above: true,
                below: true
            }
        );
        assert_eq!(
            at(199.5),
            Beyond {
                above: true,
                below: false
            },
            "a point from the end"
        );
        assert_eq!(
            at(200.0),
            Beyond {
                above: true,
                below: false
            }
        );
        assert_eq!(
            at(240.0),
            Beyond {
                above: true,
                below: false
            },
            "overscroll"
        );
    }

    #[test]
    fn a_fade_is_solid_at_its_edge_and_clear_at_the_far_side() {
        let color = Color32::from_rgb(40, 40, 40);
        let down = fade(0.0, 100.0, 200.0, -28.0, color);
        assert_eq!(down.vertices.first().map(|v| v.color), Some(color));
        assert_eq!(
            down.vertices.last().map(|v| v.color),
            Some(Color32::TRANSPARENT)
        );
        assert_eq!(down.vertices.last().map(|v| v.pos.y), Some(172.0));
        let alphas: Vec<u8> = down
            .vertices
            .iter()
            .step_by(2)
            .map(|v| v.color.a())
            .collect();
        assert!(
            alphas.windows(2).all(|pair| pair[0] >= pair[1]),
            "{alphas:?}"
        );
    }
}
