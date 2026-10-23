//! The Table tool projected onto the plot: the grid of cells, their text, the borders a row or a
//! column is resized by and the handles. A row is as tall as its tallest wrapped cell needs, never
//! shorter than the height it was given, so long text is never cut off by a short row.

use super::Projection;
use super::draw::TEXT_PAD;
use super::edit::{BoxHandle, Part};
use super::shape::{Divider, Grid, HandleSpot, Screen};
use crate::model::drawing_style::DrawingStyle;
use crate::model::table::{ScreenAnchor, TableData};
use eframe::egui::{Pos2, Rect, pos2};

/// Where a table's top-left corner is on screen: at its chart point, or at the place on the plot
/// it is pinned to.
pub(super) fn origin(table: &TableData, proj: &Projection) -> Pos2 {
    match table.screen {
        Some(anchor) => pos2(
            proj.plot.left() + anchor.x * proj.plot.width(),
            proj.plot.top() + anchor.y * proj.plot.height(),
        ),
        None => proj.point(table.at),
    }
}

/// The pinned place that is the screen point `point`.
pub(super) fn pinned_at(point: Pos2, proj: &Projection) -> ScreenAnchor {
    ScreenAnchor {
        x: (point.x - proj.plot.left()) / proj.plot.width().max(1.0),
        y: (point.y - proj.plot.top()) / proj.plot.height().max(1.0),
    }
    .clamped()
}

/// The height each row of `table` takes, in `style`: what it asks for, or what its tallest cell
/// needs when that is more.
pub(super) fn row_heights(table: &TableData, style: &DrawingStyle, proj: &Projection) -> Vec<f32> {
    (0..table.row_count())
        .map(|row| {
            let mut height = table.rows[row];
            for (column, width) in table.columns.iter().enumerate() {
                let text = table.cell(row, column);
                if !text.is_empty() {
                    let wrap = (width - 2.0 * TEXT_PAD).max(1.0);
                    let size = proj.measure.size(text, &style.text, wrap);
                    height = height.max(size.y.ceil() + 2.0 * TEXT_PAD);
                }
            }
            height
        })
        .collect()
}

/// Builds the screen shape of a table.
pub(super) fn project(
    table: &TableData,
    style: &DrawingStyle,
    proj: &Projection,
) -> Option<Screen> {
    let start = origin(table, proj);
    if !(start.x.is_finite() && start.y.is_finite()) {
        return None;
    }
    let heights = row_heights(table, style, proj);
    let mut xs = vec![start.x];
    for width in &table.columns {
        xs.push(xs[xs.len() - 1] + width);
    }
    let mut ys = vec![start.y];
    for height in &heights {
        ys.push(ys[ys.len() - 1] + height);
    }
    let (left, top) = (xs[0], ys[0]);
    let (right, bottom) = (xs[xs.len() - 1], ys[ys.len() - 1]);
    let rect = Rect::from_min_max(pos2(left, top), pos2(right, bottom));
    let outline = vec![
        pos2(left, top),
        pos2(right, top),
        pos2(right, bottom),
        pos2(left, bottom),
    ];

    let mut screen = Screen::empty();
    screen.fill = outline.clone();
    screen.frames.push(outline);
    screen.hit.push(rect);
    for x in &xs[1..xs.len() - 1] {
        screen.rules.push([pos2(*x, top), pos2(*x, bottom)]);
    }
    for y in &ys[1..ys.len() - 1] {
        screen.rules.push([pos2(left, *y), pos2(right, *y)]);
    }
    let grid = Grid { xs, ys };
    for row in 0..table.row_count() {
        for column in 0..table.column_count() {
            let text = table.cell(row, column);
            if !text.is_empty()
                && let Some(cell) = grid.cell_rect(row, column)
            {
                screen.cell_texts.push((cell, text.to_owned()));
            }
        }
    }
    for column in 0..table.column_count() {
        let x = grid.xs[column + 1];
        screen.dividers.push(Divider {
            part: Part::Column(column),
            a: pos2(x, top),
            b: pos2(x, bottom),
        });
    }
    for row in 0..table.row_count() {
        let y = grid.ys[row + 1];
        screen.dividers.push(Divider {
            part: Part::Row(row),
            a: pos2(left, y),
            b: pos2(right, y),
        });
    }
    screen.handles = vec![
        HandleSpot {
            part: Part::Anchor(0),
            pos: pos2(left, top),
            control: false,
        },
        HandleSpot {
            part: Part::Box(BoxHandle::BottomRight),
            pos: pos2(right, bottom),
            control: false,
        },
    ];
    screen.grid = Some(grid);
    screen.finish(6.0)
}
