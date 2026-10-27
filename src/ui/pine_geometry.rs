//! Pure geometry for drawing a Pine script's visual output: colours, dash patterns, where a
//! label's text and shape sit, polyline curves, table layout, how per-bar series line up with the
//! candles, and the price range / future bars a script's drawings need. Nothing here paints; the
//! painter (`pine_chart_render`) and the tests share it. A port of `PineDrawingGeometry`,
//! `PineLabelGeometry`, `PineSize+Drawing` and the layout halves of `PineChartLayer`.

// Transient: the chart card hosts the painter once the Pine session produces output.
#![allow(dead_code)]

use crate::pine::outputs::{
    BoxOutput, DISPLAY_PANE, Horizontal, LabelOutput, LabelStyle, LineExtend, LineOutput,
    LineStyle, MISSING_INDEX, PlotStyle, PolylineOutput, Size, TableCell, TableOutput,
    TablePosition, TextAlign, Vertical, YLocation, index_is_known,
};
use crate::pine::program::VisualOutput;
use eframe::egui::{self, Align2, Color32, Pos2, Rect, Vec2};

// MARK: Colour and size

/// A Pine colour, `0xRRGGBBAA`, as an egui colour.
pub fn rgba(value: u32) -> Color32 {
    Color32::from_rgba_unmultiplied(
        (value >> 24) as u8,
        (value >> 16) as u8,
        (value >> 8) as u8,
        value as u8,
    )
}

/// Whether a Pine colour can be seen at all.
pub fn is_visible(value: u32) -> bool {
    value & 0xFF != 0
}

/// `value` with its opacity scaled by `factor`.
pub fn faded(value: u32, factor: f32) -> Color32 {
    let color = rgba(value);
    Color32::from_rgba_unmultiplied(
        color.r(),
        color.g(),
        color.b(),
        (f32::from(color.a()) * factor).round() as u8,
    )
}

/// Radius of a `shape.circle` marker.
pub fn marker_radius(size: Size) -> f32 {
    match size {
        Size::Tiny => 3.0,
        Size::Small | Size::Auto => 5.0,
        Size::Normal => 7.0,
        Size::Large => 10.0,
        Size::Huge => 14.0,
    }
}

/// Point size of label and table text.
pub fn font_size(size: Size) -> f32 {
    match size {
        Size::Tiny => 8.0,
        Size::Small => 10.0,
        Size::Large => 15.0,
        Size::Huge => 20.0,
        Size::Normal | Size::Auto => 12.0,
    }
}

// MARK: Strokes

/// The dash pattern a stroke of this style uses; the arrow styles are solid lines with arrowheads.
pub fn dash_pattern(style: LineStyle) -> &'static [f32] {
    match style {
        LineStyle::Dashed => &[6.0, 4.0],
        LineStyle::Dotted => &[1.0, 3.0],
        _ => &[],
    }
}

/// Whether the style puts an arrowhead at the line's first and second point.
pub fn arrowheads(style: LineStyle) -> (bool, bool) {
    match style {
        LineStyle::ArrowLeft => (true, false),
        LineStyle::ArrowRight => (false, true),
        LineStyle::ArrowBoth => (true, true),
        _ => (false, false),
    }
}

/// The triangle of an arrowhead at `tip` for a line arriving from `origin`.
pub fn arrowhead(tip: Pos2, origin: Pos2, length: f32, half_width: f32) -> Option<[Pos2; 3]> {
    let (dx, dy) = (tip.x - origin.x, tip.y - origin.y);
    let distance = (dx * dx + dy * dy).sqrt();
    if distance <= 0.0 {
        return None;
    }
    let (ux, uy) = (dx / distance, dy / distance);
    let base = Pos2::new(tip.x - ux * length, tip.y - uy * length);
    Some([
        tip,
        Pos2::new(base.x - uy * half_width, base.y + ux * half_width),
        Pos2::new(base.x + uy * half_width, base.y - ux * half_width),
    ])
}

/// A line's endpoints, pushed out to the plot's edges along its slope when it extends left and/or
/// right. Vertical lines never extend.
pub fn extended_line(a: Pos2, b: Pos2, extend: LineExtend, rect: Rect) -> (Pos2, Pos2) {
    if a.x == b.x {
        return (a, b);
    }
    let slope = (b.y - a.y) / (b.x - a.x);
    let (mut left, mut right) = if a.x < b.x { (a, b) } else { (b, a) };
    if extend.extends_left() {
        left = Pos2::new(rect.min.x, left.y - slope * (left.x - rect.min.x));
    }
    if extend.extends_right() {
        right = Pos2::new(rect.max.x, right.y + slope * (rect.max.x - right.x));
    }
    (left, right)
}

/// The points of a polyline: straight, or with `curved` a Catmull-Rom spline through every point
/// (flattened to short segments). A closed path wraps around, so the curve is smooth at the joint.
pub fn polyline_points(points: &[Pos2], curved: bool, closed: bool) -> Vec<Pos2> {
    if points.is_empty() || !curved || points.len() <= 2 {
        return points.to_vec();
    }
    const SEGMENTS: usize = 12;
    let count = points.len() as isize;
    let point = |index: isize| {
        if closed {
            points[index.rem_euclid(count) as usize]
        } else {
            points[index.clamp(0, count - 1) as usize]
        }
    };
    let spans = if closed { count } else { count - 1 };
    let mut path = vec![points[0]];
    for index in 0..spans {
        let (p0, p1, p2, p3) = (
            point(index - 1),
            point(index),
            point(index + 1),
            point(index + 2),
        );
        let c1 = Pos2::new(p1.x + (p2.x - p0.x) / 6.0, p1.y + (p2.y - p0.y) / 6.0);
        let c2 = Pos2::new(p2.x - (p3.x - p1.x) / 6.0, p2.y - (p3.y - p1.y) / 6.0);
        for step in 1..=SEGMENTS {
            let t = step as f32 / SEGMENTS as f32;
            let u = 1.0 - t;
            path.push(Pos2::new(
                u * u * u * p1.x
                    + 3.0 * u * u * t * c1.x
                    + 3.0 * u * t * t * c2.x
                    + t * t * t * p2.x,
                u * u * u * p1.y
                    + 3.0 * u * u * t * c1.y
                    + 3.0 * u * t * t * c2.y
                    + t * t * t * p2.y,
            ));
        }
    }
    path
}

