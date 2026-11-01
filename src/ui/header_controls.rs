//! The controls in a window's header bar: menu buttons, icon buttons and the prominent action.
//! They share one height, one corner radius and one set of states, so a row of mixed controls
//! reads as one family. Empty pages reuse the prominent action and a menu button for their calls
//! to action, so a pair there matches the header.

use crate::ui::accessibility;
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, REGULAR, SEMIBOLD};
use crate::ui::theme;
use eframe::egui::{
    self, Color32, CursorIcon, Id, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};

/// Every header control is this tall.
pub const HEIGHT: f32 = 30.0;
const RADIUS: f32 = 8.0;
const GLYPH: f32 = 14.0;

/// The bordered, tinted surface of a quiet control.
fn paint_quiet(ui: &Ui, rect: Rect, response: &Response, lit: bool) {
    let fill = if lit || response.is_pointer_button_down_on() {
        theme::quaternary(1.0)
    } else if response.hovered() {
        theme::quaternary(0.75)
    } else {
        theme::quaternary(0.5)
    };
    ui.painter().rect_filled(rect, RADIUS, fill);
    ui.painter().rect_stroke(
        rect,
        RADIUS,
        Stroke::new(1.0, theme::separator().gamma_multiply(0.6)),
        StrokeKind::Inside,
    );
}

fn open_key(id: Id) -> Id {
    id.with("menu-open")
}

/// A menu button: a leading icon, a title and a chevron. `items` fills the `menu_width` menu it
/// opens (rows stretch to the menu's width, so it must be fixed). While `busy` the chevron is a
/// spinner and the menu cannot be opened. The
/// open flag lives in egui's temporary data so a menu left open never returns after a restart.
pub fn menu_button(
    ui: &mut Ui,
    id: Id,
    icon: Icon,
    title: &str,
    menu_width: f32,
    busy: bool,
    items: impl FnOnce(&mut Ui),
) -> Response {
    let label = kit::galley(
        ui,
        title,
        kit::size::BODY,
        REGULAR,
        theme::text_primary(),
        240.0,
    );
    let width = 11.0 + GLYPH + 7.0 + label.size().x + 8.0 + 9.0 + 11.0;
    let (rect, response) = ui.allocate_exact_size(
        vec2(width, HEIGHT),
        if busy { Sense::hover() } else { Sense::click() },
    );
    let response = if busy {
        response
    } else {
        response.on_hover_cursor(CursorIcon::PointingHand)
    };
    let response = accessibility::label(response, title);
    let mut open = !busy && ui.data(|data| data.get_temp::<bool>(open_key(id)).unwrap_or(false));
    if response.clicked() {
        open = !open;
    }
    paint_quiet(ui, rect, &response, open);
    icons::paint(
        ui.painter(),
        Rect::from_center_size(
            pos2(rect.left() + 11.0 + GLYPH / 2.0, rect.center().y),
            Vec2::splat(GLYPH),
        ),
        icon,
        theme::text_secondary(),
    );
    ui.painter().galley(
        pos2(
            rect.left() + 11.0 + GLYPH + 7.0,
            rect.center().y - label.size().y / 2.0,
        ),
        label,
        Color32::PLACEHOLDER,
    );
    let tail = pos2(rect.right() - 11.0 - 4.5, rect.center().y);
    if busy {
        ui.put(
            Rect::from_center_size(tail, Vec2::splat(10.0)),
            egui::Spinner::new().size(10.0),
        );
    } else {
        icons::paint(
            ui.painter(),
            Rect::from_center_size(tail, Vec2::splat(9.0)),
            Icon::ChevronDown,
            theme::text_secondary(),
        );
    }
    egui::Popup::menu(&response)
        .id(id.with("popup"))
        .open_bool(&mut open)
        .frame(kit::menu_frame())
        .gap(4.0)
        .show(|ui| {
            ui.set_width(menu_width);
            ui.spacing_mut().item_spacing.y = 0.0;
            items(ui);
        });
    ui.data_mut(|data| data.insert_temp(open_key(id), open));
    response
}

/// A square icon button. `on` tints it, for a toggle that is switched on.
pub fn icon_button(ui: &mut Ui, icon: Icon, label: &str, on: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(HEIGHT), Sense::click());
    let response = accessibility::label(response, label)
        .on_hover_text(label)
        .on_hover_cursor(CursorIcon::PointingHand);
    paint_quiet(ui, rect, &response, on);
    icons::paint(
        ui.painter(),
        Rect::from_center_size(rect.center(), Vec2::splat(16.0)),
        icon,
        if on {
            theme::text_primary()
        } else {
            theme::text_secondary()
        },
    );
    response
}

/// The window's main action: a bordered grey plate, an icon and a semibold title.
pub fn prominent_button(ui: &mut Ui, icon: Icon, title: &str) -> Response {
    let label = kit::galley(
        ui,
        title,
        kit::size::BODY,
        SEMIBOLD,
        theme::text_primary(),
        240.0,
    );
    let width = 13.0 + GLYPH + 7.0 + label.size().x + 15.0;
    let (rect, response) = ui.allocate_exact_size(vec2(width, HEIGHT), Sense::click());
    let response = accessibility::label(response, title).on_hover_cursor(CursorIcon::PointingHand);
    let base = theme::control_fill();
    let fill = if response.is_pointer_button_down_on() {
        base.gamma_multiply(0.8)
    } else if response.hovered() {
        base.gamma_multiply(1.12)
    } else {
        base
    };
    ui.painter().rect_filled(rect, RADIUS, fill);
    ui.painter().rect_stroke(
        rect,
        RADIUS,
        Stroke::new(1.0, theme::separator()),
        StrokeKind::Inside,
    );
    icons::paint(
        ui.painter(),
        Rect::from_center_size(
            pos2(rect.left() + 13.0 + GLYPH / 2.0, rect.center().y),
            Vec2::splat(GLYPH),
        ),
        icon,
        theme::text_primary(),
    );
    ui.painter().galley(
        pos2(
            rect.left() + 13.0 + GLYPH + 7.0,
            rect.center().y - label.size().y / 2.0,
        ),
        label,
        Color32::PLACEHOLDER,
    );
    response
}

/// A menu row for a header menu: a checkmark for the current choice, else the title alone.
pub fn choice(ui: &mut Ui, title: &str, current: bool) -> bool {
    let clicked = kit::menu_item_marked(ui, Icon::Check, title, current);
    if clicked {
        ui.close();
    }
    clicked
}

/// A header-menu action row with a leading icon.
pub fn action(ui: &mut Ui, icon: Icon, title: &str) -> bool {
    let clicked = kit::menu_item(ui, icon, title, true, false);
    if clicked {
        ui.close();
    }
    clicked
}

pub fn separator(ui: &mut Ui) {
    kit::menu_separator(ui);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prominent_and_menu_buttons_are_one_height() {
        let ctx = egui::Context::default();
        let mut heights = Vec::new();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.horizontal(|ui| {
                heights.push(
                    prominent_button(ui, Icon::Plus, "Add Transaction")
                        .rect
                        .height(),
                );
                heights.push(
                    menu_button(
                        ui,
                        Id::new("import"),
                        Icon::Download,
                        "Import Transactions",
                        250.0,
                        false,
                        |_| {},
                    )
                    .rect
                    .height(),
                );
            });
        });
        output.textures_delta.clear();
        assert_eq!(heights, [HEIGHT, HEIGHT]);
    }
}
