//! Editing a drawing's text where it sits on the chart. A selected drawing that takes text in
//! place shows a faint "Add text" at the place its text goes; a click there, or on text it already
//! has, opens a small editor on that spot. Return (or leaving the field) keeps the text,
//! Shift+Return adds a line, Escape drops the edit.
//!
//! The text lives in the drawing, so the edit is one `Replace` of that drawing when it ends: one
//! undo step however many keys were pressed, and nothing is written while typing. The field is
//! an ordinary egui text field, so selection, the clipboard and input methods work as in any
//! other, and while it has the keyboard the app's shortcuts leave the keys alone.
//!
//! A tool that is nothing without its text (the Text tool, a note, a callout, a comment) is not
//! added when it is placed: it is held here as a draft and added, with its text, when the edit
//! is kept. Escape, or keeping nothing, leaves no drawing behind, no undo step and no sync entry.

use super::shape::TextPlace;
use super::{DrawingFrame, DrawingLayer, Projection, text};
use crate::model::drawing_style::{HorizontalAlign, TextStyle};
use crate::model::{Drawing, DrawingCommand};
use crate::ui::{kit, series_render, theme};
use eframe::egui::{
    self, Align, Align2, Event, FontId, Id, Key, Margin, Order, Pos2, Stroke, TextEdit, Ui,
};

/// How close (px) a click must be to the text's box to be on it. Almost none: the text sits a few
/// pixels from its line, and a click on the line itself is the line's (a double-click there opens
/// its properties).
const TEXT_SLACK: f32 = 1.0;
/// The width (px) the editor starts at; longer text wraps within it.
const EDITOR_WIDTH: f32 = 200.0;
/// The widest the editor grows to follow a wide text box, in px.
const EDITOR_MAX_WIDTH: f32 = 480.0;

/// The text of one drawing while it is being edited.
pub(super) struct TextEditing {
    pub(super) id: u64,
    /// The drawing, while it is not yet on the chart (see the module note).
    draft: Option<Drawing>,
    buffer: String,
    original: String,
    /// The middle of the text's place when the edit began, where the editor sits.
    anchor: Pos2,
    /// How wide the editor is, in px.
    width: f32,
    /// The field has had the keyboard at least once, so losing it means the user left.
    focused: bool,
}

impl TextEditing {
    /// The draft as it looks with the text typed so far, so a box grows as it is typed into.
    /// `None` once the drawing is on the chart.
    pub(super) fn preview(&self) -> Option<Drawing> {
        let mut drawing = self.draft.clone()?;
        drawing.set_text(&self.buffer);
        Some(drawing)
    }
}

/// The middle of where a drawing's text goes, whether or not any is there yet.
fn place_center(place: TextPlace) -> Pos2 {
    match place {
        TextPlace::Inside(rect) => rect.center(),
        TextPlace::At { point, above } => {
            egui::pos2(point.x, point.y + if above { -14.0 } else { 14.0 })
        }
        TextPlace::Line { from, to } => from.lerp(to, 0.5),
        TextPlace::Column { x, top, bottom } => egui::pos2(x, (top + bottom) / 2.0),
    }
}

/// How wide the editor of a drawing is: its text box's width when it has one, else the default.
fn editor_width(place: TextPlace) -> f32 {
    match place {
        TextPlace::Inside(rect) => rect.width().clamp(EDITOR_WIDTH, EDITOR_MAX_WIDTH),
        _ => EDITOR_WIDTH,
    }
}

impl DrawingLayer {
    /// Starts editing the text of drawing `id`. `false` when it cannot be: it is locked or hidden
    /// or has no text to edit in place.
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
            .filter(|drawing| {
                !drawing.locked() && drawing.edits_text_in_place() && frame.shows(drawing)
            })
        else {
            return false;
        };
        let (anchor, width) = self.edit_place(drawing, proj);
        self.text_edit = Some(TextEditing {
            id,
            draft: None,
            buffer: drawing.text().to_owned(),
            original: drawing.text().to_owned(),
            anchor,
            width,
            focused: false,
        });
        frame.select = Some(vec![id]);
        true
    }

    /// Starts the text of a drawing that is not on the chart yet. It is added, with the text,
    /// when the edit is kept.
    pub(super) fn begin_draft_text_edit(&mut self, drawing: Drawing, proj: &Projection) {
        let (anchor, width) = self.edit_place(&drawing, proj);
        self.text_edit = Some(TextEditing {
            id: drawing.id(),
            buffer: String::new(),
            original: String::new(),
            draft: Some(drawing),
            anchor,
            width,
            focused: false,
        });
    }

    /// Where the editor of `drawing` sits and how wide it is: on the text where it was last laid
    /// out, else on the place the shape reserves for it.
    fn edit_place(&mut self, drawing: &Drawing, proj: &Projection) -> (Pos2, f32) {
        let place = self.screen(drawing, proj).and_then(|screen| screen.text);
        let anchor = self
            .text_boxes
            .get(&drawing.id())
            .map(|found| found.center)
            .or_else(|| place.map(place_center))
            .unwrap_or_else(|| proj.plot.center());
        (anchor, place.map_or(EDITOR_WIDTH, editor_width))
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
                !drawing.locked() && drawing.edits_text_in_place() && frame.shows(drawing)
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
        let stored = frame
            .drawings
            .iter()
            .find(|drawing| drawing.id() == editing.id)
            .filter(|drawing| !drawing.locked() && frame.shows(drawing))
            .cloned();
        let on_chart = stored.is_some();
        let Some(mut drawing) = stored.or_else(|| editing.draft.clone()) else {
            // Deleted, locked or hidden (undone, or by another Mac) since the edit began.
            return;
        };
        let style = drawing
            .style()
            .map_or_else(TextStyle::default, |style| style.text);
        let limit = drawing.max_text();
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
        // The field shows the text as it will be drawn: the same size, weight and colour.
        let mut layouter = |ui: &Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
            let job = text::layout_job(text.as_str(), &style, wrap_width);
            ui.fonts_mut(|fonts| fonts.layout_job(job))
        };
        let halign = match style.align {
            HorizontalAlign::Left => Align::LEFT,
            HorizontalAlign::Center => Align::Center,
            HorizontalAlign::Right => Align::RIGHT,
        };
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
                                .font(FontId::proportional(style.size))
                                .text_color(series_render::color(style.color).to_opaque())
                                .horizontal_align(halign)
                                .layouter(&mut layouter)
                                .desired_width(editing.width)
                                .desired_rows(1)
                                .char_limit(limit)
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
            let kept = editing.buffer.trim_end();
            if !on_chart {
                // A draft is added with its text, or not at all.
                if !kept.trim().is_empty() {
                    drawing.set_text(kept);
                    frame.select = Some(vec![editing.id]);
                    frame.commands.push(DrawingCommand::Add(drawing));
                }
            } else if kept != editing.original {
                drawing.set_text(kept);
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
