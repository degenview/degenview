//! The cursor menu of the tool strip: the pointer's modes, one row each. It only reports what was
//! picked; the workspace owns the cursor.
//!
//! Rows are as wide as their longest label (`width`), the picked one is a light plate with dark
//! text (the menu's own fill, so it reads in both themes), never an accent colour.

use crate::model::CursorMode;
use crate::ui::icons::{self, Icon};
use crate::ui::{accessibility, kit, theme};
use eframe::egui::{self, Color32, Id, Sense, Ui, Vec2, pos2, vec2};

const ROW_HEIGHT: f32 = 32.0;
/// Where a row's glyph is centred and where its label starts, from the row's left edge.
const ICON_CENTRE: f32 = 22.0;
const LABEL_X: f32 = 42.0;
/// Room after a row's label.
const END_PAD: f32 = 16.0;
const GLYPH: f32 = 18.0;

/// The glyph of a cursor, on the strip's tile and in the menu.
pub fn icon(mode: CursorMode) -> Icon {
    match mode {
        CursorMode::Cross => Icon::CursorCross,
        CursorMode::Dot => Icon::CursorDot,
        CursorMode::Arrow => Icon::CursorArrow,
    }
}

/// The popup's width: the longest label with the glyph's gutter and the end padding, plus the
/// frame.
pub fn width(ui: &Ui) -> f32 {
    let widest = CursorMode::ALL
        .iter()
        .map(|mode| {
            kit::galley(
                ui,
                mode.label(),
                kit::size::BODY,
                kit::REGULAR,
                Color32::PLACEHOLDER,
                f32::INFINITY,
            )
            .size()
            .x
        })
        .fold(0.0, f32::max);
    (widest + LABEL_X + END_PAD + kit::MENU_FRAME_MARGIN).ceil()
}

/// The menu body. `cursor` is the one in force; returns the cursor picked this frame.
pub fn show(ui: &mut Ui, cursor: CursorMode) -> Option<CursorMode> {
    ui.spacing_mut().item_spacing = Vec2::ZERO;
    let mut picked = None;
    let landed_id = Id::new("cursor-menu-focused");
    for mode in CursorMode::ALL {
        let selected = mode == cursor;
        let response = row(ui, mode, selected);
        // Land the keyboard on the chosen cursor when the menu opens.
        let landed = ui.data(|data| data.get_temp::<bool>(landed_id).unwrap_or(false));
        if !landed && selected {
            response.request_focus();
            ui.data_mut(|data| data.insert_temp(landed_id, true));
        }
        if response.clicked() {
            picked = Some(mode);
            ui.data_mut(|data| data.remove::<bool>(landed_id));
            ui.close();
        }
    }
    if !ui.ctx().memory(|memory| memory.focused().is_some()) {
        // Nothing took the focus: allow the next opening to place it again.
        ui.data_mut(|data| data.remove::<bool>(landed_id));
    }
    picked
}

fn row(ui: &mut Ui, mode: CursorMode, selected: bool) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
    let response = accessibility::selected_as(
        response,
        egui::WidgetType::RadioButton,
        mode.label(),
        selected,
    );
    let (fill, ink) = if selected {
        (Some(theme::text_primary()), kit::menu_fill())
    } else if response.hovered() || response.has_focus() {
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
    response
}
