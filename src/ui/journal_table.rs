//! The Trade Journal's trade table: sortable headers, drag-to-resize columns, a selected row, and
//! virtualised rows. Hand-drawn in the paper grid's style (`paper_grid`), which has fixed columns
//! and no row selection. Only the visible rows are asked for their cells.

use eframe::egui::{
    self, Align, CornerRadius, CursorIcon, Layout, Rect, ScrollArea, Sense, Stroke, Ui, UiBuilder,
    pos2, vec2,
};

use super::icons::{self, Icon};
use super::kit::{self, SEMIBOLD, size};
use super::theme;
use crate::model::journal::table::{Column, Sort};

pub const ROW_HEIGHT: f32 = 34.0;
pub const HEADER_HEIGHT: f32 = 30.0;
const RADIUS: f32 = 10.0;
const CELL_PAD: f32 = 10.0;
/// The grab zone at a column's right edge.
const GRIP: f32 = 7.0;

pub struct TableSpec<'a> {
    pub salt: &'a str,
    pub columns: &'a [Column],
    pub width_of: &'a dyn Fn(Column) -> f32,
    pub sort: Sort,
    pub rows: usize,
    pub selected: Option<usize>,
    /// Bring this row into view (the keyboard moved the selection).
    pub scroll_to: Option<usize>,
    /// The height to fill; the table never takes less than a few rows.
    pub height: f32,
}

#[derive(Default)]
pub struct TableOutput {
    pub sort_by: Option<Column>,
    pub clicked: Option<usize>,
    /// A column being dragged to a new width.
    pub resizing: Option<(Column, f32)>,
    /// The drag ended: the width can be saved.
    pub resize_done: bool,
}

pub fn show(
    ui: &mut Ui,
    spec: &TableSpec<'_>,
    mut cell: impl FnMut(&mut Ui, usize, Column),
) -> TableOutput {
    let mut output = TableOutput::default();
    let (area, _) = ui.allocate_exact_size(
        vec2(ui.available_width(), spec.height.max(HEADER_HEIGHT * 3.0)),
        Sense::hover(),
    );
    let frame = area;
    let widths: Vec<f32> = spec.columns.iter().map(|c| (spec.width_of)(*c)).collect();
    let total: f32 = widths.iter().sum();
    let content_width = frame.width().max(total);

    let mut body = ui.new_child(
        UiBuilder::new()
            .max_rect(frame)
            .layout(Layout::top_down(Align::Min)),
    );
    body.set_clip_rect(frame.intersect(ui.clip_rect()));
    body.spacing_mut().item_spacing = vec2(0.0, 0.0);
    ScrollArea::horizontal()
        .id_salt((spec.salt, "x"))
        .auto_shrink([false, false])
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
        .show(&mut body, |ui| {
            ui.set_min_width(content_width);
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            header(ui, spec, &widths, content_width, frame.top(), &mut output);
            let mut rows = ScrollArea::vertical()
                .id_salt((spec.salt, "y"))
                .auto_shrink([false, false]);
            if let Some(row) = spec.scroll_to {
                let visible = (frame.height() - HEADER_HEIGHT).max(ROW_HEIGHT);
                let target = row as f32 * ROW_HEIGHT - (visible - ROW_HEIGHT) / 2.0;
                rows = rows.vertical_scroll_offset(target.max(0.0));
            }
            rows.show_rows(ui, ROW_HEIGHT, spec.rows, |ui, range| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                for index in range {
                    let clicked = row(ui, spec, &widths, content_width, index, frame, &mut cell);
                    if clicked {
                        output.clicked = Some(index);
                    }
                }
            });
        });
    ui.painter().rect_stroke(
        frame,
        RADIUS,
        Stroke::new(1.0, theme::separator()),
        egui::StrokeKind::Inside,
    );
    output
}

