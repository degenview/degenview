//! The quiet bands the orders panel shows between its header and its tables: a connection that is
//! down, a key that cannot trade, the venue's last error. One look for every venue.

use eframe::egui;

use crate::ui::{kit, theme};

/// A hairline across the panel.
pub fn divider(ui: &mut egui::Ui) {
    let (line, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().rect_filled(line, 0.0, theme::separator());
}

/// A band of wrapped text: red for what went wrong, quiet for what is only worth knowing.
pub fn note(ui: &mut egui::Ui, text: &str, error: bool) {
    let tint = if error {
        kit::RED
    } else {
        theme::text_secondary()
    };
    egui::Frame::new()
        .fill(theme::with_alpha(tint, 0.1))
        .inner_margin(egui::Margin::symmetric(12, 7))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(
                egui::Label::new(kit::text(
                    text,
                    kit::size::CAPTION + 0.5,
                    kit::REGULAR,
                    tint,
                ))
                .wrap(),
            );
        });
}
