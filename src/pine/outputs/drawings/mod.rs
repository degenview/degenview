//! Drawing objects — `line.*`, `label.*`, `box.*`, `table.*`, `linefill.*`, `polyline.*` and
//! `chart.point.*` — ported from `PineRuntimeSession+Drawings.swift` + `+Polylines.swift`.
#![allow(clippy::too_many_arguments)] // faithful ports of Swift functions with many labelled params
//!
//! The objects live in `RuntimeState` keyed by a reference handle (`next_reference`), so a realtime
//! rollback restores them with the variables that hold the handles. A drawing made with an `na`
//! coordinate stores `MISSING_INDEX` / NaN and is hidden until a later `set_*` gives it a real one.
//!
//! The session evaluates the call's arguments in source order (one `Arg` each) and calls
//! [`call`], which does its own binding — overloads between coordinates and `chart.point`s are
//! resolved from the first positional argument's type, exactly as Pine does. Time-anchored
//! drawings (`xloc.bar_time`) need the current bar and the bar length, passed in as `bar` and
//! `bar_seconds`; the per-script drawing caps come from `DrawingLimits`.

use std::collections::HashMap;

use crate::model::Kline;
use crate::pine::runtime::fast_hash::FastMap;
use crate::pine::runtime::state::RuntimeState;
use crate::pine::runtime::value::RuntimeValue;
use crate::pine::syntax::token::{Diagnostic, DiagnosticCategory, SourceRange};

mod coordinates;
mod create;
mod mutate;
mod names;
mod table;

use coordinates::{is_chart_point, line_price, set_xloc};
use create::{
    chart_point, copy_chart_point, copy_drawing, delete_polyline, new_box, new_label, new_line,
    new_linefill, new_polyline,
};
use mutate::mutate_drawing;
use table::{clear_table_cells, merge_table_cells, new_table, set_table_cell};

/// `color.blue`, the default drawing color.
pub const DEFAULT_COLOR: u32 = 0x2196_f3ff;
const OPAQUE_BLACK: u32 = 0x0000_00ff;
const DEFAULT_DRAWING_LIMIT: usize = 50;
const CHART_POINT_TYPE: &str = "chart.point";

/// One evaluated argument, in source order (positional and named interleaved).
pub struct Arg {
    pub name: Option<String>,
    pub value: RuntimeValue,
}

/// The per-script drawing caps declared by `max_lines_count` etc. `None` means the default.
#[derive(Clone, Copy, Debug, Default)]
pub struct DrawingLimits {
    pub max_lines: Option<i64>,
    pub max_labels: Option<i64>,
    pub max_boxes: Option<i64>,
    pub max_polylines: Option<i64>,
}

/// An enumeration of Pine builtin constants sharing a namespace (`size.tiny`, `line.style_solid`).
pub trait FromName: Copy {
    fn from_name(name: &str) -> Option<Self>;
}

/// `absent` when the script passed nothing, `unknown` for a name this enum does not list.
pub fn parse<T: FromName>(name: Option<&str>, absent: T, unknown: T) -> T {
    match name {
        Some(name) => T::from_name(name).unwrap_or(unknown),
        None => absent,
    }
}

/// Every `line.*` function this module handles, without the prefix.
pub const LINE_MEMBERS: &[&str] = &[
    "new",
    "delete",
    "copy",
    "set_x1",
    "set_y1",
    "set_x2",
    "set_y2",
    "set_xy1",
    "set_xy2",
    "set_first_point",
    "set_second_point",
    "set_xloc",
    "set_color",
    "set_width",
    "set_style",
    "set_extend",
    "get_x1",
    "get_y1",
    "get_x2",
    "get_y2",
    "get_price",
];

/// Every `label.*` function, without the prefix.
pub const LABEL_MEMBERS: &[&str] = &[
    "new",
    "delete",
    "copy",
    "set_x",
    "set_y",
    "set_xy",
    "set_point",
    "set_xloc",
    "set_yloc",
    "set_text",
    "set_color",
    "set_textcolor",
    "set_style",
    "set_size",
    "set_tooltip",
    "set_textalign",
    "set_text_font_family",
    "set_text_formatting",
    "get_x",
    "get_y",
    "get_text",
];

