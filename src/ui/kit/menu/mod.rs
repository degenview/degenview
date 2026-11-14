//! Menus: rows, headings and separators, the frame and pop-ups they open in, and submenus.

mod fit;
mod picker;
#[cfg(test)]
mod tests;

pub(crate) use fit::{MENU_FRAME_MARGIN, MENU_MAX_HEIGHT, menu_width_for, menu_width_for_rows};
pub use fit::{
    fitted_context_menu, fitted_context_menu_min, fitted_menu, fitted_menu_rows, menu_fit,
    menu_fit_rows,
};
pub use picker::{menu_picker, menu_picker_hinted};

use super::{RED, REGULAR, SEMIBOLD, galley, hairline, primary, size};
use crate::ui::icons::{self, Icon};
use crate::ui::theme;
use eframe::egui::{self, Color32, CornerRadius, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};

/// A row of a pop-up menu: an icon, a label, the system highlight on hover. Returns whether it
/// was clicked.
pub fn menu_item(ui: &mut Ui, icon: Icon, label: &str, enabled: bool, destructive: bool) -> bool {
    menu_item_with_shortcut(ui, icon, label, None, enabled, destructive)
}

/// A menu row with a keyboard shortcut hint (like `⌘S`) right-aligned.
pub fn menu_item_with_shortcut(
    ui: &mut Ui,
    icon: Icon,
    label: &str,
    shortcut: Option<&str>,
    enabled: bool,
    destructive: bool,
) -> bool {
    let row = menu_row(
        ui,
        MenuRow {
            icon: Some(icon),
            shortcut,
            enabled,
            destructive,
            ..MenuRow::new(label)
        },
    );
    enabled && row.clicked()
}

/// Paints a menu row's custom leading mark: the painter, the slot's rect and the row's ink.
pub type MenuGlyph<'a> = dyn Fn(&egui::Painter, Rect, Color32) + 'a;

/// Everything a menu row can show. The leading slot holds the icon, or a check when the row is
/// `checked`; a submenu row ends in a chevron; `focused` is the keyboard's highlight.
pub struct MenuRow<'a> {
    pub icon: Option<Icon>,
    pub label: &'a str,
    pub shortcut: Option<&'a str>,
    pub enabled: bool,
    pub checked: bool,
    /// The row is the current choice of its group: a check at the trailing edge, so the icon at
    /// the leading edge stays.
    pub current: bool,
    pub chevron: bool,
    pub destructive: bool,
    pub focused: bool,
    /// A custom leading mark (a flag's swatch) painted in the icon's slot instead of `icon`; it is
    /// given the slot's rect and the row's ink.
    pub glyph: Option<&'a MenuGlyph<'a>>,
    /// Whether the label leaves room for an icon or a check. A menu with neither anywhere turns
    /// this off so its labels do not start behind an empty column.
    pub reserve_icon: bool,
}

impl<'a> MenuRow<'a> {
    pub fn new(label: &'a str) -> Self {
        Self {
            icon: None,
            label,
            shortcut: None,
            enabled: true,
            checked: false,
            current: false,
            chevron: false,
            destructive: false,
            focused: false,
            glyph: None,
            reserve_icon: true,
        }
    }
}

/// Paints one menu row and returns its response. A disabled row still reports hovers (for its
/// tooltip) but never clicks, and is announced to screen readers as disabled.
pub fn menu_row(ui: &mut Ui, row: MenuRow) -> egui::Response {
    let width = ui.available_width();
    let sense = if row.enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(width, 24.0), sense);
    let highlighted = row.enabled && (response.hovered() || row.focused);
    let painter = ui.painter();
    if highlighted {
        painter.rect_filled(rect, 5.0, theme::selected_tint(true));
    }
    let color = match (row.enabled, highlighted, row.destructive) {
        (false, _, _) => theme::text_tertiary(),
        (true, true, _) => theme::text_primary(),
        (true, false, true) => RED,
        (true, false, false) => theme::text_primary(),
    };
    let leading = if row.checked {
        Some(Icon::Check)
    } else {
        row.icon
    };
    if let (Some(glyph), false) = (row.glyph, row.checked) {
        glyph(
            painter,
            Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), Vec2::splat(12.0)),
            color,
        );
    } else if let Some(icon) = leading {
        let size = if row.checked { 11.0 } else { 12.0 };
        icons::paint(
            painter,
            Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), Vec2::splat(size)),
            icon,
            color,
        );
    }
    let trailing = if row.chevron || row.current {
        22.0
    } else {
        0.0
    };
    let text_left = if row.reserve_icon { 32.0 } else { 14.0 };
    let hint_color = if highlighted {
        Color32::WHITE.gamma_multiply(0.8)
    } else {
        theme::text_tertiary()
    };
    let hint = row
        .shortcut
        .map(|shortcut| galley(ui, shortcut, size::BODY, REGULAR, hint_color, width));
    let hint_room = hint.as_ref().map_or(0.0, |hint| hint.size().x + 12.0);
    let label = galley(
        ui,
        row.label,
        size::BODY,
        REGULAR,
        color,
        width - text_left - 8.0 - trailing - hint_room,
    );
    painter.galley(
        pos2(
            rect.left() + text_left,
            rect.center().y - label.size().y / 2.0,
        ),
        label,
        color,
    );
    if let Some(hint) = hint {
        painter.galley(
            pos2(
                rect.right() - 10.0 - trailing - hint.size().x,
                rect.center().y - hint.size().y / 2.0,
            ),
            hint,
            hint_color,
        );
    }
    if row.chevron || row.current {
        let mark = if row.current {
            theme::text_primary()
        } else {
            color
        };
        icons::paint(
            painter,
            Rect::from_center_size(
                pos2(rect.right() - 14.0, rect.center().y),
                Vec2::splat(11.0),
            ),
            if row.current {
                Icon::Check
            } else {
                Icon::ChevronRight
            },
            mark,
        );
    }
    let description = match row.shortcut {
        Some(shortcut) => format!("{}, {shortcut}", row.label),
        None => row.label.to_owned(),
    };
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Button,
            row.enabled,
            row.checked || row.current,
            &description,
        )
    });
    response
}

