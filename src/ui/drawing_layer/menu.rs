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

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Action {
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
        self.show_text_editor(ui, proj, frame);
        self.toolbar(ui, proj, frame);
        self.show_menu(ui, proj, frame);
    }

    // MARK: Toolbar

    /// The screen bounds of a drawing, for placing the toolbar.
    fn bounds_of(&mut self, drawing: &Drawing, proj: &Projection) -> Option<Rect> {
        match drawing {
            Drawing::Brush { .. } | Drawing::Shape { .. } | Drawing::TrendLine { .. } => {
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
                                if drawing.tool().is_some_and(|tool| !tool.caps().is_empty()) {
                                    divider(ui);
                                }
                                if drawing.takes_text()
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
            if let Some(drawing) = first.filter(|d| single && d.takes_text()) {
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

fn divider(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(7.0, 20.0), egui::Sense::hover());
    ui.painter().vline(
        rect.center().x,
        rect.y_range(),
        Stroke::new(1.0, theme::separator()),
    );
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
        if !caps.has(Caps::WIDTH) {
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
    // A drawing with text has its colour at hand; the rest of the text's look is in Settings.
    if caps.has(Caps::TEXT)
        && caps.has(Caps::WIDTH)
        && !drawing.text().is_empty()
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
    (next != style).then(|| {
        let mut changed = drawing.clone();
        changed.set_style(next);
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