fn header(
    ui: &mut Ui,
    spec: &TableSpec<'_>,
    widths: &[f32],
    width: f32,
    frame_top: f32,
    output: &mut TableOutput,
) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, HEADER_HEIGHT), Sense::hover());
    let top = if rect.top() <= frame_top + 1.0 {
        CornerRadius {
            nw: RADIUS as u8 - 1,
            ne: RADIUS as u8 - 1,
            sw: 0,
            se: 0,
        }
    } else {
        CornerRadius::ZERO
    };
    ui.painter().rect_filled(rect, top, theme::quaternary(0.5));
    ui.painter().hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        Stroke::new(1.0, theme::separator()),
    );
    let mut left = rect.left();
    for (&column, &span) in spec.columns.iter().zip(widths) {
        let cell = Rect::from_min_size(pos2(left, rect.top()), vec2(span, HEADER_HEIGHT));
        left += span;
        let id = ui.id().with((spec.salt, "head", column));
        let response = ui
            .interact(
                Rect::from_min_max(cell.min, pos2(cell.right() - GRIP, cell.bottom())),
                id,
                Sense::click(),
            )
            .on_hover_cursor(CursorIcon::PointingHand);
        if response.clicked() {
            output.sort_by = Some(column);
        }
        if response.hovered() {
            ui.painter().rect_filled(cell, 0.0, kit::primary(0.04));
        }
        let active = spec.sort.column == column;
        let color = if active {
            theme::text_primary()
        } else {
            theme::text_secondary()
        };
        let arrow = if active { 12.0 } else { 0.0 };
        let galley = kit::galley(
            ui,
            column.label(),
            size::CAPTION + 1.0,
            SEMIBOLD,
            color,
            span - 2.0 * CELL_PAD - arrow,
        );
        let text_left = if column.right_aligned() {
            cell.right() - CELL_PAD - galley.size().x - arrow
        } else {
            cell.left() + CELL_PAD
        };
        let text_pos = pos2(text_left, cell.center().y - galley.size().y / 2.0);
        let text_right = text_pos.x + galley.size().x;
        ui.painter().galley(text_pos, galley, color);
        if active {
            icons::paint(
                ui.painter(),
                Rect::from_center_size(pos2(text_right + 8.0, cell.center().y), vec2(9.0, 9.0)),
                if spec.sort.descending {
                    Icon::ArrowDown
                } else {
                    Icon::ArrowUp
                },
                color,
            );
        }
        // The grip: drag to resize.
        let grip = ui
            .interact(
                Rect::from_min_max(pos2(cell.right() - GRIP, cell.top()), cell.max),
                id.with("grip"),
                Sense::drag(),
            )
            .on_hover_cursor(CursorIcon::ResizeHorizontal);
        if grip.hovered() || grip.dragged() {
            ui.painter().vline(
                cell.right() - 1.0,
                cell.y_range().shrink(6.0),
                Stroke::new(1.5, theme::text_tertiary()),
            );
        }
        if grip.dragged() {
            let wanted = (span + grip.drag_delta().x).max(column.min_width());
            output.resizing = Some((column, wanted));
        }
        if grip.drag_stopped() {
            output.resize_done = true;
        }
    }
}

fn row(
    ui: &mut Ui,
    spec: &TableSpec<'_>,
    widths: &[f32],
    width: f32,
    index: usize,
    frame: Rect,
    cell: &mut impl FnMut(&mut Ui, usize, Column),
) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::click());
    if !ui.is_rect_visible(rect) {
        return false;
    }
    let selected = spec.selected == Some(index);
    let hovered = ui.rect_contains_pointer(rect.intersect(frame));
    let fill = if selected {
        Some(kit::primary(0.10))
    } else if hovered {
        Some(kit::primary(0.05))
    } else if index % 2 == 1 {
        Some(theme::quaternary(0.28))
    } else {
        None
    };
    if let Some(fill) = fill {
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
    let mut left = rect.left();
    for (&column, &span) in spec.columns.iter().zip(widths) {
        let bounds = Rect::from_min_max(
            pos2(left + CELL_PAD, rect.top()),
            pos2(left + span - CELL_PAD, rect.bottom()),
        );
        left += span;
        if bounds.width() <= 0.0 {
            continue;
        }
        let layout = if column.right_aligned() {
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
    response.on_hover_cursor(CursorIcon::PointingHand).clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_draws_only_the_visible_rows() {
        let ctx = egui::Context::default();
        let mut asked = std::collections::BTreeSet::new();
        let columns = [Column::Open, Column::Symbol, Column::Net];
        for _ in 0..2 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 400.0))),
                    ..Default::default()
                },
                |ui| {
                    show(
                        ui,
                        &TableSpec {
                            salt: "t",
                            columns: &columns,
                            width_of: &|c| c.default_width(),
                            sort: Sort::default(),
                            rows: 10_000,
                            selected: Some(3),
                            scroll_to: None,
                            height: 300.0,
                        },
                        |_, row, _| {
                            asked.insert(row);
                        },
                    );
                },
            );
            output.textures_delta.clear();
        }
        assert!(!asked.is_empty());
        assert!(asked.len() < 30, "10,000 rows, {} drawn", asked.len());
        assert!(asked.contains(&0));
    }
}
