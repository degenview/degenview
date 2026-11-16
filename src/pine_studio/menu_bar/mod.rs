//! The strip over the code editor: File, Edit, View and Theme, and the editor's problems at its
//! right end. Every item of the studio's context menus is in one of the menus, with its shortcut, so
//! nothing is reachable only by right-clicking. It sits at the top of the editor column, left of
//! the rail, and spans the code and the assistant and debugger panels under it.
//!
//! While one menu is open, pointing at another title opens that one instead, as in a native menu
//! bar. The rows are plain data (`entry`) built from the state each frame a menu is wanted; the
//! Edit menu's state comes from the editor lazily, since it copies the undo history.
//!
//! A native menu leaves the caret where it was; a click on the strip would take the keyboard from
//! the editor, and an editor without it collapses its selection (so Cut and Copy would grey out
//! as the menu opens). So while a menu is up, and for the frame after a row was chosen, the strip
//! gives the keyboard back to the editor, if the editor had it when the menu was opened.

mod edit;
mod entry;
mod file;
mod themes;
mod view;

#[cfg(test)]
mod tests;

use super::commands::Cmd;
use super::editor::EditorMenuState;
use super::editor::status::ProblemsChip;
use crate::host::editor_theme::EditorThemeHost;
use crate::ui::kit::{self, size};
use crate::ui::theme;
use eframe::egui::{self, Id, Popup, Rect, Response, Sense, Ui, pos2, vec2};
use entry::Entry;
pub(super) use file::FileState;
use std::cell::RefCell;
pub(super) use view::ViewState;

/// The strip's height, its separator not counted.
pub(super) const HEIGHT: f32 = 28.0;
const BUTTON_HEIGHT: f32 = 22.0;
/// Room either side of a title's text, and before the first title.
const PADDING: f32 = 10.0;
const INSET: f32 = 6.0;
const RADIUS: f32 = 6.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Menu {
    File,
    Edit,
    View,
    Theme,
}

impl Menu {
    const ALL: [Self; 4] = [Self::File, Self::Edit, Self::View, Self::Theme];

    fn title(self) -> &'static str {
        match self {
            Self::File => "File",
            Self::Edit => "Edit",
            Self::View => "View",
            Self::Theme => "Theme",
        }
    }
}

/// What the strip reads of the studio, besides the editor.
pub(super) struct MenuBarState {
    /// The editor's text field, which the strip keeps the keyboard for.
    pub editor_id: Id,
    pub file: FileState,
    pub view: ViewState,
    /// The editor's themes, read for the Theme menu only while it is up.
    pub themes: EditorThemeHost,
    /// The editor's problems, at the strip's right end.
    pub problems: ProblemsChip,
}

/// What the strip remembers between frames about the editor's keyboard.
#[derive(Clone, Copy, Default)]
struct KeptKeyboard {
    /// A menu is up and the editor had the keyboard when it was opened.
    holding: bool,
    /// A row was chosen last frame, whose click took the keyboard from the editor.
    hand_back: bool,
}

/// Draws the strip at the top of `ui` and then leaves the rest of `ui` to what follows. What a
/// menu's row chose is pushed to `cmds`. `editor` gives the editor's state, asked for only while
/// the Edit menu is up.
pub(super) fn show(
    ui: &mut Ui,
    state: &MenuBarState,
    editor: impl FnOnce() -> EditorMenuState,
    cmds: &RefCell<Vec<Cmd>>,
) {
    let chosen = egui::Panel::top("pine-studio-menu-bar")
        .exact_size(HEIGHT)
        .resizable(false)
        .frame(egui::Frame::new().fill(theme::chrome()))
        .show(ui, |ui| titles_and_menus(ui, state, editor))
        .inner;
    cmds.borrow_mut().extend(chosen);
}

fn titles_and_menus(
    ui: &mut Ui,
    state: &MenuBarState,
    editor: impl FnOnce() -> EditorMenuState,
) -> Vec<Cmd> {
    let titles = place_titles(ui);
    switch_to_pointed_title(ui.ctx(), &titles);
    for (menu, response) in &titles {
        paint_title(ui, *menu, response);
    }
    let strip = ui.max_rect();
    state
        .problems
        .show(ui, strip.right() - PADDING, strip.center().y);
    let up = titles
        .iter()
        .any(|(_, response)| wanted(ui.ctx(), response));
    let mut kept = keep_keyboard(ui.ctx(), state.editor_id, up);
    let mut editor = Some(editor);
    let mut chosen = Vec::new();
    for (menu, response) in &titles {
        if !wanted(ui.ctx(), response) {
            continue;
        }
        let entries = match menu {
            Menu::File => file::entries(&state.file),
            Menu::Edit => editor
                .take()
                .map(|editor| edit::entries(&editor()))
                .unwrap_or_default(),
            Menu::View => view::entries(&state.view),
            Menu::Theme => themes::entries(&state.themes.choices()),
        };
        show_menu(ui, response, entries, &mut chosen);
    }
    kept.hand_back = kept.holding && !chosen.is_empty();
    ui.ctx().data_mut(|data| data.insert_temp(kept_id(), kept));
    if up {
        // The menus have had Esc; the editor under them must not also act on it (close its find
        // bar, drop its extra carets).
        ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    }
    chosen
}

