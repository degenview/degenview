//! What floats over the chart around a drawing: the compact toolbar above the selection and the
//! right-click menus. Both only ask for changes (`DrawingCommand`s and the frame's other
//! outputs); the workspace applies them through its undo stack like any other edit.

use super::edit::{self, Part};
use super::{DrawingFrame, DrawingLayer, Projection};
use crate::model::chart_type::LineDash;
use crate::model::drawing_style::DrawingStyle;
use crate::model::{Caps, Drawing, DrawingCommand, Geometry, Tool};
use crate::ui::drawing_geometry as geo;
use crate::ui::icons::{self, Icon};
use crate::ui::{kit, series_render, shortcut, theme};
use eframe::egui::{
    self, Align2, Color32, Id, Order, Pos2, Rect, Stroke, Ui, color_picker::Alpha, pos2, vec2,
};

/// A step past any stack: bring to front and send to back move by this much and are clamped.
const TO_EXTREME: i32 = 1_000_000;
/// Font sizes the toolbar's size menu offers for a text box.
const FONT_SIZES: [f32; 10] = [10.0, 11.0, 12.0, 14.0, 16.0, 18.0, 20.0, 24.0, 32.0, 48.0];
/// Stroke widths the toolbar's width menu offers.
const WIDTHS: [f32; 9] = [1.0, 2.0, 3.0, 4.0, 6.0, 8.0, 12.0, 20.0, 40.0];

/// The open menu: where it was opened and which drawing (and which part of it) it is about.
/// A right-click on empty chart belongs to the chart's own menu, not this one.
pub(super) struct MenuState {
    at: Pos2,
    target: Target,
}

#[derive(Clone, Copy)]
struct Target {
    id: u64,
    part: Part,
    point: Pos2,
}

/// What can be done to a picture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum ImageOp {
    /// Choose another picture for the drawing.
    Replace,
    /// Bring the drawing back to the shape of its picture.
    ResetShape,
    Opacity(f32),
    ToggleShapeLock,
    Copy,
}

/// The opacities the toolbar offers.
const OPACITIES: [f32; 4] = [1.0, 0.75, 0.5, 0.25];

/// What can be done to the grid of a table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum TableOp {
    /// A row after the selected cell's (at the end with none selected).
    AddRow,
    AddColumn,
    DeleteRow,
    DeleteColumn,
    /// Pins the table to the screen, or back to the chart where it now is.
    TogglePin,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Action {
    Table(TableOp),
    Image(ImageOp),
    Link(super::LinkRequest),
    Settings,
    EditText,
    /// A price alert at a horizontal line's price.
    Alert,
    Copy,
    Duplicate,
    CopyStyle,
    PasteStyle,
    ToggleLock,
    Hide,
    Forward,
    Backward,
    ToFront,
    ToBack,
    AddVertex,
    RemoveVertex(usize),
    Delete,
}

impl DrawingLayer {
    /// Opens the menu for a right-click at `point` when it is on a drawing; on empty chart it
    /// reports the click in `frame.background_click` for the chart's own menu instead.
    pub(super) fn open_menu(&mut self, point: Pos2, frame: &mut DrawingFrame, proj: &Projection) {
        if self.gesture.is_some() || frame.tool != Tool::Cursor {
            return;
        }
        let Some(hit) = self.hit_test(frame, point, proj, true) else {
            frame.background_click = Some(point);
            return;
        };
        if !frame.is_selected(hit.id) {
            frame.select = Some(vec![hit.id]);
        }
        // On a table the cell under the click is the one the menu's row and column entries mean.
        if let Some(drawing) = frame.drawings.iter().find(|drawing| drawing.id() == hit.id)
            && matches!(drawing.geometry(), Some(Geometry::Table(_)))
            && let Some((row, column)) = self
                .screen(drawing, proj)
                .and_then(|screen| screen.grid.as_ref()?.cell_at(point))
        {
            self.table_cell = Some((hit.id, row, column));
        }
        self.menu = Some(MenuState {
            at: point,
            target: Target {
                id: hit.id,
                part: hit.part,
                point,
            },
        });
    }

    /// Duplicates the selection (the keyboard shortcut's way in).
    pub(super) fn duplicate_selection(&mut self, frame: &mut DrawingFrame, proj: &Projection) {
        let ids = frame.selected.clone();
        self.run(Action::Duplicate, &ids, None, frame, proj);
    }

    /// Draws what floats over the chart: the toolbar and the open menu.
    pub(in crate::ui) fn overlay(&mut self, ui: &Ui, proj: &Projection, frame: &mut DrawingFrame) {
        self.start_pending_cell(frame, proj);
        self.show_text_editor(ui, proj, frame);
        self.toolbar(ui, proj, frame);
        self.show_menu(ui, proj, frame);
    }

    // MARK: Toolbar

    /// The screen bounds of a drawing, for placing the toolbar.
    fn bounds_of(&mut self, drawing: &Drawing, proj: &Projection) -> Option<Rect> {
        match drawing {
            Drawing::Brush { .. } | Drawing::Shape { .. } => {
                self.screen(drawing, proj).map(|screen| screen.bounds)
            }
            Drawing::Fib { a, b, .. } => {
                geo::bounds(&[proj.point(*a), proj.point(*b)]).map(|rect| rect.expand(10.0))
            }
            Drawing::Ruler { .. } | Drawing::Unreadable(_) => None,
        }
    }

