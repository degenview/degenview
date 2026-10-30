//! The small pieces the debug panel is made of: section headers, icon buttons, quiet text fields.

use eframe::egui::{
    self, Align, Color32, FontId, Id, Layout, Margin, Rect, RichText, Sense, Stroke, Ui, UiBuilder,
    pos2, vec2,
};

use crate::ui::icons::{self, Icon};
use crate::ui::kit;
use crate::ui::theme;

/// One row of a list.
pub const ROW: f32 = 22.0;
const HEADER: f32 = 26.0;

/// Monospaced text, for values and expressions.
pub fn mono(text: impl Into<String>, size: f32, color: Color32) -> RichText {
    RichText::new(text)
        .font(FontId::monospace(size))
        .color(color)
}

/// A section's header: a chevron that folds it, its name in capitals, a count, and whatever
/// `trailing` puts at the right. Returns whether it was clicked to fold or unfold.
pub fn section_header(
    ui: &mut Ui,
    title: &str,
    count: Option<usize>,
    open: bool,
    trailing: impl FnOnce(&mut Ui),
) -> bool {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), HEADER), Sense::click());
    ui.painter().rect_filled(rect, 0.0, theme::ink(6));
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0, kit::hairline()),
    );
    if response.hovered() {
        ui.painter().rect_filled(rect, 0.0, theme::ink(5));
    }
    icons::paint(
        ui.painter(),
        Rect::from_center_size(pos2(rect.left() + 14.0, rect.center().y), vec2(10.0, 10.0)),
        if open {
            Icon::ChevronDown
        } else {
            Icon::ChevronRight
        },
        theme::text_tertiary(),
    );
    let galley = kit::galley(
        ui,
        &title.to_uppercase(),
        kit::size::CAPTION + 0.5,
        kit::SEMIBOLD,
        theme::text_secondary(),
        rect.width() - 80.0,
    );
    let title_pos = pos2(rect.left() + 26.0, rect.center().y - galley.size().y / 2.0);
    let title_end = title_pos.x + galley.size().x;
    ui.painter()
        .galley(title_pos, galley, theme::text_secondary());
    if let Some(count) = count.filter(|count| *count > 0) {
        kit::paint_count(
            ui,
            title_end + 6.0,
            rect.center().y,
            count,
            theme::text_secondary(),
            theme::ink(14),
        );
    }
    let mut right = ui.new_child(
        UiBuilder::new()
            .max_rect(rect.shrink2(vec2(6.0, 0.0)))
            .layout(Layout::right_to_left(Align::Center)),
    );
    right.spacing_mut().item_spacing.x = 2.0;
    trailing(&mut right);
    response.clicked()
}

/// A square icon button that can be unavailable: drawn faint, and answering only with its tooltip.
pub fn icon_button(ui: &mut Ui, icon: Icon, enabled: bool, tooltip: &str) -> bool {
    if enabled {
        return icons::button(ui, icon, 22.0, theme::text_secondary(), tooltip).clicked();
    }
    let (rect, response) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
    response.on_hover_text(tooltip);
    icons::paint(
        ui.painter(),
        Rect::from_center_size(rect.center(), vec2(22.0 * 0.62, 22.0 * 0.62)),
        icon,
        theme::text_tertiary().gamma_multiply(0.5),
    );
    false
}

