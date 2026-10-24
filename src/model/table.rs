//! The Table tool's data and every edit that can be made to it. Pure: no egui, no projection.
//!
//! A table is a grid of text cells with a width for each column and a height for each row, in
//! logical pixels like a font size (a table does not stretch with the chart). It is placed one of
//! two ways, as TradingView's is: at a chart point, so it moves with the bars, or pinned to a
//! place on the screen, so it stays put however the chart is scrolled. The pinned place is a pair
//! of fractions of the plot, never pixels, so it survives a resized pane, another display and a
//! restore from another Mac. A pinned table keeps its chart point too, so unpinning has a sensible
//! place to return to.
//!
//! Every operation keeps the grid whole (as many cells as rows times columns, every size inside
//! its limits) and refuses what would break the limits, so no sequence of edits, pastes or
//! undecodable rows can corrupt a table or make one without bounds.

use super::drawings::Anchor;
use serde::{Deserialize, Serialize};

/// The most rows and columns a table may have, and the most characters in one cell. They bound the
/// memory and the layout work of a table, and keep a synced record far below its size limit.
pub const MAX_ROWS: usize = 30;
pub const MAX_COLUMNS: usize = 12;
pub const MAX_CELL_CHARS: usize = 500;

/// The range of a column's width and of a row's height, in logical pixels.
pub const COLUMN_WIDTH_RANGE: (f32, f32) = (24.0, 800.0);
pub const ROW_HEIGHT_RANGE: (f32, f32) = (18.0, 400.0);

/// What a new table starts as.
pub const DEFAULT_ROWS: usize = 3;
pub const DEFAULT_COLUMNS: usize = 3;
pub const DEFAULT_COLUMN_WIDTH: f32 = 96.0;
pub const DEFAULT_ROW_HEIGHT: f32 = 30.0;

/// Where a pinned table's top-left corner sits: fractions of the plot's width and height from its
/// top-left corner.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct ScreenAnchor {
    pub x: f32,
    pub y: f32,
}