/// Every `box.*` function, without the prefix.
pub const BOX_MEMBERS: &[&str] = &[
    "new",
    "delete",
    "copy",
    "set_left",
    "set_right",
    "set_top",
    "set_bottom",
    "set_lefttop",
    "set_rightbottom",
    "set_top_left_point",
    "set_bottom_right_point",
    "set_xloc",
    "set_bgcolor",
    "set_border_color",
    "set_border_width",
    "set_border_style",
    "set_extend",
    "set_text",
    "set_text_color",
    "set_text_size",
    "set_text_halign",
    "set_text_valign",
    "set_text_wrap",
    "set_text_font_family",
    "set_text_formatting",
    "get_left",
    "get_right",
    "get_top",
    "get_bottom",
];

/// Every `linefill.*` function, without the prefix.
pub const LINEFILL_MEMBERS: &[&str] = &["new", "delete", "set_color", "get_line1", "get_line2"];

/// Every `table.*` function, without the prefix.
pub const TABLE_MEMBERS: &[&str] = &[
    "new",
    "delete",
    "cell",
    "merge_cells",
    "clear",
    "cell_set_text",
    "cell_set_text_color",
    "cell_set_bgcolor",
    "cell_set_width",
    "cell_set_height",
    "cell_set_text_halign",
    "cell_set_text_valign",
    "cell_set_text_size",
    "cell_set_tooltip",
    "cell_set_text_font_family",
    "cell_set_text_formatting",
    "set_position",
    "set_bgcolor",
    "set_border_color",
    "set_border_width",
    "set_frame_color",
    "set_frame_width",
];

/// Every `polyline.*` function, without the prefix.
pub const POLYLINE_MEMBERS: &[&str] = &["new", "delete"];

/// Every `chart.point` function with its prefix (the bare name is the type's constructor).
pub const CHART_POINT_NAMES: &[&str] = &[
    "chart.point.new",
    "chart.point.from_index",
    "chart.point.from_time",
    "chart.point.now",
    "chart.point.copy",
];

/// The drawing namespaces and their functions, which the editor offers as written.
pub const NAMESPACES: [(&str, &[&str]); 6] = [
    ("line", LINE_MEMBERS),
    ("label", LABEL_MEMBERS),
    ("box", BOX_MEMBERS),
    ("linefill", LINEFILL_MEMBERS),
    ("table", TABLE_MEMBERS),
    ("polyline", POLYLINE_MEMBERS),
];

/// Whether `name` is a drawing or `chart.point` function this module handles.
pub fn handles(name: &str) -> bool {
    if name == "chart.point" || CHART_POINT_NAMES.contains(&name) {
        return true;
    }
    let Some((namespace, member)) = name.split_once('.') else {
        return false;
    };
    NAMESPACES
        .iter()
        .any(|(candidate, members)| *candidate == namespace && members.contains(&member))
}

/// Runs one drawing call. `args` are the evaluated arguments in source order; the current bar and
/// bar length resolve `xloc.bar_time`; `limits` cap the number of stored objects.
pub fn call(
    name: &str,
    args: &[Arg],
    state: &mut RuntimeState,
    bar: &Kline,
    bar_seconds: f64,
    limits: &DrawingLimits,
    range: SourceRange,
) -> Result<RuntimeValue, Diagnostic> {
    match name {
        "line.new" => new_line(args, state, bar, bar_seconds, limits),
        "label.new" => new_label(args, state, bar, bar_seconds, limits),
        "box.new" => new_box(args, state, bar, bar_seconds, limits),
        "table.new" => new_table(args, state),
        "linefill.new" => new_linefill(args, state),
        "table.cell" => set_table_cell(args, state, range),
        "table.merge_cells" => merge_table_cells(args, state, range),
        "table.clear" => clear_table_cells(args, state),
        "polyline.new" => new_polyline(args, state, bar, bar_seconds, limits),
        "polyline.delete" => delete_polyline(args, state),
        "line.copy" | "label.copy" | "box.copy" => copy_drawing(name, args, state, limits),
        "chart.point.copy" => copy_chart_point(args, state),
        "line.get_price" => line_price(args, state, range),
        "line.set_xloc" | "label.set_xloc" | "box.set_xloc" => {
            set_xloc(name, args, state, bar, bar_seconds)
        }
        "chart.point"
        | "chart.point.new"
        | "chart.point.from_index"
        | "chart.point.from_time"
        | "chart.point.now" => chart_point(name, args, state, bar, bar_seconds),
        _ => mutate_drawing(name, args, state, bar, bar_seconds, range),
    }
}

