//! Editing a drawing's text where it sits on the chart. A selected line that can carry text shows
//! a faint "Add text" at the place its text goes; a click there, or on text it already has, opens
//! a small editor on that spot. Return (or leaving the field) keeps the text, Shift+Return adds a
//! line, Escape drops the edit.
//!
//! The text lives in the drawing, so the edit is one `Replace` of that drawing when it ends: one
//! undo step however many keys were pressed, and nothing is written while typing. The field is
//! an ordinary egui text field, so selection, the clipboard and input methods work as in any
//! other, and while it has the keyboard the app's shortcuts leave the keys alone.

use super::{DrawingFrame, DrawingLayer, Projection};
use crate::model::{DrawingCommand, MAX_TEXT_CHARS};
use crate::ui::{kit, theme};
use eframe::egui::{
    self, Align2, Event, FontId, Id, Key, Margin, Order, Pos2, Stroke, TextEdit, Ui,
};

/// How close (px) a click must be to the text's box to be on it. Almost none: the text sits a few
/// pixels from its line, and a click on the line itself is the line's (a double-click there opens
/// its properties).
const TEXT_SLACK: f32 = 1.0;
/// The width (px) the editor starts at; longer text wraps within it.
const EDITOR_WIDTH: f32 = 200.0;

/// The text of one drawing while it is being edited.
pub(super) struct TextEditing {
    pub(super) id: u64,
    buffer: String,
    original: String,
    /// The middle of the text's place when the edit began, where the editor sits.
    anchor: Pos2,
    /// The field has had the keyboard at least once, so losing it means the user left.
    focused: bool,
}

impl DrawingLayer {
    /// Starts editing the text of drawing `id`. `false` when it cannot be: it is locked or hidden
    /// or has no text to edit.
    pub(super) fn begin_text_edit(
        &mut self,
        id: u64,
        frame: &mut DrawingFrame,
        proj: &Projection,
    ) -> bool {
        let Some(drawing) = frame
            .drawings
            .iter()
            .find(|drawing| drawing.id() == id)
            .filter(|drawing| !drawing.locked() && drawing.text_on_line() && frame.shows(drawing))
        else {
            return false;
        };
        let anchor = self
            .text_boxes
            .get(&id)
            .map_or_else(|| proj.plot.center(), |found| found.center);
        self.text_edit = Some(TextEditing {
            id,
            buffer: drawing.text().to_owned(),
            original: drawing.text().to_owned(),
            anchor,
            focused: false,
        });
        frame.select = Some(vec![id]);
        true
    }

    /// Whether a text is being edited.
    pub(super) fn is_editing_text(&self) -> bool {
        self.text_edit.is_some()
    }

    /// The selected drawing whose text place (its text, or the invitation to add some) is under
    /// `point`. Only a single selected drawing offers it, and only an unlocked one.
    pub(super) fn text_at(&self, frame: &DrawingFrame, point: Pos2) -> Option<u64> {
        let [id] = frame.selected[..] else {
            return None;
        };
        frame
            .drawings
            .iter()
            .find(|drawing| drawing.id() == id)
            .filter(|drawing| {
                !drawing.locked() && drawing.text_on_line() && frame.shows(drawing)
            })?;
        self.text_boxes
            .get(&id)?
            .contains(point, TEXT_SLACK)
            .then_some(id)
    }

    /// The editor, while a text is being edited. Ends the edit when the text is kept, dropped, or
    /// the drawing is gone.
    pub(super) fn show_text_editor(
        &mut self,
        ui: &Ui,
        proj: &Projection,
        frame: &mut DrawingFrame,
    ) {
        let Some(mut editing) = self.text_edit.take() else {
            return;
        };
        let Some(mut drawing) = frame
            .drawings
            .iter()
            .find(|drawing| drawing.id() == editing.id)
            .filter(|drawing| !drawing.locked() && frame.shows(drawing))
            .cloned()
        else {
            // Deleted, locked or hidden (undone, or by another Mac) since the edit began.
            return;
        };
        let size = drawing.style().map_or(14.0, |style| style.text.size);
        let ctx = ui.ctx();
        // The keys that end the edit are taken first, so the field never sees them. Shift+Return
        // is not one: it adds a line.
        let (keep, cancel) = ctx.input_mut(|input| {
            let mut keep = false;
            input.events.retain(|event| match event {
                Event::Key {
                    key: Key::Enter,
                    pressed: true,
                    modifiers,
                    ..
                } if !modifiers.shift => {
                    keep = true;
                    false
                }
                _ => true,
            });
            (keep, input.key_pressed(Key::Escape))
        });
        let field = Id::new(("drawing-text-field", editing.id));
        let mut left = false;
        egui::Area::new(Id::new("drawing-text-editor"))
            .order(Order::Foreground)
            .pivot(Align2::CENTER_CENTER)
            .fixed_pos(editing.anchor)
            .constrain_to(proj.plot.shrink(4.0))
            .interactable(true)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(theme::surface())
                    .stroke(Stroke::new(1.0, kit::ACCENT))
                    .corner_radius(6.0)
                    .inner_margin(Margin::symmetric(8, 5))
                    .shadow(egui::Shadow {
                        offset: [0, 4],
                        blur: 14,
                        spread: 0,
                        color: egui::Color32::from_black_alpha(70),
                    })
                    .show(ui, |ui| {
                        let response = ui.add(
                            TextEdit::multiline(&mut editing.buffer)
                                .id(field)
                                .font(FontId::proportional(size))
                                .desired_width(EDITOR_WIDTH)
                                .desired_rows(1)
                                .char_limit(MAX_TEXT_CHARS)
                                .frame(egui::Frame::NONE)
                                .hint_text("Add text"),
                        );
                        if !editing.focused {
                            response.request_focus();
                        }
                        if response.has_focus() {
                            editing.focused = true;
                        }
                        left = editing.focused && response.lost_focus();
                    });
            });
        if cancel {
            // Escape is spent on this: it must not also deselect the drawing.
            frame.escape_consumed = true;
            return;
        }
        if keep || left {
            let text = editing.buffer.trim_end();
            if text != editing.original {
                drawing.set_text(text);
                frame.commands.push(DrawingCommand::Replace {
                    id: editing.id,
                    drawing,
                });
            }
            return;
        }
        self.text_edit = Some(editing);
    }
}
