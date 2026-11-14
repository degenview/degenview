//! The profiler's column: each measured line's share of the script's time, and its tooltip.

use crate::pine_studio::editor::{EditorState, GutterRows};
use crate::pine_studio::profiler::marks::{LineMark, ProfileColumn};
use crate::ui::icons::{self, Icon};
use crate::ui::{kit, theme};
use eframe::egui::{self, Align2, FontId, Rect, Sense, Stroke, Ui, Vec2};

/// The profiler's column, left of the gutter while the profiler is on: a flame, a bar and a share.
pub(in crate::pine_studio::editor) const PROFILE_WIDTH: f32 = 104.0;
const PROFILE_FLAME: f32 = 11.0;
const PROFILE_BAR_X: f32 = 21.0;
const PROFILE_BAR_WIDTH: f32 = 40.0;
const PROFILE_BAR_HEIGHT: f32 = 6.0;

/// The profiler's column: a bar and the share of the script's time beside each measured line, a
/// flame on the hottest, and a tooltip with the figures. Hovering a line washes the code block it
/// covers (the next frame; the wash is painted with the text). The column holds its width while the
/// profiler is on, so nothing moves when a profile arrives.
pub(in crate::pine_studio::editor) fn profile_gutter(
    ui: &mut Ui,
    column: Rect,
    rows: &GutterRows,
    state: &mut EditorState,
) {
    let Some(content) = state.profile.clone() else {
        return;
    };
    let painter = ui.painter().with_clip_rect(column);
    painter.line_segment(
        [column.right_top(), column.right_bottom()],
        Stroke::new(1.0, theme::separator()),
    );
    let response = ui.interact(column, state.id.with("profile-column"), Sense::hover());
    let row_at = |y: f32| {
        let index = rows.rows.partition_point(|(top, height)| top + height <= y);
        rows.rows
            .get(index)
            .filter(|(top, _)| *top <= y)
            .map(|_| index + 1)
    };
    let first = rows
        .rows
        .partition_point(|(top, height)| top + height < column.min.y)
        + 1;
    let last = rows.rows.partition_point(|(top, _)| *top <= column.max.y);
    let pointer_line = response.hover_pos().and_then(|pointer| row_at(pointer.y));
    // Marks of a profile whose inputs or data have changed are faded.
    let (marks, fade) = match &content {
        ProfileColumn::Marks(marks) => (marks, 1.0),
        ProfileColumn::Dated(marks) => (marks, 0.5),
        ProfileColumn::Empty(why) => {
            if response.hovered() {
                response.on_hover_text(*why);
            }
            if state.profile_hover.take().is_some() {
                ui.ctx().request_repaint();
            }
            return;
        }
    };
    let mut hovered: Option<&LineMark> = None;
    for mark in marks.in_view(first, last) {
        let (top, height) = rows.rows[mark.line - 1];
        let center_y = top + height / 2.0;
        if pointer_line == Some(mark.line) {
            hovered = Some(mark);
            painter.rect_filled(
                Rect::from_min_max(
                    egui::pos2(column.min.x, top),
                    egui::pos2(column.max.x, top + height),
                ),
                0.0,
                theme::ink(10),
            );
        }
        if mark.quiet {
            continue;
        }
        if mark.flame {
            icons::paint(
                &painter,
                Rect::from_center_size(
                    egui::pos2(column.min.x + 6.0 + PROFILE_FLAME / 2.0, center_y),
                    Vec2::splat(PROFILE_FLAME),
                ),
                Icon::Flame,
                theme::profiler_flame().gamma_multiply(fade),
            );
        }
        let bar_left = column.min.x + PROFILE_BAR_X;
        let track = Rect::from_min_size(
            egui::pos2(bar_left, center_y - PROFILE_BAR_HEIGHT / 2.0),
            Vec2::new(PROFILE_BAR_WIDTH, PROFILE_BAR_HEIGHT),
        );
        painter.rect_filled(track, 2.0, theme::profiler_track().gamma_multiply(fade));
        // Never thinner than a sliver, so a small share is still a mark.
        let width = (mark.bar * PROFILE_BAR_WIDTH).clamp(1.5, PROFILE_BAR_WIDTH);
        painter.rect_filled(
            Rect::from_min_size(track.min, Vec2::new(width, PROFILE_BAR_HEIGHT)),
            2.0,
            theme::profiler_heat(mark.heat).gamma_multiply(fade),
        );
        painter.text(
            egui::pos2(column.max.x - 6.0, center_y),
            Align2::RIGHT_CENTER,
            &mark.label,
            FontId::monospace(10.0),
            if mark.flame {
                theme::text_primary()
            } else {
                theme::text_secondary()
            }
            .gamma_multiply(fade),
        );
    }
    // A line that ran but took too little to draw can still be asked about.
    if hovered.is_none()
        && let Some(line) = pointer_line
    {
        hovered = marks.mark(line);
    }
    let next = hovered.and_then(|mark| mark.block.or(Some((mark.line, mark.line))));
    if state.profile_hover != next {
        state.profile_hover = next;
        ui.ctx().request_repaint();
    }
    if let Some(mark) = hovered
        && let Some(pointer) = response.hover_pos()
        && let Some((top, height)) = rows.rows.get(mark.line - 1).copied()
    {
        let mut at_line = response.clone();
        at_line.interact_rect = Rect::from_min_size(
            egui::pos2(column.min.x, top),
            Vec2::new(column.width(), height),
        );
        let _ = pointer;
        at_line.on_hover_ui(|ui| profile_tooltip(ui, mark));
    }
}

fn profile_tooltip(ui: &mut Ui, mark: &LineMark) {
    ui.set_max_width(320.0);
    ui.spacing_mut().item_spacing.y = 3.0;
    ui.label(kit::text(
        mark.tooltip.title.clone(),
        12.0,
        kit::SEMIBOLD,
        theme::text_primary(),
    ));
    egui::Grid::new("profile-tooltip")
        .num_columns(2)
        .spacing(Vec2::new(12.0, 2.0))
        .show(ui, |ui| {
            for (label, value) in &mark.tooltip.rows {
                ui.label(kit::text(
                    *label,
                    11.5,
                    kit::REGULAR,
                    theme::text_secondary(),
                ));
                ui.label(kit::text(
                    value.clone(),
                    11.5,
                    kit::MEDIUM,
                    theme::text_primary(),
                ));
                ui.end_row();
            }
        });
    for note in &mark.tooltip.notes {
        ui.label(kit::text(
            note.clone(),
            11.0,
            kit::REGULAR,
            theme::text_tertiary(),
        ));
    }
}
