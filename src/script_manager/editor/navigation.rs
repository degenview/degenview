//! Go to definition, the text area's right-click menu and Rename, over [`symbols`].
//!
//! The resolver is pure and cached by text; this is the egui side: the ⌘-hover cue, the menu,
//! the inline rename box and the keys that open them.

use super::*;
use crate::native_bridge::EditMenu;
use crate::script_manager::call_sites::{self, CallSite};
use crate::script_manager::symbols::{self, SymbolId, SymbolTable};
use eframe::egui::Galley;
use std::ops::Range;

#[derive(Default)]
pub(super) struct NavState {
    pub(super) table: Option<(u64, Arc<SymbolTable>)>,
    /// The character under the pointer when the context menu opened.
    menu_target: Option<usize>,
    pub(super) rename: Option<RenameBox>,
    /// Where the rename box hangs, set while the text is laid out.
    anchor: Option<Rect>,
    /// An Edit-menu item chosen since the last frame.
    pending: Option<EditAction>,
    /// What the Edit menu may do, worked out each frame the editor has the keyboard.
    menu: EditMenu,
    /// The text and caret `menu`'s name checks were made for.
    menu_key: Option<(u64, usize)>,
    /// Where the context menu was opened, for a list its "Find Usages" opens.
    menu_pos: Option<egui::Pos2>,
    /// The lines a function is called on, listed after ⌘-clicking its declaration.
    calls: Option<CallList>,
}

/// The list of calls, where the click was, and the text it was made for (any other text drops it).
pub(super) struct CallList {
    at: egui::Pos2,
    /// The function the calls are of, for the header.
    name: String,
    sites: Vec<CallSite>,
    source_hash: u64,
    /// The row the keyboard (or the pointer) is on.
    selected: usize,
    /// The list scrolls to the selected row on the next frame (after a key).
    scroll_to_selected: bool,
    /// Where the list was drawn last frame: a press outside it closes it.
    rect: Option<Rect>,
    /// Esc and Enter, taken before the text field could see them and acted on when it is drawn.
    escape: bool,
    enter: bool,
}

/// What the macOS Edit menu asks of the editor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EditAction {
    Cut,
    Copy,
    Paste,
    GoToDefinition,
    Rename,
}

pub(super) struct RenameBox {
    symbol: SymbolId,
    kind: &'static str,
    original: String,
    /// Where the name starts (where the box hangs from) and where the caret was.
    at: usize,
    caret: usize,
    text: String,
    /// The text the box was opened on; any other text cancels it.
    source_hash: u64,
    error: Option<String>,
    checked: Option<String>,
    needs_focus: bool,
    /// The box has been drawn once. An area's first frame only measures it and takes no focus.
    shown: bool,
    had_focus: bool,
}

const RENAME_ID: &str = "script-rename-field";

enum MenuAction {
    Cut,
    Copy,
    Paste,
    Definition(Range<usize>),
    FindUsages(usize),
    Rename(usize),
}

impl EditorState {
    /// The symbols of the current text, analysed once per change.
    pub(super) fn symbols(&mut self) -> Arc<SymbolTable> {
        let key = text_hash(&self.source);
        if let Some((cached, table)) = &self.nav.table
            && *cached == key
        {
            return Arc::clone(table);
        }
        let table = Arc::new(symbols::analyze(&self.source));
        self.nav.table = Some((key, Arc::clone(&table)));
        table
    }

    /// Selects `range`, focuses the editor and flashes the spot (which also scrolls to it).
    pub(super) fn go_to_range(&mut self, ctx: &egui::Context, range: Range<usize>) {
        self.carets.clear();
        self.nav.rename = None;
        let mut edit_state = TextEditState::load(ctx, self.id).unwrap_or_default();
        edit_state.cursor.set_char_range(Some(CCursorRange::two(
            CCursor::new(range.start),
            CCursor::new(range.end),
        )));
        edit_state.store(ctx, self.id);
        ctx.memory_mut(|memory| memory.request_focus(self.id));
        self.flash = Some(super::Flash::at(range.start));
        ctx.request_repaint();
    }

