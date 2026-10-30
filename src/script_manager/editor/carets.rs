//! The editor's extra carets.
//!
//! egui's text field has one caret, and it stays the editor's *primary* one: egui draws it, moves
//! it for the mouse and keeps it in its own state. The others live here as plain selections. While
//! there are any, the keys that edit or move text are taken before egui sees them and run through
//! [`multi_cursor`] on every caret together; otherwise the single-caret path is untouched.
//!
//! An extra caret is only meaningful for the text it was made in, so `hash` remembers that text
//! and any other change (egui typing, undo, find and replace, rename, a sync) drops the extras.

use super::*;
use crate::script_manager::call_sites;
use crate::script_manager::multi_cursor::{self, Cell, Motion, Sel, Selections};
use eframe::egui::{Galley, PointerButton, Pos2};

/// A modifier-click, seen before egui moved the caret for it.
struct Press {
    pos: Pos2,
    /// ⌘-click (Ctrl off the Mac): a jump to a definition if the click is on a name that has
    /// one; otherwise, like ⌥-click, a new caret.
    command: bool,
    /// Where the primary caret was before the click.
    prior: Sel,
}

#[derive(Default)]
pub(super) struct CaretState {
    pub(super) extra: Vec<Sel>,
    /// Hash of the text the extras belong to.
    hash: u64,
    press: Option<Press>,
    /// A middle-button (or ⇧ + right) press this frame, to start a column selection.
    column_press: Option<Pos2>,
    /// The cell a column selection started at, while the button is down.
    column: Option<Cell>,
    /// A right-click this frame and the selection it found: a click inside that selection keeps
    /// it, so Cut and Copy in the menu have something to act on.
    secondary: Option<(Pos2, Sel)>,
}

impl CaretState {
    pub(super) fn clear(&mut self) {
        self.extra.clear();
        self.column = None;
        self.press = None;
        self.column_press = None;
        self.secondary = None;
    }
}

fn primary_sel(ctx: &egui::Context, id: Id) -> Option<Sel> {
    let range = TextEditState::load(ctx, id)?.cursor.char_range()?;
    Some(Sel::new(range.secondary.index.0, range.primary.index.0))
}

/// Puts `selections` where they live: the primary into egui's state, the rest here.
fn write(ctx: &egui::Context, state: &mut EditorState, selections: &Selections) {
    let primary = selections.primary_sel();
    let mut edit_state = TextEditState::load(ctx, state.id).unwrap_or_default();
    edit_state.cursor.set_char_range(Some(CCursorRange::two(
        CCursor::new(primary.anchor),
        CCursor::new(primary.head),
    )));
    edit_state.store(ctx, state.id);
    state.carets.extra = selections.extras();
    state.carets.hash = text_hash(&state.source);
    ctx.request_repaint();
}

/// What one event does while there are several carets.
enum Reaction {
    /// Not ours: egui (or the single-caret assists) handle it.
    Pass,
    /// Not ours, but it would leave the extras wrong: drop them, then pass it on.
    Collapse,
    /// Ours: the event is taken.
    Take(Update),
}

#[derive(Default)]
struct Update {
    selections: Option<Selections>,
    edited: Option<multi_cursor::Edited>,
    copy: Option<String>,
}

impl Update {
    fn select(selections: Option<Selections>) -> Reaction {
        Reaction::Take(Self {
            selections,
            ..Self::default()
        })
    }

    fn edit(edited: Option<multi_cursor::Edited>) -> Reaction {
        Reaction::Take(Self {
            edited,
            ..Self::default()
        })
    }
}

fn motion_for(key: Key, modifiers: Modifiers, is_mac: bool) -> Option<(Motion, bool)> {
    let extend = modifiers.shift;
    let word = if is_mac {
        modifiers.alt
    } else {
        modifiers.command
    };
    let line = is_mac && modifiers.command;
    let document = modifiers.command;
    let other = if is_mac {
        modifiers.ctrl
    } else {
        modifiers.alt
    };
    if other {
        return None;
    }
    Some((
        match key {
            Key::ArrowLeft if line => Motion::LineStart,
            Key::ArrowRight if line => Motion::LineEnd,
            Key::ArrowLeft if word => Motion::WordLeft,
            Key::ArrowRight if word => Motion::WordRight,
            Key::ArrowLeft => Motion::Left,
            Key::ArrowRight => Motion::Right,
            Key::ArrowUp if is_mac && document => Motion::DocStart,
            Key::ArrowDown if is_mac && document => Motion::DocEnd,
            Key::ArrowUp if !modifiers.command => Motion::Up,
            Key::ArrowDown if !modifiers.command => Motion::Down,
            Key::Home if document => Motion::DocStart,
            Key::End if document => Motion::DocEnd,
            Key::Home => Motion::LineStart,
            Key::End => Motion::LineEnd,
            _ => return None,
        },
        extend,
    ))
}

