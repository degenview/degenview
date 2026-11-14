//! The change column: markers for the lines that differ from the saved text, their tooltips
//! and menu, the comparison popover, and putting the saved lines back.

use super::{CHANGE_BAR, CHANGE_HIT, CHANGE_INSET, WEDGE_HALF};
use crate::pine_studio::editor::diff::{ChangeKind, DiffHunk, DiffSnapshot};
use crate::pine_studio::editor::diff_view::PreviewEvent;
use crate::pine_studio::editor::{
    EditorState, GutterRows, TEXT_INSET, apply_undoable, diff, diff_view, plural,
};
use crate::ui::icons::Icon;
use crate::ui::{accessibility, kit, theme};
use eframe::egui::{self, Color32, Rect, Sense, Shape, Stroke, Ui, Vec2};

/// What a context-menu row on a change asks for.
#[derive(Clone, Copy)]
enum ChangeCommand {
    Show,
    Revert,
    CopyOriginal,
    CopyCurrent,
}

/// "Line 12", "Lines 18–20": a lines label starting a sentence.
fn sentence_lines(range: &std::ops::Range<usize>) -> String {
    let label = diff_view::lines_label(range);
    let mut chars = label.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// The hover text: the count and what a click does, in words (the colour is not the message).
fn change_tooltip(hunk: &DiffHunk) -> String {
    match hunk.kind {
        ChangeKind::Added => format!(
            "{} added — Click to inspect",
            plural(hunk.added_lines(), "line")
        ),
        ChangeKind::Deleted => format!(
            "{} deleted — Click to restore",
            plural(hunk.removed_lines(), "line")
        ),
        ChangeKind::Modified => format!(
            "{} modified — Click to compare",
            sentence_lines(&hunk.new_lines)
        ),
    }
}

/// The spoken description of a marker.
fn change_description(hunk: &DiffHunk) -> String {
    match hunk.kind {
        ChangeKind::Added => format!("{}: Added since last save", sentence_lines(&hunk.new_lines)),
        ChangeKind::Modified => format!(
            "{}: Modified since last save",
            sentence_lines(&hunk.new_lines)
        ),
        ChangeKind::Deleted if hunk.new_lines.start == 0 => format!(
            "At the start: {} deleted since last save",
            plural(hunk.removed_lines(), "line")
        ),
        ChangeKind::Deleted => format!(
            "After line {}: {} deleted since last save",
            hunk.new_lines.start,
            plural(hunk.removed_lines(), "line")
        ),
    }
}

fn change_color(kind: ChangeKind) -> Color32 {
    match kind {
        ChangeKind::Added => theme::editor_diff_added(),
        ChangeKind::Deleted => theme::editor_diff_deleted(),
        ChangeKind::Modified => theme::editor_diff_modified(),
    }
}

/// A change's vertical extent on screen, from the rows the text view laid out. A deleted change
/// has no rows of its own: it is the boundary above the line it used to precede (the foot of the
/// last row at the end of the text), so its extent is one y.
fn change_span(rows: &GutterRows, hunk: &DiffHunk, empty_top: f32) -> (f32, f32) {
    let top_of = |line: usize| rows.rows.get(line).map(|(top, _)| *top);
    match hunk.kind {
        ChangeKind::Deleted => {
            let y = top_of(hunk.new_lines.start)
                .or_else(|| rows.rows.last().map(|(top, height)| top + height))
                .unwrap_or(empty_top);
            (y, y)
        }
        ChangeKind::Added | ChangeKind::Modified => {
            let top = top_of(hunk.new_lines.start).unwrap_or(empty_top);
            let bottom = rows
                .rows
                .get(hunk.new_lines.end.saturating_sub(1))
                .map_or(top, |(row_top, height)| row_top + height);
            (top, bottom)
        }
    }
}

/// The clickable strip of a change at the gutter's edge.
fn change_hit(gutter: Rect, span: (f32, f32), kind: ChangeKind) -> Rect {
    let (top, bottom) = match kind {
        ChangeKind::Deleted => (span.0 - 6.0, span.1 + 6.0),
        _ => span,
    };
    Rect::from_min_max(
        egui::pos2(gutter.min.x, top),
        egui::pos2(gutter.min.x + CHANGE_HIT, bottom),
    )
    .intersect(gutter)
}

/// Paints and handles the change markers of the visible lines, and hosts the open comparison.
/// Returns whether a revert changed the text.
pub(in crate::pine_studio::editor) fn change_gutter(
    ui: &mut Ui,
    gutter: Rect,
    rows: &GutterRows,
    state: &mut EditorState,
    changes: &DiffSnapshot,
) -> bool {
    let ctx = ui.ctx().clone();
    let empty_top = gutter.min.y + TEXT_INSET;
    let first = rows
        .rows
        .partition_point(|(top, height)| top + height < gutter.min.y);
    let last = rows
        .rows
        .partition_point(|(top, _)| *top <= gutter.max.y)
        .saturating_sub(1);
    let painter = ui.painter().with_clip_rect(gutter);
    let dragging = ui.input(|input| input.pointer.any_down());
    let bar_x = gutter.min.x + CHANGE_INSET;
    let mut command: Option<(DiffHunk, ChangeCommand)> = None;
    for index in changes.in_view(first, last) {
        let hunk = &changes.hunks()[index];
        let span = change_span(rows, hunk, empty_top);
        let hit = change_hit(gutter, span, hunk.kind);
        if hit.height() <= 0.0 {
            continue;
        }
        let response = ui.interact(hit, state.id.with(("change", index)), Sense::click());
        let response = accessibility::label(response, change_description(hunk));
        let hovered = response.hovered();
        if hovered {
            ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let color = change_color(hunk.kind);
        match hunk.kind {
            ChangeKind::Deleted => {
                // A wedge pointing into the text at the boundary where the lines were.
                let (x, y) = (bar_x, span.0);
                let reach = if hovered { 7.0 } else { 5.0 };
                painter.add(Shape::convex_polygon(
                    vec![
                        egui::pos2(x, y - WEDGE_HALF),
                        egui::pos2(x + reach, y),
                        egui::pos2(x, y + WEDGE_HALF),
                    ],
                    color,
                    Stroke::NONE,
                ));
            }
            ChangeKind::Added | ChangeKind::Modified => {
                let width = CHANGE_BAR + if hovered { 1.5 } else { 0.0 };
                painter.rect_filled(
                    Rect::from_min_max(
                        egui::pos2(bar_x, span.0 + 1.0),
                        egui::pos2(bar_x + width, (span.1 - 1.0).max(span.0 + 1.0)),
                    ),
                    1.5,
                    color,
                );
            }
        }
        if !dragging {
            response.clone().on_hover_text(change_tooltip(hunk));
        }
        if response.clicked() {
            let open = state.preview.as_ref().is_some_and(|p| p.same_change(hunk));
            state.preview = (!open).then(|| hunk.clone());
        }
        egui::Popup::context_menu(&response)
            .frame(kit::menu_frame())
            .show(|ui| {
                ui.set_width(200.0);
                ui.spacing_mut().item_spacing.y = 0.0;
                let (show, revert, original, current) = match hunk.kind {
                    ChangeKind::Added => (
                        "Show Changes",
                        "Revert Added Lines",
                        None,
                        Some("Copy Current Lines"),
                    ),
                    ChangeKind::Deleted => (
                        "Show Deleted Lines",
                        "Restore Deleted Lines",
                        Some("Copy Original Lines"),
                        None,
                    ),
                    ChangeKind::Modified => (
                        "Compare Changes",
                        "Revert Change",
                        Some("Copy Original"),
                        Some("Copy Current"),
                    ),
                };
                let rows = [
                    (Icon::Eye, show, Some(ChangeCommand::Show)),
                    (Icon::Undo, revert, Some(ChangeCommand::Revert)),
                    (
                        Icon::CopyDoc,
                        original.unwrap_or(""),
                        original.map(|_| ChangeCommand::CopyOriginal),
                    ),
                    (
                        Icon::CopyDoc,
                        current.unwrap_or(""),
                        current.map(|_| ChangeCommand::CopyCurrent),
                    ),
                ];
                for (icon, label, chosen) in rows {
                    if let Some(chosen) = chosen
                        && kit::menu_item(ui, icon, label, true, false)
                    {
                        command = Some((hunk.clone(), chosen));
                        ui.close();
                    }
                }
            });
    }

    let mut edited = false;
    if let Some((hunk, chosen)) = command {
        match chosen {
            ChangeCommand::Show => state.preview = Some(hunk),
            ChangeCommand::Revert => edited |= apply_hunk_edit(&ctx, state, &hunk),
            ChangeCommand::CopyOriginal => ctx.copy_text(hunk.old_text),
            ChangeCommand::CopyCurrent => ctx.copy_text(hunk.new_text),
        }
    }

    // The open comparison follows its change through edits elsewhere and closes when the change
    // is gone (reverted by hand, saved, or edited into something else).
    let Some(wanted) = state.preview.clone() else {
        return edited;
    };
    let Some(index) = changes.find(&wanted) else {
        state.preview = None;
        return edited;
    };
    let hunk = changes.hunks()[index].clone();
    let span = change_span(rows, &hunk, empty_top);
    let anchor = Rect::from_min_max(
        egui::pos2(gutter.min.x, span.0.clamp(gutter.min.y, gutter.max.y)),
        egui::pos2(
            gutter.min.x + CHANGE_HIT,
            span.1.clamp(gutter.min.y, gutter.max.y),
        ),
    )
    .expand2(Vec2::new(0.0, 2.0));
    let event = diff_view::show(
        &ctx,
        state.id.with("change-preview"),
        anchor,
        &hunk,
        (index, changes.hunks().len()),
    );
    match event {
        PreviewEvent::None => {}
        PreviewEvent::Dismiss => state.preview = None,
        PreviewEvent::Apply => edited |= apply_hunk_edit(&ctx, state, &hunk),
        PreviewEvent::Step(forward) => {
            if let Some(next) = changes.neighbour(index, forward) {
                let next = &changes.hunks()[next];
                state.scroll_to_line = Some(next.new_lines.start);
                state.preview = Some(next.clone());
                ctx.request_repaint();
            }
        }
    }
    edited
}

/// Puts the saved lines back for `hunk`, through the same edit the typing assists use, as one
/// undo step. Refuses (and closes the comparison) if the text is no longer what the hunk was
/// computed from. Returns whether the text changed.
pub(in crate::pine_studio::editor) fn apply_hunk_edit(
    ctx: &egui::Context,
    state: &mut EditorState,
    hunk: &DiffHunk,
) -> bool {
    state.preview = None;
    let Ok(edit) = diff::revert_edit(hunk, &state.source) else {
        return false;
    };
    apply_undoable(ctx, state, &edit);
    true
}
