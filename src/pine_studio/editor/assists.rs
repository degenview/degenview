//! The editing assists: Return, Tab, Backspace, paired typing and the line shortcuts, handled
//! before the text field sees them.

use super::edit_ops::{Context, Edit};
use super::{EditorState, edit_ops};
use crate::ui::shortcut;
use eframe::egui::{
    self, Event, Key, Modifiers,
    text::{CCursor, CCursorRange},
    text_edit::TextEditState,
};

/// Handles Return, Tab, Backspace, paired typing and the line shortcuts before the text field
/// sees them. Returns whether any edit changed the text.
pub(super) fn apply_assists(ctx: &egui::Context, state: &mut EditorState) -> bool {
    let id = state.id;
    if !ctx.memory(|memory| memory.has_focus(id)) {
        return false;
    }
    let Some(mut edit_state) = TextEditState::load(ctx, id) else {
        return false;
    };
    let mut selection = edit_state
        .cursor
        .char_range()
        .map(|range| {
            let (a, b) = (range.primary.index.0, range.secondary.index.0);
            a.min(b)..a.max(b)
        })
        .unwrap_or(0..0);
    let mut changed = false;
    let mut handled_any = false;

    let events = ctx.input(|input| input.events.clone());
    let mut remaining = Vec::with_capacity(events.len());
    for event in events {
        let edit = match &event {
            Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => key_edit(&state.source, selection.clone(), *key, *modifiers),
            Event::Text(text) => {
                edit_ops::typed(&Context::new(&state.source, selection.clone()), text)
            }
            _ => None,
        };
        match edit {
            Some(edit) => {
                state.source = edit.apply(&state.source);
                selection = edit.selection.clone();
                changed = true;
                handled_any = true;
            }
            None => remaining.push(event),
        }
    }
    if handled_any {
        ctx.input_mut(|input| input.events = remaining);
        edit_state.cursor.set_char_range(Some(CCursorRange::two(
            CCursor::new(selection.start),
            CCursor::new(selection.end),
        )));
        edit_state.store(ctx, id);
    }
    changed
}

fn key_edit(
    source: &str,
    selection: std::ops::Range<usize>,
    key: Key,
    modifiers: Modifiers,
) -> Option<Edit> {
    let context = Context::new(source, selection);
    let plain = !modifiers.command && !modifiers.alt && !modifiers.ctrl;
    let is_mac = shortcut::is_mac();
    let chord = |shortcut: shortcut::Shortcut| shortcut.matches_for(is_mac, key, modifiers);
    match key {
        Key::Enter if plain && !modifiers.shift => Some(edit_ops::newline(&context)),
        Key::Tab if plain && !modifiers.shift => Some(edit_ops::indent(&context)),
        Key::Tab if plain && modifiers.shift => edit_ops::outdent(&context),
        Key::Backspace if plain && !modifiers.shift => edit_ops::backspace(&context),
        Key::Slash if chord(shortcut::TOGGLE_COMMENT) => edit_ops::toggle_comment(&context),
        Key::ArrowUp | Key::ArrowDown => {
            let down = key == Key::ArrowDown;
            if chord(shortcut::DUPLICATE_LINE_UP) || chord(shortcut::DUPLICATE_LINE_DOWN) {
                edit_ops::duplicate(&context, down)
            } else if chord(shortcut::MOVE_LINE_UP) || chord(shortcut::MOVE_LINE_DOWN) {
                edit_ops::move_lines(&context, down)
            } else {
                None
            }
        }
        _ => None,
    }
}
