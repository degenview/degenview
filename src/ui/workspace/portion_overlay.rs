//! What Select portion puts over the grid: a layer that takes the pointer from the charts under
//! it, the dimming around the rectangle being dragged with its border, and before the drag a hint.
//!
//! The layer sits above the charts and below every menu and sheet. It is flat: no animation, no
//! shadow, and nothing blue; the border is the label colour so it reads on a dark and a light
//! chart alike.

use crate::ui::{kit, theme};
use eframe::egui::{
    self, Align2, Color32, Context, CornerRadius, Id, Order, Painter, Rect, Response, Sense,
    Stroke, StrokeKind, vec2,
};
use eframe::emath::GuiRounding;

/// The dimming outside the rectangle being dragged. A dark chart is near black already, so it
/// takes more black (about 55%) than a light one (about 20%) before the candles outside visibly
/// recede.
fn outside_dim() -> Color32 {
    Color32::from_black_alpha(if theme::is_light() { 52 } else { 140 })
}

/// The light dimming of the whole grid before the drag begins.
fn idle_dim() -> Color32 {
    Color32::from_black_alpha(if theme::is_light() { 28 } else { 46 })
}
/// What the hint says.
pub const HINT: &str = "Drag to select an area · Esc to cancel";
/// How far under the top of the charts' plots the hint sits (below their headers).
const HINT_TOP: f32 = 14.0;

/// The four strips of `grid` that lie outside `selection`, which is inside it: the part that is
/// dimmed. A strip with no area is left out.
pub fn outside(grid: Rect, selection: Rect) -> Vec<Rect> {
    [
        Rect::from_min_max(grid.min, egui::pos2(grid.right(), selection.top())),
        Rect::from_min_max(egui::pos2(grid.left(), selection.bottom()), grid.max),
        Rect::from_min_max(
            egui::pos2(grid.left(), selection.top()),
            egui::pos2(selection.left(), selection.bottom()),
        ),
        Rect::from_min_max(
            egui::pos2(selection.right(), selection.top()),
            egui::pos2(grid.right(), selection.bottom()),
        ),
    ]
    .into_iter()
    .filter(|strip| strip.width() > 0.0 && strip.height() > 0.0)
    .collect()
}

/// Puts the layer over `grid` and returns its response: hovered while the pointer is over the
/// grid and nothing is above it, so the caller knows a press is the selection's. Everything
/// under the layer stops answering the pointer (from the next frame on, as the widgets of a
/// frame are what the pointer is tested against in the one after).
pub fn shield(ctx: &Context, id: Id, grid: Rect) -> Response {
    egui::Area::new(id)
        .order(Order::Middle)
        .fixed_pos(grid.min)
        .interactable(true)
        .show(ctx, |ui| {
            ui.allocate_exact_size(grid.size(), Sense::click_and_drag())
                .1
        })
        .inner
}

/// Paints the layer `shield` made: the dimming and the border around `selection`, the rectangle
/// being dragged in window points, or before the drag the whole grid lightly dimmed with the
/// hint, centred at the top of `plots`, the part of the grid a selection can cover.
pub fn paint(painter: &Painter, grid: Rect, plots: Rect, selection: Option<Rect>) {
    match selection {
        Some(selection) => paint_selection(painter, grid, selection),
        None => paint_idle(painter, grid, plots),
    }
}

fn paint_selection(painter: &Painter, grid: Rect, selection: Rect) {
    let selection = selection.round_to_pixels(painter.ctx().pixels_per_point());
    for strip in outside(grid, selection) {
        painter.rect_filled(strip, 0.0, outside_dim());
    }
    painter.rect_stroke(
        selection,
        CornerRadius::ZERO,
        Stroke::new(1.0, theme::text_primary()),
        StrokeKind::Inside,
    );
}

fn paint_idle(painter: &Painter, grid: Rect, plots: Rect) {
    painter.rect_filled(grid, 0.0, idle_dim());
    let galley = kit::galley_on(
        painter,
        HINT,
        kit::size::CAPTION,
        kit::REGULAR,
        theme::text_primary(),
        grid.width(),
    );
    let padding = vec2(10.0, 5.0);
    let pill = Align2::CENTER_TOP.align_size_within_rect(
        galley.size() + padding * 2.0,
        Rect::from_min_max(
            egui::pos2(plots.left(), plots.top() + HINT_TOP),
            plots.right_bottom(),
        ),
    );
    painter.rect_filled(pill, 6.0, theme::chrome());
    painter.rect_stroke(
        pill,
        6.0,
        Stroke::new(1.0, theme::border()),
        StrokeKind::Inside,
    );
    painter.galley(pill.min + padding, galley, theme::text_primary());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_outside_is_the_grid_less_the_selection_with_nothing_twice() {
        let grid = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(400.0, 300.0));
        let selection = Rect::from_min_max(egui::pos2(100.0, 50.0), egui::pos2(300.0, 200.0));
        let strips = outside(grid, selection);
        assert_eq!(strips.len(), 4);
        let area: f32 = strips.iter().map(|strip| strip.area()).sum();
        assert_eq!(area, grid.area() - selection.area());
        for (index, strip) in strips.iter().enumerate() {
            assert!(!strip.intersects(selection.shrink(0.01)));
            for other in &strips[index + 1..] {
                assert!(
                    !strip.shrink(0.01).intersects(*other),
                    "{strip:?} {other:?}"
                );
            }
        }
    }

    #[test]
    fn a_selection_on_the_grid_edge_leaves_no_strip_there() {
        let grid = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(400.0, 300.0));
        assert_eq!(outside(grid, grid), Vec::<Rect>::new());
        let corner = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(100.0, 100.0));
        assert_eq!(outside(grid, corner).len(), 2);
    }
}
