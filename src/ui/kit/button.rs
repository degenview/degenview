//! Buttons: push, footer, sheet, large, tinted and destructive buttons, and hover-only icon
//! buttons.

use super::loading::LoadingIndicator;
use super::{MEDIUM, RED, REGULAR, SEMIBOLD, galley, primary, size};
use crate::ui::icons::{self, Icon};
use crate::ui::theme;
use eframe::egui::{self, Color32, Id, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};

/// An icon button that is drawn only while `shown` (the row is hovered), like the hover actions
/// on a card; returns whether it was clicked.
pub fn hover_button(
    ui: &mut Ui,
    rect: Rect,
    id: Id,
    icon: Icon,
    tooltip: &str,
    shown: bool,
    enabled: bool,
) -> egui::Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = ui.interact(rect, id, sense);
    let response = crate::ui::accessibility::label(response, tooltip);
    if shown || response.hovered() {
        let painter = ui.painter();
        if response.hovered() && enabled {
            painter.rect_filled(
                rect,
                6.0,
                primary(if response.is_pointer_button_down_on() {
                    0.12
                } else {
                    0.07
                }),
            );
        }
        icons::paint(
            painter,
            Rect::from_center_size(rect.center(), Vec2::splat(12.0)),
            icon,
            if enabled {
                theme::text_secondary()
            } else {
                theme::text_tertiary()
            },
        );
    }
    if shown {
        response.on_hover_text(tooltip)
    } else {
        response
    }
}

/// A bordered push button with an optional leading icon, at the regular control size.
pub fn push_button(ui: &mut Ui, icon: Option<Icon>, label: &str, enabled: bool) -> egui::Response {
    let color = if enabled {
        theme::text_primary()
    } else {
        theme::text_tertiary()
    };
    let label_galley = galley(ui, label, size::BODY, REGULAR, color, f32::INFINITY);
    let icon_width = if icon.is_some() { 17.0 } else { 0.0 };
    let size = vec2(label_galley.size().x + icon_width + 22.0, 24.0);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    let response = crate::ui::accessibility::label(response, label);
    let fill = primary(
        match (
            enabled,
            response.is_pointer_button_down_on(),
            response.hovered(),
        ) {
            (false, _, _) => 0.05,
            (true, true, _) => 0.2,
            (true, false, true) => 0.14,
            _ => 0.1,
        },
    );
    let painter = ui.painter();
    painter.rect_filled(rect, 6.0, fill);
    painter.rect_stroke(
        rect,
        6.0,
        Stroke::new(1.0, primary(0.06)),
        StrokeKind::Inside,
    );
    let mut x = rect.left() + 11.0;
    if let Some(icon) = icon {
        icons::paint(
            painter,
            Rect::from_min_size(pos2(x, rect.center().y - 6.0), Vec2::splat(12.0)),
            icon,
            color,
        );
        x += icon_width;
    }
    painter.galley(
        pos2(x, rect.center().y - label_galley.size().y / 2.0),
        label_galley,
        color,
    );
    response
}

/// How a sheet-footer button looks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FooterButton {
    /// Cancel and the like: a quiet bordered button.
    Secondary,
    /// The sheet's main action in the neutral grey: a heavier plate and a semibold label.
    Primary,
}

/// A sheet-footer button at the large control size (28 pt). `min_width` keeps a primary action
/// from shrinking to its label.
pub fn footer_button(
    ui: &mut Ui,
    label: &str,
    kind: FooterButton,
    enabled: bool,
    min_width: f32,
) -> egui::Response {
    let text_color = match (kind, enabled) {
        (FooterButton::Primary | FooterButton::Secondary, true) => theme::text_primary(),
        (FooterButton::Primary | FooterButton::Secondary, false) => theme::text_tertiary(),
    };
    let weight = match kind {
        FooterButton::Primary => SEMIBOLD,
        FooterButton::Secondary => REGULAR,
    };
    let text = galley(ui, label, size::BODY, weight, text_color, 400.0);
    let width = (text.size().x + 28.0).max(min_width);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(width, 28.0), sense);
    let response = crate::ui::accessibility::label(response, label);
    let pressed = enabled && response.is_pointer_button_down_on();
    let hovered = enabled && response.hovered();
    let painter = ui.painter();
    match kind {
        FooterButton::Primary | FooterButton::Secondary => {
            let lift = if kind == FooterButton::Primary {
                0.06
            } else {
                0.0
            };
            let fill = primary(
                lift + match (enabled, pressed, hovered) {
                    (false, _, _) => 0.05,
                    (true, true, _) => 0.2,
                    (true, false, true) => 0.14,
                    _ => 0.1,
                },
            );
            painter.rect_filled(rect, 7.0, fill);
            painter.rect_stroke(
                rect,
                7.0,
                Stroke::new(1.0, primary(0.1)),
                StrokeKind::Inside,
            );
        }
    }
    painter.galley(
        pos2(
            rect.center().x - text.size().x / 2.0,
            rect.center().y - text.size().y / 2.0,
        ),
        text,
        Color32::PLACEHOLDER,
    );
    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