    fn toolbar(&mut self, ui: &Ui, proj: &Projection, frame: &mut DrawingFrame) {
        if self.gesture.is_some()
            || frame.preview.is_some()
            || frame.selected.is_empty()
            || self.menu.is_some()
            || self.is_editing_text()
        {
            return;
        }
        let selected: Vec<&Drawing> = frame
            .drawings
            .iter()
            .filter(|drawing| frame.is_selected(drawing.id()) && frame.shows(drawing))
            .collect();
        if selected.is_empty() {
            return;
        }
        let mut bounds: Option<Rect> = None;
        for drawing in &selected {
            if let Some(rect) = self.bounds_of(drawing, proj) {
                bounds = Some(bounds.map_or(rect, |all| all.union(rect)));
            }
        }
        // A shape running off the plot keeps its toolbar over the part that is showing.
        let Some(bounds) = bounds
            .map(|rect| rect.intersect(proj.plot))
            .filter(|rect| rect.is_positive())
        else {
            return;
        };
        // Above the selection, or below it when there is no room.
        let above = bounds.top() - 8.0 > proj.plot.top() + 40.0;
        let (anchor, pivot) = if above {
            (
                pos2(bounds.center().x, bounds.top() - 8.0),
                Align2::CENTER_BOTTOM,
            )
        } else {
            (
                pos2(bounds.center().x, bounds.bottom() + 8.0),
                Align2::CENTER_TOP,
            )
        };
        let single = (selected.len() == 1).then(|| selected[0].clone());
        let ids: Vec<u64> = selected.iter().map(|drawing| drawing.id()).collect();
        let all_locked = selected.iter().all(|drawing| drawing.locked());
        let mut action: Option<Action> = None;
        let mut restyled: Option<Drawing> = None;

        egui::Area::new(Id::new("drawing-toolbar"))
            .order(Order::Foreground)
            .pivot(pivot)
            .fixed_pos(anchor)
            .constrain_to(proj.plot.shrink(4.0))
            .interactable(true)
            .show(ui.ctx(), |ui| {
                egui::Frame::new()
                    .fill(theme::with_alpha(theme::surface(), 0.97))
                    .stroke(Stroke::new(1.0, theme::border()))
                    .corner_radius(9.0)
                    .inner_margin(egui::Margin::symmetric(5, 4))
                    .shadow(egui::Shadow {
                        offset: [0, 4],
                        blur: 14,
                        spread: 0,
                        color: Color32::from_black_alpha(70),
                    })
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing = vec2(3.0, 0.0);
                        ui.horizontal(|ui| {
                            // A locked drawing resists edits: only the lock itself, hiding,
                            // copying and deleting are offered.
                            if let Some(drawing) = single.as_ref().filter(|d| !d.locked()) {
                                restyled = style_controls(ui, drawing);
                                if has_style_controls(drawing) {
                                    divider(ui);
                                }
                                if drawing.edits_text_in_place()
                                    && icons::button(
                                        ui,
                                        Icon::TextFormat,
                                        26.0,
                                        theme::text_primary(),
                                        if drawing.text().is_empty() {
                                            "Add text"
                                        } else {
                                            "Edit text"
                                        },
                                    )
                                    .clicked()
                                {
                                    action = Some(Action::EditText);
                                }
                                if let Some(op) = self.table_controls(ui, drawing) {
                                    action = Some(Action::Table(op));
                                }
                                if let Some(op) = image_controls(ui, drawing) {
                                    action = Some(Action::Image(op));
                                }
                                if let Some(request) = link_controls(ui, drawing) {
                                    action = Some(Action::Link(request));
                                }
                                if is_price_line(drawing)
                                    && icons::button(
                                        ui,
                                        Icon::Bell,
                                        26.0,
                                        theme::text_primary(),
                                        "Add alert at this price",
                                    )
                                    .clicked()
                                {
                                    action = Some(Action::Alert);
                                }
                                if icons::button(
                                    ui,
                                    Icon::Sliders,
                                    26.0,
                                    theme::text_primary(),
                                    "Settings",
                                )
                                .clicked()
                                {
                                    action = Some(Action::Settings);
                                }
                            }
                            let lock_icon = if all_locked {
                                Icon::LockFill
                            } else {
                                Icon::LockOpen
                            };
                            let lock_tip = if all_locked { "Unlock" } else { "Lock" };
                            let lock_color = if all_locked {
                                kit::ACCENT
                            } else {
                                theme::text_primary()
                            };
                            if icons::button(ui, lock_icon, 26.0, lock_color, lock_tip).clicked() {
                                action = Some(Action::ToggleLock);
                            }
                            if icons::button(
                                ui,
                                Icon::EyeSlash,
                                26.0,
                                theme::text_primary(),
                                "Hide",
                            )
                            .clicked()
                            {
                                action = Some(Action::Hide);
                            }
                            if icons::button(
                                ui,
                                Icon::CopyDoc,
                                26.0,
                                theme::text_primary(),
                                "Duplicate",
                            )
                            .clicked()
                            {
                                action = Some(Action::Duplicate);
                            }
                            if icons::button(ui, Icon::Trash, 26.0, kit::RED, "Delete").clicked() {
                                action = Some(Action::Delete);
                            }
                        });
                    });
            });

        if let Some(drawing) = restyled {
            frame.commands.push(DrawingCommand::Style {
                id: drawing.id(),
                drawing,
            });
        }
        if let Some(action) = action {
            self.run(action, &ids, None, frame, proj);
        }
    }

    /// The toolbar's controls for a table: rows and columns to add, and (with a cell selected)
    /// to delete, and the pin. Returns the one pressed.
    fn table_controls(&self, ui: &mut Ui, drawing: &Drawing) -> Option<TableOp> {
        let Some(Geometry::Table(table)) = drawing.geometry() else {
            return None;
        };
        let has_cell = self.table_cell.is_some_and(|(id, ..)| id == drawing.id());
        let mut op = None;
        let mut chip =
            |ui: &mut Ui, label: &str, selected: bool, enabled: bool, wanted: TableOp| {
                if enabled && kit::value_chip(ui, label, selected).clicked() {
                    op = Some(wanted);
                }
            };
        chip(
            ui,
            "+ Row",
            false,
            table.row_count() < crate::model::table::MAX_ROWS,
            TableOp::AddRow,
        );
        chip(
            ui,
            "+ Column",
            false,
            table.column_count() < crate::model::table::MAX_COLUMNS,
            TableOp::AddColumn,
        );
        chip(
            ui,
            "− Row",
            false,
            has_cell && table.row_count() > 1,
            TableOp::DeleteRow,
        );
        chip(
            ui,
            "− Column",
            false,
            has_cell && table.column_count() > 1,
            TableOp::DeleteColumn,
        );
        chip(
            ui,
            if table.screen.is_some() {
                "Pinned to screen"
            } else {
                "Pin to screen"
            },
            table.screen.is_some(),
            true,
            TableOp::TogglePin,
        );
        divider(ui);
        op
    }

    /// `drawing`, a picture, after `op`; `None` when it changes nothing or has to be asked of
    /// the workspace instead.
    fn image_edit(
        &mut self,
        drawing: &Drawing,
        op: ImageOp,
        frame: &mut DrawingFrame,
    ) -> Option<Drawing> {
        let Some(Geometry::Image(image)) = drawing.geometry() else {
            return None;
        };
        match op {
            ImageOp::Replace => {
                frame.replace_request = Some(drawing.id());
                return None;
            }
            ImageOp::Copy => {
                frame.copy_image = Some(image.asset.clone());
                return None;
            }
            _ => {}
        }
        let mut changed = drawing.clone();
        let Some(Geometry::Image(edited)) = changed.geometry_mut() else {
            return None;
        };
        match op {
            ImageOp::Opacity(value) => edited.opacity = value,
            ImageOp::ToggleShapeLock => edited.keep_aspect = !edited.keep_aspect,
            ImageOp::ResetShape => {
                // The same width and top-left corner, the height the picture's own shape gives.
                edited.size[1] = edited.size[0] / edited.aspect();
            }
            ImageOp::Replace | ImageOp::Copy => return None,
        }
        (changed != *drawing).then_some(changed)
    }

    /// `drawing`, a table, after `op`; `None` when it changes nothing. A selected cell stays on
    /// the grid it is in.
    fn table_edit(&mut self, drawing: &Drawing, op: TableOp, proj: &Projection) -> Option<Drawing> {
        let cell = self
            .table_cell
            .filter(|(id, ..)| *id == drawing.id())
            .map(|(_, row, column)| (row, column));
        let mut changed = drawing.clone();
        let Some(Geometry::Table(table)) = changed.geometry_mut() else {
            return None;
        };
        let done = match op {
            TableOp::AddRow => table.insert_row(cell.map(|cell| cell.0)),
            TableOp::AddColumn => table.insert_column(cell.map(|cell| cell.1)),
            TableOp::DeleteRow => cell.is_some_and(|cell| table.delete_row(cell.0)),
            TableOp::DeleteColumn => cell.is_some_and(|cell| table.delete_column(cell.1)),
            TableOp::TogglePin => {
                // Where the corner is now stays where it is: only what it is tied to changes.
                let corner = super::table_shapes::origin(table, proj);
                if table.screen.is_some() {
                    table.at = proj.anchor(corner);
                    table.screen = None;
                } else {
                    table.screen = Some(super::table_shapes::pinned_at(corner, proj));
                }
                true
            }
        };
        if let Some((id, row, column)) = self.table_cell.filter(|(id, ..)| *id == drawing.id()) {
            self.table_cell = Some((
                id,
                row.min(table.row_count() - 1),
                column.min(table.column_count() - 1),
            ));
        }
        done.then_some(changed)
    }

    // MARK: Menu

    fn show_menu(&mut self, ui: &Ui, proj: &Projection, frame: &mut DrawingFrame) {
        let Some(state) = self.menu.take() else {
            return;
        };
        let ctx = ui.ctx();
        let (escape, pressed) = ctx.input(|input| {
            (
                input.key_pressed(egui::Key::Escape),
                input
                    .pointer
                    .any_pressed()
                    .then(|| input.pointer.interact_pos())
                    .flatten(),
            )
        });
        let id = state.target.id;
        let ids = if frame.is_selected(id) {
            frame.selected.clone()
        } else {
            vec![id]
        };
        // The drawing may have gone (deleted, undone) since the menu opened.
        if !frame.drawings.iter().any(|drawing| drawing.id() == id) {
            return;
        }
        let mut chosen: Option<Action> = None;
        let area = egui::Area::new(Id::new("drawing-menu"))
            .order(Order::Foreground)
            .fixed_pos(state.at + vec2(4.0, 4.0))
            .constrain(true)
            .show(ctx, |ui| {
                kit::menu_frame().show(ui, |ui| {
                    ui.set_width(220.0);
                    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                    chosen = self.menu_items(ui, state.target, &ids, frame);
                });
            });
        let outside = pressed.is_some_and(|point| !area.response.rect.contains(point));
        if escape {
            frame.escape_consumed = true;
        }
        match chosen {
            Some(action) => {
                let at = Some((state.target.part, state.target.point));
                self.run(action, &ids, at, frame, proj);
            }
            None if escape || outside => {}
            None => self.menu = Some(state),
        }
    }

    /// The menu rows for `target`; returns the one picked.
    fn menu_items(
        &mut self,
        ui: &mut Ui,
        target: Target,
        ids: &[u64],
        frame: &DrawingFrame,
    ) -> Option<Action> {
        let mut chosen = None;
        let mut item = |ui: &mut Ui,
                        icon: Icon,
                        label: &str,
                        hint: Option<String>,
                        enabled: bool,
                        destructive: bool,
                        action: Action| {
            if kit::menu_item_with_shortcut(ui, icon, label, hint.as_deref(), enabled, destructive)
            {
                chosen = Some(action);
            }
        };
        {
            let part = target.part;
            let drawings: Vec<&Drawing> = frame
                .drawings
                .iter()
                .filter(|drawing| ids.contains(&drawing.id()))
                .collect();
            let single = drawings.len() == 1;
            let first = drawings.first().copied();
            let all_locked = drawings.iter().all(|drawing| drawing.locked());
            let editable = drawings.iter().any(|drawing| !drawing.locked());
            let has_style = single && first.is_some_and(|d| d.style().is_some());
            let can_paste = match (&frame.style_clipboard, first) {
                (Some((tool, _)), Some(drawing)) => {
                    editable
                        && drawings.iter().all(|d| d.tool() == Some(*tool))
                        && drawing.style().is_some()
                }
                _ => false,
            };
            let properties = single && first.is_some_and(|d| !d.locked());
            item(
                ui,
                Icon::Sliders,
                "Settings…",
                Some("↩".into()),
                properties,
                false,
                Action::Settings,
            );
            if let Some(drawing) = first.filter(|d| single && is_price_line(d)) {
                item(
                    ui,
                    Icon::Bell,
                    "Add alert at this price",
                    None,
                    !drawing.locked(),
                    false,
                    Action::Alert,
                );
            }
            if matches!(
                first.filter(|_| single).and_then(|d| d.geometry()),
                Some(Geometry::Image(_))
            ) {
                let unlocked = first.is_some_and(|d| !d.locked());
                item(
                    ui,
                    Icon::ImageTool,
                    "Replace image…",
                    None,
                    unlocked,
                    false,
                    Action::Image(ImageOp::Replace),
                );
                item(
                    ui,
                    Icon::ImageTool,
                    "Reset aspect ratio",
                    None,
                    unlocked,
                    false,
                    Action::Image(ImageOp::ResetShape),
                );
                item(
                    ui,
                    Icon::CopyDoc,
                    "Copy image",
                    None,
                    true,
                    false,
                    Action::Image(ImageOp::Copy),
                );
                kit::menu_separator(ui);
            }
            if matches!(
                first.filter(|_| single).and_then(|d| d.geometry()),
                Some(Geometry::Card { .. })
            ) {
                let unlocked = first.is_some_and(|d| !d.locked());
                item(
                    ui,
                    Icon::ArrowUpRightSquare,
                    "Open link",
                    None,
                    true,
                    false,
                    Action::Link(super::LinkRequest::Open),
                );
                item(
                    ui,
                    Icon::CopyDoc,
                    "Copy link",
                    None,
                    true,
                    false,
                    Action::Link(super::LinkRequest::Copy),
                );
                item(
                    ui,
                    Icon::Link,
                    "Replace link…",
                    None,
                    unlocked,
                    false,
                    Action::Link(super::LinkRequest::Replace),
                );
                item(
                    ui,
                    Icon::Cycle,
                    "Fetch preview…",
                    None,
                    unlocked,
                    false,
                    Action::Link(super::LinkRequest::Fetch),
                );
                kit::menu_separator(ui);
            }
            if let Some(Geometry::Table(table)) =
                first.filter(|_| single).and_then(|d| d.geometry())
            {
                let unlocked = first.is_some_and(|d| !d.locked());
                let has_cell = self
                    .table_cell
                    .is_some_and(|(id, ..)| first.is_some_and(|d| d.id() == id));
                item(
                    ui,
                    Icon::Plus,
                    "Add row",
                    None,
                    unlocked && table.row_count() < crate::model::table::MAX_ROWS,
                    false,
                    Action::Table(TableOp::AddRow),
                );
                item(
                    ui,
                    Icon::Plus,
                    "Add column",
                    None,
                    unlocked && table.column_count() < crate::model::table::MAX_COLUMNS,
                    false,
                    Action::Table(TableOp::AddColumn),
                );
                item(
                    ui,
                    Icon::Minus,
                    "Delete row",
                    None,
                    unlocked && has_cell && table.row_count() > 1,
                    false,
                    Action::Table(TableOp::DeleteRow),
                );
                item(
                    ui,
                    Icon::Minus,
                    "Delete column",
                    None,
                    unlocked && has_cell && table.column_count() > 1,
                    false,
                    Action::Table(TableOp::DeleteColumn),
                );
                item(
                    ui,
                    Icon::TableTool,
                    if table.screen.is_some() {
                        "Pin to chart"
                    } else {
                        "Pin to screen"
                    },
                    None,
                    unlocked,
                    false,
                    Action::Table(TableOp::TogglePin),
                );
                kit::menu_separator(ui);
            }
            if let Some(drawing) = first.filter(|d| single && d.edits_text_in_place()) {
                item(
                    ui,
                    Icon::TextFormat,
                    if drawing.text().is_empty() {
                        "Add text"
                    } else {
                        "Edit text"
                    },
                    None,
                    !drawing.locked(),
                    false,
                    Action::EditText,
                );
            }
            item(
                ui,
                Icon::CopyDoc,
                "Copy",
                Some(shortcut::COPY.label()),
                true,
                false,
                Action::Copy,
            );
            item(
                ui,
                Icon::CopyDoc,
                "Duplicate",
                Some(shortcut::DUPLICATE_DRAWING.label()),
                true,
                false,
                Action::Duplicate,
            );
            kit::menu_separator(ui);
            item(
                ui,
                Icon::CopyDoc,
                "Copy style",
                None,
                has_style,
                false,
                Action::CopyStyle,
            );
            item(
                ui,
                Icon::Palette,
                "Paste style",
                None,
                can_paste,
                false,
                Action::PasteStyle,
            );
            kit::menu_separator(ui);
            let (lock_icon, lock_label) = if all_locked {
                (Icon::LockOpen, "Unlock")
            } else {
                (Icon::LockFill, "Lock")
            };
            item(
                ui,
                lock_icon,
                lock_label,
                None,
                true,
                false,
                Action::ToggleLock,
            );
            item(ui, Icon::EyeSlash, "Hide", None, true, false, Action::Hide);
            kit::menu_separator(ui);
            item(
                ui,
                Icon::BringForward,
                "Bring forward",
                None,
                true,
                false,
                Action::Forward,
            );
            item(
                ui,
                Icon::SendBackward,
                "Send backward",
                None,
                true,
                false,
                Action::Backward,
            );
            item(
                ui,
                Icon::BringForward,
                "Bring to front",
                None,
                true,
                false,
                Action::ToFront,
            );
            item(
                ui,
                Icon::SendBackward,
                "Send to back",
                None,
                true,
                false,
                Action::ToBack,
            );
            if single && let Some(drawing) = first {
                let vertices = vertex_kind(drawing);
                if let Some((minimum, count)) = vertices {
                    kit::menu_separator(ui);
                    item(
                        ui,
                        Icon::Plus,
                        "Add vertex here",
                        None,
                        !drawing.locked() && part == Part::Body,
                        false,
                        Action::AddVertex,
                    );
                    let index = match part {
                        Part::Anchor(index) => Some(index),
                        _ => None,
                    };
                    item(
                        ui,
                        Icon::Minus,
                        "Remove vertex",
                        None,
                        !drawing.locked() && index.is_some() && count > minimum,
                        false,
                        Action::RemoveVertex(index.unwrap_or(0)),
                    );
                }
            }
            kit::menu_separator(ui);
            item(
                ui,
                Icon::Trash,
                "Delete",
                Some("⌫".into()),
                editable,
                true,
                Action::Delete,
            );
        }
        chosen
    }

    // MARK: Actions

    /// Turns a picked action into commands and outputs on `frame`.
    pub(super) fn run(
        &mut self,
        action: Action,
        ids: &[u64],
        at: Option<(Part, Pos2)>,
        frame: &mut DrawingFrame,
        proj: &Projection,
    ) {
        let targets: Vec<Drawing> = frame
            .drawings
            .iter()
            .filter(|drawing| ids.contains(&drawing.id()))
            .cloned()
            .collect();
        let replace = |frame: &mut DrawingFrame, drawing: Drawing| {
            frame.commands.push(DrawingCommand::Replace {
                id: drawing.id(),
                drawing,
            });
        };
        match action {
            Action::Settings => {
                if let [drawing] = &targets[..] {
                    frame.open_properties = Some(drawing.id());
                }
            }
            Action::EditText => {
                if let [drawing] = &targets[..] {
                    self.begin_text_edit(drawing.id(), frame, proj);
                }
            }
            Action::Link(request) => {
                if let [drawing] = &targets[..]
                    && matches!(drawing.geometry(), Some(Geometry::Card { .. }))
                    && (!drawing.locked()
                        || matches!(request, super::LinkRequest::Open | super::LinkRequest::Copy))
                {
                    frame.link_request = Some((drawing.id(), request));
                }
            }
            Action::Image(op) => {
                if let [drawing] = &targets[..]
                    && !drawing.locked()
                    && let Some(changed) = self.image_edit(drawing, op, frame)
                {
                    replace(frame, changed);
                }
            }
            Action::Table(op) => {
                if let [drawing] = &targets[..]
                    && !drawing.locked()
                    && let Some(changed) = self.table_edit(drawing, op, proj)
                {
                    replace(frame, changed);
                }
            }
            Action::Alert => {
                if let [drawing] = &targets[..] {
                    frame.alert_price = drawing.anchors().first().map(|anchor| anchor.price);
                }
            }
            Action::Copy => {
                // The workspace owns the clipboard: it copies the selection it holds.
                frame.copy_requested = true;
            }
            Action::Duplicate => {
                let mut created = Vec::new();
                for drawing in targets.iter().filter(|d| !d.is_ruler()) {
                    let id = frame.next_id;
                    frame.next_id += 1;
                    frame
                        .commands
                        .push(DrawingCommand::Add(edit::duplicate(drawing, id, proj)));
                    created.push(id);
                }
                if !created.is_empty() {
                    frame.select = Some(created);
                }
            }
            Action::CopyStyle => {
                if let [drawing] = &targets[..]
                    && let (Some(tool), Some(style)) = (drawing.tool(), drawing.style())
                {
                    frame.style_copied = Some((tool, *style));
                }
            }
            Action::PasteStyle => {
                if let Some((tool, style)) = frame.style_clipboard {
                    for drawing in targets
                        .iter()
                        .filter(|d| !d.locked() && d.tool() == Some(tool))
                    {
                        let mut changed = drawing.clone();
                        changed.set_style(style);
                        frame.commands.push(DrawingCommand::Style {
                            id: changed.id(),
                            drawing: changed,
                        });
                    }
                }
            }
            Action::ToggleLock => {
                let lock = !targets.iter().all(Drawing::locked);
                for drawing in targets.iter().filter(|d| d.locked() != lock) {
                    let mut changed = drawing.clone();
                    changed.set_locked(lock);
                    replace(frame, changed);
                }
            }
            Action::Hide => {
                for drawing in &targets {
                    let mut changed = drawing.clone();
                    changed.set_hidden(true);
                    replace(frame, changed);
                }
                frame.select = Some(Vec::new());
            }
            Action::Forward | Action::Backward | Action::ToFront | Action::ToBack => {
                let step = match action {
                    Action::Forward => 1,
                    Action::Backward => -1,
                    Action::ToFront => TO_EXTREME,
                    _ => -TO_EXTREME,
                };
                // The order a group moves in keeps its members' stacking among themselves: the
                // top one first when going forward by a step or to the back, the bottom one first
                // otherwise.
                let mut ordered: Vec<u64> = frame
                    .drawings
                    .iter()
                    .map(Drawing::id)
                    .filter(|id| ids.contains(id))
                    .collect();
                if matches!(action, Action::Forward | Action::ToBack) {
                    ordered.reverse();
                }
                for id in ordered {
                    frame.commands.push(DrawingCommand::Reorder { id, step });
                }
            }
            Action::AddVertex => {
                if let ([drawing], Some((_, point))) = (&targets[..], at)
                    && let Some(screen) = self.screen(drawing, proj)
                {
                    let path = screen
                        .lines
                        .first()
                        .map(|path| (path.clone(), false))
                        .or_else(|| screen.outlines.first().map(|path| (path.clone(), true)));
                    if let Some((path, closed)) = path
                        && let Some(segment) = nearest_segment(&path, closed, point)
                        && let Some(changed) =
                            edit::with_inserted_vertex(drawing, segment, proj.anchor(point))
                    {
                        replace(frame, changed);
                    }
                }
            }
            Action::RemoveVertex(index) => {
                if let [drawing] = &targets[..]
                    && let Some(changed) = edit::without_vertex(drawing, index)
                {
                    replace(frame, changed);
                }
            }
            Action::Delete => {
                let mut any = false;
                for drawing in targets.iter().filter(|d| !d.locked()) {
                    frame.commands.push(DrawingCommand::Remove(drawing.id()));
                    any = true;
                }
                if any {
                    frame.select = Some(Vec::new());
                }
            }
        }
    }
}

