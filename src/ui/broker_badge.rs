//! The mark that says "this is real money". It appears wherever live trading does (the panel, the
//! ticket, the confirmation, the chart's order lines) and is never used for paper trading, so the
//! two cannot be mistaken for each other. Amber: not the buy and sell colours, and not blue.

use eframe::egui::{Color32, Sense, Ui, vec2};

use crate::ui::{kit, theme};

/// The live colour.
pub const LIVE: Color32 = kit::ORANGE;

/// A thin rule across the top of a live surface.
pub fn rule(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 2.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, 0.0, theme::with_alpha(LIVE, 0.7));
}