/// A menu row whose leading glyph is a check shown only on the chosen item.
pub fn menu_item_marked(ui: &mut Ui, icon: Icon, label: &str, marked: bool) -> bool {
    menu_item_marked_response(ui, icon, label, marked).0
}

/// `menu_item_marked`, also handing back the row's response.
pub(super) fn menu_item_marked_response(
    ui: &mut Ui,
    icon: Icon,
    label: &str,
    marked: bool,
) -> (bool, egui::Response) {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, 24.0), Sense::click());
    let highlighted = response.hovered();
    if highlighted {
        ui.painter()
            .rect_filled(rect, 5.0, theme::selected_tint(true));
    }
    let color = theme::text_primary();
    if marked {
        icons::paint(
            ui.painter(),
            Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), Vec2::splat(11.0)),
            icon,
            color,
        );
    }
    let label = galley(ui, label, size::BODY, REGULAR, color, width - 40.0);
    ui.painter().galley(
        pos2(rect.left() + 32.0, rect.center().y - label.size().y / 2.0),
        label,
        color,
    );
    (response.clicked(), response)
}

/// A small label over a group of menu rows ("My templates"): not a row, never focused or clicked.
pub fn menu_heading(ui: &mut Ui, text: &str) {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
    let color = theme::text_tertiary();
    let label = galley(
        ui,
        &text.to_uppercase(),
        size::CAPTION,
        SEMIBOLD,
        color,
        rect.width() - 28.0,
    );
    ui.painter().galley(
        pos2(
            rect.left() + 14.0,
            rect.center().y - label.size().y / 2.0 + 2.0,
        ),
        label,
        color,
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text));
}

/// A thin divider between groups of menu items.
pub fn menu_separator(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 9.0), Sense::hover());
    ui.painter().hline(
        rect.shrink2(vec2(8.0, 0.0)).x_range(),
        rect.center().y,
        Stroke::new(1.0, hairline()),
    );
}

/// What menus and popovers are filled with.
pub fn menu_fill() -> Color32 {
    theme::pick(
        Color32::from_rgb(28, 34, 38),
        Color32::from_rgb(250, 250, 251),
    )
}

const MENU_RADIUS: u8 = 9;
const MENU_MARGIN: i8 = 5;

fn menu_shadow() -> egui::Shadow {
    egui::Shadow {
        offset: [0, 6],
        blur: 18,
        spread: 0,
        color: Color32::from_black_alpha(110),
    }
}

/// The frame menus pop up in.
pub fn menu_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(menu_fill())
        .corner_radius(MENU_RADIUS)
        .stroke(Stroke::new(1.0, primary(0.1)))
        .inner_margin(egui::Margin::same(MENU_MARGIN))
        .shadow(menu_shadow())
}

/// A pull-down menu on `response`, in the menu frame. A click inside leaves it open: a row that
/// should end the menu calls `ui.close()`, one that toggles a setting does not.
pub fn menu_popup<'a>(response: &egui::Response) -> egui::Popup<'a> {
    egui::Popup::menu(response)
        .frame(menu_frame())
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
}

/// `menu_popup` for a right-click on `response`, opening at the pointer.
pub fn context_popup<'a>(response: &egui::Response) -> egui::Popup<'a> {
    egui::Popup::context_menu(response)
        .frame(menu_frame())
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
}

/// Sets up the `Ui` inside a menu: rows touch, the menu is at least `min_width` wide, and the
/// submenus egui frames itself (it takes its frame from the style, not from `menu_frame`) match
/// the menu they open from. Call it first in every menu body and submenu body.
pub fn menu_scope(ui: &mut Ui, min_width: f32) {
    ui.set_min_width(min_width);
    let style = ui.style_mut();
    style.spacing.item_spacing = Vec2::ZERO;
    style.spacing.menu_margin = egui::Margin::same(MENU_MARGIN);
    style.visuals.window_fill = menu_fill();
    style.visuals.window_stroke = Stroke::new(1.0, primary(0.1));
    style.visuals.menu_corner_radius = CornerRadius::same(MENU_RADIUS);
    style.visuals.popup_shadow = menu_shadow();
}

/// A menu row that opens a submenu holding `content`, lit while that submenu is open. Rows of the
/// submenu are placed like the main menu's (`menu_scope`, at least `min_width` wide).
pub fn menu_submenu<R>(
    ui: &mut Ui,
    row: MenuRow,
    min_width: f32,
    content: impl FnOnce(&mut Ui) -> R,
) -> egui::Response {
    use egui::containers::menu::{MenuState, SubMenu};
    if !row.enabled {
        return menu_row(
            ui,
            MenuRow {
                chevron: true,
                ..row
            },
        );
    }
    // The row takes the next id, which is the one egui's submenu logic keys on.
    let id = ui.next_auto_id();
    let open = MenuState::from_ui(ui, |state, _| {
        state.open_item == Some(SubMenu::id_from_widget_id(id))
    });
    let response = menu_row(
        ui,
        MenuRow {
            chevron: true,
            focused: row.focused || open,
            ..row
        },
    );
    SubMenu::new().show(ui, &response, |ui| {
        // Rows fill what they are given, and a fresh area is given the whole window.
        ui.set_width(min_width);
        menu_scope(ui, min_width);
        content(ui)
    });
    response
}