/// The vertex limits of a path or polyline: its minimum and its current count.
/// Whether the drawing marks one price: a horizontal line or ray, or a crossline. An alert needs
/// a price that does not change with time, so a sloped line offers none.
pub(super) fn is_price_line(drawing: &Drawing) -> bool {
    matches!(
        drawing.tool(),
        Some(Tool::HorizontalLine | Tool::HorizontalRay | Tool::Crossline)
    )
}

fn vertex_kind(drawing: &Drawing) -> Option<(usize, usize)> {
    match drawing.geometry()? {
        Geometry::Path { points } => Some((2, points.len())),
        Geometry::Polyline { points, closed } => Some((if *closed { 3 } else { 2 }, points.len())),
        _ => None,
    }
}

/// The index of the segment of a polyline (or closed loop) nearest `point`: the vertex it starts
/// at.
fn nearest_segment(path: &[Pos2], closed: bool, point: Pos2) -> Option<usize> {
    let count = path.len();
    let segments = if closed {
        count
    } else {
        count.saturating_sub(1)
    };
    (0..segments)
        .map(|index| {
            let (a, b) = (path[index], path[(index + 1) % count]);
            (index, geo::distance_to_segment(point, a, b))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(index, _)| index)
}

/// The toolbar's controls for a picture: opacity, the shape lock, replace. Returns the one used.
fn image_controls(ui: &mut Ui, drawing: &Drawing) -> Option<ImageOp> {
    let Some(Geometry::Image(image)) = drawing.geometry() else {
        return None;
    };
    let mut op = None;
    let labels: Vec<String> = OPACITIES
        .iter()
        .map(|value| format!("{:.0}%", value * 100.0))
        .collect();
    let current = OPACITIES
        .iter()
        .enumerate()
        .min_by(|a, b| {
            (a.1 - image.opacity)
                .abs()
                .total_cmp(&(b.1 - image.opacity).abs())
        })
        .map_or(0, |(index, _)| index);
    if let Some(index) = kit::menu_picker(ui, 62.0, &labels, current, true) {
        op = Some(ImageOp::Opacity(OPACITIES[index]));
    }
    if kit::value_chip(ui, "Keep shape", image.keep_aspect).clicked() {
        op = Some(ImageOp::ToggleShapeLock);
    }
    if kit::value_chip(ui, "Reset shape", false).clicked() {
        op = Some(ImageOp::ResetShape);
    }
    if kit::value_chip(ui, "Replace", false).clicked() {
        op = Some(ImageOp::Replace);
    }
    divider(ui);
    op
}

/// The chips of a link card's toolbar. Fetching a preview is offered only while the card has none:
/// once it is loaded there is nothing to ask for.
pub(super) fn link_chips(drawing: &Drawing) -> Vec<(&'static str, super::LinkRequest)> {
    use super::LinkRequest;
    let mut chips = vec![
        ("Open", LinkRequest::Open),
        ("Copy link", LinkRequest::Copy),
        ("Replace", LinkRequest::Replace),
    ];
    if matches!(drawing.geometry(), Some(Geometry::Card { card, .. }) if card.meta.is_none()) {
        chips.push(("Preview", LinkRequest::Fetch));
    }
    chips
}

/// The toolbar's controls for a link card: open, copy, replace, fetch the preview. Returns the
/// one used.
fn link_controls(ui: &mut Ui, drawing: &Drawing) -> Option<super::LinkRequest> {
    if !matches!(drawing.geometry(), Some(Geometry::Card { .. })) {
        return None;
    }
    let mut request = None;
    for (label, asked) in link_chips(drawing) {
        if kit::value_chip(ui, label, false).clicked() {
            request = Some(asked);
        }
    }
    divider(ui);
    request
}

fn divider(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(7.0, 20.0), egui::Sense::hover());
    ui.painter().vline(
        rect.center().x,
        rect.y_range(),
        Stroke::new(1.0, theme::separator()),
    );
}