/// Where a box's text is drawn and which part of the text sits on that point.
pub fn box_text_placement(
    rect: Rect,
    horizontal: TextAlign,
    vertical: TextAlign,
    margin: f32,
) -> (Pos2, Align2) {
    let (x, anchor_x) = match horizontal {
        TextAlign::Left => (rect.min.x + margin, egui::Align::Min),
        TextAlign::Right => (rect.max.x - margin, egui::Align::Max),
        _ => (rect.center().x, egui::Align::Center),
    };
    let (y, anchor_y) = match vertical {
        TextAlign::Top => (rect.min.y + margin, egui::Align::Min),
        TextAlign::Bottom => (rect.max.y - margin, egui::Align::Max),
        _ => (rect.center().y, egui::Align::Center),
    };
    (Pos2::new(x, y), Align2([anchor_x, anchor_y]))
}

// MARK: Labels

/// A glyph drawn at the anchor by the marker label styles.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LabelGlyph {
    Cross,
    Xcross,
    TriangleUp,
    TriangleDown,
    ArrowUp,
    ArrowDown,
    Flag,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LabelShape {
    /// A rounded rectangle behind the text, with a pointer (three points ending at the anchor) or
    /// none.
    Bubble {
        pointer: Vec<Pos2>,
    },
    Ellipse,
    Rectangle,
    Diamond,
    /// A glyph in `LabelLayout::glyph`, with the text beside it.
    Marker(LabelGlyph),
    /// Text only: no bubble, no glyph.
    TextOnly,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LabelLayout {
    /// The rectangle the text is centred in (and, for bubbles and shapes, the filled area).
    pub body: Rect,
    pub shape: LabelShape,
    pub glyph: Option<Rect>,
}

/// Where a label's text and shape sit relative to its anchor, for each `label.style_*`. `size` is
/// the text plus padding; `pointer` the half-width and length of a bubble's pointer.
pub fn label_layout(style: LabelStyle, anchor: Pos2, size: Vec2, pointer: f32) -> LabelLayout {
    let p = pointer;
    let rect = |x: f32, y: f32| Rect::from_min_size(Pos2::new(x, y), size);
    let vertical = |base_y: f32| LabelShape::Bubble {
        pointer: vec![
            Pos2::new(anchor.x - p, base_y),
            anchor,
            Pos2::new(anchor.x + p, base_y),
        ],
    };
    let horizontal = |base_x: f32| LabelShape::Bubble {
        pointer: vec![
            Pos2::new(base_x, anchor.y - p),
            anchor,
            Pos2::new(base_x, anchor.y + p),
        ],
    };
    let centred = rect(anchor.x - size.x / 2.0, anchor.y - size.y / 2.0);
    let simple = |body: Rect, shape: LabelShape| LabelLayout {
        body,
        shape,
        glyph: None,
    };
    match style {
        LabelStyle::LabelDown => {
            let body = rect(anchor.x - size.x / 2.0, anchor.y - p - size.y);
            simple(body, vertical(body.max.y))
        }
        LabelStyle::LabelUp => {
            let body = rect(anchor.x - size.x / 2.0, anchor.y + p);
            simple(body, vertical(body.min.y))
        }
        LabelStyle::LabelLeft => {
            let body = rect(anchor.x + p, anchor.y - size.y / 2.0);
            simple(body, horizontal(body.min.x))
        }
        LabelStyle::LabelRight => {
            let body = rect(anchor.x - p - size.x, anchor.y - size.y / 2.0);
            simple(body, horizontal(body.max.x))
        }
        LabelStyle::LabelLowerLeft
        | LabelStyle::LabelLowerRight
        | LabelStyle::LabelUpperLeft
        | LabelStyle::LabelUpperRight => corner(style, anchor, size, p),
        LabelStyle::LabelCenter => simple(
            centred,
            LabelShape::Bubble {
                pointer: Vec::new(),
            },
        ),
        LabelStyle::None | LabelStyle::TextOutline => simple(centred, LabelShape::TextOnly),
        LabelStyle::Circle => {
            // A circle that holds the text: its diameter is the text's diagonal.
            let diameter = (size.x * size.x + size.y * size.y).sqrt();
            let body = Rect::from_center_size(centred.center(), Vec2::splat(diameter));
            simple(body, LabelShape::Ellipse)
        }
        LabelStyle::Square => simple(centred, LabelShape::Rectangle),
        LabelStyle::Diamond => {
            // The diamond's corners reach the edges of a rectangle twice as large as the text needs.
            let body = Rect::from_center_size(centred.center(), size * 2.0);
            simple(body, LabelShape::Diamond)
        }
        LabelStyle::Cross
        | LabelStyle::Xcross
        | LabelStyle::Flag
        | LabelStyle::TriangleDown
        | LabelStyle::ArrowDown => marker_layout(style, anchor, size, true),
        LabelStyle::TriangleUp | LabelStyle::ArrowUp => marker_layout(style, anchor, size, false),
    }
}

/// The anchor is a corner of the bubble, with a pointer there.
fn corner(style: LabelStyle, anchor: Pos2, size: Vec2, p: f32) -> LabelLayout {
    let left_edge = matches!(
        style,
        LabelStyle::LabelLowerLeft | LabelStyle::LabelUpperLeft
    );
    let top = matches!(
        style,
        LabelStyle::LabelUpperLeft | LabelStyle::LabelUpperRight
    );
    let origin = Pos2::new(
        if left_edge {
            anchor.x
        } else {
            anchor.x - size.x
        },
        if top { anchor.y } else { anchor.y - size.y },
    );
    let horizontal = Pos2::new(anchor.x + if left_edge { p } else { -p }, anchor.y);
    let vertical = Pos2::new(anchor.x, anchor.y + if top { p } else { -p });
    LabelLayout {
        body: Rect::from_min_size(origin, size),
        shape: LabelShape::Bubble {
            pointer: vec![horizontal, anchor, vertical],
        },
        glyph: None,
    }
}

/// A glyph centred on the anchor, as tall as the text line, with the text above or below it.
fn marker_layout(style: LabelStyle, anchor: Pos2, size: Vec2, text_above: bool) -> LabelLayout {
    let side = size.y;
    let glyph = Rect::from_center_size(anchor, Vec2::splat(side));
    let origin_y = if text_above {
        glyph.min.y - size.y
    } else {
        glyph.max.y
    };
    let body = Rect::from_min_size(Pos2::new(anchor.x - size.x / 2.0, origin_y), size);
    let marker = match style {
        LabelStyle::Cross => LabelGlyph::Cross,
        LabelStyle::Xcross => LabelGlyph::Xcross,
        LabelStyle::Flag => LabelGlyph::Flag,
        LabelStyle::TriangleDown => LabelGlyph::TriangleDown,
        LabelStyle::TriangleUp => LabelGlyph::TriangleUp,
        LabelStyle::ArrowDown => LabelGlyph::ArrowDown,
        _ => LabelGlyph::ArrowUp,
    };
    LabelLayout {
        body,
        shape: LabelShape::Marker(marker),
        glyph: Some(glyph),
    }
}

// MARK: Tables

/// The padding a table cell adds around its text, and the gap to the plot's edge.
pub const CELL_PADDING: Vec2 = Vec2::new(12.0, 6.0);
pub const TABLE_MARGIN: f32 = 8.0;

/// A table's cells measured, with the column widths and row heights they imply.
#[derive(Clone, Debug, PartialEq)]
pub struct TableLayout {
    pub widths: Vec<f32>,
    pub heights: Vec<f32>,
}

impl TableLayout {
    pub fn size(&self) -> Vec2 {
        Vec2::new(self.widths.iter().sum(), self.heights.iter().sum())
    }

    /// Where `cell` sits, covering every column and row it spans.
    pub fn rect_of(&self, cell: &TableCell, origin: Pos2) -> Rect {
        let last_column = (cell.column + cell.column_span).min(self.widths.len());
        let last_row = (cell.row + cell.row_span).min(self.heights.len());
        let first_column = cell.column.min(self.widths.len());
        let first_row = cell.row.min(self.heights.len());
        Rect::from_min_size(
            Pos2::new(
                origin.x + self.widths[..first_column].iter().sum::<f32>(),
                origin.y + self.heights[..first_row].iter().sum::<f32>(),
            ),
            Vec2::new(
                self.widths[first_column..last_column.max(first_column)]
                    .iter()
                    .sum(),
                self.heights[first_row..last_row.max(first_row)]
                    .iter()
                    .sum(),
            ),
        )
    }
}

/// Sizes the grid to its cell text. `measure(text, font_size)` is the text's size; a cell's
/// `width` and `height` are minimums, as a percentage of the plot area.
pub fn measure_table(
    table: &TableOutput,
    area: Vec2,
    measure: &dyn Fn(&str, f32) -> Vec2,
) -> TableLayout {
    let mut layout = TableLayout {
        widths: vec![0.0; table.columns],
        heights: vec![0.0; table.rows],
    };
    for cell in &table.cells {
        if cell.column >= table.columns || cell.row >= table.rows {
            continue;
        }
        let size = measure(&cell.text, font_size(cell.text_size));
        let width = (size.x + CELL_PADDING.x).max(area.x * cell.width as f32 / 100.0);
        let height = (size.y + CELL_PADDING.y).max(area.y * cell.height as f32 / 100.0);
        if cell.column_span == 1 {
            layout.widths[cell.column] = layout.widths[cell.column].max(width);
        }
        if cell.row_span == 1 {
            layout.heights[cell.row] = layout.heights[cell.row].max(height);
        }
    }
    // A merged cell only widens the grid when its text does not fit the columns it spans.
    for cell in &table.cells {
        if (cell.column_span <= 1 && cell.row_span <= 1)
            || cell.column >= table.columns
            || cell.row >= table.rows
        {
            continue;
        }
        let size = measure(&cell.text, font_size(cell.text_size));
        let last = (cell.column + cell.column_span).min(table.columns) - 1;
        let last_row = (cell.row + cell.row_span).min(table.rows) - 1;
        let span_width: f32 = layout.widths[cell.column..=last].iter().sum();
        let span_height: f32 = layout.heights[cell.row..=last_row].iter().sum();
        layout.widths[last] += (size.x + CELL_PADDING.x - span_width).max(0.0);
        layout.heights[last_row] += (size.y + CELL_PADDING.y - span_height).max(0.0);
    }
    layout
}

/// Where a table's top-left corner goes for its `position`, inside `area`.
pub fn table_origin(size: Vec2, position: TablePosition, area: Rect) -> Pos2 {
    let x = match position.horizontal() {
        Horizontal::Left => area.min.x,
        Horizontal::Center => area.center().x - size.x / 2.0,
        Horizontal::Right => area.max.x - size.x,
    };
    let y = match position.vertical() {
        Vertical::Top => area.min.y,
        Vertical::Middle => area.center().y - size.y / 2.0,
        Vertical::Bottom => area.max.y - size.y,
    };
    Pos2::new(x, y)
}

/// The internal separators between columns and rows, minus the stretches that run through a
/// merged cell. Each is a segment `[from, to]`.
pub fn grid_lines(table: &TableOutput, layout: &TableLayout, frame: Rect) -> Vec<[Pos2; 2]> {
    let merged: Vec<&TableCell> = table
        .cells
        .iter()
        .filter(|cell| cell.column_span > 1 || cell.row_span > 1)
        .collect();
    let crosses_column = |boundary: usize, row: usize| {
        merged.iter().any(|cell| {
            cell.column < boundary
                && boundary < cell.column + cell.column_span
                && cell.row <= row
                && row < cell.row + cell.row_span
        })
    };
    let crosses_row = |boundary: usize, column: usize| {
        merged.iter().any(|cell| {
            cell.row < boundary
                && boundary < cell.row + cell.row_span
                && cell.column <= column
                && column < cell.column + cell.column_span
        })
    };
    let mut lines = Vec::new();
    let mut x = frame.min.x;
    for boundary in 1..layout.widths.len().max(1) {
        x += layout.widths[boundary - 1];
        let mut y = frame.min.y;
        for (row, height) in layout.heights.iter().enumerate() {
            if !crosses_column(boundary, row) {
                lines.push([Pos2::new(x, y), Pos2::new(x, y + height)]);
            }
            y += height;
        }
    }
    let mut y = frame.min.y;
    for boundary in 1..layout.heights.len().max(1) {
        y += layout.heights[boundary - 1];
        let mut x = frame.min.x;
        for (column, width) in layout.widths.iter().enumerate() {
            if !crosses_row(boundary, column) {
                lines.push([Pos2::new(x, y), Pos2::new(x + width, y)]);
            }
            x += width;
        }
    }
    lines
}

// MARK: Series alignment

/// Per-bar series are right-aligned with the candles: the last value belongs to the last candle.
/// The index into a series of `series_len` for the visible candle `candle_index`.
pub fn series_index(series_len: usize, candle_count: usize, candle_index: usize) -> Option<usize> {
    let index = series_len as isize - candle_count as isize + candle_index as isize;
    (index >= 0 && (index as usize) < series_len).then_some(index as usize)
}

/// The visible candle slot of an absolute Pine `bar_index` (drawing-object coordinates); may be
/// negative or past the last candle.
pub fn candle_index_for_bar(bar: i64, bar_count: usize, candle_count: usize) -> i64 {
    bar - (bar_count as i64 - candle_count as i64)
}

// MARK: Value range

/// Prices of plots, plotcandles and the baseline of filled styles that show in the pane.
fn series_values(pine: &VisualOutput, candle_count: usize) -> Vec<f64> {
    let mut values = Vec::new();
    for output in pine.plots.iter().filter(|o| o.display & DISPLAY_PANE != 0) {
        let skip = output.values.len().saturating_sub(candle_count);
        values.extend(output.values[skip..].iter().flatten().copied());
        if matches!(
            output.style,
            PlotStyle::Histogram | PlotStyle::Columns | PlotStyle::Area
        ) {
            values.push(output.hist_base);
        }
    }
    for output in pine
        .candles
        .iter()
        .filter(|o| o.display & DISPLAY_PANE != 0)
    {
        let skip = output.bars.len().saturating_sub(candle_count);
        for bar in output.bars[skip..].iter().flatten() {
            let (high, low) =
                crate::model::chart_look::drawn_extremes(bar.open, bar.high, bar.low, bar.close);
            values.extend([high, low]);
        }
    }
    values
}

/// Boxes, lines, labels and polylines that overlap the visible bars (and `future_bars` past them).
pub fn drawing_values(pine: &VisualOutput, candle_count: usize, future_bars: usize) -> Vec<f64> {
    if candle_count == 0 {
        return Vec::new();
    }
    let first = pine.bar_count as i64 - candle_count as i64;
    let last = pine.bar_count as i64 - 1 + future_bars as i64;
    let in_view = |a: i64, b: i64| a.min(b) <= last && a.max(b) >= first;
    let mut values = Vec::new();
    for item in pine
        .boxes
        .iter()
        .filter(|b| b.is_complete() && in_view(b.left, b.right))
    {
        values.extend([item.top, item.bottom]);
    }
    for item in pine
        .lines
        .iter()
        .filter(|l| l.is_complete() && in_view(l.x1, l.x2))
    {
        values.extend([item.y1, item.y2]);
    }
    for item in pine
        .labels
        .iter()
        .filter(|l| l.is_complete() && l.yloc == YLocation::Price && (first..=last).contains(&l.x))
    {
        values.push(item.y);
    }
    for polyline in &pine.polylines {
        let indexes = polyline.points.iter().map(|point| point.index);
        let (Some(low), Some(high)) = (indexes.clone().min(), indexes.max()) else {
            continue;
        };
        if index_is_known(low) && in_view(low, high) {
            values.extend(polyline.points.iter().map(|point| point.price));
        }
    }
    values
}

/// Vertical domain for a script pane: every value-anchored output in view, padded so nothing
/// touches the frame (`padding` is a share of the range, as for the chart's price range).
pub fn pane_value_range(pine: &VisualOutput, candle_count: usize, padding: f64) -> (f64, f64) {
    let mut values = series_values(pine, candle_count);
    values.extend(drawing_values(pine, candle_count, 0));
    values.extend(pine.hlines.iter().map(|line| line.value));
    values.retain(|value| value.is_finite());
    let (Some(low), Some(high)) = (
        values.iter().copied().reduce(f64::min),
        values.iter().copied().reduce(f64::max),
    ) else {
        return (0.0, 1.0);
    };
    if low == high {
        let inset = (low.abs() * 0.01).max(0.01);
        return (low - inset, high + inset);
    }
    let inset = (high - low) * padding;
    (low - inset, high + inset)
}

/// Farthest future slots the chart reserves, as a share of the candles shown, so a far-off
/// drawing never squeezes the candles into a sliver.
pub const MAXIMUM_FUTURE_SHARE: f64 = 1.0;

/// Slots past the last candle that this script's drawings reach, capped. Drawings placed by time
/// (`xloc.bar_time`) are mapped by the renderer and not counted.
pub fn future_bars(pine: &VisualOutput, candle_count: usize) -> usize {
    if candle_count == 0 {
        return 0;
    }
    let last_bar = pine.bar_count as i64 - 1;
    let mut bars: Vec<i64> = Vec::new();
    for item in pine
        .boxes
        .iter()
        .filter(|b| b.is_complete() && !b.time_anchored)
    {
        bars.extend([item.left, item.right]);
    }
    for item in pine
        .lines
        .iter()
        .filter(|l| l.is_complete() && !l.time_anchored)
    {
        bars.extend([item.x1, item.x2]);
    }
    for item in pine
        .labels
        .iter()
        .filter(|l| l.is_complete() && !l.time_anchored)
    {
        bars.push(item.x);
    }
    let reach = bars
        .into_iter()
        .filter(|bar| index_is_known(*bar))
        .max()
        .unwrap_or(last_bar);
    let cap = (candle_count as f64 * MAXIMUM_FUTURE_SHARE).floor() as i64;
    (reach - last_bar).clamp(0, cap) as usize
}

/// What overlay scripts add to the chart's own geometry.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OverlayExtent {
    pub future_bars: usize,
    pub values: Vec<f64>,
}