    /// Jumps from the name at `index` to its declaration. Returns whether it had one.
    pub(super) fn go_to_definition(&mut self, ctx: &egui::Context, index: usize) -> bool {
        match self.symbols().definition_of(index) {
            Some(range) => {
                self.go_to_range(ctx, range);
                true
            }
            None => false,
        }
    }

    /// The function or method at `index`, with a row for each line that calls it and the function
    /// each call is in. `declaration_only`: only a click on its declaration counts (⌘-click, where a
    /// click on a call goes to the declaration instead); otherwise a call will do too (the menu).
    /// `None` when `index` is not on a function that is called.
    pub(super) fn call_sites_at(
        &mut self,
        index: usize,
        declaration_only: bool,
    ) -> Option<(String, Vec<CallSite>)> {
        let table = self.symbols();
        let name = table
            .name_at(index)
            .map(|(id, _)| table.symbol(id).name.clone())?;
        let calls = if declaration_only {
            table.calls_of(index)?
        } else {
            table.usages_of(index)?
        };
        let mut sites = call_sites::describe(&self.source, &calls);
        if sites.is_empty() {
            return None;
        }
        for site in &mut sites {
            site.context = table
                .enclosing_callable(site.range.start)
                .map(str::to_owned);
        }
        Some((name, sites))
    }

    /// "Find Usages": jumps to the one call of the function at `index`, or lists them at `at`.
    /// Returns whether there was a function with calls.
    pub(super) fn find_usages(
        &mut self,
        ctx: &egui::Context,
        index: usize,
        at: egui::Pos2,
    ) -> bool {
        match self.call_sites_at(index, false) {
            Some((name, sites)) => {
                self.open_call_list(ctx, at, name, sites);
                true
            }
            None => false,
        }
    }

    /// What ⌘-clicking a declaration does with the calls found: one call is jumped to, several are
    /// listed at the click.
    pub(super) fn open_call_list(
        &mut self,
        ctx: &egui::Context,
        at: egui::Pos2,
        name: String,
        sites: Vec<CallSite>,
    ) {
        match sites.as_slice() {
            [only] => self.go_to_range(ctx, only.range.clone()),
            _ => {
                self.carets.clear();
                self.nav.calls = Some(CallList {
                    at,
                    name,
                    sites,
                    source_hash: text_hash(&self.source),
                    selected: 0,
                    scroll_to_selected: false,
                    rect: None,
                    escape: false,
                    enter: false,
                });
            }
        }
    }

    /// Runs an Edit-menu item on the next frame (the menu lives outside the window's frames).
    pub fn request_edit(&mut self, action: EditAction) {
        self.nav.pending = Some(action);
    }

    /// What the Edit menu may do now.
    pub fn edit_menu(&self) -> EditMenu {
        self.nav.menu
    }

    /// Opens the rename box on the name at `index`. Returns whether there was a name.
    pub(super) fn begin_rename(&mut self, index: usize) -> bool {
        let table = self.symbols();
        let Some((id, range)) = table.name_at(index) else {
            return false;
        };
        let symbol = table.symbol(id);
        self.nav.rename = Some(RenameBox {
            symbol: id,
            kind: symbol.kind.label(),
            original: symbol.name.clone(),
            at: range.start,
            caret: index,
            text: symbol.name.clone(),
            source_hash: text_hash(&self.source),
            error: None,
            checked: None,
            needs_focus: true,
            shown: false,
            had_focus: false,
        });
        true
    }

    #[cfg(test)]
    pub fn rename_error(&self) -> Option<String> {
        self.nav
            .rename
            .as_ref()
            .and_then(|rename| rename.error.clone())
    }

    /// The character the context menu was opened on.
    #[cfg(test)]
    pub fn menu_target(&self) -> Option<usize> {
        self.nav.menu_target
    }

    /// Closes the call list (tests).
    #[cfg(test)]
    pub fn nav_clear_calls(&mut self) {
        self.nav.calls = None;
    }

