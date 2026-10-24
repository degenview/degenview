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
use crate::model::table::MAX_CELL_CHARS;
use crate::model::{Caps, Drawing, DrawingCommand, Geometry};
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
/// The narrowest and widest the editor of a table's cell is: a narrow column still gets room to
/// type in.
const TABLE_EDITOR_WIDTH: (f32, f32) = (140.0, 480.0);

/// The cell after (or before) `row`, `column` in reading order, wrapping round at either end.
fn neighbour_cell(
    row: usize,
    column: usize,
    rows: usize,
    columns: usize,
    backwards: bool,
) -> (usize, usize) {
    let total = rows * columns;
    let index = row * columns + column;
    let next = if backwards {
        (index + total - 1) % total
    } else {
        (index + 1) % total
    };
    (next / columns, next % columns)
}

/// The text of one drawing while it is being edited.
pub(super) struct TextEditing {
    pub(super) id: u64,
    /// The cell of a table being edited, as `(row, column)`; `None` for a drawing's own text.
    cell: Option<(usize, usize)>,
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
        // A pin's note is edited where it opens, so it opens.
        if drawing
            .tool()
            .is_some_and(|tool| tool.caps().has(Caps::COLLAPSIBLE))
        {
            self.open_notes.insert(id);
        }
        self.text_edit = Some(TextEditing {
            id,
            cell: None,
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
            cell: None,
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

    /// Starts editing one cell of the table `id`, with `typed` (what the user began typing with a
    /// cell selected) added to what is there. `false` when it cannot be: the drawing is locked,
    /// hidden or no table, or the cell is not in it.
    pub(super) fn begin_cell_edit(
        &mut self,
        id: u64,
        row: usize,
        column: usize,
        typed: Option<&str>,
        frame: &mut DrawingFrame,
        proj: &Projection,
    ) -> bool {
        let Some(drawing) = frame
            .drawings
            .iter()
            .find(|drawing| drawing.id() == id)
            .filter(|drawing| !drawing.locked() && frame.shows(drawing))
        else {
            return false;
        };
        let Some(Geometry::Table(table)) = drawing.geometry() else {
            return false;
        };
        if row >= table.row_count() || column >= table.column_count() {
            return false;
        }
        let original = table.cell(row, column).to_owned();
        let Some(rect) = self
            .screen(drawing, proj)
            .and_then(|screen| screen.grid.as_ref()?.cell_rect(row, column))
        else {
            return false;
        };
        let mut buffer = original.clone();
        buffer.push_str(typed.unwrap_or(""));
        self.table_cell = Some((id, row, column));
        self.text_edit = Some(TextEditing {
            id,
            cell: Some((row, column)),
            draft: None,
            buffer,
            original,
            anchor: rect.center(),
            width: rect
                .width()
                .clamp(TABLE_EDITOR_WIDTH.0, TABLE_EDITOR_WIDTH.1),
            focused: false,
        });
        frame.select = Some(vec![id]);
        true
    }

    /// The cell of the one selected table that `point` is on, as `(drawing, row, column)`.
    pub(super) fn table_cell_at(
        &mut self,
        frame: &DrawingFrame,
        point: Pos2,
        proj: &Projection,
    ) -> Option<(u64, usize, usize)> {
        let [id] = frame.selected[..] else {
            return None;
        };
        let drawing = frame
            .drawings
            .iter()
            .find(|drawing| drawing.id() == id)
            .filter(|drawing| !drawing.locked() && frame.shows(drawing))?;
        if !matches!(drawing.geometry(), Some(Geometry::Table(_))) {
            return None;
        }
        let (row, column) = self.screen(drawing, proj)?.grid.as_ref()?.cell_at(point)?;
        Some((id, row, column))
    }

    /// Starts the first cell of a table that has just been placed, once it is on the chart.
    pub(super) fn start_pending_cell(&mut self, frame: &mut DrawingFrame, proj: &Projection) {
        let Some((id, waited)) = self.pending_cell.take() else {
            return;
        };
        if frame.drawings.iter().any(|drawing| drawing.id() == id) {
            self.begin_cell_edit(id, 0, 0, None, frame, proj);
        } else if waited < 5 {
            self.pending_cell = Some((id, waited + 1));
        }
    }

    /// What the keyboard does to the selected cell of a table while it is not being edited:
    /// Return edits it, Tab and the arrows move to another, Delete clears it, and typing starts
    /// editing it with what was typed.
    pub(super) fn handle_table_keys(
        &mut self,
        ui: &Ui,
        frame: &mut DrawingFrame,
        proj: &Projection,
    ) {
        let Some((id, row, column)) = self.table_cell else {
            return;
        };
        let ctx = ui.ctx();
        // A text field, an open menu or a popup has the keyboard: it is theirs.
        if self.gesture.is_some()
            || self.is_editing_text()
            || frame.keys_blocked
            || self.menu.is_some()
            || ctx.text_edit_focused()
            || egui::Popup::is_any_open(ctx)
        {
            return;
        }
        // egui's own Tab and arrow navigation may have focused some widget this very frame (the
        // plot, a button of the toolbar). With a cell selected the keys are the cell's, so that
        // focus is let go: it would otherwise take the app's shortcuts with it.
        ctx.memory_mut(|memory| {
            if let Some(focused) = memory.focused() {
                memory.surrender_focus(focused);
            }
        });
        let Some(table) = frame
            .drawings
            .iter()
            .find(|drawing| drawing.id() == id && !drawing.locked() && frame.shows(drawing))
            .and_then(|drawing| match drawing.geometry() {
                Some(Geometry::Table(table)) => Some(table.clone()),
                _ => None,
            })
        else {
            self.table_cell = None;
            return;
        };
        enum Step {
            Edit(Option<String>),
            Move(isize, isize),
            Tab(bool),
            Clear,
            Copy { cut: bool },
            Paste(String),
        }
        let step = ui.ctx().input_mut(|input| {
            let typed: String = input
                .events
                .iter()
                .filter_map(|event| match event {
                    Event::Text(text) if !text.chars().all(char::is_control) => Some(text.as_str()),
                    _ => None,
                })
                .collect();
            if !typed.is_empty() {
                input
                    .events
                    .retain(|event| !matches!(event, Event::Text(_)));
                return Some(Step::Edit(Some(typed)));
            }
            // The platform reports the clipboard keys as events. They belong to the cell while
            // one is selected, so they are taken here.
            let clipboard = input.events.iter().find_map(|event| match event {
                Event::Copy => Some(Step::Copy { cut: false }),
                Event::Cut => Some(Step::Copy { cut: true }),
                Event::Paste(text) => Some(Step::Paste(text.clone())),
                _ => None,
            });
            if clipboard.is_some() {
                input
                    .events
                    .retain(|event| !matches!(event, Event::Copy | Event::Cut | Event::Paste(_)));
                return clipboard;
            }
            let none = egui::Modifiers::NONE;
            if input.consume_key(none, Key::Enter) {
                Some(Step::Edit(None))
            } else if input.consume_key(egui::Modifiers::SHIFT, Key::Tab) {
                Some(Step::Tab(true))
            } else if input.consume_key(none, Key::Tab) {
                Some(Step::Tab(false))
            } else if input.consume_key(none, Key::ArrowUp) {
                Some(Step::Move(-1, 0))
            } else if input.consume_key(none, Key::ArrowDown) {
                Some(Step::Move(1, 0))
            } else if input.consume_key(none, Key::ArrowLeft) {
                Some(Step::Move(0, -1))
            } else if input.consume_key(none, Key::ArrowRight) {
                Some(Step::Move(0, 1))
            } else if input.consume_key(none, Key::Delete)
                || input.consume_key(none, Key::Backspace)
            {
                Some(Step::Clear)
            } else {
                None
            }
        });
        let (rows, columns) = (table.row_count(), table.column_count());
        match step {
            None => {}
            Some(Step::Edit(typed)) => {
                self.begin_cell_edit(id, row, column, typed.as_deref(), frame, proj);
            }
            Some(Step::Move(dy, dx)) => {
                let row = row.saturating_add_signed(dy).min(rows - 1);
                let column = column.saturating_add_signed(dx).min(columns - 1);
                self.table_cell = Some((id, row, column));
            }
            Some(Step::Tab(backwards)) => {
                let (row, column) = neighbour_cell(row, column, rows, columns, backwards);
                self.table_cell = Some((id, row, column));
            }
            Some(Step::Clear) => {
                self.replace_table(frame, id, |table| table.set_cell(row, column, ""));
            }
            Some(Step::Copy { cut }) => {
                ui.ctx().copy_text(table.cell(row, column).to_owned());
                if cut {
                    self.replace_table(frame, id, |table| table.set_cell(row, column, ""));
                }
            }
            Some(Step::Paste(text)) => {
                self.replace_table(frame, id, |table| table.paste(row, column, &text) > 0);
            }
        }
    }

    /// Replaces the table `id` with itself after `change`, when that reports a change and the
    /// table is not locked or hidden.
    fn replace_table(
        &mut self,
        frame: &mut DrawingFrame,
        id: u64,
        change: impl FnOnce(&mut crate::model::table::TableData) -> bool,
    ) {
        let Some(drawing) = frame
            .drawings
            .iter()
            .find(|drawing| drawing.id() == id && !drawing.locked() && frame.shows(drawing))
        else {
            return;
        };
        let mut changed = drawing.clone();
        if let Some(Geometry::Table(table)) = changed.geometry_mut()
            && change(table)
            && changed != *drawing
        {
            frame.commands.push(DrawingCommand::Replace {
                id,
                drawing: changed,
            });
        }
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
        let limit = if editing.cell.is_some() {
            MAX_CELL_CHARS
        } else {
            drawing.max_text()
        };
        let ctx = ui.ctx();
        // The keys that end the edit are taken first, so the field never sees them. Shift+Return
        // is not one: it adds a line. Tab ends a table cell's edit and starts the next cell's
        // (Shift+Tab the one before).
        let in_table = editing.cell.is_some();
        let (keep, tab, cancel) = ctx.input_mut(|input| {
            let (mut keep, mut tab) = (false, None);
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
                Event::Key {
                    key: Key::Tab,
                    pressed: true,
                    modifiers,
                    ..
                } if in_table => {
                    tab = Some(modifiers.shift);
                    false
                }
                _ => true,
            });
            (keep, tab, input.key_pressed(Key::Escape))
        });
        let field = Id::new(("drawing-text-field", editing.id));
        let mut left = false;
        // A field that starts with text in it has the caret after it, to add to it.
        if !editing.focused {
            let mut state = egui::text_edit::TextEditState::load(ctx, field).unwrap_or_default();
            let end = editing.buffer.chars().count();
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::one(
                    egui::text::CCursor::new(end),
                )));
            state.store(ctx, field);
        }
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
        if keep || left || tab.is_some() {
            let kept = editing.buffer.trim_end();
            if let Some((row, column)) = editing.cell {
                // A cell is one edit of the table: one undo step, and only if it changed.
                if kept != editing.original && drawing.set_table_cell(row, column, kept) {
                    frame.commands.push(DrawingCommand::Replace {
                        id: editing.id,
                        drawing,
                    });
                }
                self.table_cell = Some((editing.id, row, column));
                if let Some(backwards) = tab
                    && let Some(Geometry::Table(table)) = stored_table(frame, editing.id)
                {
                    let (next_row, next_column) = neighbour_cell(
                        row,
                        column,
                        table.row_count(),
                        table.column_count(),
                        backwards,
                    );
                    self.begin_cell_edit(editing.id, next_row, next_column, None, frame, proj);
                }
            } else if !on_chart {
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

/// The geometry of the drawing `id` as it stands on the chart.
fn stored_table<'a>(frame: &'a DrawingFrame<'_>, id: u64) -> Option<&'a Geometry> {
    frame
        .drawings
        .iter()
        .find(|drawing| drawing.id() == id)?
        .geometry()
}
