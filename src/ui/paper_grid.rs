//! The Paper Trading panel's table surface: a hairline-bordered rounded frame holding a header
//! band and zebra rows, drawn by hand so the columns, the padding and the figures line up the way
//! SwiftUI's inset `Table` does. Rows are virtualised; a table narrower than its columns' minimum
//! widths scrolls sideways instead of clipping.

use super::kit::{self, BOLD, MEDIUM, SEMIBOLD, size};
use super::theme;
use eframe::egui::{
    self, Align, Color32, CornerRadius, Layout, Rect, ScrollArea, Sense, Stroke, Ui, UiBuilder,
    pos2, vec2,
};

pub const ROW_HEIGHT: f32 = 36.0;
pub const HEADER_HEIGHT: f32 = 28.0;
/// Space between the panel's edge and the table.
const MARGIN_X: f32 = 12.0;
const MARGIN_Y: f32 = 8.0;
const RADIUS: f32 = 8.0;
/// Space between a cell's edge and its content.
const CELL_PAD: f32 = 10.0;
/// The footer bar under the table (`kit::footer_bar`'s height), which the table leaves room for.
pub const FOOTER_HEIGHT: f32 = 45.0;

/// One column: its header, the narrowest it may get, how much of the spare width it takes and
/// which edge its content sits against.
#[derive(Clone, Copy, Debug)]
pub struct Col {
    pub title: &'static str,
    pub min: f32,
    pub grow: f32,
    pub right: bool,
}

pub const fn col(title: &'static str, min: f32, grow: f32, right: bool) -> Col {
    Col {
        title,
        min,
        grow,
        right,
    }
}

/// Each column's `(left, width)` across `width`: every column gets its minimum and the spare
/// room is shared by `grow`. A table with nothing growable gives the spare room to the last
/// column.
pub fn column_spans(cols: &[Col], width: f32) -> Vec<(f32, f32)> {
    let minimum: f32 = cols.iter().map(|col| col.min).sum();
    let spare = (width - minimum).max(0.0);
    let total_grow: f32 = cols.iter().map(|col| col.grow).sum();
    let mut left = 0.0;
    cols.iter()
        .enumerate()
        .map(|(index, col)| {
            let extra = if total_grow > 0.0 {
                spare * col.grow / total_grow
            } else if index + 1 == cols.len() {
                spare
            } else {
                0.0
            };
            let span = (left, col.min + extra);
            left += span.1;
            span
        })
        .collect()
}

/// The room left for a table above the footer.
pub fn table_height(ui: &Ui) -> f32 {
    (ui.available_height() - FOOTER_HEIGHT).max(80.0)
}

/// Draws the table. `cell` fills one cell of `row` / `col`, in a child `Ui` already laid out for
/// the column's alignment and clipped to the cell.
pub fn show(
    ui: &mut Ui,
    scope: &str,
    cols: &[Col],
    rows: usize,
    mut cell: impl FnMut(&mut Ui, usize, usize),
) {
    let area_height = table_height(ui);
    let (area, _) = ui.allocate_exact_size(vec2(ui.available_width(), area_height), Sense::hover());
    let frame = area.shrink2(vec2(MARGIN_X, MARGIN_Y));
    let minimum: f32 = cols.iter().map(|col| col.min).sum();
    let content_width = frame.width().max(minimum);
    let spans = column_spans(cols, content_width);
    let stroke = Stroke::new(1.0, theme::separator());

    let mut body = ui.new_child(
        UiBuilder::new()
            .max_rect(frame)
            .layout(Layout::top_down(Align::Min)),
    );
    body.set_clip_rect(frame.intersect(ui.clip_rect()));
    body.spacing_mut().item_spacing = vec2(0.0, 0.0);
    ScrollArea::horizontal()
        .id_salt((scope, "x"))
        .auto_shrink([false, false])
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
        .show(&mut body, |ui| {
            ui.set_min_width(content_width);
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            header(ui, cols, &spans, content_width, frame.top());
            ScrollArea::vertical()
                .id_salt((scope, "y"))
                .auto_shrink([false, false])
                .show_rows(ui, ROW_HEIGHT, rows, |ui, range| {
                    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                    for index in range {
                        row(ui, cols, &spans, content_width, index, frame, &mut cell);
                    }
                });
        });
    // The border goes over the rows, so a square zebra band never pokes out of a rounded corner.
    ui.painter()
        .rect_stroke(frame, RADIUS, stroke, egui::StrokeKind::Inside);
}

