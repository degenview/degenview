//! Menus as wide as their rows: the measuring, and pull-downs and context menus sized by it.

use super::{context_popup, menu_frame, menu_scope};
use crate::ui::kit::{REGULAR, galley, size};
use eframe::egui::{self, Color32, Ui, Vec2};

/// Fits a context menu to its rows: as wide as the longest of `labels` needs (icon gutter and
/// trailing padding included), so a short menu is not padded out to a guessed width.
pub fn menu_fit(ui: &mut Ui, labels: &[&str]) {
    let widest = labels
        .iter()
        .map(|label| {
            galley(
                ui,
                label,
                size::BODY,
                REGULAR,
                Color32::PLACEHOLDER,
                f32::INFINITY,
            )
            .size()
            .x
        })
        .fold(0.0, f32::max);
    ui.set_width((widest + MENU_ROW_INSET).ceil());
}

/// `menu_fit` for rows that carry a shortcut hint: `(label, shortcut)` per row. Call it after
/// `menu_scope(ui, 0.0)`, inside the menu body.
pub fn menu_fit_rows(ui: &mut Ui, rows: &[(&str, Option<&str>)]) {
    let width = menu_width_for_rows(ui, rows, 0.0) - MENU_FRAME_MARGIN;
    ui.set_width(width);
}

/// Widest a pull-down menu grows before its rows start to ellipsize.
pub(super) const MENU_MAX_WIDTH: f32 = 360.0;
/// Tallest a pull-down menu gets before it scrolls.
pub(crate) const MENU_MAX_HEIGHT: f32 = 320.0;
/// A menu row's leading check gutter and trailing padding, around its label.
pub(super) const MENU_ROW_INSET: f32 = 32.0 + 12.0;
/// The menu frame's margin on both sides.
pub(crate) const MENU_FRAME_MARGIN: f32 = 10.0;

/// The width a pull-down menu needs for `labels`: its widest row, the trigger's width at least,
/// at most `MENU_MAX_WIDTH`. The trigger can be capped narrower than its choices (a long title
/// would crowd its card), so the menu is sized by its content, not the trigger.
pub(crate) fn menu_width_for(ui: &Ui, labels: &[impl AsRef<str>], trigger_width: f32) -> f32 {
    let rows: Vec<(&str, Option<&str>)> = labels.iter().map(|l| (l.as_ref(), None)).collect();
    menu_width_for_rows(ui, &rows, trigger_width)
}

/// `menu_width_for` for rows that may end in a keyboard shortcut hint: each row is its label and
/// its shortcut, and the widest label-plus-hint sets the width.
pub(crate) fn menu_width_for_rows(
    ui: &Ui,
    rows: &[(&str, Option<&str>)],
    trigger_width: f32,
) -> f32 {
    let text = |text: &str| {
        galley(
            ui,
            text,
            size::BODY,
            REGULAR,
            Color32::PLACEHOLDER,
            f32::INFINITY,
        )
        .size()
        .x
    };
    let widest = rows
        .iter()
        .map(|(label, hint)| text(label) + hint.map_or(0.0, |hint| text(hint) + 12.0))
        .fold(0.0, f32::max);
    let wanted = (widest + MENU_ROW_INSET + MENU_FRAME_MARGIN).max(trigger_width.max(72.0));
    // A trigger already wider than the cap keeps its own width.
    wanted.min(MENU_MAX_WIDTH.max(trigger_width))
}

/// A pull-down on `response` that is exactly as wide as its rows need (`labels` are the rows'
/// texts), at least 72 pt and at most `MENU_MAX_WIDTH`, never padded out to a guessed width.
/// Rows touch; a click inside ends the menu only where the row calls `ui.close()`.
pub fn fitted_menu<R>(
    ui: &Ui,
    response: &egui::Response,
    align: egui::RectAlign,
    labels: &[impl AsRef<str>],
    body: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    let rows: Vec<(&str, Option<&str>)> = labels.iter().map(|l| (l.as_ref(), None)).collect();
    fitted_menu_rows(ui, response, align, &rows, body)
}

/// `fitted_menu` for rows that carry a shortcut hint: `(label, shortcut)` per row.
pub fn fitted_menu_rows<R>(
    ui: &Ui,
    response: &egui::Response,
    align: egui::RectAlign,
    rows: &[(&str, Option<&str>)],
    body: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    // Menus hang off rows and cards drawn every frame: measure only once one is wanted.
    let wanted = response.clicked()
        || egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(response));
    if !wanted {
        return None;
    }
    let width = menu_width_for_rows(ui, rows, 0.0);
    egui::Popup::menu(response)
        .frame(menu_frame())
        .align(align)
        .gap(4.0)
        .width(width)
        .show(|ui| {
            ui.set_width(width - MENU_FRAME_MARGIN);
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            body(ui)
        })
        .map(|shown| shown.inner)
}

/// `fitted_menu_rows` for a right-click on `response`, opening at the pointer.
pub fn fitted_context_menu<R>(
    ui: &Ui,
    response: &egui::Response,
    rows: &[(&str, Option<&str>)],
    body: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    fitted_context_menu_min(ui, response, rows, 0.0, body)
}

/// [`fitted_context_menu`] for a menu that also holds something of its own width (`content_width`
/// is the room that content needs inside the menu): as wide as the rows or that, whichever is more.
pub fn fitted_context_menu_min<R>(
    ui: &Ui,
    response: &egui::Response,
    rows: &[(&str, Option<&str>)],
    content_width: f32,
    body: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    // Rows are drawn by the hundred and this runs for each: measure only once a menu is wanted.
    let wanted = response.secondary_clicked()
        || egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(response));
    if !wanted {
        return None;
    }
    let width = menu_width_for_rows(ui, rows, 0.0).max(content_width + MENU_FRAME_MARGIN);
    context_popup(response)
        .width(width)
        .show(|ui| {
            menu_scope(ui, 0.0);
            ui.set_width(width - MENU_FRAME_MARGIN);
            body(ui)
        })
        .map(|shown| shown.inner)
}