    /// The rows of the call list, as shown (tests).
    #[cfg(test)]
    pub fn call_list_rows(&self) -> Vec<String> {
        self.nav
            .calls
            .as_ref()
            .map(|list| list.sites.iter().map(CallSite::label).collect())
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub fn rename_open(&self) -> bool {
        self.nav.rename.is_some()
    }

    /// Opens the rename box on the name at the caret (tests and the menu share this).
    #[cfg(test)]
    pub fn rename_text(&mut self, text: &str) {
        if let Some(rename) = self.nav.rename.as_mut() {
            rename.text = text.to_owned();
        }
    }
}

/// Cut, Copy or Paste: the platform answers with the matching event next frame, which the text
/// field (or `carets`) handles like the key.
fn clipboard(ctx: &egui::Context, state: &EditorState, action: EditAction) {
    ctx.send_viewport_cmd(match action {
        EditAction::Cut => egui::ViewportCommand::RequestCut,
        EditAction::Copy => egui::ViewportCommand::RequestCopy,
        _ => egui::ViewportCommand::RequestPaste,
    });
    ctx.memory_mut(|memory| memory.request_focus(state.id));
}

/// The Edit menu's side of a frame: runs the item chosen since the last one, then works out what
/// the menu may do next. The items are on only while this editor has the keyboard.
pub(super) fn edit_menu(ctx: &egui::Context, state: &mut EditorState) {
    let caret = caret_index(ctx, state.id);
    if let Some(action) = state.nav.pending.take() {
        match action {
            EditAction::Cut | EditAction::Copy | EditAction::Paste => clipboard(ctx, state, action),
            EditAction::GoToDefinition => {
                if let Some(caret) = caret {
                    state.go_to_definition(ctx, caret);
                }
            }
            EditAction::Rename => {
                if let Some(caret) = caret {
                    state.begin_rename(caret);
                }
            }
        }
        ctx.request_repaint();
    }
    let viewport_focused = ctx.input(|input| input.focused);
    let focused = ctx.memory(|memory| memory.has_focus(state.id)) && viewport_focused;
    update_menu(ctx, state, caret.filter(|_| focused));
    // Published from here as well as by the app, so a change of focus or caret is in the menu bar
    // at once rather than at the root window's next frame. A window without the keyboard leaves
    // it to the one that has it.
    if viewport_focused {
        crate::native_bridge::set_edit_menu(state.nav.menu);
    }
}

/// Works out what the Edit menu may do; `caret` is `None` unless this editor has the keyboard.
fn update_menu(ctx: &egui::Context, state: &mut EditorState, caret: Option<usize>) {
    let Some(caret) = caret else {
        state.nav.menu = EditMenu::default();
        state.nav.menu_key = None;
        return;
    };
    let selected = TextEditState::load(ctx, state.id)
        .and_then(|edit| edit.cursor.char_range())
        .is_some_and(|range| range.primary != range.secondary);
    state.nav.menu.cut = selected;
    state.nav.menu.copy = selected;
    state.nav.menu.paste = true;
    // Worked out once per text and caret, not per frame.
    let key = (text_hash(&state.source), caret);
    if state.nav.menu_key != Some(key) {
        let table = state.symbols();
        state.nav.menu.go_to_definition = table.definition_of(caret).is_some();
        state.nav.menu.rename = table.name_at(caret).is_some();
        state.nav.menu_key = Some(key);
    }
}

fn caret_index(ctx: &egui::Context, id: Id) -> Option<usize> {
    TextEditState::load(ctx, id)?
        .cursor
        .char_range()
        .map(|range| range.primary.index.0)
}

/// F12 and F2 while the editor has the keyboard.
pub(super) fn keys(ctx: &egui::Context, state: &mut EditorState) {
    if !ctx.memory(|memory| memory.has_focus(state.id)) {
        return;
    }
    let (go, rename) = ctx.input_mut(|input| {
        (
            shortcut::GO_TO_DEFINITION.consume(input),
            shortcut::RENAME_SYMBOL.consume(input),
        )
    });
    let Some(caret) = caret_index(ctx, state.id) else {
        return;
    };
    if go {
        state.go_to_definition(ctx, caret);
    }
    if rename {
        state.begin_rename(caret);
    }
}

fn name_rect(galley: &Galley, origin: egui::Pos2, range: &Range<usize>) -> Option<Rect> {
    let a = galley.pos_from_cursor(CCursor::new(range.start));
    let b = galley.pos_from_cursor(CCursor::new(range.end));
    ((a.min.y - b.min.y).abs() < 1.0).then(|| {
        Rect::from_min_max(a.min, egui::pos2(b.min.x, a.max.y)).translate(origin.to_vec2())
    })
}

/// While ⌘ (Ctrl off the Mac) is held: underlines the name under the pointer if it has a
/// definition to jump to, and shows the hand.
pub(super) fn hover(
    ui: &Ui,
    state: &mut EditorState,
    response: &egui::Response,
    galley: &Galley,
    origin: egui::Pos2,
) {
    if !ui.input(|input| input.modifiers.command) || !response.hovered() {
        return;
    }
    let Some(pointer) = ui.input(|input| input.pointer.hover_pos()) else {
        return;
    };
    let index = galley
        .cursor_from_pos(pointer.to_vec2() - origin.to_vec2())
        .index
        .0;
    let table = state.symbols();
    let Some((_, range)) = table.name_at(index) else {
        return;
    };
    // A use has a declaration to jump to; a declaration may have calls to jump to.
    let has_calls = || table.calls_of(index).is_some_and(|calls| !calls.is_empty());
    if table.definition_of(index).is_none() && !has_calls() {
        return;
    }
    if let Some(rect) = name_rect(galley, origin, &range)
        && rect.expand2(Vec2::new(0.0, 1.0)).contains(pointer)
    {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        ui.painter().line_segment(
            [rect.left_bottom(), rect.right_bottom()],
            Stroke::new(1.0, theme::text_primary()),
        );
    }
}

/// The right-click menu of the text area. Applies what was chosen; returns nothing to repaint
/// for because it only moves the caret or opens the rename box.
pub(super) fn context_menu(
    ctx: &egui::Context,
    state: &mut EditorState,
    response: &egui::Response,
    galley: &Galley,
    origin: egui::Pos2,
) {
    state.nav.anchor = state.nav.rename.as_ref().map(|rename| {
        galley
            .pos_from_cursor(CCursor::new(rename.at))
            .translate(origin.to_vec2())
    });
    if response.secondary_clicked()
        && let Some(pos) = response.interact_pointer_pos()
        && !ctx.input(|input| input.modifiers.shift)
    {
        let index = galley
            .cursor_from_pos(pos.to_vec2() - origin.to_vec2())
            .index
            .0;
        state.nav.menu_target = Some(index);
        state.nav.menu_pos = Some(pos);
    }
    let mut action = None;
    kit::context_popup(response).show(|ui| {
        // Analysed when the menu is open, not on every frame the editor is drawn.
        let table = state.symbols();
        let target = state.nav.menu_target;
        let named = target.and_then(|index| table.name_at(index)).is_some();
        let definition = target.and_then(|index| table.definition_of(index));
        let has_usages = target
            .and_then(|index| table.usages_of(index))
            .is_some_and(|calls| !calls.is_empty());
        kit::menu_scope(ui, 200.0);
        kit::menu_fit(ui, &["Go to Definition", "Find Usages", "Rename Symbol…"]);
        let selected = TextEditState::load(ctx, state.id)
            .and_then(|edit| edit.cursor.char_range())
            .is_some_and(|range| range.primary != range.secondary);
        for (icon, label, shortcut, enabled, chosen) in [
            (
                Some(Icon::Scissors),
                "Cut",
                shortcut::CUT.label(),
                selected,
                MenuAction::Cut,
            ),
            (
                Some(Icon::CopyDoc),
                "Copy",
                shortcut::COPY.label(),
                selected,
                MenuAction::Copy,
            ),
            (
                Some(Icon::Clipboard),
                "Paste",
                shortcut::PASTE.label(),
                true,
                MenuAction::Paste,
            ),
        ] {
            let row = kit::MenuRow {
                icon,
                shortcut: Some(&shortcut),
                enabled,
                ..kit::MenuRow::new(label)
            };
            if kit::menu_row(ui, row).clicked() {
                action = Some(chosen);
                ui.close();
            }
        }
        kit::menu_separator(ui);
        if kit::menu_item_with_shortcut(
            ui,
            Icon::ArrowUpRight,
            "Go to Definition",
            Some(&shortcut::GO_TO_DEFINITION.label()),
            definition.is_some(),
            false,
        ) {
            action = definition.map(MenuAction::Definition);
            ui.close();
        }
        if kit::menu_item_with_shortcut(ui, Icon::Magnifier, "Find Usages", None, has_usages, false)
        {
            action = target.map(MenuAction::FindUsages);
            ui.close();
        }
        if kit::menu_item_with_shortcut(
            ui,
            Icon::Pencil,
            "Rename Symbol…",
            Some(&shortcut::RENAME_SYMBOL.label()),
            named,
            false,
        ) {
            action = target.map(MenuAction::Rename);
            ui.close();
        }
    });
    match action {
        Some(MenuAction::Cut) => clipboard(ctx, state, EditAction::Cut),
        Some(MenuAction::Copy) => clipboard(ctx, state, EditAction::Copy),
        Some(MenuAction::Paste) => clipboard(ctx, state, EditAction::Paste),
        Some(MenuAction::Definition(range)) => state.go_to_range(ctx, range),
        Some(MenuAction::FindUsages(index)) => {
            let at = state.nav.menu_pos.unwrap_or(egui::Pos2::ZERO);
            state.find_usages(ctx, index, at);
        }
        Some(MenuAction::Rename(index)) => {
            state.begin_rename(index);
        }
        None => {}
    }
}

/// ↑ ↓ Home End Enter and Esc while the call list is open. Taken before the text field is drawn, so
/// the caret does not move and Enter types nothing; `call_list` acts on them.
pub(super) fn call_list_keys(ctx: &egui::Context, state: &mut EditorState) {
    let Some(list) = state.nav.calls.as_mut() else {
        return;
    };
    let (escape, up, down, home, end, enter) = ctx.input_mut(|input| {
        let none = Modifiers::NONE;
        (
            input.consume_key(none, Key::Escape),
            input.consume_key(none, Key::ArrowUp),
            input.consume_key(none, Key::ArrowDown),
            input.consume_key(none, Key::Home),
            input.consume_key(none, Key::End),
            input.consume_key(none, Key::Enter),
        )
    });
    let last = list.sites.len().saturating_sub(1);
    let before = list.selected;
    if up {
        list.selected = list.selected.saturating_sub(1);
    }
    if down {
        list.selected = (list.selected + 1).min(last);
    }
    if home {
        list.selected = 0;
    }
    if end {
        list.selected = last;
    }
    list.scroll_to_selected |= list.selected != before;
    list.escape |= escape;
    list.enter |= enter;
}

/// Rows the list shows before it scrolls.
const CALL_ROWS: f32 = 10.0;
const CALL_ROW_HEIGHT: f32 = 28.0;
const CALL_PADDING: f32 = 6.0;

/// The list of calls opened by ⌘-clicking a declaration: one row per calling line, its number and
/// the start of its code with the called name emphasised. ↑ ↓ Home End move, Enter or a click goes
/// there. Esc, a press anywhere outside it, scrolling away, losing the window's focus or any edit
/// closes it; the press that closed it still reaches whatever it was aimed at.
pub(super) fn call_list(ui: &Ui, state: &mut EditorState, look: &Appearance, code: &EditorFont) {
    let ctx = ui.ctx().clone();
    let Some(mut list) = state.nav.calls.take() else {
        return;
    };
    if list.source_hash != text_hash(&state.source) {
        return;
    }
    let away = ctx.input(|input| {
        let outside = |position: Option<egui::Pos2>| {
            list.rect
                .zip(position)
                .is_some_and(|(rect, position)| !rect.contains(position))
        };
        !input.focused
            || (input.pointer.any_pressed() && outside(input.pointer.interact_pos()))
            || (input.smooth_scroll_delta != Vec2::ZERO && outside(input.pointer.hover_pos()))
    });
    if list.escape || away {
        return;
    }

    let scale = code.chrome_scale();
    let mono = code.at(&ctx, 12.0 * scale);
    let small = code.at(&ctx, 11.0 * scale);
    let ui_font = FontId::proportional(12.5);
    let hint_font = FontId::proportional(11.0);
    let number = look.surface(SurfaceColor::LineNumber);
    let secondary = theme::text_secondary();
    let primary = theme::text_primary();
    let tertiary = theme::text_tertiary();
    let painter = ui.painter();
    let widest = list
        .sites
        .iter()
        .map(|site| {
            painter
                .layout_no_wrap(site.preview.clone(), mono.clone(), primary)
                .size()
                .x
        })
        .fold(0.0, f32::max);
    let number_width = painter
        .layout_no_wrap("0000".to_owned(), small.clone(), number)
        .size()
        .x;
    let screen = ctx.content_rect();
    let context_width = list
        .sites
        .iter()
        .filter_map(|site| site.context.as_ref())
        .map(|context| {
            painter
                .layout_no_wrap(format!("in {context}"), hint_font.clone(), tertiary)
                .size()
                .x
        })
        .fold(0.0, f32::max);
    let inner_width = (number_width + widest + context_width + 56.0)
        .clamp(340.0, 600.0)
        .min(screen.width() - 2.0 * CALL_PADDING - 24.0);
    let height = list.sites.len().min(CALL_ROWS as usize) as f32 * CALL_ROW_HEIGHT;

    let mut chosen = list.enter.then_some(list.selected);
    list.enter = false;
    let pointer_moved = ctx.input(|input| input.pointer.delta() != Vec2::ZERO);
    let calls = list.sites.len();
    let shown = egui::Area::new(Id::new("script-call-list"))
        .order(egui::Order::Foreground)
        .fixed_pos(list.at + Vec2::new(0.0, 10.0))
        .show(&ctx, |ui| {
            kit::menu_frame()
                .inner_margin(Margin::same(CALL_PADDING as i8))
                .show(ui, |ui| {
                    ui.set_width(inner_width);
                    ui.spacing_mut().item_spacing = Vec2::ZERO;
                    // Header: whose calls these are, and how many.
                    let (header, _) =
                        ui.allocate_exact_size(Vec2::new(inner_width, 30.0), Sense::hover());
                    let title =
                        ui.painter()
                            .layout_no_wrap(list.name.clone(), ui_font.clone(), primary);
                    let title_width = title.size().x;
                    ui.painter().galley(
                        egui::pos2(
                            header.left() + 8.0,
                            header.center().y - title.size().y / 2.0,
                        ),
                        title,
                        primary,
                    );
                    ui.painter().text(
                        egui::pos2(header.left() + 8.0 + title_width + 8.0, header.center().y),
                        Align2::LEFT_CENTER,
                        format!("{calls} calls"),
                        hint_font.clone(),
                        tertiary,
                    );
                    ui.painter().hline(
                        header.x_range(),
                        header.bottom(),
                        Stroke::new(1.0, theme::separator()),
                    );
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical()
                        .max_height(height)
                        .min_scrolled_height(height.min(CALL_ROWS * CALL_ROW_HEIGHT))
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            for (index, site) in list.sites.iter().enumerate() {
                                let (rect, response) = ui.allocate_exact_size(
                                    Vec2::new(inner_width, CALL_ROW_HEIGHT),
                                    Sense::click(),
                                );
                                if response.hovered() {
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                    if pointer_moved {
                                        list.selected = index;
                                    }
                                }
                                let on = index == list.selected;
                                if on {
                                    ui.painter().rect_filled(rect, 5.0, theme::ink(18));
                                    ui.painter().rect_filled(
                                        Rect::from_min_size(
                                            rect.min + Vec2::new(0.0, 7.0),
                                            Vec2::new(2.0, CALL_ROW_HEIGHT - 14.0),
                                        ),
                                        1.0,
                                        theme::ink(150),
                                    );
                                    if list.scroll_to_selected {
                                        ui.scroll_to_rect(rect, None);
                                    }
                                }
                                ui.painter().text(
                                    egui::pos2(rect.left() + 10.0 + number_width, rect.center().y),
                                    Align2::RIGHT_CENTER,
                                    site.line,
                                    small.clone(),
                                    if on { secondary } else { number },
                                );
                                let characters: Vec<char> = site.preview.chars().collect();
                                let (from, to) = site
                                    .name
                                    .as_ref()
                                    .map_or((characters.len(), characters.len()), |name| {
                                        (name.start, name.end)
                                    });
                                // The code gets the room the function name on the right leaves; a line
                                // that does not fit ends in `…` instead of running under the name.
                                let context_room = site.context.as_ref().map_or(0.0, |context| {
                                    let width = ui
                                        .painter()
                                        .layout_no_wrap(
                                            format!("in {context}"),
                                            hint_font.clone(),
                                            tertiary,
                                        )
                                        .size()
                                        .x;
                                    width + 18.0
                                });
                                let code_left = 22.0 + number_width;
                                let mut job = egui::text::LayoutJob::default();
                                job.wrap.max_width =
                                    (inner_width - code_left - 10.0 - context_room).max(40.0);
                                job.wrap.max_rows = 1;
                                job.wrap.break_anywhere = true;
                                job.wrap.overflow_character = Some('…');
                                // The called name in the strong ink, the rest of the line quieter.
                                for (text, color) in [
                                    (&characters[..from], secondary),
                                    (&characters[from..to], primary),
                                    (&characters[to..], secondary),
                                ] {
                                    let text: String = text.iter().collect();
                                    job.append(
                                        &text,
                                        0.0,
                                        egui::TextFormat::simple(mono.clone(), color),
                                    );
                                }
                                let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
                                ui.painter().galley(
                                    egui::pos2(
                                        rect.left() + code_left,
                                        rect.center().y - galley.size().y / 2.0,
                                    ),
                                    galley,
                                    primary,
                                );
                                if let Some(context) = &site.context {
                                    ui.painter().text(
                                        egui::pos2(rect.right() - 10.0, rect.center().y),
                                        Align2::RIGHT_CENTER,
                                        format!("in {context}"),
                                        hint_font.clone(),
                                        tertiary,
                                    );
                                }
                                if response.clicked() {
                                    chosen = Some(index);
                                }
                            }
                        });
                    ui.add_space(4.0);
                    let (footer, _) =
                        ui.allocate_exact_size(Vec2::new(inner_width, 26.0), Sense::hover());
                    ui.painter().hline(
                        footer.x_range(),
                        footer.top(),
                        Stroke::new(1.0, theme::separator()),
                    );
                    ui.painter().text(
                        egui::pos2(footer.left() + 8.0, footer.center().y),
                        Align2::LEFT_CENTER,
                        "↑↓  navigate      ↵  open      esc  close",
                        hint_font.clone(),
                        tertiary,
                    );
                });
        });
    list.scroll_to_selected = false;
    list.rect = Some(shown.response.rect);
    match chosen.and_then(|index| list.sites.get(index)) {
        Some(site) => state.go_to_range(&ctx, site.range.clone()),
        None => state.nav.calls = Some(list),
    }
}