/// The filter of the watchlist, for a list in the panel: a magnifier, a bare field with hint text,
/// and a clear button once there is text. No well and no border.
pub fn flat_filter(ui: &mut Ui, id: Id, value: &mut String, hint: &str, width: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 24.0), Sense::hover());
    icons::paint(
        ui.painter(),
        Rect::from_center_size(pos2(rect.left() + 9.0, rect.center().y), vec2(13.0, 13.0)),
        Icon::Magnifier,
        theme::text_secondary(),
    );
    let clear_width = if value.is_empty() { 0.0 } else { 24.0 };
    let field = Rect::from_min_max(
        pos2(rect.left() + 22.0, rect.top()),
        pos2(rect.right() - clear_width, rect.bottom()),
    );
    let response = ui.put(
        field,
        egui::TextEdit::singleline(value)
            .id(id)
            .hint_text(hint)
            .frame(egui::Frame::NONE)
            .margin(Margin::ZERO)
            .vertical_align(Align::Center),
    );
    if response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Escape)) {
        value.clear();
    }
    if !value.is_empty() {
        let clear =
            Rect::from_center_size(pos2(rect.right() - 11.0, rect.center().y), vec2(20.0, 20.0));
        if kit::hover_button(
            ui,
            clear,
            id.with("clear"),
            Icon::XmarkCircleFill,
            "Clear filter",
            true,
            true,
        )
        .clicked()
        {
            value.clear();
            response.request_focus();
        }
    }
}

/// The height of a text field: that of the pickers and buttons it sits beside.
pub const FIELD_HEIGHT: f32 = 24.0;

/// A quiet single-line field on a faint well, ringed in the accent while it has focus.
pub fn text_field(
    ui: &mut Ui,
    id: Id,
    value: &mut String,
    hint: &str,
    width: f32,
    monospace: bool,
) -> egui::Response {
    let focused = ui.memory(|memory| memory.has_focus(id));
    let font = if monospace {
        FontId::monospace(11.5)
    } else {
        FontId::proportional(kit::size::BODY)
    };
    egui::Frame::new()
        .fill(theme::quaternary(0.45))
        .corner_radius(6.0)
        .stroke(Stroke::new(
            1.0,
            if focused {
                theme::with_alpha(theme::text_secondary(), 0.7)
            } else {
                kit::primary(0.12)
            },
        ))
        .inner_margin(Margin::symmetric(8, 3))
        .show(ui, |ui| {
            // The well's margins and ring take 8 points of the height.
            ui.spacing_mut().interact_size.y = FIELD_HEIGHT - 8.0;
            ui.add(
                egui::TextEdit::singleline(value)
                    .id(id)
                    .frame(egui::Frame::NONE)
                    .vertical_align(egui::Align::Center)
                    .font(font)
                    .text_color(theme::text_primary())
                    .hint_text(
                        RichText::new(hint)
                            .size(kit::size::SUBHEADLINE + 1.0)
                            .color(theme::text_tertiary()),
                    )
                    .desired_width((width - 18.0).max(40.0)),
            )
        })
        .inner
}

/// Text in the middle of an empty area.
pub fn placeholder(ui: &mut Ui, title: &str, detail: &str) {
    ui.add_space(14.0);
    ui.vertical_centered(|ui| {
        ui.label(kit::text(
            title,
            kit::size::BODY,
            kit::SEMIBOLD,
            theme::text_secondary(),
        ));
        if !detail.is_empty() {
            ui.add_space(2.0);
            ui.add(
                egui::Label::new(kit::text(
                    detail,
                    kit::size::SUBHEADLINE,
                    kit::REGULAR,
                    theme::text_tertiary(),
                ))
                .halign(Align::Center),
            );
        }
    });
}

/// A small coloured word on a tinted pill, painted at `left`; returns its right edge.
pub fn pill(ui: &Ui, left: f32, center_y: f32, text: &str, color: Color32) -> f32 {
    let galley = kit::galley(
        ui,
        text,
        kit::size::CAPTION + 0.5,
        kit::SEMIBOLD,
        color,
        200.0,
    );
    let size = vec2(galley.size().x + 10.0, galley.size().y + 2.0);
    let rect = Rect::from_min_size(pos2(left, center_y - size.y / 2.0), size);
    ui.painter()
        .rect_filled(rect, size.y / 2.0, theme::with_alpha(color, 0.16));
    ui.painter().galley(
        pos2(rect.left() + 5.0, center_y - galley.size().y / 2.0),
        galley,
        color,
    );
    rect.right()
}