/// Whether `style_controls` draws anything for `drawing`, so the divider after them has something
/// to divide. A picture or a post has caps (a Content page) but no colours or lines.
pub(super) fn has_style_controls(drawing: &Drawing) -> bool {
    let Some(tool) = drawing.tool() else {
        return false;
    };
    let caps = tool.caps();
    [
        Caps::COLOR,
        Caps::FILL,
        Caps::BOX,
        Caps::AUTO_TEXT,
        Caps::WIDTH,
        Caps::DASH,
        Caps::VWAP,
        Caps::PROFILE,
        Caps::BARS,
    ]
    .into_iter()
    .any(|cap| caps.has(cap))
        || (drawing.edits_text_in_place() && !drawing.text().is_empty())
}

/// The toolbar's style controls for one drawing: colours, thickness and pattern, only those its
/// tool has. Returns the drawing restyled when the user changed something.
fn style_controls(ui: &mut Ui, drawing: &Drawing) -> Option<Drawing> {
    let tool = drawing.tool()?;
    let caps = tool.caps();
    let style = *drawing.style()?;
    let mut next: DrawingStyle = style;
    if caps.has(Caps::COLOR)
        && let Some(picked) = kit::color_well(
            ui,
            vec2(24.0, 20.0),
            "Line color",
            series_render::color(style.stroke.color),
            Alpha::OnlyBlend,
        )
    {
        next.stroke.color = picked.to_srgba_unmultiplied();
        if !caps.has(Caps::WIDTH) && !caps.has(Caps::BOX) {
            next.text.color = next.stroke.color;
        }
        // A wave's labels are in its line's colour until they are given one of their own.
        if caps.has(Caps::WAVE) && style.text.color == style.stroke.color {
            next.text.color = next.stroke.color;
        }
    }
    if caps.has(Caps::FILL)
        && let Some(picked) = kit::color_well(
            ui,
            vec2(24.0, 20.0),
            "Fill color",
            series_render::color(style.fill.color),
            Alpha::OnlyBlend,
        )
    {
        next.fill.color = picked.to_srgba_unmultiplied();
        next.fill.enabled = true;
    }
    if caps.has(Caps::BOX)
        && let Some(picked) = kit::color_well(
            ui,
            vec2(24.0, 20.0),
            "Border color",
            series_render::color(style.border.color),
            Alpha::OnlyBlend,
        )
    {
        next.border.color = picked.to_srgba_unmultiplied();
        next.border.enabled = true;
    }
    // A drawing with text has its colour at hand; the rest of the text's look is in Settings.
    let has_text =
        caps.has(Caps::AUTO_TEXT) || (drawing.edits_text_in_place() && !drawing.text().is_empty());
    if has_text
        && let Some(picked) = kit::color_well(
            ui,
            vec2(24.0, 20.0),
            "Text color",
            series_render::color(style.text.color),
            Alpha::OnlyBlend,
        )
    {
        next.text.color = picked.to_srgba_unmultiplied();
    }
    if caps.has(Caps::BOX) && has_text {
        let labels: Vec<String> = FONT_SIZES.iter().map(|size| format!("{size:.0}")).collect();
        let selected = FONT_SIZES
            .iter()
            .enumerate()
            .min_by(|a, b| {
                (a.1 - style.text.size)
                    .abs()
                    .total_cmp(&(b.1 - style.text.size).abs())
            })
            .map_or(0, |(index, _)| index);
        if let Some(index) = kit::menu_picker(ui, 56.0, &labels, selected, true) {
            next.text.size = FONT_SIZES[index];
        }
        if kit::value_chip(ui, "B", style.text.bold).clicked() {
            next.text.bold = !style.text.bold;
        }
        if kit::value_chip(ui, "I", style.text.italic).clicked() {
            next.text.italic = !style.text.italic;
        }
    }
    if caps.has(Caps::WIDTH) {
        let labels: Vec<String> = WIDTHS
            .iter()
            .map(|width| format!("{width:.0} px"))
            .collect();
        let selected = WIDTHS
            .iter()
            .enumerate()
            .min_by(|a, b| {
                (a.1 - style.stroke.width)
                    .abs()
                    .total_cmp(&(b.1 - style.stroke.width).abs())
            })
            .map_or(0, |(index, _)| index);
        if let Some(index) = kit::menu_picker(ui, 70.0, &labels, selected, true) {
            next.stroke.width = WIDTHS[index];
        }
    }
    if caps.has(Caps::DASH) {
        let labels: Vec<String> = LineDash::ALL
            .iter()
            .map(|dash| dash.label().to_owned())
            .collect();
        let selected = LineDash::ALL
            .iter()
            .position(|dash| *dash == style.stroke.dash)
            .unwrap_or(0);
        if let Some(index) = kit::menu_picker(ui, 78.0, &labels, selected, true) {
            next.stroke.dash = LineDash::ALL[index];
        }
    }
    if caps.has(Caps::WAVE) {
        use crate::model::elliott::Degree;
        let labels: Vec<String> = Degree::ALL
            .iter()
            .map(|degree| format!("{}  {}", degree.label(), degree.sample()))
            .collect();
        let selected = Degree::ALL
            .iter()
            .position(|degree| *degree == style.pattern.degree)
            .unwrap_or(0);
        if let Some(index) = kit::menu_picker(ui, 176.0, &labels, selected, true) {
            next.pattern.degree = Degree::ALL[index];
        }
        if kit::value_chip(ui, "Labels", style.pattern.labels).clicked() {
            next.pattern.labels = !style.pattern.labels;
        }
    }
    if caps.has(Caps::PATTERN) {
        if kit::value_chip(ui, "Names", style.pattern.labels).clicked() {
            next.pattern.labels = !style.pattern.labels;
        }
        if kit::value_chip(ui, "Ratios", style.pattern.ratios).clicked() {
            next.pattern.ratios = !style.pattern.ratios;
        }
    }
    if caps.has(Caps::POSITION) {
        if kit::value_chip(ui, "Compact", style.position.compact).clicked() {
            next.position.compact = !style.position.compact;
        }
        if kit::value_chip(ui, "Always stats", style.position.always_stats).clicked() {
            next.position.always_stats = !style.position.always_stats;
        }
    }
    if caps.has(Caps::VWAP) {
        use crate::model::chart_type::PriceSource;
        let labels: Vec<String> = PriceSource::ALL
            .iter()
            .map(|source| source.label().to_owned())
            .collect();
        let selected = PriceSource::ALL
            .iter()
            .position(|source| *source == style.vwap.source)
            .unwrap_or(0);
        if let Some(index) = kit::menu_picker(ui, 84.0, &labels, selected, true) {
            next.vwap.source = PriceSource::ALL[index];
        }
    }
    if caps.has(Caps::PROFILE) {
        use crate::model::forecast_style::ProfileVolume;
        let labels: Vec<String> = ProfileVolume::ALL
            .iter()
            .map(|volume| volume.label().to_owned())
            .collect();
        let selected = ProfileVolume::ALL
            .iter()
            .position(|volume| *volume == style.profile.volume)
            .unwrap_or(0);
        if let Some(index) = kit::menu_picker(ui, 92.0, &labels, selected, true) {
            next.profile.volume = ProfileVolume::ALL[index];
        }
    }
    // A bars pattern's mode and reflections are its geometry's, not its style's.
    let mut pattern = None;
    if caps.has(Caps::BARS)
        && let Some(Geometry::Bars(current)) = drawing.geometry()
    {
        use crate::model::bars_pattern::BarsMode;
        let mut edited = current.clone();
        let labels: Vec<String> = BarsMode::ALL
            .iter()
            .map(|mode| mode.label().to_owned())
            .collect();
        let selected = BarsMode::ALL
            .iter()
            .position(|mode| *mode == current.mode)
            .unwrap_or(0);
        if let Some(index) = kit::menu_picker(ui, 112.0, &labels, selected, true) {
            edited.mode = BarsMode::ALL[index];
        }
        if kit::value_chip(ui, "Mirrored", current.mirrored).clicked() {
            edited.mirrored = !current.mirrored;
        }
        if kit::value_chip(ui, "Flipped", current.flipped).clicked() {
            edited.flipped = !current.flipped;
        }
        pattern = (edited != *current).then_some(edited);
    }
    (next != style || pattern.is_some()).then(|| {
        let mut changed = drawing.clone();
        changed.set_style(next);
        if let Some(pattern) = pattern
            && let Some(geometry) = changed.geometry_mut()
        {
            *geometry = Geometry::Bars(pattern);
        }
        changed
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_nearest_segment_is_the_one_whose_start_a_new_vertex_follows() {
        let path = [pos2(0.0, 0.0), pos2(100.0, 0.0), pos2(100.0, 100.0)];
        assert_eq!(nearest_segment(&path, false, pos2(50.0, 3.0)), Some(0));
        assert_eq!(nearest_segment(&path, false, pos2(97.0, 60.0)), Some(1));
        // A closed loop also has the segment back to the start.
        assert_eq!(nearest_segment(&path, true, pos2(40.0, 55.0)), Some(2));
        assert_eq!(nearest_segment(&path[..1], false, pos2(0.0, 0.0)), None);
        assert_eq!(nearest_segment(&[], true, pos2(0.0, 0.0)), None);
    }
}