/// The rename box, under the name being renamed. Returns whether the text changed.
pub(super) fn rename_box(ctx: &egui::Context, state: &mut EditorState) -> bool {
    let Some(mut rename) = state.nav.rename.take() else {
        return false;
    };
    if rename.source_hash != text_hash(&state.source) {
        return false;
    }
    let Some(anchor) = state.nav.anchor else {
        state.nav.rename = Some(rename);
        ctx.request_repaint();
        return false;
    };
    if rename.checked.as_deref() != Some(rename.text.as_str()) {
        let table = state.symbols();
        rename.error = if rename.text == rename.original {
            None
        } else {
            table
                .rename(&state.source, rename.symbol, &rename.text)
                .err()
                .map(|why| why.message())
        };
        rename.checked = Some(rename.text.clone());
    }
    let field = Id::new(RENAME_ID);
    if rename.needs_focus && rename.shown {
        let mut edit_state = TextEditState::load(ctx, field).unwrap_or_default();
        edit_state.cursor.set_char_range(Some(CCursorRange::two(
            CCursor::new(0),
            CCursor::new(rename.text.chars().count()),
        )));
        edit_state.store(ctx, field);
        ctx.memory_mut(|memory| memory.request_focus(field));
        rename.needs_focus = false;
    }
    let mut accept = false;
    let mut cancel = ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape));
    egui::Area::new(Id::new("script-rename"))
        .order(egui::Order::Foreground)
        .fixed_pos(egui::pos2(anchor.left(), anchor.bottom() + 4.0))
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(theme::chrome())
                .stroke(Stroke::new(1.0, theme::separator()))
                .corner_radius(6.0)
                .inner_margin(Margin::same(6))
                .show(ui, |ui| {
                    ui.set_width(240.0);
                    ui.label(
                        RichText::new(format!("Rename {} {}", rename.kind, rename.original))
                            .size(11.0)
                            .color(theme::text_secondary()),
                    );
                    let output = egui::TextEdit::singleline(&mut rename.text)
                        .id(field)
                        .font(editor_appearance::editor_font().code(ctx))
                        .desired_width(f32::INFINITY)
                        .show(ui);
                    let response = output.response;
                    rename.had_focus |= response.has_focus();
                    if response.lost_focus() {
                        let entered = ui.input(|input| input.key_pressed(Key::Enter));
                        if entered {
                            accept = true;
                        } else if rename.had_focus {
                            cancel = true;
                        }
                    }
                    if let Some(error) = &rename.error {
                        ui.label(
                            RichText::new(error)
                                .size(11.0)
                                .color(theme::syntax::string()),
                        );
                    }
                });
        });
    if !rename.shown {
        rename.shown = true;
        ctx.request_repaint();
    }
    if accept && rename.error.is_none() {
        return finish(ctx, state, &rename);
    }
    if accept {
        // Enter on a name that cannot be used: stay open, and keep the keyboard.
        ctx.memory_mut(|memory| memory.request_focus(field));
        state.nav.rename = Some(rename);
        return false;
    }
    if !cancel {
        state.nav.rename = Some(rename);
    } else {
        ctx.memory_mut(|memory| memory.request_focus(state.id));
    }
    false
}