fn kept_id() -> Id {
    Id::new("pine-studio-menu-bar-keyboard")
}

/// Works out whether the editor keeps the keyboard this frame and gives it back if so. Runs
/// before the editor is drawn, so the field never sees a frame without it. Only a keyboard nobody
/// holds is given back: a field that took it since (the find bar, the picker's search, a sheet's
/// name) keeps it, and the strip stops holding.
fn keep_keyboard(ctx: &egui::Context, editor: Id, up: bool) -> KeptKeyboard {
    let mut kept: KeptKeyboard = ctx
        .data(|data| data.get_temp(kept_id()))
        .unwrap_or_default();
    let focused = ctx.memory(|memory| memory.focused());
    let free = focused.is_none_or(|id| id == editor);
    kept.holding = up && free && (kept.holding || focused == Some(editor));
    if free && (kept.holding || kept.hand_back) {
        ctx.memory_mut(|memory| memory.request_focus(editor));
    }
    kept.hand_back = false;
    kept
}

/// The titles side by side from the strip's left edge.
fn place_titles(ui: &mut Ui) -> Vec<(Menu, Response)> {
    let strip = ui.max_rect();
    let top = strip.center().y - BUTTON_HEIGHT / 2.0;
    let mut x = strip.left() + INSET;
    Menu::ALL
        .into_iter()
        .map(|menu| {
            let text = kit::galley(
                ui,
                menu.title(),
                size::BODY,
                kit::REGULAR,
                theme::text_primary(),
                f32::INFINITY,
            );
            let rect = Rect::from_min_size(
                pos2(x, top),
                vec2(text.size().x + 2.0 * PADDING, BUTTON_HEIGHT),
            );
            x = rect.right();
            let response = ui.interact(rect, title_id(ui, menu), Sense::click());
            (
                menu,
                crate::ui::accessibility::label(response, menu.title()),
            )
        })
        .collect()
}

fn title_id(ui: &Ui, menu: Menu) -> Id {
    ui.id().with(menu.title())
}

fn popup_id(response: &Response) -> Id {
    Popup::default_response_id(response)
}

fn is_open(ctx: &egui::Context, response: &Response) -> bool {
    Popup::is_id_open(ctx, popup_id(response))
}

/// Whether the title's menu is shown this frame: it was clicked or is already up.
fn wanted(ctx: &egui::Context, response: &Response) -> bool {
    response.clicked() || is_open(ctx, response)
}

/// With a menu up, the pointer on another title opens that one in its place.
fn switch_to_pointed_title(ctx: &egui::Context, titles: &[(Menu, Response)]) {
    if !titles.iter().any(|(_, response)| is_open(ctx, response)) {
        return;
    }
    for (_, response) in titles {
        if response.hovered() && !is_open(ctx, response) {
            Popup::open_id(ctx, popup_id(response));
            ctx.request_repaint();
        }
    }
}

/// A flat title: nothing at rest, the bar's faint tint under the pointer and while its menu is up.
fn paint_title(ui: &Ui, menu: Menu, response: &Response) {
    let open = is_open(ui.ctx(), response);
    let tint = if response.is_pointer_button_down_on() {
        0.16
    } else if open {
        0.11
    } else if response.hovered() {
        0.09
    } else {
        0.0
    };
    if tint > 0.0 {
        ui.painter()
            .rect_filled(response.rect, RADIUS, kit::primary(tint));
    }
    let color = if open || response.hovered() {
        theme::text_primary()
    } else {
        theme::text_secondary()
    };
    let text = kit::galley(
        ui,
        menu.title(),
        size::BODY,
        kit::REGULAR,
        color,
        f32::INFINITY,
    );
    let at = response.rect.center() - text.size() / 2.0;
    ui.painter().galley(at, text, color);
}

/// The pull-down under a title, as wide as its rows.
fn show_menu(ui: &Ui, title: &Response, entries: Vec<Entry>, chosen: &mut Vec<Cmd>) {
    let measured = entry::measured(&entries);
    kit::fitted_menu_rows(
        ui,
        title,
        egui::RectAlign::BOTTOM_START,
        &entry::as_rows(&measured),
        |ui| entry::show(ui, entries, chosen),
    );
}

/// The middle of `menu`'s title in a strip whose top left is `at`, for the tests that click it.
#[cfg(test)]
pub(super) fn title_centre(menu: Menu, at: egui::Pos2) -> egui::Pos2 {
    let ctx = egui::Context::default();
    crate::ui::theme::install_offscreen(&ctx);
    let mut centre = at;
    let mut frame = ctx.run_ui(egui::RawInput::default(), |ui| {
        let mut x = at.x + INSET;
        for candidate in Menu::ALL {
            let width = kit::galley(
                ui,
                candidate.title(),
                size::BODY,
                kit::REGULAR,
                theme::text_primary(),
                f32::INFINITY,
            )
            .size()
            .x + 2.0 * PADDING;
            if candidate == menu {
                centre = pos2(x + width / 2.0, at.y + HEIGHT / 2.0);
            }
            x += width;
        }
    });
    frame.textures_delta.clear();
    centre
}