/// The extent of every overlay script in `outputs`: bar slots past the last candle for drawings
/// anchored beyond the latest bar, and prices the vertical scale has to reach. Scripts in their
/// own pane add nothing — they have a value axis of their own.
pub fn overlay_extent<'a>(
    outputs: impl IntoIterator<Item = &'a VisualOutput> + Clone,
    candle_count: usize,
) -> OverlayExtent {
    if candle_count == 0 {
        return OverlayExtent::default();
    }
    let mut extent = OverlayExtent::default();
    for output in outputs.clone().into_iter().filter(|o| o.overlay) {
        extent.future_bars = extent.future_bars.max(future_bars(output, candle_count));
    }
    for output in outputs.into_iter().filter(|o| o.overlay) {
        let mut values = series_values_in_pane(output, candle_count);
        values.extend(output.hlines.iter().map(|line| line.value));
        values.extend(drawing_values(output, candle_count, extent.future_bars));
        values.retain(|value| value.is_finite());
        extent.values.extend(values);
    }
    extent
}

/// Plot and plotcandle values shown in the main pane (no filled-style baselines, as in Swift).
fn series_values_in_pane(pine: &VisualOutput, candle_count: usize) -> Vec<f64> {
    let mut values = Vec::new();
    for output in pine.plots.iter().filter(|o| o.display & DISPLAY_PANE != 0) {
        let skip = output.values.len().saturating_sub(candle_count);
        values.extend(output.values[skip..].iter().flatten().copied());
    }
    for output in pine
        .candles
        .iter()
        .filter(|o| o.display & DISPLAY_PANE != 0)
    {
        let skip = output.bars.len().saturating_sub(candle_count);
        for bar in output.bars[skip..].iter().flatten() {
            let (high, low) =
                crate::model::chart_look::drawn_extremes(bar.open, bar.high, bar.low, bar.close);
            values.extend([high, low]);
        }
    }
    values
}

