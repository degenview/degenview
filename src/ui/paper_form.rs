//! The form controls the Paper Trading sheets share (the order ticket and the account sheets):
//! a caption label, a red error or quiet hint under a field, and the bordered text box with a
//! trailing unit and an optional accessory button.

use crate::ui::icons::{self, Icon};
use crate::ui::kit;
use crate::ui::theme;
use eframe::egui::{self, Id, Vec2};

pub(crate) const FIELD_HEIGHT: f32 = 34.0;

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
            theme::text_secondary(),
            f32::INFINITY,
        );
        let (button, response) = inner.allocate_exact_size(
            Vec2::new(galley.size().x + 12.0, 22.0),
            egui::Sense::click(),
        );
        let response = crate::ui::accessibility::label(response, title)
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if response.hovered() {
            inner.painter().rect_filled(button, 6.0, kit::primary(0.1));
        }
        inner.painter().galley(
            button.center() - galley.size() / 2.0,
            galley,
            if response.hovered() {
                theme::text_primary()
            } else {
                theme::text_secondary()
            },
        );
        clicked = response.clicked();
    }
    let room = inner.available_width().max(36.0);
    kit::plain_field(&mut inner, id, text, placeholder, 14.0, kit::REGULAR, room);
    clicked
}

/// A pull-down that looks like `text_field` (the same 34 pt box) and opens a menu exactly as wide
/// as its choices. `labels` are the choices and `selected` the current one; returns the index
/// picked this frame.
pub fn dropdown_field(
    ui: &mut egui::Ui,
    labels: &[String],
    selected: usize,
    enabled: bool,
) -> Option<usize> {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), FIELD_HEIGHT),
        if enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    let current = labels.get(selected).map_or("", String::as_str);
    let response = crate::ui::accessibility::label(response, current)
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response));
    let fill = theme::quaternary(if open || response.hovered() { 0.8 } else { 0.5 });
    ui.painter().rect_filled(rect, 8.0, fill);
    ui.painter().rect_stroke(
        rect,
        8.0,
        egui::Stroke::new(
            if open { 1.5 } else { 1.0 },
            if open {
                theme::with_alpha(theme::text_secondary(), 0.8)
            } else {
                theme::with_alpha(theme::separator(), 0.6)
            },
        ),
        egui::StrokeKind::Inside,
    );
    let color = if enabled {
        theme::text_primary()
    } else {
        theme::text_tertiary()
    };
    let text = kit::galley(ui, current, 14.0, kit::REGULAR, color, rect.width() - 40.0);
    ui.painter().galley(
        egui::pos2(rect.left() + 12.0, rect.center().y - text.size().y / 2.0),
        text,
        color,
    );
    icons::paint(
        ui.painter(),
        egui::Rect::from_center_size(
            egui::pos2(rect.right() - 14.0, rect.center().y),
            Vec2::splat(11.0),
        ),
        Icon::ChevronUpDown,
        theme::text_secondary(),
    );
    let mut chosen = None;
    if enabled {
        kit::fitted_menu(ui, &response, egui::RectAlign::BOTTOM_START, labels, |ui| {
            for (index, label) in labels.iter().enumerate() {
                if kit::menu_item_marked(ui, Icon::Check, label, index == selected) {
                    chosen = Some(index);
                    ui.close();
                }
            }
        });
    }
    chosen
}