fn react(text: &str, sels: &Selections, event: &Event, is_mac: bool) -> Reaction {
    let multi = sels.sels.len() > 1;
    match event {
        Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } => {
            let chord = |s: shortcut::Shortcut| s.matches_for(is_mac, *key, *modifiers);
            if chord(shortcut::SELECT_NEXT_OCCURRENCE) {
                return Update::select(multi_cursor::select_next(text, sels));
            }
            if chord(shortcut::SELECT_ALL_OCCURRENCES) {
                return Update::select(multi_cursor::select_all_matches(text, sels));
            }
            if chord(shortcut::ADD_CURSOR_ABOVE) {
                return Update::select(multi_cursor::add_caret_vertically(text, sels, true));
            }
            if chord(shortcut::ADD_CURSOR_BELOW) {
                return Update::select(multi_cursor::add_caret_vertically(text, sels, false));
            }
            if !multi {
                return Reaction::Pass;
            }
            if modifiers.command && matches!(key, Key::Z | Key::Y | Key::A | Key::Slash)
                || modifiers.alt && matches!(key, Key::ArrowUp | Key::ArrowDown)
            {
                return Reaction::Collapse;
            }
            let plain = !modifiers.command && !modifiers.alt && !modifiers.ctrl;
            match key {
                Key::Escape if plain && !modifiers.shift => {
                    Update::select(Some(Selections::single(sels.primary_sel())))
                }
                Key::Enter if plain && !modifiers.shift => {
                    Update::edit(multi_cursor::newline(text, sels))
                }
                Key::Tab if plain && modifiers.shift => {
                    Update::edit(multi_cursor::outdent(text, sels))
                }
                Key::Tab if plain => Update::edit(multi_cursor::indent(text, sels)),
                Key::Backspace if plain => Update::edit(multi_cursor::backspace(text, sels)),
                Key::Delete if plain => Update::edit(multi_cursor::delete(text, sels)),
                _ => match motion_for(*key, *modifiers, is_mac) {
                    Some((motion, extend)) => {
                        Update::select(Some(multi_cursor::moved(text, sels, motion, extend)))
                    }
                    None => Reaction::Pass,
                },
            }
        }
        Event::Text(typed) if multi && !typed.chars().any(char::is_control) => {
            Update::edit(multi_cursor::type_text(text, sels, typed))
        }
        Event::Paste(pasted) if multi => Update::edit(multi_cursor::paste(text, sels, pasted)),
        Event::Copy if multi => match multi_cursor::copy_text(text, sels) {
            Some(copied) => Reaction::Take(Update {
                copy: Some(copied),
                ..Update::default()
            }),
            None => Reaction::Pass,
        },
        Event::Cut if multi => match multi_cursor::copy_text(text, sels) {
            Some(copied) => Reaction::Take(Update {
                copy: Some(copied),
                edited: multi_cursor::cut(text, sels),
                ..Update::default()
            }),
            None => Reaction::Pass,
        },
        _ => Reaction::Pass,
    }
}

