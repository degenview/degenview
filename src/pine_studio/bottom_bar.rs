//! The bar pinned to the bottom of each Pine Studio pane (the sidebar's community link, the
//! preview's drawer tabs, the editor's status and Save): one height, one fill, one hairline, so
//! the three read as a single strip across the window.

use crate::ui::theme;
use eframe::egui::{Rect, Stroke, Ui, pos2};

pub const HEIGHT: f32 = 32.0;

/// The bar's rectangle at the bottom of `pane`.
pub fn rect(pane: Rect) -> Rect {
    Rect::from_min_max(pos2(pane.min.x, pane.max.y - HEIGHT), pane.max)
}

/// The bar's fill and the hairline on its top edge.
pub fn paint(ui: &Ui, bar: Rect) {
    ui.painter().rect_filled(bar, 0.0, theme::chrome());
    ui.painter().line_segment(
        [bar.left_top(), bar.right_top()],
        Stroke::new(1.0, theme::separator()),
    );
}
