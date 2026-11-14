//! Breakpoints in the gutter: their markers, the arrow at the line a script is stopped at, and
//! the clicks and menu that set and change them.

use super::{CHANGE_HIT, MARKER_RADIUS, MARKER_X};
use crate::pine_studio::debug::marks::{BreakpointMark, GutterEvent, MarkKind, StopMark};
use crate::pine_studio::editor::{EditorState, GutterRows};
use crate::ui::icons::Icon;
use crate::ui::{accessibility, kit, theme};
use eframe::egui::{self, Color32, Rect, Sense, Shape, Stroke, Ui, Vec2};

pub(in crate::pine_studio::editor) fn paint_breakpoint(
    painter: &egui::Painter,
    center: egui::Pos2,
    mark: &BreakpointMark,
    r: f32,
) {
    let live = mark.enabled && !mark.unverified;
    let color = if live {
        theme::editor_breakpoint()
    } else {
        theme::editor_breakpoint_muted()
    };
    match mark.kind {
        MarkKind::Logpoint => {
            let points = vec![
                center + Vec2::new(0.0, -r - 0.8),
                center + Vec2::new(r + 0.8, 0.0),
                center + Vec2::new(0.0, r + 0.8),
                center + Vec2::new(-r - 0.8, 0.0),
            ];
            if live {
                painter.add(Shape::convex_polygon(points, color, Stroke::NONE));
            } else {
                painter.add(Shape::closed_line(points, Stroke::new(1.4, color)));
            }
        }
        MarkKind::Line | MarkKind::Conditional => {
            if live {
                painter.circle_filled(center, r, color);
            } else {
                painter.circle_stroke(center, r - 0.6, Stroke::new(1.4, color));
            }
            if mark.kind == MarkKind::Conditional {
                // An equals sign cut into the dot: it stops only when something holds.
                let ink = if live { Color32::WHITE } else { color };
                let (half, dy) = (r * 0.48, r * 0.28);
                for dy in [-dy, dy] {
                    painter.line_segment(
                        [center + Vec2::new(-half, dy), center + Vec2::new(half, dy)],
                        Stroke::new(1.3 * r / MARKER_RADIUS, ink),
                    );
                }
            }
        }
    }
}

/// The marker for the line a stopped script is at. `carrying` is the row height when a breakpoint
/// shares the line: the arrow is then larger (as far as the row allows) and the breakpoint is drawn
/// inside its body, so neither hides the other.
pub(in crate::pine_studio::editor) fn paint_stop_arrow(
    painter: &egui::Painter,
    center: egui::Pos2,
    stop: StopMark,
    carrying: Option<f32>,
) {
    let color = if stop.error {
        theme::editor_breakpoint()
    } else {
        theme::editor_debug_line()
    };
    let points = match carrying {
        Some(row) => {
            let k = ((row - 3.0) / 13.0).clamp(0.5, 1.0);
            [
                (-7.0, -6.5),
                (1.5, -6.5),
                (8.5, 0.0),
                (1.5, 6.5),
                (-7.0, 6.5),
            ]
            .map(|(x, y)| center + Vec2::new(x, y * k))
            .to_vec()
        }
        None => vec![
            center + Vec2::new(-5.6, -3.8),
            center + Vec2::new(0.6, -3.8),
            center + Vec2::new(5.8, 0.0),
            center + Vec2::new(0.6, 3.8),
            center + Vec2::new(-5.6, 3.8),
        ],
    };
    if stop.innermost || stop.error {
        painter.add(Shape::convex_polygon(points, color, Stroke::NONE));
    } else {
        // A caller's line: the same arrow, outlined and filled with the gutter colour.
        painter.add(Shape::convex_polygon(
            points,
            theme::gutter_background(),
            Stroke::new(1.5, color),
        ));
    }
}