/// Before the text field: notes this frame's clicks, then takes the keys that act on every caret.
/// Returns whether the text changed.
pub(super) fn apply(ctx: &egui::Context, state: &mut EditorState, text_area: Rect) -> bool {
    note_pointer(ctx, state, text_area);
    if !state.carets.extra.is_empty() && state.carets.hash != text_hash(&state.source) {
        state.carets.clear();
    }
    let focused = ctx.memory(|memory| memory.has_focus(state.id));
    if !focused && !state.carets.extra.is_empty() {
        // egui drops the focus of a text field on Escape before anything here runs; with several
        // carets Escape means "back to one", so it also keeps the keyboard.
        let held = ctx.memory(|memory| memory.had_focus_last_frame(state.id));
        let escape = held && ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape));
        if escape {
            if let Some(primary) = primary_sel(ctx, state.id) {
                write(ctx, state, &Selections::single(primary));
            }
            ctx.memory_mut(|memory| memory.request_focus(state.id));
            return false;
        }
    }
    if !focused {
        return false;
    }
    let Some(primary) = primary_sel(ctx, state.id) else {
        return false;
    };
    let is_mac = shortcut::is_mac();
    let events = ctx.input(|input| input.events.clone());
    let mut selections = Selections::with_extras(primary, &state.carets.extra);
    let mut text: Option<String> = None;
    let mut remaining = Vec::with_capacity(events.len());
    let mut touched = false;
    let mut copied: Option<String> = None;
    for event in events {
        let current = text.as_deref().unwrap_or(&state.source);
        match react(current, &selections, &event, is_mac) {
            Reaction::Pass => remaining.push(event),
            Reaction::Collapse => {
                selections = Selections::single(selections.primary_sel());
                touched = true;
                remaining.push(event);
            }
            Reaction::Take(update) => {
                touched = true;
                if let Some(next) = update.selections {
                    selections = next;
                }
                if let Some(edited) = update.edited {
                    text = Some(edited.text);
                    selections = edited.selections;
                }
                if update.copy.is_some() {
                    copied = update.copy;
                }
            }
        }
    }
    if !touched {
        return false;
    }
    ctx.input_mut(|input| input.events = remaining);
    if let Some(copied) = copied {
        ctx.copy_text(copied);
    }
    let changed = text.is_some();
    if let Some(text) = text {
        state.source = text;
    }
    write(ctx, state, &selections);
    changed
}

/// Records modifier-clicks and column-select presses (without taking them: egui still needs
/// them), and drops the extras on a plain click.
fn note_pointer(ctx: &egui::Context, state: &mut EditorState, text_area: Rect) {
    let events = ctx.input(|input| input.events.clone());
    for event in &events {
        let Event::PointerButton {
            pos,
            button,
            pressed: true,
            modifiers,
        } = event
        else {
            continue;
        };
        if !text_area.contains(*pos) {
            continue;
        }
        match button {
            PointerButton::Primary if modifiers.alt || modifiers.command => {
                if let Some(prior) = primary_sel(ctx, state.id) {
                    state.carets.press = Some(Press {
                        pos: *pos,
                        command: modifiers.command && !modifiers.alt,
                        prior,
                    });
                }
            }
            PointerButton::Secondary if !modifiers.shift => {
                state.carets.extra.clear();
                state.carets.secondary = primary_sel(ctx, state.id).map(|prior| (*pos, prior));
            }
            PointerButton::Primary if !modifiers.shift => state.carets.extra.clear(),
            PointerButton::Middle => state.carets.column_press = Some(*pos),
            PointerButton::Secondary => state.carets.column_press = Some(*pos),
            PointerButton::Primary => state.carets.extra.clear(),
            _ => {}
        }
    }
}

fn cell_at(source: &str, galley: &Galley, origin: Pos2, char_width: f32, pos: Pos2) -> Cell {
    let index = galley
        .cursor_from_pos(Vec2::new(0.0, pos.y - origin.y))
        .index
        .0;
    Cell {
        line: multi_cursor::cell_of(source, index).line,
        column: ((pos.x - origin.x) / char_width).round().max(0.0) as usize,
    }
}