fn header(ui: &mut Ui, cols: &[Col], spans: &[(f32, f32)], width: f32, frame_top: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, HEADER_HEIGHT), Sense::hover());
    let painter = ui.painter();
    let top_corners = if rect.top() <= frame_top + 1.0 {
        CornerRadius {
            nw: RADIUS as u8 - 1,
            ne: RADIUS as u8 - 1,
            sw: 0,
            se: 0,
        }
    } else {
        CornerRadius::ZERO
    };
    painter.rect_filled(rect, top_corners, theme::quaternary(0.5));
    painter.hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        Stroke::new(1.0, theme::separator()),
    );
    for (col, &(left, span)) in cols.iter().zip(spans) {
        if col.title.is_empty() {
            continue;
        }
        let galley = kit::galley(
            ui,
            col.title,
            size::CAPTION + 1.0,
            SEMIBOLD,
            theme::text_secondary(),
            span - 2.0 * CELL_PAD,
        );
        let x = if col.right {
            rect.left() + left + span - CELL_PAD - galley.size().x
        } else {
            rect.left() + left + CELL_PAD
        };
        painter.galley(
            pos2(x, rect.center().y - galley.size().y / 2.0),
            galley,
            theme::text_secondary(),
        );
    }
}

fn row(
    ui: &mut Ui,
    cols: &[Col],
    spans: &[(f32, f32)],
    width: f32,
    index: usize,
    frame: Rect,
    cell: &mut impl FnMut(&mut Ui, usize, usize),
) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let hovered = ui.rect_contains_pointer(rect.intersect(frame));
    let fill = if hovered {
        Some(kit::primary(0.06))
    } else if index % 2 == 1 {
        Some(theme::quaternary(0.28))
    } else {
        None
    };
    if let Some(fill) = fill {
        // The band that touches the frame's bottom follows its rounded corners.
        let corners = if rect.bottom() >= frame.bottom() - 1.0 {
            CornerRadius {
                nw: 0,
                ne: 0,
                sw: RADIUS as u8 - 1,
                se: RADIUS as u8 - 1,
            }
        } else {
            CornerRadius::ZERO
        };
        ui.painter().rect_filled(rect, corners, fill);
    }
    for (column, (col, &(left, span))) in cols.iter().zip(spans).enumerate() {
        let pad = if column == 0 {
            CELL_PAD + 4.0
        } else {
            CELL_PAD
        };
        let right_pad = if column + 1 == cols.len() {
            CELL_PAD + 2.0
        } else {
            CELL_PAD
        };
        let bounds = Rect::from_min_max(
            pos2(rect.left() + left + pad, rect.top()),
            pos2(rect.left() + left + span - right_pad, rect.bottom()),
        );
        if bounds.width() <= 0.0 {
            continue;
        }
        let layout = if col.right {
            Layout::right_to_left(Align::Center)
        } else {
            Layout::left_to_right(Align::Center)
        };
        ui.scope_builder(UiBuilder::new().max_rect(bounds).layout(layout), |ui| {
            ui.set_clip_rect(bounds.intersect(ui.clip_rect()));
            ui.spacing_mut().item_spacing = vec2(6.0, 0.0);
            cell(ui, index, column);
        });
    }
    // A cell that draws nothing leaves its scope's rect at the middle of the row (centred, no
    // height), and a scope moves the cursor to its own end: the next row would start half a row
    // too high. Always continue from the row's own bottom.
    ui.advance_cursor_after_rect(rect);
}

// MARK: Cells

/// A figure: the proportional face at 12 pt, cut with an ellipsis rather than overflowing.
pub fn figure(ui: &mut Ui, text: &str, color: Color32) {
    ui.add(egui::Label::new(kit::text(text, 12.0, kit::REGULAR, color)).truncate());
}

/// A figure that carries the row's result: a touch heavier.
pub fn figure_bold(ui: &mut Ui, text: &str, color: Color32) {
    ui.add(egui::Label::new(kit::text(text, 12.0, SEMIBOLD, color)).truncate());
}