/// A large sheet button: the prominent one is accent-filled, the other a quiet fill.
pub fn sheet_button(ui: &mut Ui, label: &str, prominent: bool, enabled: bool) -> egui::Response {
    let text = galley(
        ui,
        label,
        size::BODY,
        if prominent { MEDIUM } else { REGULAR },
        Color32::PLACEHOLDER,
        f32::INFINITY,
    );
    let width = (text.size().x + 32.0).max(76.0);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(width, 28.0), sense);
    let response = crate::ui::accessibility::label(response, label);
    let pressed = response.is_pointer_button_down_on();
    let hovered = response.hovered() && enabled;
    let (fill, color) = match (prominent, enabled) {
        (true, true) => (
            primary(if pressed {
                0.26
            } else if hovered {
                0.2
            } else {
                0.16
            }),
            theme::text_primary(),
        ),
        (true, false) => (primary(0.06), primary(0.45)),
        (false, _) => (
            primary(if pressed {
                0.18
            } else if hovered {
                0.14
            } else {
                0.1
            }),
            theme::text_primary(),
        ),
    };
    ui.painter().rect_filled(rect, 7.0, fill);
    ui.painter()
        .galley(rect.center() - text.size() / 2.0, text, color);
    response
}

/// The neutral partner of `tinted_button`: a quiet 32 pt button, for a sheet's Cancel.
pub fn large_button(ui: &mut Ui, label: &str) -> egui::Response {
    let text = galley(
        ui,
        label,
        size::BODY + 1.0,
        REGULAR,
        theme::text_primary(),
        f32::INFINITY,
    );
    let (rect, response) =
        ui.allocate_exact_size(vec2((text.size().x + 36.0).max(80.0), 32.0), Sense::click());
    let response = crate::ui::accessibility::label(response, label);
    let fill = primary(if response.is_pointer_button_down_on() {
        0.18
    } else if response.hovered() {
        0.14
    } else {
        0.1
    });
    ui.painter().rect_filled(rect, 8.0, fill);
    ui.painter().galley(
        pos2(
            rect.center().x - text.size().x / 2.0,
            rect.center().y - text.size().y / 2.0,
        ),
        text,
        theme::text_primary(),
    );
    response
}

