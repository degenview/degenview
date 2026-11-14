//! Tables: `table.new`, `table.cell`, the `table.cell_set_*` setters, `table.merge_cells` and
//! `table.clear`.

use super::names::size_of;
use super::{Arg, OPAQUE_BLACK, allocate, bind, count, parse, runtime_error, width_or};
use crate::pine::outputs::{Size, TableCell, TableOutput, TablePosition, TextAlign};
use crate::pine::runtime::state::RuntimeState;
use crate::pine::runtime::value::{RefKind, RuntimeValue, color_value};
use crate::pine::syntax::token::{Diagnostic, SourceRange};

pub(super) fn new_table(
    args: &[Arg],
    state: &mut RuntimeState,
) -> Result<RuntimeValue, Diagnostic> {
    let b = bind(
        args,
        &[
            "position",
            "columns",
            "rows",
            "bgcolor",
            "frame_color",
            "frame_width",
            "border_color",
            "border_width",
        ],
    );
    let id = allocate(state);
    state.tables.insert(
        id,
        TableOutput {
            id,
            position: parse(
                b.get("position").and_then(RuntimeValue::text_value),
                TablePosition::TopRight,
                TablePosition::TopRight,
            ),
            columns: count(b.get("columns")),
            rows: count(b.get("rows")),
            background_color: color_value(b.get("bgcolor"), None),
            border_color: color_value(b.get("border_color"), None),
            border_width: width_or(b.get("border_width"), 0),
            frame_color: color_value(b.get("frame_color"), None),
            frame_width: width_or(b.get("frame_width"), 0),
            cells: Vec::new(),
        },
    );
    Ok(RuntimeValue::Ref(RefKind::Table, id))
}

pub(super) fn set_table_cell(
    args: &[Arg],
    state: &mut RuntimeState,
    range: SourceRange,
) -> Result<RuntimeValue, Diagnostic> {
    let b = bind(
        args,
        &[
            "table_id",
            "column",
            "row",
            "text",
            "width",
            "height",
            "text_color",
            "text_halign",
            "text_valign",
            "text_size",
            "bgcolor",
            "tooltip",
            "text_font_family",
            "text_formatting",
        ],
    );
    let Some(RuntimeValue::Ref(RefKind::Table, id)) = b.get("table_id") else {
        return Ok(RuntimeValue::Void);
    };
    let Some(mut table) = state.tables.get(id).cloned() else {
        return Ok(RuntimeValue::Void);
    };
    let (Some(column), Some(row)) = (
        b.get("column").and_then(RuntimeValue::int_value),
        b.get("row").and_then(RuntimeValue::int_value),
    ) else {
        return Ok(RuntimeValue::Void);
    };
    if column < 0 || row < 0 || column as usize >= table.columns || row as usize >= table.rows {
        return Err(runtime_error(
            "PINE4013",
            format!(
                "table.cell position is outside the table's {}×{} grid.",
                table.columns, table.rows
            ),
            range,
        ));
    }
    table
        .cells
        .retain(|cell| cell.column != column as usize || cell.row != row as usize);
    let mut cell = blank_cell(column as usize, row as usize);
    for property in CELL_PROPERTIES {
        if let Some(value) = b.get(property) {
            set_cell_property(&mut cell, property, value);
        }
    }
    table.cells.push(cell);
    state.tables.insert(*id, table);
    Ok(RuntimeValue::Void)
}

/// The cell properties `table.cell` takes, by argument name; `table.cell_set_<name>` changes one.
const CELL_PROPERTIES: [&str; 9] = [
    "text",
    "width",
    "height",
    "text_color",
    "text_halign",
    "text_valign",
    "text_size",
    "bgcolor",
    "tooltip",
];

/// A cell `table.cell` was given nothing for.
fn blank_cell(column: usize, row: usize) -> TableCell {
    TableCell {
        column,
        row,
        text: String::new(),
        text_color: OPAQUE_BLACK,
        background_color: None,
        text_size: Size::Normal,
        column_span: 1,
        row_span: 1,
        text_horizontal_align: TextAlign::Center,
        text_vertical_align: TextAlign::Center,
        width: 0.0,
        height: 0.0,
        tooltip: None,
    }
}

/// Sets one property of a cell from a script value; `na` gives the property's default.
fn set_cell_property(cell: &mut TableCell, property: &str, value: &RuntimeValue) {
    let size = |value: &RuntimeValue| value.number().unwrap_or(0.0).max(0.0);
    match property {
        "text" => cell.text = value.text_value().unwrap_or_default().to_owned(),
        "width" => cell.width = size(value),
        "height" => cell.height = size(value),
        "text_color" => {
            cell.text_color = color_value(Some(value), Some(OPAQUE_BLACK)).unwrap_or(0);
        }
        "bgcolor" => cell.background_color = color_value(Some(value), None),
        "text_halign" => {
            cell.text_horizontal_align =
                parse(value.text_value(), TextAlign::Center, TextAlign::Center);
        }
        "text_valign" => {
            cell.text_vertical_align =
                parse(value.text_value(), TextAlign::Center, TextAlign::Center);
        }
        "text_size" => cell.text_size = size_of(Some(value), Size::Normal),
        "tooltip" => {
            cell.tooltip = value
                .text_value()
                .filter(|text| !text.is_empty())
                .map(str::to_owned);
        }
        _ => {}
    }
}