/// After the text field: finishes this frame's clicks against the laid-out text, and follows a
/// column selection while its button is down.
pub(super) fn resolve(
    ctx: &egui::Context,
    state: &mut EditorState,
    galley: &Galley,
    origin: Pos2,
    char_width: f32,
) {
    if !state.carets.extra.is_empty() && state.carets.hash != text_hash(&state.source) {
        state.carets.clear();
    }
    if let Some(press) = state.carets.press.take() {
        let index = galley
            .cursor_from_pos(press.pos.to_vec2() - origin.to_vec2())
            .index
            .0;
        let definition = press
            .command
            .then(|| state.symbols().definition_of(index))
            .flatten();
        // ⌘-click on a function's or method's declaration goes to where it is called.
        let calls = match definition {
            None if press.command => state
                .symbols()
                .calls_of(index)
                .map(|calls| call_sites::describe(&state.source, &calls))
                .filter(|sites| !sites.is_empty()),
            _ => None,
        };
        match (definition, calls) {
            (Some(range), _) => state.go_to_range(ctx, range),
            (None, Some(sites)) => state.open_call_list(ctx, press.pos, sites),
            (None, None) => {
                state.carets.extra.push(press.prior);
                let selections = Selections::with_extras(
                    primary_sel(ctx, state.id).unwrap_or(press.prior),
                    &state.carets.extra,
                );
                state.carets.extra = selections.extras();
                state.carets.hash = text_hash(&state.source);
            }
        }
    }
    if let Some((pos, prior)) = state.carets.secondary.take() {
        let index = galley
            .cursor_from_pos(pos.to_vec2() - origin.to_vec2())
            .index
            .0;
        let range = prior.range();
        if !range.is_empty() && range.contains(&index) {
            write(ctx, state, &Selections::single(prior));
        }
    }
    if let Some(pos) = state.carets.column_press.take() {
        state.carets.column = Some(cell_at(&state.source, galley, origin, char_width, pos));
    }
    let Some(anchor) = state.carets.column else {
        return;
    };
    let held = ctx.input(|input| {
        input.pointer.button_down(PointerButton::Middle)
            || (input.pointer.button_down(PointerButton::Secondary) && input.modifiers.shift)
    });
    let pointer = ctx.input(|input| input.pointer.latest_pos());
    let Some(pointer) = pointer.filter(|_| held) else {
        state.carets.column = None;
        return;
    };
    let head = cell_at(&state.source, galley, origin, char_width, pointer);
    let selections = multi_cursor::column_select(&state.source, anchor, head);
    write(ctx, state, &selections);
}

/// The extra carets as shapes: selections to go behind the text, carets over it.
pub(super) fn shapes(
    state: &EditorState,
    galley: &Galley,
    origin: Pos2,
    visuals: &egui::Visuals,
    char_width: f32,
) -> (Vec<Shape>, Vec<Shape>) {
    let (mut behind, mut over) = (Vec::new(), Vec::new());
    if state.carets.extra.is_empty() {
        return (behind, over);
    }
    let length = state.source.chars().count();
    let row_of = |index: usize| galley.layout_from_cursor(CCursor::new(index)).row;
    for sel in &state.carets.extra {
        let range = sel.range();
        let (start, end) = (range.start.min(length), range.end.min(length));
        if start < end {
            let (first, last) = (row_of(start), row_of(end));
            for row in first..=last {
                let Some(placed) = galley.rows.get(row) else {
                    continue;
                };
                let left = if row == first {
                    galley.pos_from_cursor(CCursor::new(start)).min.x
                } else {
                    placed.pos.x
                };
                let right = if row == last {
                    galley.pos_from_cursor(CCursor::new(end)).min.x
                } else {
                    placed.pos.x + placed.row.size.x + char_width * 0.5
                };
                let top = placed.pos.y;
                let rect = Rect::from_min_max(
                    egui::pos2(left, top),
                    egui::pos2(right.max(left + 1.0), top + placed.row.size.y),
                )
                .translate(origin.to_vec2());
                behind.push(Shape::rect_filled(rect, 0.0, visuals.selection.bg_fill));
            }
        }
        let head = sel.head.min(length);
        let caret = galley
            .pos_from_cursor(CCursor::new(head))
            .translate(origin.to_vec2());
        over.push(Shape::line_segment(
            [
                egui::pos2(caret.min.x, caret.min.y),
                egui::pos2(caret.min.x, caret.max.y),
            ],
            visuals.text_cursor.stroke,
        ));
    }
    (behind, over)
}

#[cfg(test)]
impl EditorState {
    /// Adds bare carets beyond egui's own, as if made by clicks.
    pub fn set_extra_carets(&mut self, at: &[usize]) {
        self.carets.extra = at.iter().map(|i| Sel::caret(*i)).collect();
        self.carets.hash = text_hash(&self.source);
    }

    /// The carets beyond egui's own.
    pub fn extra_carets(&self) -> Vec<(usize, usize)> {
        self.carets
            .extra
            .iter()
            .map(|sel| (sel.anchor, sel.head))
            .collect()
    }
}