/// Whether a drawing coordinate was given (a missing bar index is `MISSING_INDEX`).
pub fn bar_is_known(bar: i64) -> bool {
    bar != MISSING_INDEX
}

pub fn box_is_drawable(item: &BoxOutput) -> bool {
    item.is_complete()
}

pub fn line_is_drawable(item: &LineOutput) -> bool {
    item.is_complete()
}

pub fn label_is_drawable(item: &LabelOutput) -> bool {
    item.is_complete()
}

pub fn polyline_is_drawable(item: &PolylineOutput) -> bool {
    item.points.len() > 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::outputs::{
        CandleBar, CandleOutput, HorizontalLine, PlotOutput, PolylinePoint,
    };

    fn plot(values: Vec<Option<f64>>, style: PlotStyle) -> PlotOutput {
        PlotOutput {
            id: 0,
            title: None,
            values,
            color: 0xFF00_00FF,
            line_width: 1,
            style,
            colors: Vec::new(),
            display: DISPLAY_PANE,
            hist_base: 0.0,
            offset: 0,
            force_overlay: false,
        }
    }

    fn line(x1: i64, y1: f64, x2: i64, y2: f64) -> LineOutput {
        LineOutput {
            id: 0,
            x1,
            y1,
            x2,
            y2,
            color: 0,
            width: 1,
            style: LineStyle::Solid,
            extend: LineExtend::None,
            time_anchored: false,
        }
    }

    #[test]
    fn colours_unpack_rgba_and_fade() {
        assert_eq!(
            rgba(0xFF80_00FF),
            Color32::from_rgba_unmultiplied(255, 128, 0, 255)
        );
        assert_eq!(
            rgba(0x0000_0000),
            Color32::from_rgba_unmultiplied(0, 0, 0, 0)
        );
        assert!(is_visible(0x1234_5601) && !is_visible(0x1234_5600));
        assert_eq!(faded(0xFF00_00FF, 0.5).a(), 128);
        assert_eq!(font_size(Size::Auto), 12.0);
        assert_eq!(font_size(Size::Huge), 20.0);
        assert_eq!(marker_radius(Size::Tiny), 3.0);
        assert_eq!(marker_radius(Size::Auto), 5.0);
    }

    #[test]
    fn line_styles_pick_dashes_and_arrowheads() {
        assert_eq!(dash_pattern(LineStyle::Dashed), [6.0, 4.0]);
        assert_eq!(dash_pattern(LineStyle::Dotted), [1.0, 3.0]);
        assert!(dash_pattern(LineStyle::ArrowBoth).is_empty());
        assert_eq!(arrowheads(LineStyle::ArrowLeft), (true, false));
        assert_eq!(arrowheads(LineStyle::ArrowBoth), (true, true));
        assert_eq!(arrowheads(LineStyle::Solid), (false, false));
    }

    #[test]
    fn an_arrowhead_points_along_the_line() {
        let head = arrowhead(Pos2::new(10.0, 0.0), Pos2::new(0.0, 0.0), 4.0, 2.0).unwrap();
        assert_eq!(head[0], Pos2::new(10.0, 0.0));
        assert_eq!(head[1], Pos2::new(6.0, 2.0));
        assert_eq!(head[2], Pos2::new(6.0, -2.0));
        assert_eq!(arrowhead(Pos2::ZERO, Pos2::ZERO, 4.0, 2.0), None);
    }

    #[test]
    fn extended_lines_follow_their_slope_to_the_plots_edges() {
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(100.0, 100.0));
        let (a, b) = (Pos2::new(40.0, 40.0), Pos2::new(60.0, 50.0));
        assert_eq!(extended_line(a, b, LineExtend::None, rect), (a, b));
        let (left, right) = extended_line(a, b, LineExtend::Both, rect);
        assert_eq!(left, Pos2::new(0.0, 20.0));
        assert_eq!(right, Pos2::new(100.0, 70.0));
        let (left, right) = extended_line(b, a, LineExtend::Right, rect);
        assert_eq!(
            (left, right),
            (a, Pos2::new(100.0, 70.0)),
            "order of the ends does not matter"
        );
        let vertical = (Pos2::new(5.0, 0.0), Pos2::new(5.0, 9.0));
        assert_eq!(
            extended_line(vertical.0, vertical.1, LineExtend::Both, rect),
            vertical
        );
    }

    #[test]
    fn straight_polylines_keep_their_points_and_curves_pass_through_them() {
        let points = [
            Pos2::new(0.0, 0.0),
            Pos2::new(10.0, 10.0),
            Pos2::new(20.0, 0.0),
        ];
        assert_eq!(polyline_points(&points, false, false), points.to_vec());
        assert_eq!(
            polyline_points(&points[..2], true, false),
            points[..2].to_vec()
        );
        let curve = polyline_points(&points, true, false);
        assert_eq!(curve.len(), 1 + 2 * 12);
        assert_eq!(curve[0], points[0]);
        assert_eq!(curve[12], points[1], "the spline passes through each point");
        assert_eq!(curve[24], points[2]);
        let closed = polyline_points(&points, true, true);
        assert_eq!(closed.len(), 1 + 3 * 12);
        assert!(
            (closed.last().unwrap().x - points[0].x).abs() < 1e-4,
            "wraps to the start"
        );
        assert!(polyline_points(&[], true, false).is_empty());
    }

    #[test]
    fn box_text_anchors_to_the_aligned_edge() {
        let rect = Rect::from_min_max(Pos2::new(10.0, 20.0), Pos2::new(110.0, 70.0));
        let (point, anchor) = box_text_placement(rect, TextAlign::Left, TextAlign::Top, 4.0);
        assert_eq!(point, Pos2::new(14.0, 24.0));
        assert_eq!(anchor, Align2::LEFT_TOP);
        let (point, anchor) = box_text_placement(rect, TextAlign::Right, TextAlign::Bottom, 4.0);
        assert_eq!(point, Pos2::new(106.0, 66.0));
        assert_eq!(anchor, Align2::RIGHT_BOTTOM);
        let (point, anchor) = box_text_placement(rect, TextAlign::Center, TextAlign::Center, 4.0);
        assert_eq!(point, rect.center());
        assert_eq!(anchor, Align2::CENTER_CENTER);
    }

    #[test]
    fn label_down_sits_above_its_anchor_with_a_pointer() {
        let anchor = Pos2::new(100.0, 100.0);
        let layout = label_layout(LabelStyle::LabelDown, anchor, Vec2::new(40.0, 20.0), 5.0);
        assert_eq!(layout.body.min, Pos2::new(80.0, 75.0));
        assert_eq!(layout.body.max, Pos2::new(120.0, 95.0));
        assert_eq!(
            layout.shape,
            LabelShape::Bubble {
                pointer: vec![Pos2::new(95.0, 95.0), anchor, Pos2::new(105.0, 95.0)]
            }
        );
        let up = label_layout(LabelStyle::LabelUp, anchor, Vec2::new(40.0, 20.0), 5.0);
        assert_eq!(up.body.min.y, 105.0, "label_up hangs below the anchor");
        let left = label_layout(LabelStyle::LabelLeft, anchor, Vec2::new(40.0, 20.0), 5.0);
        assert_eq!(left.body.min, Pos2::new(105.0, 90.0));
        let right = label_layout(LabelStyle::LabelRight, anchor, Vec2::new(40.0, 20.0), 5.0);
        assert_eq!(right.body.max.x, 95.0);
    }

    #[test]
    fn corner_labels_anchor_a_corner_of_the_bubble() {
        let anchor = Pos2::new(50.0, 50.0);
        let size = Vec2::new(30.0, 10.0);
        let upper_left = label_layout(LabelStyle::LabelUpperLeft, anchor, size, 4.0);
        assert_eq!(upper_left.body.min, anchor);
        assert_eq!(
            upper_left.shape,
            LabelShape::Bubble {
                pointer: vec![Pos2::new(54.0, 50.0), anchor, Pos2::new(50.0, 54.0)]
            }
        );
        let lower_right = label_layout(LabelStyle::LabelLowerRight, anchor, size, 4.0);
        assert_eq!(lower_right.body.max, anchor);
    }

    #[test]
    fn plain_shapes_centre_on_the_anchor() {
        let anchor = Pos2::new(50.0, 50.0);
        let size = Vec2::new(30.0, 40.0);
        let square = label_layout(LabelStyle::Square, anchor, size, 4.0);
        assert_eq!(square.body.center(), anchor);
        assert_eq!(square.shape, LabelShape::Rectangle);
        let circle = label_layout(LabelStyle::Circle, anchor, size, 4.0);
        assert_eq!(
            circle.body.width(),
            50.0,
            "the diameter is the text's diagonal"
        );
        let diamond = label_layout(LabelStyle::Diamond, anchor, size, 4.0);
        assert_eq!(diamond.body.size(), size * 2.0);
        assert_eq!(
            label_layout(LabelStyle::None, anchor, size, 4.0).shape,
            LabelShape::TextOnly
        );
        assert_eq!(
            label_layout(LabelStyle::LabelCenter, anchor, size, 4.0).shape,
            LabelShape::Bubble {
                pointer: Vec::new()
            }
        );
    }

    #[test]
    fn marker_labels_put_a_glyph_on_the_anchor_and_the_text_beside_it() {
        let anchor = Pos2::new(50.0, 50.0);
        let size = Vec2::new(30.0, 12.0);
        let down = label_layout(LabelStyle::TriangleDown, anchor, size, 4.0);
        assert_eq!(down.shape, LabelShape::Marker(LabelGlyph::TriangleDown));
        assert_eq!(
            down.glyph,
            Some(Rect::from_center_size(anchor, Vec2::splat(12.0)))
        );
        assert_eq!(down.body.max.y, 44.0, "text above the glyph");
        let up = label_layout(LabelStyle::ArrowUp, anchor, size, 4.0);
        assert_eq!(up.body.min.y, 56.0, "text below the glyph");
        assert_eq!(up.shape, LabelShape::Marker(LabelGlyph::ArrowUp));
    }

    fn table(columns: usize, rows: usize, cells: Vec<TableCell>) -> TableOutput {
        TableOutput {
            id: 0,
            position: TablePosition::TopRight,
            columns,
            rows,
            background_color: None,
            border_color: None,
            border_width: 0,
            frame_color: None,
            frame_width: 0,
            cells,
        }
    }

    fn cell(column: usize, row: usize, text: &str) -> TableCell {
        TableCell {
            column,
            row,
            text: text.into(),
            text_color: 0,
            background_color: None,
            text_size: Size::Normal,
            column_span: 1,
            row_span: 1,
            text_horizontal_align: TextAlign::Center,
            text_vertical_align: TextAlign::Center,
            width: 0.0,
            height: 0.0,
        }
    }

    fn measure(text: &str, font: f32) -> Vec2 {
        Vec2::new(text.chars().count() as f32 * font / 2.0, font)
    }

    #[test]
    fn a_table_grid_is_as_wide_as_its_widest_cell_in_each_column() {
        let t = table(
            2,
            2,
            vec![
                cell(0, 0, "ab"),
                cell(0, 1, "abcd"),
                cell(1, 0, "x"),
                cell(1, 1, ""),
            ],
        );
        let layout = measure_table(&t, Vec2::new(400.0, 300.0), &measure);
        assert_eq!(layout.widths, [24.0 + 12.0, 6.0 + 12.0]);
        assert_eq!(layout.heights, [12.0 + 6.0, 12.0 + 6.0]);
        assert_eq!(layout.size(), Vec2::new(54.0, 36.0));
        let wide = TableCell {
            width: 50.0,
            height: 10.0,
            ..cell(0, 0, "a")
        };
        let minimums = measure_table(&table(1, 1, vec![wide]), Vec2::new(400.0, 300.0), &measure);
        assert_eq!(
            (minimums.widths[0], minimums.heights[0]),
            (200.0, 30.0),
            "percent minimums"
        );
    }

    #[test]
    fn a_merged_cell_widens_only_the_last_column_it_spans_and_only_when_needed() {
        let merged = TableCell {
            column_span: 2,
            ..cell(0, 0, "a long merged heading")
        };
        let t = table(2, 1, vec![merged.clone()]);
        let layout = measure_table(&t, Vec2::new(400.0, 300.0), &measure);
        assert_eq!(layout.widths, [0.0, 21.0 * 6.0 + 12.0]);
        let roomy = table(2, 1, vec![merged, cell(0, 0, "x")]);
        let roomy_layout = measure_table(&roomy, Vec2::new(400.0, 300.0), &|_, _| {
            Vec2::new(10.0, 12.0)
        });
        assert_eq!(
            roomy_layout.widths,
            [22.0, 0.0 + 0.0],
            "text already fits: nothing widens"
        );
        let rect = layout.rect_of(&t.cells[0], Pos2::new(5.0, 5.0));
        assert_eq!(rect.width(), layout.widths.iter().sum::<f32>());
    }

    #[test]
    fn tables_pin_to_the_corner_or_edge_they_name() {
        let area = Rect::from_min_max(Pos2::new(8.0, 8.0), Pos2::new(208.0, 108.0));
        let size = Vec2::new(40.0, 20.0);
        assert_eq!(
            table_origin(size, TablePosition::TopLeft, area),
            Pos2::new(8.0, 8.0)
        );
        assert_eq!(
            table_origin(size, TablePosition::TopRight, area),
            Pos2::new(168.0, 8.0)
        );
        assert_eq!(
            table_origin(size, TablePosition::MiddleCenter, area),
            Pos2::new(88.0, 48.0)
        );
        assert_eq!(
            table_origin(size, TablePosition::BottomRight, area),
            Pos2::new(168.0, 88.0)
        );
    }

    #[test]
    fn grid_separators_skip_the_stretches_through_merged_cells() {
        let merged = TableCell {
            column_span: 2,
            ..cell(0, 0, "head")
        };
        let t = table(2, 2, vec![merged, cell(0, 1, "a"), cell(1, 1, "b")]);
        let layout = TableLayout {
            widths: vec![10.0, 10.0],
            heights: vec![5.0, 5.0],
        };
        let frame = Rect::from_min_size(Pos2::new(0.0, 0.0), layout.size());
        let lines = grid_lines(&t, &layout, frame);
        // The column boundary at x = 10 runs only through the second row; the row boundary at
        // y = 5 is complete (nothing merges across rows).
        assert!(lines.contains(&[Pos2::new(10.0, 5.0), Pos2::new(10.0, 10.0)]));
        assert!(!lines.contains(&[Pos2::new(10.0, 0.0), Pos2::new(10.0, 5.0)]));
        assert!(lines.contains(&[Pos2::new(0.0, 5.0), Pos2::new(10.0, 5.0)]));
        assert!(lines.contains(&[Pos2::new(10.0, 5.0), Pos2::new(20.0, 5.0)]));
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn series_line_up_on_the_right_edge_of_the_candles() {
        assert_eq!(series_index(5, 3, 0), Some(2));
        assert_eq!(series_index(5, 3, 2), Some(4));
        assert_eq!(series_index(2, 3, 0), None, "a short series starts later");
        assert_eq!(series_index(2, 3, 1), Some(0));
        assert_eq!(candle_index_for_bar(10, 11, 5), 4);
        assert_eq!(candle_index_for_bar(4, 11, 5), -2, "off the left edge");
        assert_eq!(candle_index_for_bar(12, 11, 5), 6, "past the last candle");
    }

    fn visual(bar_count: usize) -> VisualOutput {
        VisualOutput {
            overlay: true,
            bar_count,
            ..VisualOutput::default()
        }
    }

    #[test]
    fn a_pane_ranges_over_what_it_shows_with_padding() {
        let mut output = visual(10);
        output.plots.push(plot(
            vec![Some(10.0), None, Some(20.0), Some(15.0)],
            PlotStyle::Line,
        ));
        output.hlines.push(HorizontalLine {
            id: 0,
            value: 30.0,
            color: 0,
            title: None,
        });
        let (low, high) = pane_value_range(&output, 3, 0.1);
        // The plot's last three values (nan, 20, 15) plus the hline at 30.
        assert_eq!((low, high), (15.0 - 1.5, 30.0 + 1.5));
        assert_eq!(
            pane_value_range(&visual(3), 3, 0.1),
            (0.0, 1.0),
            "nothing to show"
        );
        let mut flat = visual(3);
        flat.plots.push(plot(vec![Some(100.0)], PlotStyle::Line));
        assert_eq!(pane_value_range(&flat, 3, 0.1), (99.0, 101.0));
        let mut histogram = visual(3);
        histogram
            .plots
            .push(plot(vec![Some(5.0), Some(8.0)], PlotStyle::Histogram));
        assert_eq!(
            pane_value_range(&histogram, 3, 0.0),
            (0.0, 8.0),
            "the baseline counts"
        );
    }

    #[test]
    fn plotcandles_extend_the_range_by_their_wicks() {
        let mut output = visual(3);
        output.candles.push(CandleOutput {
            id: 0,
            title: None,
            bars: vec![Some(CandleBar {
                open: 5.0,
                high: 9.0,
                low: 1.0,
                close: 6.0,
                color: None,
                wick_color: None,
                border_color: None,
            })],
            display: DISPLAY_PANE,
            bar_style: false,
            editable: true,
            show_last: None,
            force_overlay: false,
        });
        assert_eq!(pane_value_range(&output, 3, 0.0), (1.0, 9.0));
    }

    #[test]
    fn only_drawings_that_overlap_the_visible_bars_count() {
        let mut output = visual(100);
        output.lines.push(line(95, 10.0, 99, 20.0));
        output.lines.push(line(10, 500.0, 20, 600.0));
        output.lines.push(line(98, 40.0, MISSING_INDEX, f64::NAN));
        let values = drawing_values(&output, 10, 0);
        assert_eq!(values, [10.0, 20.0]);
        output.polylines.push(PolylineOutput {
            id: 0,
            points: vec![
                PolylinePoint {
                    index: 96,
                    price: 7.0,
                },
                PolylinePoint {
                    index: 105,
                    price: 8.0,
                },
            ],
            closed: false,
            line_color: None,
            fill_color: None,
            style: LineStyle::Solid,
            width: 1,
            curved: false,
        });
        assert_eq!(
            drawing_values(&output, 10, 0),
            [10.0, 20.0, 7.0, 8.0].to_vec()
        );
        output.lines.push(line(104, 70.0, 108, 80.0));
        assert!(!drawing_values(&output, 10, 0).contains(&70.0));
        assert!(
            drawing_values(&output, 10, 6).contains(&70.0),
            "future bars widen the window"
        );
        assert!(drawing_values(&output, 0, 0).is_empty());
    }

    #[test]
    fn future_bars_are_capped_at_the_share_of_candles_shown() {
        let mut output = visual(100);
        output.lines.push(line(90, 1.0, 104, 2.0));
        assert_eq!(future_bars(&output, 50), 5);
        output.lines.push(line(90, 1.0, 400, 2.0));
        assert_eq!(future_bars(&output, 50), 50, "capped at one screen");
        let mut anchored_by_time = visual(100);
        anchored_by_time.lines.push(LineOutput {
            time_anchored: true,
            ..line(0, 1.0, 400, 2.0)
        });
        assert_eq!(
            future_bars(&anchored_by_time, 50),
            0,
            "time-anchored drawings are mapped later"
        );
        assert_eq!(future_bars(&visual(100), 50), 0);
        assert_eq!(future_bars(&output, 0), 0);
    }

    #[test]
    fn overlay_extent_covers_only_overlay_scripts() {
        let mut overlay = visual(10);
        overlay
            .plots
            .push(plot(vec![Some(50.0), Some(60.0)], PlotStyle::Histogram));
        overlay.lines.push(line(8, 40.0, 12, 70.0));
        let mut pane = VisualOutput {
            overlay: false,
            ..visual(10)
        };
        pane.plots.push(plot(vec![Some(1_000.0)], PlotStyle::Line));
        let extent = overlay_extent(&[overlay, pane], 5);
        assert_eq!(extent.future_bars, 3);
        assert!(extent.values.contains(&40.0) && extent.values.contains(&70.0));
        assert!(extent.values.contains(&60.0));
        assert!(
            !extent.values.contains(&1_000.0),
            "a script in its own pane adds nothing"
        );
        assert!(
            !extent.values.contains(&0.0),
            "the histogram baseline is not an overlay value"
        );
        assert_eq!(overlay_extent(&[], 5), OverlayExtent::default());
        assert!(bar_is_known(3) && !bar_is_known(MISSING_INDEX));
    }
}