/// The breakpoint column: a click on a line (its number or the marker beside it) sets or clears its
/// breakpoint, and a right-click opens that breakpoint's menu. Painted over by `paint_gutter`.
pub(in crate::pine_studio::editor) fn breakpoint_gutter(
    ui: &mut Ui,
    gutter: Rect,
    rows: &GutterRows,
    state: &mut EditorState,
) -> Vec<GutterEvent> {
    let hit = Rect::from_min_max(
        egui::pos2(gutter.min.x + CHANGE_HIT, gutter.min.y),
        gutter.max,
    );
    let response = ui.interact(hit, state.id.with("breakpoints"), Sense::click());
    let response = accessibility::label(
        response,
        "Breakpoint gutter: click a line to set a breakpoint",
    );
    let row_at = |y: f32| {
        let index = rows.rows.partition_point(|(top, height)| top + height <= y);
        rows.rows
            .get(index)
            .filter(|(top, _)| *top <= y)
            .map(|_| index + 1)
    };
    let mut events = Vec::new();
    if let Some(pointer) = response.hover_pos()
        && let Some(line) = row_at(pointer.y)
    {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        if state.debug.breakpoint_at(line).is_none() {
            let (top, height) = rows.rows[line - 1];
            // A faint dot where a click would put one.
            ui.painter().with_clip_rect(gutter).circle_filled(
                egui::pos2(gutter.min.x + MARKER_X, top + height / 2.0),
                MARKER_RADIUS,
                theme::with_alpha(theme::editor_breakpoint(), 0.32),
            );
        }
    }
    let clicked_line = |position: Option<egui::Pos2>| position.and_then(|p| row_at(p.y));
    if response.clicked()
        && let Some(line) = clicked_line(response.interact_pointer_pos())
    {
        events.push(GutterEvent::Toggle { line });
    }
    if response.secondary_clicked()
        && let Some(line) = clicked_line(response.interact_pointer_pos())
    {
        state.menu_line = Some(line);
    }
    if let Some(line) = state.menu_line {
        let mark = state.debug.breakpoint_at(line).copied();
        egui::Popup::context_menu(&response)
            .frame(kit::menu_frame())
            .show(|ui| {
                kit::menu_fit(
                    ui,
                    if mark.is_some() {
                        &[
                            "Edit Breakpoint…",
                            "Disable Breakpoint",
                            "Remove Breakpoint",
                        ]
                    } else {
                        &[
                            "Add Breakpoint",
                            "Add Conditional Breakpoint…",
                            "Add Logpoint…",
                        ]
                    },
                );
                ui.spacing_mut().item_spacing.y = 0.0;
                kit::menu_heading(ui, &format!("Line {line}"));
                match mark {
                    Some(mark) => {
                        if kit::menu_item(ui, Icon::Pencil, "Edit Breakpoint…", true, false) {
                            events.push(GutterEvent::Edit { line });
                            ui.close();
                        }
                        let (icon, label) = if mark.enabled {
                            (Icon::EyeSlash, "Disable Breakpoint")
                        } else {
                            (Icon::Eye, "Enable Breakpoint")
                        };
                        if kit::menu_item(ui, icon, label, true, false) {
                            events.push(GutterEvent::SetEnabled {
                                line,
                                enabled: !mark.enabled,
                            });
                            ui.close();
                        }
                        if kit::menu_item(ui, Icon::Trash, "Remove Breakpoint", true, true) {
                            events.push(GutterEvent::Toggle { line });
                            ui.close();
                        }
                    }
                    None => {
                        if kit::menu_item(ui, Icon::Plus, "Add Breakpoint", true, false) {
                            events.push(GutterEvent::Toggle { line });
                            ui.close();
                        }
                        if kit::menu_item(
                            ui,
                            Icon::Pencil,
                            "Add Conditional Breakpoint…",
                            true,
                            false,
                        ) {
                            events.push(GutterEvent::Edit { line });
                            ui.close();
                        }
                        if kit::menu_item(ui, Icon::Code, "Add Logpoint…", true, false) {
                            events.push(GutterEvent::AddLogpoint { line });
                            ui.close();
                        }
                    }
                }
            });
    }
    events
}
