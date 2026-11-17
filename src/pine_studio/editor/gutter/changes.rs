//! The change column: markers for the lines that differ from the saved text, their tooltips
//! and menu, the comparison popover, and putting the saved lines back.

use super::{CHANGE_BAR, CHANGE_HIT, CHANGE_INSET, WEDGE_HALF};
use crate::i18n::{tr, tr_n};
use crate::pine_studio::editor::diff::{ChangeKind, DiffHunk, DiffSnapshot};
use crate::pine_studio::editor::diff_view::PreviewEvent;
use crate::pine_studio::editor::{
    EditorState, GutterRows, TEXT_INSET, apply_undoable, diff, diff_view,
};
use crate::ui::icons::Icon;
use crate::ui::{accessibility, kit, theme};
use eframe::egui::{self, Color32, Rect, Sense, Shape, Stroke, Ui, Vec2};

/// What a menu row on a change asks for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChangeCommand {
    Show,
    Revert,
    CopyOriginal,
    CopyCurrent,
}

/// A row of a change's menu: the change column's right-click menu and the Edit menu over the
/// editor list the same ones.
pub(in crate::pine_studio) struct ChangeRow {
    pub icon: Icon,
    pub label: &'static str,
    pub command: ChangeCommand,
}

/// What can be done to a change of `kind`; the words follow what the change is.
pub(in crate::pine_studio) fn change_rows(kind: ChangeKind) -> Vec<ChangeRow> {
    let (show, revert, original, current) = match kind {
        ChangeKind::Added => (
            tr!("Show Changes"),
            tr!("Revert Added Lines"),
            None,
            Some(tr!("Copy Current Lines")),
        ),
        ChangeKind::Deleted => (
            tr!("Show Deleted Lines"),
            tr!("Restore Deleted Lines"),
            Some(tr!("Copy Original Lines")),
            None,
        ),
        ChangeKind::Modified => (
            tr!("Compare Changes"),
            tr!("Revert Change"),
            Some(tr!("Copy Original")),
            Some(tr!("Copy Current")),
        ),
    };
    let row = |icon, label, command| ChangeRow {
        icon,
        label,
        command,
    };
    let mut rows = vec![
        row(Icon::Eye, show, ChangeCommand::Show),
        row(Icon::Undo, revert, ChangeCommand::Revert),
    ];
    rows.extend(original.map(|label| row(Icon::CopyDoc, label, ChangeCommand::CopyOriginal)));
    rows.extend(current.map(|label| row(Icon::CopyDoc, label, ChangeCommand::CopyCurrent)));
    rows
}

/// Does what a change's menu row asks. Returns whether the text changed.
pub(in crate::pine_studio::editor) fn run_change(
    ctx: &egui::Context,
    state: &mut EditorState,
    hunk: DiffHunk,
    command: ChangeCommand,
) -> bool {
    match command {
        ChangeCommand::Show => state.preview = Some(hunk),
        ChangeCommand::Revert => return apply_hunk_edit(ctx, state, &hunk),
        ChangeCommand::CopyOriginal => ctx.copy_text(hunk.old_text),
        ChangeCommand::CopyCurrent => ctx.copy_text(hunk.new_text),
    }
    false
}

/// The hover text: the count and what a click does, in words (the colour is not the message).
fn change_tooltip(hunk: &DiffHunk) -> String {
    match hunk.kind {
        ChangeKind::Added => tr_n!(
            hunk.added_lines(),
            "{count} line added — Click to inspect",
            "{count} lines added — Click to inspect"
        ),
        ChangeKind::Deleted => tr_n!(
            hunk.removed_lines(),
            "{count} line deleted — Click to restore",
            "{count} lines deleted — Click to restore"
        ),
        ChangeKind::Modified if diff_view::is_single_line(&hunk.new_lines) => tr!(
            "Line {line} modified — Click to compare",
            line = hunk.new_lines.start + 1
        ),
        ChangeKind::Modified => tr!(
            "Lines {first}–{last} modified — Click to compare",
            first = hunk.new_lines.start + 1,
            last = hunk.new_lines.end
        ),
    }
}

/// The spoken description of a marker.
fn change_description(hunk: &DiffHunk) -> String {
    match hunk.kind {
        ChangeKind::Added if diff_view::is_single_line(&hunk.new_lines) => tr!(
            "Line {line}: Added since last save",
            line = hunk.new_lines.start + 1
        ),
        ChangeKind::Added => tr!(
            "Lines {first}–{last}: Added since last save",
            first = hunk.new_lines.start + 1,
            last = hunk.new_lines.end
        ),
        ChangeKind::Modified if diff_view::is_single_line(&hunk.new_lines) => tr!(
            "Line {line}: Modified since last save",
            line = hunk.new_lines.start + 1
        ),
        ChangeKind::Modified => tr!(
            "Lines {first}–{last}: Modified since last save",
            first = hunk.new_lines.start + 1,
            last = hunk.new_lines.end
        ),
        ChangeKind::Deleted if hunk.new_lines.start == 0 => tr_n!(
            hunk.removed_lines(),
            "At the start: {count} line deleted since last save",
            "At the start: {count} lines deleted since last save"
        ),
        ChangeKind::Deleted => tr_n!(
            hunk.removed_lines(),
            "After line {line}: {count} line deleted since last save",
            "After line {line}: {count} lines deleted since last save",
            line = hunk.new_lines.start
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
                let rows = change_rows(hunk.kind);
                let labels: Vec<&str> = rows.iter().map(|row| row.label).collect();
                kit::menu_fit(ui, &labels);
                ui.spacing_mut().item_spacing.y = 0.0;
                for row in rows {
                    if kit::menu_item(ui, row.icon, row.label, true, false) {
                        command = Some((hunk.clone(), row.command));
                        ui.close();
                    }
                }
            });
    }

    let mut edited = false;
    if let Some((hunk, chosen)) = command {
        edited |= run_change(&ctx, state, hunk, chosen);
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
