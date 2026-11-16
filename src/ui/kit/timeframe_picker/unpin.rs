//! The right-click menu of a pinned chip: one row, Unpin.
//!
//! egui keeps one popup open in its memory, and `kit::fitted_context_menu` opens a memory popup:
//! inside the picker it would replace the picker itself. This menu keeps its own open state (the
//! point it was asked at, in the picker's [`State`](super::state::State)). While it is open the
//! picker ignores clicks outside itself (a click on this menu is one, to egui), so the first
//! click away only dismisses the menu.

use super::super::{
    MENU_FRAME_MARGIN, MenuRow, menu_frame, menu_row, menu_scope, menu_width_for_rows,
};
use eframe::egui::{self, Pos2, Response, Ui};

/// The menu that opens at the pointer on a right-click of `chip`; `at` is where it is open, if it
/// is, and is where a right-click opens it. Whether "Unpin" was chosen this frame.
pub(super) fn menu(ui: &Ui, chip: &Response, at: &mut Option<Pos2>) -> bool {
    if chip.secondary_clicked() {
        *at = chip.interact_pointer_pos().or(Some(chip.rect.center()));
    }
    let Some(position) = *at else {
        return false;
    };
    let width = menu_width_for_rows(ui, &[("Unpin", None)], 0.0);
    let mut open = true;
    let mut chosen = false;
    egui::Popup::menu(chip)
        .id(chip.id.with("unpin-menu"))
        .open_bool(&mut open)
        .at_position(position)
        .frame(menu_frame())
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .width(width)
        .show(|ui| {
            menu_scope(ui, 0.0);
            ui.set_width(width - MENU_FRAME_MARGIN);
            let row = MenuRow {
                reserve_icon: false,
                ..MenuRow::new("Unpin")
            };
            if menu_row(ui, row).clicked() {
                chosen = true;
                ui.close();
            }
        });
    if !open || chosen {
        *at = None;
    }
    chosen
}
