//! The gutter left of the text: line numbers, breakpoint and stop markers, the change column
//! and, while the profiler is on, the profile column left of it.

pub(super) mod breakpoints;
pub(super) mod changes;
pub(super) mod profile;

use super::{FONT_SIZE, GutterRows};
use crate::model::editor_theme::{SurfaceColor, SyntaxRole};
use crate::pine_studio::debug::marks::DebugMarks;
use crate::ui::editor_appearance::{Appearance, EditorFont};
use crate::ui::theme;
use breakpoints::{paint_breakpoint, paint_stop_arrow};
use eframe::egui::{self, Align2, Rect, Ui};

pub(super) const GUTTER_WIDTH: f32 = 64.0;

/// Where a breakpoint marker sits in the gutter, from its left edge: right of the change strip,
/// left of the line numbers.
const MARKER_X: f32 = 21.0;
const MARKER_RADIUS: f32 = 5.0;
/// A breakpoint marker's radius on the line the script is stopped at, where it sits inside the
/// larger arrow.
const STOPPED_MARKER_RADIUS: f32 = 4.0;
/// The change column at the gutter's left edge: the bar's inset and width, and the (much wider,
/// so no one has to aim at three points) strip that takes the click.
const CHANGE_INSET: f32 = 2.0;
const CHANGE_BAR: f32 = 3.0;
const CHANGE_HIT: f32 = 12.0;
/// How far a deleted-lines wedge reaches above and below its boundary.
const WEDGE_HALF: f32 = 4.0;

pub(super) const LINE_NUMBER_SIZE: f32 = 11.0;

/// The size of the line numbers: the editor's size less two points, as it always was.
pub(super) fn line_number_size(code: &EditorFont) -> f32 {
    (code.size - (FONT_SIZE - LINE_NUMBER_SIZE)).max(7.0)
}

/// The gutter's width: [`GUTTER_WIDTH`], or more where the numbers of a long script in a large font
/// would not fit (the breakpoint column and the room either side stay as they are).
pub(super) fn gutter_width(ui: &Ui, code: &EditorFont, lines: usize) -> f32 {
    let digits = lines.max(1).to_string().len().max(4);
    let font = code.at(ui.ctx(), line_number_size(code));
    let digit = ui.fonts_mut(|fonts| fonts.glyph_width(&font, '0'));
    GUTTER_WIDTH.max(digit * digits as f32 + 36.0)
}

pub(super) fn paint_gutter(
    ui: &Ui,
    gutter: Rect,
    rows: &GutterRows,
    marks: &DebugMarks,
    look: &Appearance,
    code: &EditorFont,
) {
    let painter = ui.painter().with_clip_rect(gutter);
    let font = code.at(ui.ctx(), line_number_size(code));
    for (index, (top, height)) in rows.rows.iter().enumerate() {
        if top + height < gutter.min.y || *top > gutter.max.y {
            continue;
        }
        let line = index + 1;
        let stopped = marks.stop.is_some_and(|stop| stop.line == line);
        painter.text(
            egui::pos2(gutter.max.x - 8.0, top + height / 2.0),
            Align2::RIGHT_CENTER,
            line.to_string(),
            font.clone(),
            if stopped {
                look.role(SyntaxRole::Default)
            } else if rows.caret_line == Some(index) {
                look.surface(SurfaceColor::LineNumberActive)
            } else {
                look.surface(SurfaceColor::LineNumber)
            },
        );
        let center = egui::pos2(gutter.min.x + MARKER_X, top + height / 2.0);
        let stop = marks.stop.filter(|stop| stop.line == line);
        let mark = marks.breakpoint_at(line);
        if let Some(stop) = stop {
            // With a breakpoint on the line the arrow is drawn larger and carries it in its body.
            paint_stop_arrow(&painter, center, stop, mark.is_some().then_some(*height));
        }
        match (mark, stop) {
            (Some(mark), Some(_)) => {
                // A ring of gutter colour sets the dot off from the arrow it sits in.
                painter.circle_filled(
                    center,
                    STOPPED_MARKER_RADIUS + 1.0,
                    theme::gutter_background(),
                );
                paint_breakpoint(&painter, center, mark, STOPPED_MARKER_RADIUS);
            }
            (Some(mark), None) => paint_breakpoint(&painter, center, mark, MARKER_RADIUS),
            _ => {}
        }
    }
}