impl ScreenAnchor {
    /// The anchor brought into the plot: a table whose corner is off it could not be found again.
    pub fn clamped(self) -> Self {
        Self {
            x: self.x.clamp(0.0, 1.0),
            y: self.y.clamp(0.0, 1.0),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TableData {
    /// The chart point of the table's top-left corner. Where it is drawn unless it is pinned.
    pub at: Anchor,
    /// The width of each column.
    pub columns: Vec<f32>,
    /// The height each row asks for (a row grows past it to hold wrapped text).
    pub rows: Vec<f32>,
    /// The text of every cell, row by row.
    pub cells: Vec<String>,
    /// Pinned to the screen when set; the chart point is then not where it is drawn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen: Option<ScreenAnchor>,
}

fn clamp_size(value: f32, (low, high): (f32, f32), fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(low, high)
    } else {
        fallback
    }
}

fn cut(text: &str) -> String {
    text.chars().take(MAX_CELL_CHARS).collect()
}

impl TableData {
    /// A new table of the default size with its top-left corner at `at`.
    pub fn new(at: Anchor) -> Self {
        Self {
            at,
            columns: vec![DEFAULT_COLUMN_WIDTH; DEFAULT_COLUMNS],
            rows: vec![DEFAULT_ROW_HEIGHT; DEFAULT_ROWS],
            cells: vec![String::new(); DEFAULT_ROWS * DEFAULT_COLUMNS],
            screen: None,
        }
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    pub fn column_count(&self) -> usize {
        self.columns.len()
    }

    /// The text of the cell at `row`, `column`; empty outside the grid.
    pub fn cell(&self, row: usize, column: usize) -> &str {
        if row < self.row_count() && column < self.column_count() {
            self.cells
                .get(row * self.column_count() + column)
                .map_or("", String::as_str)
        } else {
            ""
        }
    }

    /// Sets a cell's text (cut to the limit). `false` for a cell outside the grid.
    pub fn set_cell(&mut self, row: usize, column: usize, text: &str) -> bool {
        if row >= self.row_count() || column >= self.column_count() {
            return false;
        }
        let index = row * self.column_count() + column;
        self.cells[index] = cut(text);
        true
    }

    /// The table's stored width and height: the sum of its columns and of its rows.
    pub fn size(&self) -> (f32, f32) {
        (self.columns.iter().sum(), self.rows.iter().sum())
    }

    /// A row after row `after` (at the end for `None`), as tall as the row it follows. `false` at
    /// the limit.
    pub fn insert_row(&mut self, after: Option<usize>) -> bool {
        if self.row_count() >= MAX_ROWS {
            return false;
        }
        let at = after.map_or(self.row_count(), |row| (row + 1).min(self.row_count()));
        let height = after
            .and_then(|row| self.rows.get(row))
            .or_else(|| self.rows.last())
            .copied()
            .unwrap_or(DEFAULT_ROW_HEIGHT);
        let width = self.column_count();
        self.rows.insert(at, height);
        let start = at * width;
        self.cells
            .splice(start..start, std::iter::repeat_n(String::new(), width));
        true
    }

    /// A column after column `after` (at the end for `None`), as wide as the one it follows.
    /// `false` at the limit.
    pub fn insert_column(&mut self, after: Option<usize>) -> bool {
        if self.column_count() >= MAX_COLUMNS {
            return false;
        }
        let at = after.map_or(self.column_count(), |column| {
            (column + 1).min(self.column_count())
        });
        let width = after
            .and_then(|column| self.columns.get(column))
            .or_else(|| self.columns.last())
            .copied()
            .unwrap_or(DEFAULT_COLUMN_WIDTH);
        let old = self.column_count();
        self.columns.insert(at, width);
        // Rows are stored one after the other, so each grows by one, from the last backwards.
        for row in (0..self.row_count()).rev() {
            self.cells.insert(row * old + at, String::new());
        }
        true
    }

    /// Removes a row. `false` for a row that is not there or the only one left.
    pub fn delete_row(&mut self, row: usize) -> bool {
        if row >= self.row_count() || self.row_count() == 1 {
            return false;
        }
        let width = self.column_count();
        self.rows.remove(row);
        self.cells.drain(row * width..(row + 1) * width);
        true
    }

    /// Removes a column. `false` for a column that is not there or the only one left.
    pub fn delete_column(&mut self, column: usize) -> bool {
        if column >= self.column_count() || self.column_count() == 1 {
            return false;
        }
        let old = self.column_count();
        self.columns.remove(column);
        for row in (0..self.row_count()).rev() {
            self.cells.remove(row * old + column);
        }
        true
    }

    /// Sets a column's width, held to its range. `false` for a column that is not there.
    pub fn resize_column(&mut self, column: usize, width: f32) -> bool {
        let Some(slot) = self.columns.get_mut(column) else {
            return false;
        };
        *slot = clamp_size(width, COLUMN_WIDTH_RANGE, *slot);
        true
    }

    /// Sets the height a row asks for, held to its range. `false` for a row that is not there.
    pub fn resize_row(&mut self, row: usize, height: f32) -> bool {
        let Some(slot) = self.rows.get_mut(row) else {
            return false;
        };
        *slot = clamp_size(height, ROW_HEIGHT_RANGE, *slot);
        true
    }

    /// Resizes the whole table to about `width` by `height`, every column and row by the same
    /// factor, each held to its range.
    pub fn scale_to(&mut self, width: f32, height: f32) {
        let (now_width, now_height) = self.size();
        if !(width.is_finite() && height.is_finite()) || now_width <= 0.0 || now_height <= 0.0 {
            return;
        }
        let (fx, fy) = (width / now_width, height / now_height);
        for column in &mut self.columns {
            *column = clamp_size(*column * fx, COLUMN_WIDTH_RANGE, *column);
        }
        for row in &mut self.rows {
            *row = clamp_size(*row * fy, ROW_HEIGHT_RANGE, *row);
        }
    }

    /// Writes pasted `text` into the grid with its first cell at `row`, `column`: tabs separate
    /// the cells of a line, line breaks the lines (the way a spreadsheet copies). The table grows
    /// by rows and columns as the text needs, up to the limits, and what does not fit is left out
    /// rather than breaking the grid. Returns how many cells were written.
    pub fn paste(&mut self, row: usize, column: usize, text: &str) -> usize {
        let lines: Vec<&str> = text
            .trim_end_matches(['\n', '\r'])
            .split('\n')
            .map(|line| line.trim_end_matches('\r'))
            .collect();
        let mut written = 0;
        for (dy, line) in lines.iter().enumerate() {
            for (dx, value) in line.split('\t').enumerate() {
                let (r, c) = (row + dy, column + dx);
                if r >= MAX_ROWS || c >= MAX_COLUMNS {
                    continue;
                }
                while self.row_count() <= r {
                    self.insert_row(None);
                }
                while self.column_count() <= c {
                    self.insert_column(None);
                }
                if self.set_cell(r, c, value) {
                    written += 1;
                }
            }
        }
        written
    }

    /// Whether this is a table a tool could have made: a whole grid with every size and every
    /// cell inside its limits and finite coordinates.
    pub fn is_valid(&self) -> bool {
        let (rows, columns) = (self.row_count(), self.column_count());
        (1..=MAX_ROWS).contains(&rows)
            && (1..=MAX_COLUMNS).contains(&columns)
            && self.cells.len() == rows * columns
            && self.at.is_finite()
            && self
                .columns
                .iter()
                .all(|w| w.is_finite() && (COLUMN_WIDTH_RANGE.0..=COLUMN_WIDTH_RANGE.1).contains(w))
            && self
                .rows
                .iter()
                .all(|h| h.is_finite() && (ROW_HEIGHT_RANGE.0..=ROW_HEIGHT_RANGE.1).contains(h))
            && self
                .cells
                .iter()
                .all(|cell| cell.chars().count() <= MAX_CELL_CHARS)
            && self
                .screen
                .is_none_or(|anchor| anchor.x.is_finite() && anchor.y.is_finite())
    }

    /// A short description for lists: "3 × 4 table".
    pub fn summary(&self) -> String {
        format!("{} × {} table", self.row_count(), self.column_count())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at() -> Anchor {
        Anchor {
            time_ms: 1_000,
            price: 50.0,
        }
    }

    /// A table whose cells say where they are, so a shuffle shows.
    fn numbered(rows: usize, columns: usize) -> TableData {
        let mut table = TableData::new(at());
        table.rows = vec![DEFAULT_ROW_HEIGHT; rows];
        table.columns = vec![DEFAULT_COLUMN_WIDTH; columns];
        table.cells = (0..rows * columns).map(|i| format!("{}", i)).collect();
        table
    }

    fn rows_of(table: &TableData) -> Vec<Vec<String>> {
        (0..table.row_count())
            .map(|r| {
                (0..table.column_count())
                    .map(|c| table.cell(r, c).to_owned())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn a_new_table_is_a_whole_default_grid() {
        let table = TableData::new(at());
        assert_eq!((table.row_count(), table.column_count()), (3, 3));
        assert_eq!(table.cells.len(), 9);
        assert!(table.is_valid());
        assert_eq!(table.screen, None);
        assert_eq!(
            table.size(),
            (3.0 * DEFAULT_COLUMN_WIDTH, 3.0 * DEFAULT_ROW_HEIGHT)
        );
        assert_eq!(table.summary(), "3 × 3 table");
    }

    #[test]
    fn a_cell_is_set_read_and_cut_to_its_limit() {
        let mut table = TableData::new(at());
        assert!(table.set_cell(1, 2, "hello"));
        assert_eq!(table.cell(1, 2), "hello");
        assert_eq!(table.cell(0, 0), "");
        assert!(table.set_cell(0, 0, &"é".repeat(MAX_CELL_CHARS + 40)));
        assert_eq!(table.cell(0, 0).chars().count(), MAX_CELL_CHARS);
        // Outside the grid there is nothing to set or read.
        assert!(!table.set_cell(3, 0, "x") && !table.set_cell(0, 3, "x"));
        assert_eq!(table.cell(9, 9), "");
        assert!(table.is_valid());
        // Multi-line, multi-byte text is kept as it is.
        assert!(table.set_cell(2, 2, "line one\nligne deux ✓ 日本語"));
        assert_eq!(table.cell(2, 2), "line one\nligne deux ✓ 日本語");
    }

    #[test]
    fn a_row_is_inserted_after_the_selected_one_or_at_the_end() {
        let mut table = numbered(2, 3);
        assert!(table.insert_row(Some(0)));
        assert_eq!(
            rows_of(&table),
            [["0", "1", "2"], ["", "", ""], ["3", "4", "5"]]
        );
        assert!(table.insert_row(None));
        assert_eq!(table.row_count(), 4);
        assert_eq!(rows_of(&table)[3], ["", "", ""]);
        assert_eq!(rows_of(&table)[2], ["3", "4", "5"]);
        // The new row is as tall as the one it follows.
        table.resize_row(0, 60.0);
        assert!(table.insert_row(Some(0)));
        assert_eq!(table.rows[1], 60.0);
        assert!(table.is_valid());
    }

    #[test]
    fn a_column_is_inserted_after_the_selected_one_or_at_the_end() {
        let mut table = numbered(2, 2);
        table.resize_column(0, 150.0);
        assert!(table.insert_column(Some(0)));
        assert_eq!(rows_of(&table), [["0", "", "1"], ["2", "", "3"]]);
        assert_eq!(table.columns, [150.0, 150.0, DEFAULT_COLUMN_WIDTH]);
        assert!(table.insert_column(None));
        assert_eq!(rows_of(&table), [["0", "", "1", ""], ["2", "", "3", ""]]);
        assert!(table.is_valid());
    }

    #[test]
    fn rows_and_columns_are_deleted_with_their_cells_and_the_last_one_stays() {
        let mut table = numbered(3, 3);
        assert!(table.delete_row(1));
        assert_eq!(rows_of(&table), [["0", "1", "2"], ["6", "7", "8"]]);
        assert!(table.delete_column(0));
        assert_eq!(rows_of(&table), [["1", "2"], ["7", "8"]]);
        assert!(!table.delete_row(5) && !table.delete_column(5));
        assert!(table.delete_row(0));
        assert!(!table.delete_row(0), "the last row stays");
        assert!(table.delete_column(1));
        assert!(!table.delete_column(0), "the last column stays");
        assert_eq!(rows_of(&table), [["7"]]);
        assert!(table.is_valid());
    }

    #[test]
    fn the_grid_stops_growing_at_its_limits() {
        let mut table = TableData::new(at());
        while table.insert_row(None) {}
        while table.insert_column(None) {}
        assert_eq!(
            (table.row_count(), table.column_count()),
            (MAX_ROWS, MAX_COLUMNS)
        );
        assert_eq!(table.cells.len(), MAX_ROWS * MAX_COLUMNS);
        assert!(!table.insert_row(Some(0)) && !table.insert_column(Some(0)));
        assert!(table.is_valid());
    }

    #[test]
    fn column_widths_and_row_heights_are_held_to_their_ranges() {
        let mut table = TableData::new(at());
        assert!(table.resize_column(1, 5.0));
        assert_eq!(table.columns[1], COLUMN_WIDTH_RANGE.0);
        assert!(table.resize_column(1, 1.0e9));
        assert_eq!(table.columns[1], COLUMN_WIDTH_RANGE.1);
        assert!(table.resize_column(1, f32::NAN));
        assert_eq!(
            table.columns[1], COLUMN_WIDTH_RANGE.1,
            "NaN changes nothing"
        );
        assert!(table.resize_row(2, -4.0));
        assert_eq!(table.rows[2], ROW_HEIGHT_RANGE.0);
        assert!(!table.resize_column(9, 50.0) && !table.resize_row(9, 50.0));
        assert!(table.is_valid());
    }

    #[test]
    fn the_whole_table_scales_by_one_factor_and_stays_in_range() {
        let mut table = TableData::new(at());
        let (width, height) = table.size();
        table.scale_to(width * 2.0, height * 1.5);
        assert!((table.size().0 - width * 2.0).abs() < 0.01);
        assert!((table.size().1 - height * 1.5).abs() < 0.01);
        table.scale_to(1.0, 1.0);
        assert!(table.columns.iter().all(|w| *w == COLUMN_WIDTH_RANGE.0));
        assert!(table.rows.iter().all(|h| *h == ROW_HEIGHT_RANGE.0));
        let before = table.clone();
        table.scale_to(f32::NAN, 100.0);
        assert_eq!(table, before);
        assert!(table.is_valid());
    }

    #[test]
    fn pasted_text_fills_cells_from_the_selected_one_and_grows_the_grid_within_its_limits() {
        let mut table = TableData::new(at());
        let written = table.paste(1, 1, "a\tb\tc\r\nd\te\tf\n");
        assert_eq!(written, 6);
        assert_eq!(table.row_count(), 3);
        assert_eq!(
            table.column_count(),
            4,
            "one column more for the third value"
        );
        assert_eq!(table.cell(1, 1), "a");
        assert_eq!(table.cell(1, 3), "c");
        assert_eq!(table.cell(2, 3), "f");
        // A single value is one cell, with no structure of its own.
        assert_eq!(table.paste(0, 0, "just one"), 1);
        assert_eq!(table.cell(0, 0), "just one");
        // More than the limits hold is dropped, never wrapped round or past the end.
        let big = (0..40)
            .map(|r| {
                (0..20)
                    .map(|c| format!("{r}.{c}"))
                    .collect::<Vec<_>>()
                    .join("\t")
            })
            .collect::<Vec<_>>()
            .join("\n");
        let mut table = TableData::new(at());
        let written = table.paste(0, 0, &big);
        assert_eq!(written, MAX_ROWS * MAX_COLUMNS);
        assert_eq!(
            (table.row_count(), table.column_count()),
            (MAX_ROWS, MAX_COLUMNS)
        );
        assert_eq!(table.cell(29, 11), "29.11");
        assert!(table.is_valid());
        // Pasting starting past the end of the grid writes nothing.
        assert_eq!(table.paste(MAX_ROWS, 0, "x"), 0);
    }

    #[test]
    fn a_table_is_only_valid_whole_and_inside_its_limits() {
        let good = TableData::new(at());
        let mut bad = good.clone();
        bad.cells.pop();
        assert!(!bad.is_valid(), "a missing cell");
        let mut bad = good.clone();
        bad.columns[0] = f32::NAN;
        assert!(!bad.is_valid());
        let mut bad = good.clone();
        bad.rows[0] = 1.0e6;
        assert!(!bad.is_valid());
        let mut bad = good.clone();
        bad.cells[0] = "x".repeat(MAX_CELL_CHARS + 1);
        assert!(!bad.is_valid());
        let mut bad = good.clone();
        bad.columns = vec![50.0; MAX_COLUMNS + 1];
        bad.cells = vec![String::new(); (MAX_COLUMNS + 1) * 3];
        assert!(!bad.is_valid(), "too many columns");
        let mut bad = good.clone();
        bad.rows.clear();
        bad.cells.clear();
        assert!(!bad.is_valid(), "no rows");
        let mut bad = good.clone();
        bad.at.price = f64::INFINITY;
        assert!(!bad.is_valid());
        let mut bad = good.clone();
        bad.screen = Some(ScreenAnchor {
            x: f32::NAN,
            y: 0.0,
        });
        assert!(!bad.is_valid());
    }

    #[test]
    fn a_table_round_trips_and_a_chart_table_stores_no_screen_place() {
        let mut table = TableData::new(at());
        table.set_cell(0, 1, "Entry");
        let json = serde_json::to_value(&table).unwrap();
        assert!(json.get("screen").is_none());
        assert_eq!(serde_json::from_value::<TableData>(json).unwrap(), table);
        table.screen = Some(ScreenAnchor { x: 0.25, y: 0.75 });
        let json = serde_json::to_value(&table).unwrap();
        assert_eq!(json["screen"]["x"], 0.25);
        assert_eq!(serde_json::from_value::<TableData>(json).unwrap(), table);
        assert_eq!(
            ScreenAnchor { x: -1.0, y: 3.0 }.clamped(),
            ScreenAnchor { x: 0.0, y: 1.0 }
        );
    }
}