/// `large_button`'s emphasised twin, for a sheet's primary action: the same 32 pt size on a
/// stronger neutral fill, so it reads as the default without any colour. Greyed when disabled.
pub fn large_prominent_button(
    ui: &mut Ui,
    label: &str,
    enabled: bool,
    min_width: f32,
) -> egui::Response {
    let color = if enabled {
        theme::text_primary()
    } else {
        theme::text_tertiary()
    };
    let text = galley(ui, label, size::BODY + 1.0, MEDIUM, color, f32::INFINITY);
    let (rect, response) = ui.allocate_exact_size(
        vec2((text.size().x + 36.0).max(min_width), 32.0),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let response = crate::ui::accessibility::label(response, label);
    let hovered = response.hovered() && enabled;
    let fill = primary(
        match (enabled, response.is_pointer_button_down_on(), hovered) {
            (false, _, _) => 0.05,
            (true, true, _) => 0.3,
            (true, false, true) => 0.24,
            _ => 0.2,
        },
    );
    ui.painter().rect_filled(rect, 8.0, fill);
    if enabled {
        ui.painter().rect_stroke(
            rect,
            8.0,
            Stroke::new(1.0, primary(0.12)),
            StrokeKind::Inside,
        );
    }
    ui.painter()
        .galley(rect.center() - text.size() / 2.0, text, color);
    response
}

/// A large (32 pt) button filled with `tint`: the order ticket's Buy / Sell. While `busy` a
/// spinner replaces the label's left margin and the button ignores clicks.
pub fn tinted_button(
    ui: &mut Ui,
    label: &str,
    tint: Color32,
    enabled: bool,
    busy: bool,
    min_width: f32,
) -> egui::Response {
    button_in_tint(
        ui,
        label,
        ButtonLook {
            tint,
            label: Color32::WHITE,
            height: 32.0,
            size: size::BODY + 1.0,
            radius: 8.0,
        },
        enabled,
        busy,
        min_width,
    )
}

/// The call to action of a sheet: [`tinted_button`], taller and larger, with the label colour
/// the caller chooses for the fill (dark on a light green).
pub fn cta_button(
    ui: &mut Ui,
    label: &str,
    tint: Color32,
    label_color: Color32,
    enabled: bool,
    busy: bool,
    min_width: f32,
) -> egui::Response {
    button_in_tint(
        ui,
        label,
        ButtonLook {
            tint,
            label: label_color,
            height: 40.0,
            size: size::BODY + 1.0,
            radius: 10.0,
        },
        enabled,
        busy,
        min_width,
    )
}

struct ButtonLook {
    tint: Color32,
    /// The label's colour while the button can be pressed.
    label: Color32,
    height: f32,
    size: f32,
    radius: f32,
}

fn button_in_tint(
    ui: &mut Ui,
    label: &str,
    look: ButtonLook,
    enabled: bool,
    busy: bool,
    min_width: f32,
) -> egui::Response {
    let tint = look.tint;
    let color = if enabled || busy {
        look.label
    } else {
        primary(0.55)
    };
    let text = galley(ui, label, look.size, SEMIBOLD, color, f32::INFINITY);
    let spinner = if busy { 22.0 } else { 0.0 };
    let width = (text.size().x + 36.0 + spinner).max(min_width);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(width, look.height), sense);
    let response = crate::ui::accessibility::label(response, label);
    let pressed = response.is_pointer_button_down_on() && enabled;
    let hovered = response.hovered() && enabled;
    let fill = if !enabled && !busy {
        theme::with_alpha(tint, 0.3)
    } else if pressed {
        tint.gamma_multiply(0.8)
    } else if hovered {
        tint.gamma_multiply(1.12)
    } else {
        tint
    };
    ui.painter().rect_filled(rect, look.radius, fill);
    let content = text.size().x + spinner;
    let mut x = rect.center().x - content / 2.0;
    if busy {
        LoadingIndicator::new().size(14.0).color(color).paint_at(
            ui,
            Rect::from_center_size(pos2(x + 7.0, rect.center().y), Vec2::splat(14.0)),
        );
        x += spinner;
    }
    ui.painter()
        .galley(pos2(x, rect.center().y - text.size().y / 2.0), text, color);
    response
}

/// A large bordered destructive button with a leading glyph (`Button(role: .destructive)`):
/// red title and glyph on a quiet fill.
pub fn destructive_sheet_button(ui: &mut Ui, icon: Icon, label: &str) -> egui::Response {
    let text = galley(ui, label, size::BODY, REGULAR, RED, f32::INFINITY);
    let (rect, response) =
        ui.allocate_exact_size(vec2(text.size().x + 17.0 + 32.0, 28.0), Sense::click());
    let response = crate::ui::accessibility::label(response, label)
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let pressed = response.is_pointer_button_down_on();
    let fill = if pressed {
        theme::with_alpha(RED, 0.2)
    } else if response.hovered() {
        theme::with_alpha(RED, 0.14)
    } else {
        theme::with_alpha(RED, 0.09)
    };
    ui.painter().rect_filled(rect, 7.0, fill);
    ui.painter().rect_stroke(
        rect,
        7.0,
        Stroke::new(1.0, theme::with_alpha(RED, 0.22)),
        StrokeKind::Inside,
    );
    let left = rect.center().x - (text.size().x + 17.0) / 2.0;
    icons::paint(
        ui.painter(),
        Rect::from_center_size(pos2(left + 6.0, rect.center().y), Vec2::splat(12.0)),
        icon,
        RED,
    );
    ui.painter().galley(
        pos2(left + 17.0, rect.center().y - text.size().y / 2.0),
        text,
        RED,
    );
    response
}

/// A button that opens `url` in the browser, beside the page's other buttons (Get API Key).
pub fn open_button(ui: &mut egui::Ui, label: &str, url: &str) {
    if sheet_button(ui, label, false, true)
        .on_hover_text(url)
        .clicked()
    {
        ui.ctx().open_url(egui::OpenUrl::new_tab(url));
    }
}

/// A destructive button: the regular button with red text.
pub fn destructive_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let galley = galley(ui, label, size::BODY, REGULAR, RED, f32::INFINITY);
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2((galley.size().x + 32.0).max(76.0), 28.0),
        egui::Sense::click(),
    );
    let response = crate::ui::accessibility::label(response, label);
    ui.painter().rect_filled(
        rect,
        7.0,
        primary(if response.is_pointer_button_down_on() {
            0.18
        } else if response.hovered() {
            0.14
        } else {
            0.1
        }),
    );
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, RED);
    response
}
