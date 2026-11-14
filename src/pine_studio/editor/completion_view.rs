//! The suggestion list: the keys it owns, accepting a suggestion, and drawing the list under
//! the caret.

use super::edit_ops::Edit;
use super::{CompletionView, EditorState, completion};
use crate::model::editor_theme::{SurfaceColor, SyntaxRole};
use crate::ui::editor_appearance::{Appearance, EditorFont};
use crate::ui::{shortcut, theme};
use eframe::egui::{
    self, Align2, Color32, Event, FontId, Id, Key, Margin, Rect, Sense, Stroke, Ui, Vec2,
    text::{CCursor, CCursorRange},
    text_edit::TextEditState,
};

/// Suggestion rows visible at once; longer lists scroll.
pub(super) const COMPLETION_ROWS: usize = 10;
/// The width of the suggestion list's rows.
const COMPLETION_WIDTH: f32 = 412.0;
pub(super) const COMPLETION_ROW_HEIGHT: f32 = 22.0;

/// Handles the keys the suggestion list owns while it is open (↑ ↓ Return Tab Esc), and the
/// explicit invocations (`shortcut::COMPLETE`) always. Returns whether the text changed.
pub(super) fn completion_keys(ctx: &egui::Context, state: &mut EditorState) -> bool {
    let focused = ctx.memory(|memory| memory.has_focus(state.id));
    if !focused {
        return false;
    }
    let mut accept = false;
    ctx.input_mut(|input| {
        let events = std::mem::take(&mut input.events);
        for event in events {
            let Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = &event
            else {
                input.events.push(event);
                continue;
            };
            let plain = !modifiers.command && !modifiers.alt && !modifiers.ctrl && !modifiers.shift;
            if shortcut::COMPLETE.matches_for(shortcut::is_mac(), *key, *modifiers) {
                state.completion_wanted = true;
                state.completion_explicit = true;
                continue;
            }
            let Some(view) = state.completion.as_mut().filter(|_| plain) else {
                input.events.push(event);
                continue;
            };
            let count = view.completion.items.len();
            match key {
                Key::ArrowDown => {
                    view.selected = (view.selected + 1) % count;
                    view.scroll_to_selected = true;
                }
                Key::ArrowUp => {
                    view.selected = (view.selected + count - 1) % count;
                    view.scroll_to_selected = true;
                }
                Key::Enter | Key::Tab => accept = true,
                Key::Escape => {
                    state.completion = None;
                    state.completion_wanted = false;
                    state.completion_explicit = false;
                }
                _ => input.events.push(event),
            }
        }
    });
    accept && accept_completion(ctx, state)
}

/// Replaces the typed prefix with the highlighted suggestion. Returns whether the text changed.
pub(super) fn accept_completion(ctx: &egui::Context, state: &mut EditorState) -> bool {
    let Some(view) = state.completion.take() else {
        return false;
    };
    state.completion_wanted = false;
    state.completion_explicit = false;
    let Some(item) = view.completion.items.get(view.selected) else {
        return false;
    };
    let range = view.completion.replace;
    let name_end = range.start + item.label.chars().count();
    // A function gets its call parentheses and the caret between them, unless the name is
    // already followed by a call.
    let call = item.callable && state.source.chars().nth(range.end) != Some('(');
    let edit = Edit {
        range: range.clone(),
        replacement: if call {
            format!("{}()", item.label)
        } else {
            item.label.clone()
        },
        selection: {
            let caret = name_end + usize::from(call);
            caret..caret
        },
    };
    state.source = edit.apply(&state.source);
    if let Some(mut edit_state) = TextEditState::load(ctx, state.id) {
        edit_state
            .cursor
            .set_char_range(Some(CCursorRange::one(CCursor::new(edit.selection.start))));
        edit_state.store(ctx, state.id);
    }
    true
}