/// `table.cell_set_text`, `cell_set_bgcolor` and the other `table.cell_set_*` setters:
/// `(table_id, column, row, value)`. A cell `table.cell` has not defined yet is created blank first,
/// so the setter does not lose what it was given. `cell_set_text_font_family` and
/// `cell_set_text_formatting` change nothing the chart draws; a script is warned at compile time.
pub(super) fn set_cell_setter(
    member: &str,
    args: &[Arg],
    state: &mut RuntimeState,
    range: SourceRange,
) -> Result<RuntimeValue, Diagnostic> {
    let property = member.strip_prefix("cell_set_").unwrap_or(member);
    let b = bind(args, &["table_id", "column", "row", property]);
    let Some(RuntimeValue::Ref(RefKind::Table, id)) = b.get("table_id") else {
        return Ok(RuntimeValue::Void);
    };
    let Some(table) = state.tables.get_mut(id) else {
        return Ok(RuntimeValue::Void);
    };
    let (Some(column), Some(row)) = (
        b.get("column").and_then(RuntimeValue::int_value),
        b.get("row").and_then(RuntimeValue::int_value),
    ) else {
        return Ok(RuntimeValue::Void);
    };
    if column < 0 || row < 0 || column as usize >= table.columns || row as usize >= table.rows {
        return Err(runtime_error(
            "PINE4013",
            format!(
                "table.{member} position is outside the table's {}×{} grid.",
                table.columns, table.rows
            ),
            range,
        ));
    }
    let Some(value) = b.get(property) else {
        return Ok(RuntimeValue::Void);
    };
    if !CELL_PROPERTIES.contains(&property) {
        return Ok(RuntimeValue::Void);
    }
    let at = table
        .cells
        .iter()
        .position(|cell| cell.column == column as usize && cell.row == row as usize);
    let at = at.unwrap_or_else(|| {
        table.cells.push(blank_cell(column as usize, row as usize));
        table.cells.len() - 1
    });
    set_cell_property(&mut table.cells[at], property, value);
    Ok(RuntimeValue::Void)
}

pub(super) fn merge_table_cells(
    args: &[Arg],
    state: &mut RuntimeState,
    range: SourceRange,
) -> Result<RuntimeValue, Diagnostic> {
    let b = bind(
        args,
        &[
            "table_id",
            "start_column",
            "start_row",
            "end_column",
            "end_row",
        ],
    );
    let Some(RuntimeValue::Ref(RefKind::Table, id)) = b.get("table_id") else {
        return Ok(RuntimeValue::Void);
    };
    let Some(mut table) = state.tables.get(id).cloned() else {
        return Ok(RuntimeValue::Void);
    };
    let (Some(start_column), Some(start_row), Some(end_column), Some(end_row)) = (
        b.get("start_column").and_then(RuntimeValue::int_value),
        b.get("start_row").and_then(RuntimeValue::int_value),
        b.get("end_column").and_then(RuntimeValue::int_value),
        b.get("end_row").and_then(RuntimeValue::int_value),
    ) else {
        return Ok(RuntimeValue::Void);
    };
    let in_grid = start_column >= 0
        && start_row >= 0
        && (start_column as usize) < table.columns
        && (start_row as usize) < table.rows
        && end_column >= start_column
        && (end_column as usize) < table.columns
        && end_row >= start_row
        && (end_row as usize) < table.rows;
    if !in_grid {
        return Err(runtime_error(
            "PINE4013",
            format!(
                "table.merge_cells range is outside the table's {}×{} grid.",
                table.columns, table.rows
            ),
            range,
        ));
    }
    let mut anchor = table
        .cells
        .iter()
        .find(|cell| cell.column == start_column as usize && cell.row == start_row as usize)
        .cloned()
        .unwrap_or_else(|| TableCell {
            column: start_column as usize,
            row: start_row as usize,
            text: String::new(),
            text_color: OPAQUE_BLACK,
            background_color: None,
            text_size: Size::Normal,
            column_span: 1,
            row_span: 1,
            text_horizontal_align: TextAlign::Center,
            text_vertical_align: TextAlign::Center,
            width: 0.0,
            height: 0.0,
            tooltip: None,
        });
    anchor.column_span = (end_column - start_column + 1) as usize;
    anchor.row_span = (end_row - start_row + 1) as usize;
    table.cells.retain(|cell| {
        !((start_column..=end_column).contains(&(cell.column as i64))
            && (start_row..=end_row).contains(&(cell.row as i64)))
    });
    table.cells.push(anchor);
    state.tables.insert(*id, table);
    Ok(RuntimeValue::Void)
}

pub(super) fn clear_table_cells(
    args: &[Arg],
    state: &mut RuntimeState,
) -> Result<RuntimeValue, Diagnostic> {
    let b = bind(
        args,
        &[
            "table_id",
            "start_column",
            "start_row",
            "end_column",
            "end_row",
        ],
    );
    let Some(RuntimeValue::Ref(RefKind::Table, id)) = b.get("table_id") else {
        return Ok(RuntimeValue::Void);
    };
    let Some(mut table) = state.tables.get(id).cloned() else {
        return Ok(RuntimeValue::Void);
    };
    let columns = b
        .get("start_column")
        .and_then(RuntimeValue::int_value)
        .unwrap_or(0)
        ..=b.get("end_column")
            .and_then(RuntimeValue::int_value)
            .unwrap_or(table.columns as i64 - 1);
    let rows = b
        .get("start_row")
        .and_then(RuntimeValue::int_value)
        .unwrap_or(0)
        ..=b.get("end_row")
            .and_then(RuntimeValue::int_value)
            .unwrap_or(table.rows as i64 - 1);
    table.cells.retain(|cell| {
        !(columns.contains(&(cell.column as i64)) && rows.contains(&(cell.row as i64)))
    });
    state.tables.insert(*id, table);
    Ok(RuntimeValue::Void)
}