fn runtime_error(code: &str, message: String, range: SourceRange) -> Diagnostic {
    Diagnostic::error(code, DiagnosticCategory::Runtime, message, range)
}

fn unknown_function(name: &str, range: SourceRange) -> Diagnostic {
    runtime_error(
        "PINE4007",
        format!("Unknown or unsupported function '{name}'."),
        range,
    )
}

fn allocate(state: &mut RuntimeState) -> usize {
    state.next_reference += 1;
    state.next_reference
}

fn limit(declared: Option<i64>) -> usize {
    declared
        .map(|count| count.max(1) as usize)
        .unwrap_or(DEFAULT_DRAWING_LIMIT)
}

fn cap<T>(objects: &mut FastMap<usize, T>, declared: Option<i64>) {
    let cap = limit(declared);
    while objects.len() > cap {
        let Some(oldest) = objects.keys().copied().min() else {
            break;
        };
        objects.remove(&oldest);
    }
}

/// An `intValue` as a non-negative `usize`, defaulting to `default` (Pine keeps an int that is
/// present; a negative width is folded to zero because the model stores widths unsigned).
fn width_or(value: Option<&RuntimeValue>, default: i64) -> usize {
    value
        .and_then(RuntimeValue::int_value)
        .unwrap_or(default)
        .max(0) as usize
}

fn count(value: Option<&RuntimeValue>) -> usize {
    value.and_then(RuntimeValue::int_value).unwrap_or(0).max(0) as usize
}

fn text(bound: &HashMap<String, RuntimeValue>, key: &str) -> Option<String> {
    bound
        .get(key)
        .and_then(RuntimeValue::text_value)
        .map(str::to_owned)
}

// MARK: Argument binding

/// Binds positional arguments to `names` in order; named arguments bind by name (a named argument
/// not in `names` is still bound under its own name).
pub fn bind(args: &[Arg], names: &[&str]) -> HashMap<String, RuntimeValue> {
    let mut out = HashMap::new();
    let mut position = 0usize;
    for arg in args {
        match &arg.name {
            Some(name) => {
                out.insert(name.clone(), arg.value.clone());
            }
            None => {
                if position < names.len() {
                    out.insert(names[position].to_owned(), arg.value.clone());
                }
                position += 1;
            }
        }
    }
    out
}

/// Like `bind`, but switches to the `point` parameter set when the first positional argument is a
/// `chart.point`, or a point parameter is named.
fn bind_overload(
    args: &[Arg],
    plain: &[&str],
    point: &[&str],
    state: &RuntimeState,
) -> (HashMap<String, RuntimeValue>, bool) {
    let mut out = HashMap::new();
    let mut names = plain;
    let mut is_point = false;
    let mut position = 0usize;
    for arg in args {
        let value = arg.value.clone();
        match &arg.name {
            Some(name) => {
                out.insert(name.clone(), value);
                if point.first() == Some(&name.as_str()) {
                    is_point = true;
                }
            }
            None => {
                if position == 0 && is_chart_point(&value, state) {
                    is_point = true;
                    names = point;
                }
                if position < names.len() {
                    out.insert(names[position].to_owned(), value);
                }
                position += 1;
            }
        }
    }
    (out, is_point)
}

#[cfg(test)]
mod tests;
