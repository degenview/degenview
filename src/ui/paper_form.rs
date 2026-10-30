//! The form controls the Paper Trading sheets share (the order ticket and the account sheets):
//! a caption label, a red error or quiet hint under a field, and the bordered text box with a
//! trailing unit and an optional accessory button.

use crate::ui::kit;
use crate::ui::theme;
use eframe::egui::{self, Id, Vec2};

const FIELD_HEIGHT: f32 = 34.0;

pub fn field_label(ui: &mut egui::Ui, text: &str) {
    ui.label(kit::text(
        text,
        kit::size::CAPTION + 1.0,
        kit::MEDIUM,
        theme::text_secondary(),
    ));
    ui.add_space(5.0);
}

/// The red error, or the quiet hint, under a field.
pub fn field_footer(ui: &mut egui::Ui, error: Option<&String>, hint: Option<&str>) {
    let (text, color) = match (error, hint) {
        (Some(error), _) => (error.as_str(), theme::DOWN),
        (None, Some(hint)) => (hint, theme::text_tertiary()),
        _ => return,
    };
    ui.add_space(5.0);
    ui.add(
        egui::Label::new(kit::text(
            text,
            kit::size::CAPTION + 1.0,
            kit::REGULAR,
            color,
        ))
        .wrap(),
    );
}

/// A text box with a trailing unit and an optional accessory ("Market"); returns the accessory's
/// click. A red outline marks a value the form will not take.
pub fn text_field(
    ui: &mut egui::Ui,
    salt: &str,
    text: &mut String,
    placeholder: &str,
    suffix: &str,
    is_error: bool,
    accessory: Option<&str>,
) -> bool {
    let id = Id::new(("paper-text-field", salt));
    let focused = ui.memory(|memory| memory.has_focus(id));
    let (rect, background) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), FIELD_HEIGHT),
        egui::Sense::click(),
    );
    let (border, width) = if is_error {
        (theme::with_alpha(theme::DOWN, 0.8), 1.5)
    } else if focused {
        (theme::with_alpha(theme::text_secondary(), 0.8), 1.5)
    } else {
        (theme::with_alpha(theme::separator(), 0.6), 1.0)
    };
    ui.painter().rect_filled(rect, 8.0, theme::quaternary(0.5));
    ui.painter().rect_stroke(
        rect,
        8.0,
        egui::Stroke::new(width, border),
        egui::StrokeKind::Inside,
    );
    if background.clicked() {
        ui.memory_mut(|memory| memory.request_focus(id));
    }

    let mut clicked = false;
    let mut inner = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(Vec2::new(10.0, 0.0)))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    inner.spacing_mut().item_spacing.x = 8.0;
    // Right to left: the unit, the accessory, then the text takes what is left.
    inner.label(kit::text(
        suffix,
        kit::size::BODY,
        kit::REGULAR,
        theme::text_secondary(),
    ));
    if let Some(title) = accessory {
        let galley = kit::galley(
            &inner,
            title,
            kit::size::CAPTION + 1.0,
            kit::SEMIBOLD,
            kit::ACCENT,
            f32::INFINITY,
        );
        let (button, response) = inner.allocate_exact_size(
            Vec2::new(galley.size().x + 12.0, 22.0),
            egui::Sense::click(),
        );
        let response = crate::ui::accessibility::label(response, title)
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if response.hovered() {
            inner
                .painter()
                .rect_filled(button, 6.0, theme::with_alpha(kit::ACCENT, 0.14));
        }
        inner
            .painter()
            .galley(button.center() - galley.size() / 2.0, galley, kit::ACCENT);
        clicked = response.clicked();
    }
    let room = inner.available_width().max(36.0);
    kit::plain_field(&mut inner, id, text, placeholder, 14.0, kit::REGULAR, room);
    clicked
}
