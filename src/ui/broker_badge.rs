//! The mark that says "this is real money". It appears wherever live trading does (the panel, the
//! ticket, the confirmation, the chart's order lines) and is never used for paper trading, so the
//! two cannot be mistaken for each other. Amber: not the buy and sell colours, and not blue.

use eframe::egui::{self, Color32, Sense, Ui, vec2};

use crate::ui::{kit, theme};

/// The live colour.
pub const LIVE: Color32 = kit::ORANGE;

/// `LIVE · COINBASE`-style pill, sized to its text.
pub fn badge(ui: &mut Ui, broker_name: &str) -> egui::Response {
    let text = format!("LIVE · {}", broker_name.to_uppercase());
    let galley = kit::galley(ui, &text, 10.5, kit::BOLD, LIVE, f32::INFINITY);
    let size = vec2(galley.size().x + 16.0, galley.size().y + 6.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter()
        .rect_filled(rect, size.y / 2.0, theme::with_alpha(LIVE, 0.16));
    ui.painter().rect_stroke(
        rect,
        size.y / 2.0,
        egui::Stroke::new(1.0, theme::with_alpha(LIVE, 0.5)),
        egui::StrokeKind::Inside,
    );
    ui.painter().galley(rect.min + vec2(8.0, 3.0), galley, LIVE);
    response.on_hover_text("Real money. Orders here are sent to the broker.")
}

/// A thin rule across the top of a live surface.
pub fn rule(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 2.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, 0.0, theme::with_alpha(LIVE, 0.7));
}