/// Renames, as one undo step, and puts the caret back on the name it was on.
fn finish(ctx: &egui::Context, state: &mut EditorState, rename: &RenameBox) -> bool {
    ctx.memory_mut(|memory| memory.request_focus(state.id));
    if rename.text == rename.original {
        return false;
    }
    let table = state.symbols();
    let Ok(edits) = table.rename(&state.source, rename.symbol, &rename.text) else {
        return false;
    };
    let mut chars: Vec<char> = state.source.chars().collect();
    for (range, text) in edits.iter().rev() {
        chars.splice(range.clone(), text.chars());
    }
    // The caret keeps its place in the name, and moves with every change before it.
    let mut shift = 0isize;
    let mut inside = None;
    for (range, text) in &edits {
        let length = text.chars().count();
        if range.end <= rename.caret {
            shift += length as isize - (range.end - range.start) as isize;
        } else {
            if range.start < rename.caret {
                let offset = (rename.caret - range.start).min(length);
                inside = Some(range.start as isize + shift + offset as isize);
            }
            break;
        }
    }
    let caret = inside.unwrap_or(rename.caret as isize + shift);
    let renamed: String = chars.into_iter().collect();
    let mut edit_state = TextEditState::load(ctx, state.id).unwrap_or_default();
    // A rename is its own undo step, apart from the typing before it.
    let before = edit_state
        .cursor
        .char_range()
        .unwrap_or_else(|| CCursorRange::one(CCursor::new(0)));
    let mut undoer = edit_state.undoer();
    undoer.add_undo(&(before, state.source.clone()));
    state.source = renamed;
    edit_state.set_undoer(undoer);
    edit_state
        .cursor
        .set_char_range(Some(CCursorRange::one(CCursor::new(caret.max(0) as usize))));
    edit_state.store(ctx, state.id);
    state.carets.clear();
    state.completion = None;
    state.completion_wanted = false;
    state.completion_explicit = false;
    ctx.request_repaint();
    true
}
