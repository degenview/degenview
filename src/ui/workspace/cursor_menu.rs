//! The cursor menu of the tool strip: the pointer's modes, one row each with the star that makes it
//! a favorite, and the switch for the values tooltip. It only reports what was picked; the
//! workspace owns the cursor.
//!
//! Rows are as wide as their longest label (`width`), the picked one is a light plate with dark
//! text (the menu's own fill, so it reads in both themes), never an accent colour.

use super::tool_sidebar::{STAR_TARGET, paint_star, star_button};
use crate::i18n::tr;
use crate::model::CursorMode;
use crate::model::drawings::favorites::Favorite;
use crate::ui::icons::{self, Icon};
use crate::ui::{accessibility, kit, theme};
use eframe::egui::{self, Color32, Id, Sense, Ui, Vec2, pos2, vec2};

/// The switch under the rows: holding the button still shows the values of the bar.
pub fn toggle_label() -> &'static str {
    tr!("Values tooltip on long press")
}

const ROW_HEIGHT: f32 = 32.0;
/// Where a row's glyph is centred and where its label starts, from the row's left edge.
const ICON_CENTRE: f32 = 22.0;
const LABEL_X: f32 = 42.0;
/// Room after a row's label for its star, and around the toggle's switch.
const STAR_ROOM: f32 = STAR_TARGET + 12.0;
const END_PAD: f32 = 16.0;
const SWITCH: Vec2 = vec2(32.0, 18.0);
const SWITCH_GAP: f32 = 16.0;
const GLYPH: f32 = 18.0;
/// The toggle's label starts under the rows' glyphs.
const TOGGLE_X: f32 = ICON_CENTRE - GLYPH / 2.0;

/// What the menu asked for this frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CursorMenuAction {
    Set(CursorMode),
    /// Star this cursor, or take its star away. The menu stays open.
    ToggleFavorite(CursorMode),
    ToggleValuesTooltip,
}

/// The glyph of a cursor, on the strip's tile and in the menu.
pub fn icon(mode: CursorMode) -> Icon {
    match mode {
        CursorMode::Cross => Icon::CursorCross,
        CursorMode::Dot => Icon::CursorDot,
        CursorMode::Arrow => Icon::CursorArrow,
        CursorMode::Demonstration => Icon::CursorDemonstration,
        CursorMode::Eraser => Icon::CursorEraser,
    }
}

/// The popup's width: the longest row, and the toggle's label with its switch, plus the frame.
pub fn width(ui: &Ui) -> f32 {
    let text = |text: &str| {
        kit::galley(
            ui,
            text,
            kit::size::BODY,
            kit::REGULAR,
            Color32::PLACEHOLDER,
            f32::INFINITY,
        )
        .size()
        .x
    };
    let rows = CursorMode::ALL
        .iter()
        .map(|mode| text(mode.label()))
        .fold(0.0, f32::max)
        + LABEL_X
        + STAR_ROOM;
    let toggle = text(toggle_label()) + TOGGLE_X + SWITCH_GAP + SWITCH.x + END_PAD / 2.0;
    (rows.max(toggle) + kit::MENU_FRAME_MARGIN).ceil()
}

/// The menu body. `cursor` is the one in force, `values_tooltip` the switch's state and
/// `favorites` what is starred.
pub fn show(
    ui: &mut Ui,
    cursor: CursorMode,
    values_tooltip: bool,
    favorites: &[Favorite],
) -> Vec<CursorMenuAction> {
    ui.spacing_mut().item_spacing = Vec2::ZERO;
    let mut actions = Vec::new();
    let landed_id = Id::new("cursor-menu-focused");
    for mode in CursorMode::ALL {
        let selected = mode == cursor;
        let favorite = favorites.contains(&Favorite::Cursor(mode));
        let (response, star) = row(ui, mode, selected, favorite);
        if star.clicked() {
            actions.push(CursorMenuAction::ToggleFavorite(mode));
        }
        // Land the keyboard on the chosen cursor when the menu opens.
        let landed = ui.data(|data| data.get_temp::<bool>(landed_id).unwrap_or(false));
        if !landed && selected {
            response.request_focus();
            ui.data_mut(|data| data.insert_temp(landed_id, true));
        }
        if response.clicked() {
            actions.push(CursorMenuAction::Set(mode));
            ui.data_mut(|data| data.remove::<bool>(landed_id));
            ui.close();
        }
    }
    kit::menu_separator(ui);
    if toggle_row(ui, values_tooltip).clicked() {
        actions.push(CursorMenuAction::ToggleValuesTooltip);
    }
    if !ui.ctx().memory(|memory| memory.focused().is_some()) {
        // Nothing took the focus: allow the next opening to place it again.
        ui.data_mut(|data| data.remove::<bool>(landed_id));
    }
    actions
}

/// One cursor's row, and its star: a target of its own over the row's end, so a click on it never
/// picks the cursor and the menu stays open.
fn row(
    ui: &mut Ui,
    mode: CursorMode,
    selected: bool,
    favorite: bool,
) -> (egui::Response, egui::Response) {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
    let response = accessibility::selected_as(
        response,
        egui::WidgetType::RadioButton,
        mode.label(),
        selected,
    );
    let star = star_button(
        ui,
        rect,
        Id::new(("cursor-menu-star", mode.label())),
        mode.label(),
        favorite,
    );
    let (fill, ink) = if selected {
        (Some(theme::text_primary()), kit::menu_fill())
    } else if response.hovered() || star.hovered() || response.has_focus() {
        (Some(theme::selected_tint(true)), theme::text_primary())
    } else {
        (None, theme::text_primary())
    };
    if let Some(fill) = fill {
        ui.painter().rect_filled(rect, 6.0, fill);
    }
    icons::paint(
        ui.painter(),
        egui::Rect::from_center_size(
            pos2(rect.left() + ICON_CENTRE, rect.center().y),
            Vec2::splat(GLYPH),
        ),
        icon(mode),
        ink,
    );
    let galley = kit::galley(
        ui,
        mode.label(),
        kit::size::BODY,
        kit::REGULAR,
        ink,
        // The popup is as wide as its longest label needs: never cut it.
        f32::INFINITY,
    );
    ui.painter().galley(
        pos2(
            rect.left() + LABEL_X,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        ink,
    );
    let highlighted = response.hovered() || star.hovered() || response.has_focus();
    paint_star(ui, &star, favorite, highlighted, selected.then_some(ink));
    (response, star)
}

fn toggle_row(ui: &mut Ui, on: bool) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
    let response =
        accessibility::selected_as(response, egui::WidgetType::Checkbox, toggle_label(), on);
    if response.hovered() || response.has_focus() {
        ui.painter()
            .rect_filled(rect, 6.0, theme::selected_tint(true));
    }
    let galley = kit::galley(
        ui,
        toggle_label(),
        kit::size::BODY,
        kit::REGULAR,
        theme::text_primary(),
        f32::INFINITY,
    );
    ui.painter().galley(
        pos2(
            rect.left() + TOGGLE_X,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        theme::text_primary(),
    );
    let progress = ui.ctx().animate_bool(Id::new("cursor-menu-switch"), on);
    let switch = egui::Rect::from_center_size(
        pos2(
            rect.right() - END_PAD / 2.0 - SWITCH.x / 2.0,
            rect.center().y,
        ),
        SWITCH,
    );
    kit::paint_switch(ui.painter(), switch, progress, false);
    response
}
