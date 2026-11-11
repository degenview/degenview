//! The small key labels of a shortcut hint ("Esc", "↑", "↵"): a rounded outline around the key's name,
//! with the arrow keys drawn as shapes because their glyphs are not in every font.

use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, MEDIUM};
use crate::ui::theme;
use eframe::egui::{self, Rect, Sense, Stroke, Vec2};

/// The arrows a keycap draws as shapes. Their font glyphs (↑ ↓ ↵) are in no font the app is
/// sure to have: macOS has them in its system fonts, a Linux machine shows tofu squares.
fn keycap_icon(key: &str) -> Option<Icon> {
    match key {
        "\u{2191}" => Some(Icon::ArrowUp),
        "\u{2193}" => Some(Icon::ArrowDown),
        "\u{21b5}" => Some(Icon::Return),
        _ => None,
    }
}

/// How wide [`keycap`] draws `key`.
pub(crate) fn keycap_width(ui: &egui::Ui, key: &str) -> f32 {
    if keycap_icon(key).is_some() {
        return 24.0;
    }
    kit::galley(
        ui,
        key,
        kit::size::CAPTION + 0.5,
        MEDIUM,
        theme::text_secondary(),
        f32::INFINITY,
    )
    .size()
    .x + 12.0
}

/// One key of a hint, drawn as a keycap.
pub(crate) fn keycap(ui: &mut egui::Ui, key: &str) {
    let width = keycap_width(ui, key);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 18.0), Sense::hover());
    ui.painter().rect_filled(rect, 5.0, kit::primary(0.07));
    ui.painter().rect_stroke(
        rect,
        5.0,
        Stroke::new(1.0, kit::primary(0.12)),
        egui::StrokeKind::Inside,
    );
    if let Some(icon) = keycap_icon(key) {
        icons::paint(
            ui.painter(),
            Rect::from_center_size(rect.center(), Vec2::splat(11.0)),
            icon,
            theme::text_secondary(),
        );
        return;
    }
    let text = kit::galley(
        ui,
        key,
        kit::size::CAPTION + 0.5,
        MEDIUM,
        theme::text_secondary(),
        f32::INFINITY,
    );
    ui.painter().galley(
        rect.center() - text.size() / 2.0,
        text,
        theme::text_secondary(),
    );
}