/// Plain 12 pt text, truncated.
pub fn text_cell(ui: &mut Ui, text: &str, color: Color32) {
    ui.add(egui::Label::new(kit::text(text, 12.0, MEDIUM, color)).truncate());
}

/// `PaperSideChip`: BUY / SELL / LONG / SHORT / TP / SL on a tinted capsule.
pub fn tag(ui: &mut Ui, text: &str, tint: Color32) {
    let galley = kit::galley(ui, text, 10.5, BOLD, tint, f32::INFINITY);
    let size = vec2(galley.size().x + 14.0, galley.size().y + 4.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter()
        .rect_filled(rect, size.y / 2.0, theme::with_alpha(tint, 0.14));
    ui.painter().galley(rect.min + vec2(7.0, 2.0), galley, tint);
}

#[cfg(test)]
mod tests {
    use super::*;

    const COLS: [Col; 3] = [
        col("A", 100.0, 2.0, false),
        col("B", 50.0, 0.0, true),
        col("C", 80.0, 1.0, true),
    ];

    #[test]
    fn columns_take_their_minimum_and_share_the_spare_room_by_weight() {
        let spans = column_spans(&COLS, 330.0);
        // 100 spare: A gets 2/3, C 1/3, B none.
        assert_eq!(spans[0], (0.0, 100.0 + 100.0 * 2.0 / 3.0));
        assert_eq!(spans[1].1, 50.0);
        assert!((spans[2].1 - (80.0 + 100.0 / 3.0)).abs() < 1e-3);
        let last = spans.last().unwrap();
        assert!((last.0 + last.1 - 330.0).abs() < 1e-3, "fills the width");
    }

    #[test]
    fn a_narrow_table_keeps_the_minimums() {
        let spans = column_spans(&COLS, 10.0);
        assert_eq!(spans[0], (0.0, 100.0));
        assert_eq!(spans[1], (100.0, 50.0));
        assert_eq!(spans[2], (150.0, 80.0));
    }

    #[test]
    fn with_nothing_growable_the_last_column_takes_the_spare_room() {
        let cols = [col("A", 40.0, 0.0, false), col("B", 40.0, 0.0, false)];
        let spans = column_spans(&cols, 200.0);
        assert_eq!(spans[0], (0.0, 40.0));
        assert_eq!(spans[1], (40.0, 160.0));
    }

    #[test]
    fn the_grid_draws_rows_and_cells() {
        let ctx = egui::Context::default();
        let mut cells = 0;
        for _ in 0..2 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                show(ui, "test-grid", &COLS, 3, |ui, _, _| {
                    cells += 1;
                    figure(ui, "1.00", Color32::WHITE);
                });
            });
            output.textures_delta.clear();
        }
        assert!(cells >= 3, "the cells ran: {cells}");
    }
    /// A row whose last cell draws nothing (a finished order has no buttons) must not pull the
    /// rows after it up: every row starts one row height below the one before.
    #[test]
    fn rows_stay_one_row_height_apart_when_the_last_cell_is_empty() {
        let ctx = egui::Context::default();
        let tops = std::cell::RefCell::new(Vec::new());
        for _ in 0..2 {
            tops.borrow_mut().clear();
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                show(ui, "gap-grid", &COLS, 5, |ui, row, column| match column {
                    0 => {
                        tops.borrow_mut().push((row, ui.max_rect().top()));
                        figure(ui, "x", Color32::WHITE);
                    }
                    // Rows 1 and 2 draw nothing in the last column, as a finished order does.
                    2 if row == 0 || row > 2 => figure(ui, "y", Color32::WHITE),
                    _ => {}
                });
            });
            output.textures_delta.clear();
        }
        let tops = tops.into_inner();
        assert!(tops.len() >= 4, "the rows were drawn: {tops:?}");
        for pair in tops.windows(2) {
            let gap = pair[1].1 - pair[0].1;
            assert!(
                (gap - ROW_HEIGHT).abs() < 0.01,
                "row {} starts {gap} below row {}, not {ROW_HEIGHT}: {tops:?}",
                pair[1].0,
                pair[0].0
            );
        }
    }
}