/// Recomputes the suggestions after the frame's edits and draws the list under the caret.
/// Returns whether a row click changed the text.
pub(super) fn update_completion(
    ui: &mut Ui,
    state: &mut EditorState,
    look: &Appearance,
    code: &EditorFont,
    edited: bool,
) -> bool {
    let scale = code.chrome_scale();
    let row_height = COMPLETION_ROW_HEIGHT * scale;
    let list_width = COMPLETION_WIDTH * scale;
    let ctx = ui.ctx().clone();
    if !state.carets.extra.is_empty() {
        // Suggestions follow one caret.
        state.completion = None;
        state.completion_wanted = false;
        state.completion_explicit = false;
        return false;
    }
    let focused = ctx.memory(|memory| memory.has_focus(state.id));
    let caret = TextEditState::load(&ctx, state.id)
        .and_then(|edit| edit.cursor.char_range())
        .filter(|range| range.primary == range.secondary)
        .map(|range| range.primary.index.0);
    if edited {
        state.completion_wanted = true;
    } else if caret != state.last_caret && !state.completion_explicit {
        // The caret moved on its own (click, arrows): the suggestions no longer apply.
        state.completion_wanted = false;
    }
    state.last_caret = caret;
    if state.caret_hidden {
        // Scrolled away: nothing to draw, and Enter or Tab must not take an invisible row.
        state.completion = None;
        return false;
    }
    let Some(caret) = caret.filter(|_| focused && state.completion_wanted) else {
        state.completion = None;
        return false;
    };
    let table = state.symbols();
    let fresh = completion::complete_with(&state.source, caret, state.completion_explicit, &table);
    state.completion = match (fresh, state.completion.take()) {
        (Some(next), Some(old)) => {
            let same = old.completion.items == next.items;
            Some(CompletionView {
                selected: if same {
                    old.selected.min(next.items.len() - 1)
                } else {
                    0
                },
                scroll_to_selected: old.scroll_to_selected || !same,
                completion: next,
            })
        }
        (Some(next), None) => Some(CompletionView {
            completion: next,
            selected: 0,
            scroll_to_selected: false,
        }),
        (None, _) => None,
    };
    let (Some(view), Some(caret_rect)) = (state.completion.as_ref(), state.caret_rect) else {
        return false;
    };
    let mut clicked = None;
    let selected = view.selected;
    let scroll_to_selected = view.scroll_to_selected;
    let visible_height = view.completion.items.len().min(COMPLETION_ROWS) as f32 * row_height;
    // Under the caret, or over it when the list would run out of the editor (onto the panel
    // below) and there is room above.
    let list_height = visible_height + 10.0;
    let below = caret_rect.bottom() + 4.0;
    let mut position = egui::pos2(caret_rect.left(), below);
    if let Some(visible) = state.text_visible {
        let above = caret_rect.top() - 4.0 - list_height;
        if below + list_height > visible.bottom() && above >= visible.top() {
            position.y = above;
        }
        position.x = position
            .x
            .min(visible.right() - (list_width + 10.0))
            .max(visible.left());
    }
    egui::Area::new(Id::new("script-completion"))
        .order(egui::Order::Foreground)
        .fixed_pos(position)
        .show(&ctx, |ui| {
            egui::Frame::new()
                .fill(look.surface(SurfaceColor::PopupBackground))
                .stroke(Stroke::new(1.0, theme::separator()))
                .corner_radius(6.0)
                .inner_margin(Margin::same(4))
                .show(ui, |ui| {
                    ui.set_width(list_width);
                    egui::ScrollArea::vertical()
                        .min_scrolled_height(visible_height)
                        .max_height(visible_height)
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            for (index, item) in view.completion.items.iter().enumerate() {
                                let (rect, response) = ui.allocate_exact_size(
                                    Vec2::new(list_width, row_height),
                                    Sense::click(),
                                );
                                let on = index == selected;
                                if on {
                                    ui.painter().rect_filled(
                                        rect,
                                        4.0,
                                        look.surface(SurfaceColor::PopupSelection),
                                    );
                                    if scroll_to_selected {
                                        ui.scroll_to_rect(rect, None);
                                    }
                                } else if response.hovered() {
                                    ui.painter().rect_filled(rect, 4.0, theme::ink(14));
                                }
                                let ink = if on {
                                    Color32::WHITE
                                } else {
                                    look.role(SyntaxRole::Default)
                                };
                                let dim = if on {
                                    Color32::WHITE.gamma_multiply(0.8)
                                } else {
                                    look.surface(SurfaceColor::LineNumber)
                                };
                                let badge = Rect::from_center_size(
                                    egui::pos2(rect.min.x + 14.0, rect.center().y),
                                    Vec2::splat(16.0),
                                );
                                ui.painter().rect_stroke(
                                    badge,
                                    3.0,
                                    Stroke::new(1.0, dim),
                                    egui::StrokeKind::Inside,
                                );
                                ui.painter().text(
                                    badge.center(),
                                    Align2::CENTER_CENTER,
                                    item.kind.badge().to_ascii_uppercase(),
                                    code.at(ui.ctx(), 10.0 * scale),
                                    dim,
                                );
                                ui.painter().text(
                                    egui::pos2(rect.min.x + 30.0, rect.center().y),
                                    Align2::LEFT_CENTER,
                                    &item.label,
                                    code.at(ui.ctx(), 12.0 * scale),
                                    ink,
                                );
                                ui.painter().text(
                                    egui::pos2(rect.max.x - 8.0, rect.center().y),
                                    Align2::RIGHT_CENTER,
                                    item.kind.detail(),
                                    FontId::proportional(11.0),
                                    dim,
                                );
                                if response.clicked() {
                                    clicked = Some(index);
                                }
                            }
                        });
                });
        });
    if let Some(view) = state.completion.as_mut() {
        view.scroll_to_selected = false;
    }
    match clicked {
        Some(index) => {
            if let Some(view) = state.completion.as_mut() {
                view.selected = index;
            }
            accept_completion(&ctx, state)
        }
        None => false,
    }
}
