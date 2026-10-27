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

use super::containers::Object;
use super::fast_hash::FastMap;
use super::outputs::{
    BoxOutput, LabelOutput, LabelStyle, LineExtend, LineOutput, LineStyle, LinefillOutput,
    MISSING_INDEX, PolylineOutput, PolylinePoint, Size, TableCell, TableOutput, TablePosition,
    TextAlign, YLocation, index_is_known, price_is_known,
};
use super::runtime_value::{RefKind, RuntimeValue, color_value};
use super::state::RuntimeState;
use super::token::{Diagnostic, DiagnosticCategory, SourceRange};
use crate::model::Kline;

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

/// A text size given as a `size.*` constant or as a number of points (0 is `size.auto`); `fallback`
/// when the script passed nothing or something that is neither.
fn size_of(value: Option<&RuntimeValue>, fallback: Size) -> Size {
    match value {
        Some(RuntimeValue::Int(0)) => Size::Auto,
        Some(RuntimeValue::Int(points)) if *points > 0 => {
            Size::Points(u16::try_from(*points).unwrap_or(u16::MAX))
        }
        Some(value) => parse(value.text_value(), fallback, fallback),
        None => fallback,
    }
}

/// `absent` when the script passed nothing, `unknown` for a name this enum does not list.
pub fn parse<T: FromName>(name: Option<&str>, absent: T, unknown: T) -> T {
    match name {
        Some(name) => T::from_name(name).unwrap_or(unknown),
        None => absent,
    }
}

impl FromName for LineStyle {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "line.style_solid" => Some(Self::Solid),
            "line.style_dotted" => Some(Self::Dotted),
            "line.style_dashed" => Some(Self::Dashed),
            "line.style_arrow_left" => Some(Self::ArrowLeft),
            "line.style_arrow_right" => Some(Self::ArrowRight),
            "line.style_arrow_both" => Some(Self::ArrowBoth),
            _ => None,
        }
    }
}

impl FromName for LineExtend {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "extend.none" => Some(Self::None),
            "extend.left" => Some(Self::Left),
            "extend.right" => Some(Self::Right),
            "extend.both" => Some(Self::Both),
            _ => None,
        }
    }
}

impl FromName for LabelStyle {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "label.style_none" => Some(Self::None),
            "label.style_label_down" => Some(Self::LabelDown),
            "label.style_label_up" => Some(Self::LabelUp),
            "label.style_label_left" => Some(Self::LabelLeft),
            "label.style_label_right" => Some(Self::LabelRight),
            "label.style_label_center" => Some(Self::LabelCenter),
            "label.style_label_lower_left" => Some(Self::LabelLowerLeft),
            "label.style_label_lower_right" => Some(Self::LabelLowerRight),
            "label.style_label_upper_left" => Some(Self::LabelUpperLeft),
            "label.style_label_upper_right" => Some(Self::LabelUpperRight),
            "label.style_circle" => Some(Self::Circle),
            "label.style_square" => Some(Self::Square),
            "label.style_diamond" => Some(Self::Diamond),
            "label.style_cross" => Some(Self::Cross),
            "label.style_xcross" => Some(Self::Xcross),
            "label.style_flag" => Some(Self::Flag),
            "label.style_triangleup" => Some(Self::TriangleUp),
            "label.style_triangledown" => Some(Self::TriangleDown),
            "label.style_arrowup" => Some(Self::ArrowUp),
            "label.style_arrowdown" => Some(Self::ArrowDown),
            "label.style_text_outline" => Some(Self::TextOutline),
            _ => None,
        }
    }
}

impl FromName for Size {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "size.auto" => Some(Self::Auto),
            "size.tiny" => Some(Self::Tiny),
            "size.small" => Some(Self::Small),
            "size.normal" => Some(Self::Normal),
            "size.large" => Some(Self::Large),
            "size.huge" => Some(Self::Huge),
            _ => None,
        }
    }
}

impl FromName for TextAlign {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "text.align_left" => Some(Self::Left),
            "text.align_center" => Some(Self::Center),
            "text.align_right" => Some(Self::Right),
            "text.align_top" => Some(Self::Top),
            "text.align_bottom" => Some(Self::Bottom),
            _ => None,
        }
    }
}

impl FromName for YLocation {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "yloc.price" => Some(Self::Price),
            "yloc.abovebar" => Some(Self::AboveBar),
            "yloc.belowbar" => Some(Self::BelowBar),
            _ => None,
        }
    }
}

impl FromName for TablePosition {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "position.top_left" => Some(Self::TopLeft),
            "position.top_center" => Some(Self::TopCenter),
            "position.top_right" => Some(Self::TopRight),
            "position.middle_left" => Some(Self::MiddleLeft),
            "position.middle_center" => Some(Self::MiddleCenter),
            "position.middle_right" => Some(Self::MiddleRight),
            "position.bottom_left" => Some(Self::BottomLeft),
            "position.bottom_center" => Some(Self::BottomCenter),
            "position.bottom_right" => Some(Self::BottomRight),
            _ => None,
        }
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

/// `value` as a bar index when the drawing is anchored to time, else unchanged.
fn x_index(
    value: RuntimeValue,
    anchored_to_time: bool,
    bar: &Kline,
    bar_seconds: f64,
    state: &RuntimeState,
) -> RuntimeValue {
    if anchored_to_time && let Some(millis) = value.int_value() {
        return RuntimeValue::Int(bar_index_for_time(millis, bar, bar_seconds, state));
    }
    value
}

fn is_time_anchored(bound: &HashMap<String, RuntimeValue>) -> bool {
    bound.get("xloc").and_then(RuntimeValue::text_value) == Some("xloc.bar_time")
}

/// The bar index a time-anchored coordinate stands for, mirroring `PineRuntimeSession.barIndex`.
fn bar_index_for_time(millis: i64, bar: &Kline, bar_seconds: f64, state: &RuntimeState) -> i64 {
    let current_time = bar.open_time;
    let step = (bar_seconds * 1000.0).round() as i64;
    let current = state.bar_index;
    if millis >= current_time {
        if step <= 0 {
            return current;
        }
        return current + (((millis - current_time) as f64 / step as f64).round() as i64);
    }
    let times = state.histories.get("time").cloned().unwrap_or_default();
    let mut low = 0usize;
    let mut high = times.len();
    while low < high {
        let middle = (low + high) / 2;
        match times[middle].int_value() {
            Some(time) if time <= millis => low = middle + 1,
            _ => high = middle,
        }
    }
    if low > 0 {
        return low as i64 - 1;
    }
    if step <= 0 {
        return 0;
    }
    let Some(first) = times.first().and_then(RuntimeValue::int_value) else {
        return 0;
    };
    -(((first - millis) as f64 / step as f64).ceil() as i64)
}

/// Whether `value` is a `chart.point` object.
fn is_chart_point(value: &RuntimeValue, state: &RuntimeState) -> bool {
    matches!(value, RuntimeValue::Ref(RefKind::Object, id)
        if state.instances.get(id).is_some_and(|object| object.type_name == CHART_POINT_TYPE))
}

/// The bar index and price of a `chart.point`, resolving its `time` when time-anchored.
fn point_coordinates(
    value: &RuntimeValue,
    anchored_to_time: bool,
    bar: &Kline,
    bar_seconds: f64,
    state: &RuntimeState,
) -> Option<(i64, f64)> {
    let RuntimeValue::Ref(RefKind::Object, id) = value else {
        return None;
    };
    let object = state.instances.get(id)?;
    if object.type_name != CHART_POINT_TYPE {
        return None;
    }
    let price = object
        .fields
        .get("price")
        .and_then(RuntimeValue::number)
        .unwrap_or(f64::NAN);
    let time = object.fields.get("time").and_then(RuntimeValue::int_value);
    let index = object.fields.get("index").and_then(RuntimeValue::int_value);
    let from_time = time.map(|t| bar_index_for_time(t, bar, bar_seconds, state));
    let resolved = if anchored_to_time {
        from_time.or(index)
    } else {
        index.or(from_time)
    };
    Some((resolved.unwrap_or(MISSING_INDEX), price))
}

fn known_index(index: i64) -> RuntimeValue {
    if index_is_known(index) {
        RuntimeValue::Int(index)
    } else {
        RuntimeValue::Na
    }
}

fn known_price(price: f64) -> RuntimeValue {
    if price_is_known(price) {
        RuntimeValue::Float(price)
    } else {
        RuntimeValue::Na
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

// MARK: Creation

const PLAIN_LINE: [&str; 9] = [
    "x1", "y1", "x2", "y2", "xloc", "extend", "color", "style", "width",
];
const POINT_LINE: [&str; 7] = [
    "first_point",
    "second_point",
    "xloc",
    "extend",
    "color",
    "style",
    "width",
];

fn new_line(
    args: &[Arg],
    state: &mut RuntimeState,
    bar: &Kline,
    bar_seconds: f64,
    limits: &DrawingLimits,
) -> Result<RuntimeValue, Diagnostic> {
    let (b, is_point) = bind_overload(args, &PLAIN_LINE, &POINT_LINE, state);
    let timed = is_time_anchored(&b);
    let (x1, y1, x2, y2) = if is_point {
        let Some(first) = b
            .get("first_point")
            .and_then(|value| point_coordinates(value, timed, bar, bar_seconds, state))
        else {
            return Ok(RuntimeValue::Na);
        };
        let Some(second) = b
            .get("second_point")
            .and_then(|value| point_coordinates(value, timed, bar, bar_seconds, state))
        else {
            return Ok(RuntimeValue::Na);
        };
        (first.0, first.1, second.0, second.1)
    } else {
        let x1 = b
            .get("x1")
            .and_then(RuntimeValue::int_value)
            .map(|value| {
                if timed {
                    bar_index_for_time(value, bar, bar_seconds, state)
                } else {
                    value
                }
            })
            .unwrap_or(MISSING_INDEX);
        let x2 = b
            .get("x2")
            .and_then(RuntimeValue::int_value)
            .map(|value| {
                if timed {
                    bar_index_for_time(value, bar, bar_seconds, state)
                } else {
                    value
                }
            })
            .unwrap_or(MISSING_INDEX);
        let y1 = b
            .get("y1")
            .and_then(RuntimeValue::number)
            .unwrap_or(f64::NAN);
        let y2 = b
            .get("y2")
            .and_then(RuntimeValue::number)
            .unwrap_or(f64::NAN);
        (x1, y1, x2, y2)
    };
    let id = allocate(state);
    state.lines.insert(
        id,
        LineOutput {
            id,
            x1,
            y1,
            x2,
            y2,
            color: color_value(b.get("color"), Some(DEFAULT_COLOR)).unwrap_or(0),
            width: width_or(b.get("width"), 1),
            style: parse(
                b.get("style").and_then(RuntimeValue::text_value),
                LineStyle::Solid,
                LineStyle::Solid,
            ),
            extend: parse(
                b.get("extend").and_then(RuntimeValue::text_value),
                LineExtend::None,
                LineExtend::None,
            ),
            time_anchored: timed,
        },
    );
    cap(&mut state.lines, limits.max_lines);
    Ok(RuntimeValue::Ref(RefKind::Line, id))
}

const PLAIN_LABEL: [&str; 11] = [
    "x",
    "y",
    "text",
    "xloc",
    "yloc",
    "color",
    "style",
    "textcolor",
    "size",
    "textalign",
    "tooltip",
];
const POINT_LABEL: [&str; 10] = [
    "point",
    "text",
    "xloc",
    "yloc",
    "color",
    "style",
    "textcolor",
    "size",
    "textalign",
    "tooltip",
];

fn new_label(
    args: &[Arg],
    state: &mut RuntimeState,
    bar: &Kline,
    bar_seconds: f64,
    limits: &DrawingLimits,
) -> Result<RuntimeValue, Diagnostic> {
    let (b, is_point) = bind_overload(args, &PLAIN_LABEL, &POINT_LABEL, state);
    let timed = is_time_anchored(&b);
    let (x, y) = if is_point {
        let Some(point) = b
            .get("point")
            .and_then(|value| point_coordinates(value, timed, bar, bar_seconds, state))
        else {
            return Ok(RuntimeValue::Na);
        };
        (point.0, point.1)
    } else {
        let x = b
            .get("x")
            .and_then(RuntimeValue::int_value)
            .map(|value| {
                if timed {
                    bar_index_for_time(value, bar, bar_seconds, state)
                } else {
                    value
                }
            })
            .unwrap_or(MISSING_INDEX);
        let y = b
            .get("y")
            .and_then(RuntimeValue::number)
            .unwrap_or(f64::NAN);
        (x, y)
    };
    let id = allocate(state);
    state.labels.insert(
        id,
        LabelOutput {
            id,
            x,
            y,
            text: text(&b, "text").unwrap_or_default(),
            color: color_value(b.get("color"), Some(DEFAULT_COLOR)),
            text_color: color_value(b.get("textcolor"), Some(OPAQUE_BLACK)).unwrap_or(0),
            style: parse(
                b.get("style").and_then(RuntimeValue::text_value),
                LabelStyle::LabelDown,
                LabelStyle::LabelCenter,
            ),
            size: size_of(b.get("size"), Size::Normal),
            tooltip: text(&b, "tooltip"),
            text_align: parse(
                b.get("textalign").and_then(RuntimeValue::text_value),
                TextAlign::Center,
                TextAlign::Center,
            ),
            yloc: parse(
                b.get("yloc").and_then(RuntimeValue::text_value),
                YLocation::Price,
                YLocation::Price,
            ),
            time_anchored: timed,
        },
    );
    cap(&mut state.labels, limits.max_labels);
    Ok(RuntimeValue::Ref(RefKind::Label, id))
}

const PLAIN_BOX: [&str; 15] = [
    "left",
    "top",
    "right",
    "bottom",
    "border_color",
    "border_width",
    "border_style",
    "extend",
    "xloc",
    "bgcolor",
    "text",
    "text_size",
    "text_color",
    "text_halign",
    "text_valign",
];
const POINT_BOX: [&str; 13] = [
    "top_left",
    "bottom_right",
    "border_color",
    "border_width",
    "border_style",
    "extend",
    "xloc",
    "bgcolor",
    "text",
    "text_size",
    "text_color",
    "text_halign",
    "text_valign",
];

fn new_box(
    args: &[Arg],
    state: &mut RuntimeState,
    bar: &Kline,
    bar_seconds: f64,
    limits: &DrawingLimits,
) -> Result<RuntimeValue, Diagnostic> {
    let (b, is_point) = bind_overload(args, &PLAIN_BOX, &POINT_BOX, state);
    let timed = is_time_anchored(&b);
    let (left, top, right, bottom) = if is_point {
        let Some(first) = b
            .get("top_left")
            .and_then(|value| point_coordinates(value, timed, bar, bar_seconds, state))
        else {
            return Ok(RuntimeValue::Na);
        };
        let Some(second) = b
            .get("bottom_right")
            .and_then(|value| point_coordinates(value, timed, bar, bar_seconds, state))
        else {
            return Ok(RuntimeValue::Na);
        };
        (first.0, first.1, second.0, second.1)
    } else {
        let left = b
            .get("left")
            .and_then(RuntimeValue::int_value)
            .map(|value| {
                if timed {
                    bar_index_for_time(value, bar, bar_seconds, state)
                } else {
                    value
                }
            })
            .unwrap_or(MISSING_INDEX);
        let right = b
            .get("right")
            .and_then(RuntimeValue::int_value)
            .map(|value| {
                if timed {
                    bar_index_for_time(value, bar, bar_seconds, state)
                } else {
                    value
                }
            })
            .unwrap_or(MISSING_INDEX);
        let top = b
            .get("top")
            .and_then(RuntimeValue::number)
            .unwrap_or(f64::NAN);
        let bottom = b
            .get("bottom")
            .and_then(RuntimeValue::number)
            .unwrap_or(f64::NAN);
        (left, top, right, bottom)
    };
    let id = allocate(state);
    state.boxes.insert(
        id,
        BoxOutput {
            id,
            left,
            top,
            right,
            bottom,
            border_color: color_value(b.get("border_color"), Some(DEFAULT_COLOR)),
            border_width: width_or(b.get("border_width"), 1),
            background_color: color_value(b.get("bgcolor"), Some(DEFAULT_COLOR)),
            border_style: parse(
                b.get("border_style").and_then(RuntimeValue::text_value),
                LineStyle::Solid,
                LineStyle::Solid,
            ),
            extend: parse(
                b.get("extend").and_then(RuntimeValue::text_value),
                LineExtend::None,
                LineExtend::None,
            ),
            text: text(&b, "text").unwrap_or_default(),
            text_color: color_value(b.get("text_color"), Some(OPAQUE_BLACK)).unwrap_or(0),
            text_size: size_of(b.get("text_size"), Size::Normal),
            text_horizontal_align: parse(
                b.get("text_halign").and_then(RuntimeValue::text_value),
                TextAlign::Center,
                TextAlign::Center,
            ),
            text_vertical_align: parse(
                b.get("text_valign").and_then(RuntimeValue::text_value),
                TextAlign::Center,
                TextAlign::Center,
            ),
            time_anchored: timed,
        },
    );
    cap(&mut state.boxes, limits.max_boxes);
    Ok(RuntimeValue::Ref(RefKind::Box, id))
}

fn new_table(args: &[Arg], state: &mut RuntimeState) -> Result<RuntimeValue, Diagnostic> {
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

fn new_linefill(args: &[Arg], state: &mut RuntimeState) -> Result<RuntimeValue, Diagnostic> {
    let b = bind(args, &["line1", "line2", "color"]);
    let (Some(line1), Some(line2)) = (b.get("line1"), b.get("line2")) else {
        return Ok(RuntimeValue::Na);
    };
    let (RuntimeValue::Ref(RefKind::Line, first), RuntimeValue::Ref(RefKind::Line, second)) =
        (line1, line2)
    else {
        return Ok(RuntimeValue::Na);
    };
    let (first, second) = (*first, *second);
    if !state.lines.contains_key(&first) || !state.lines.contains_key(&second) {
        return Ok(RuntimeValue::Na);
    }
    let id = allocate(state);
    state.linefills.insert(
        id,
        LinefillOutput {
            id,
            line1: first,
            line2: second,
            color: color_value(b.get("color"), Some(DEFAULT_COLOR)).unwrap_or(0),
        },
    );
    cap(&mut state.linefills, None);
    Ok(RuntimeValue::Ref(RefKind::Linefill, id))
}

fn set_table_cell(
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
const CELL_PROPERTIES: [&str; 8] = [
    "text",
    "width",
    "height",
    "text_color",
    "text_halign",
    "text_valign",
    "text_size",
    "bgcolor",
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
        _ => {}
    }
}

/// `table.cell_set_text`, `cell_set_bgcolor` and the other `table.cell_set_*` setters:
/// `(table_id, column, row, value)`. A cell `table.cell` has not defined yet is created blank first,
/// so the setter does not lose what it was given. `cell_set_tooltip`, `cell_set_text_font_family`
/// and `cell_set_text_formatting` change nothing the chart draws; a script is warned at compile time.
fn set_cell_setter(
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

fn merge_table_cells(
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

fn clear_table_cells(args: &[Arg], state: &mut RuntimeState) -> Result<RuntimeValue, Diagnostic> {
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

fn new_polyline(
    args: &[Arg],
    state: &mut RuntimeState,
    bar: &Kline,
    bar_seconds: f64,
    limits: &DrawingLimits,
) -> Result<RuntimeValue, Diagnostic> {
    let b = bind(
        args,
        &[
            "points",
            "curved",
            "closed",
            "xloc",
            "line_color",
            "fill_color",
            "line_style",
            "line_width",
        ],
    );
    let Some(RuntimeValue::Ref(RefKind::Array, array_id)) = b.get("points") else {
        return Ok(RuntimeValue::Na);
    };
    let Some(items) = state.arrays.get(array_id).cloned() else {
        return Ok(RuntimeValue::Na);
    };
    let timed = b.get("xloc").and_then(RuntimeValue::text_value) == Some("xloc.bar_time");
    let mut points = Vec::new();
    for item in &items {
        let Some((index, price)) = point_coordinates(item, timed, bar, bar_seconds, state) else {
            continue;
        };
        if index_is_known(index) && price_is_known(price) {
            points.push(PolylinePoint { index, price });
        }
    }
    let id = allocate(state);
    state.polylines.insert(
        id,
        PolylineOutput {
            id,
            points,
            closed: b
                .get("closed")
                .and_then(RuntimeValue::bool)
                .unwrap_or(false),
            line_color: color_value(b.get("line_color"), Some(DEFAULT_COLOR)),
            fill_color: color_value(b.get("fill_color"), None),
            style: parse(
                b.get("line_style").and_then(RuntimeValue::text_value),
                LineStyle::Solid,
                LineStyle::Solid,
            ),
            width: width_or(b.get("line_width"), 1),
            curved: b
                .get("curved")
                .and_then(RuntimeValue::bool)
                .unwrap_or(false),
        },
    );
    cap(&mut state.polylines, limits.max_polylines);
    Ok(RuntimeValue::Ref(RefKind::Polyline, id))
}

fn delete_polyline(args: &[Arg], state: &mut RuntimeState) -> Result<RuntimeValue, Diagnostic> {
    if let Some(RuntimeValue::Ref(RefKind::Polyline, id)) = args.first().map(|arg| &arg.value) {
        state.polylines.remove(id);
    }
    Ok(RuntimeValue::Void)
}

fn chart_point(
    name: &str,
    args: &[Arg],
    state: &mut RuntimeState,
    bar: &Kline,
    bar_seconds: f64,
) -> Result<RuntimeValue, Diagnostic> {
    let parameters: &[&str] = match name {
        "chart.point.from_index" => &["index", "price"],
        "chart.point.from_time" => &["time", "price"],
        "chart.point.now" => &["price"],
        _ => &["time", "index", "price"],
    };
    let b = bind(args, parameters);
    let mut index = b.get("index").cloned();
    let mut time = b.get("time").cloned();
    if name == "chart.point.now" {
        index = Some(RuntimeValue::Int(state.bar_index));
        time = Some(RuntimeValue::Int(bar.open_time));
    } else if let Some(millis) = time.as_ref().and_then(RuntimeValue::int_value)
        && index
            .as_ref()
            .is_none_or(|value| *value == RuntimeValue::Na)
    {
        index = Some(RuntimeValue::Int(bar_index_for_time(
            millis,
            bar,
            bar_seconds,
            state,
        )));
    }
    let price = b
        .get("price")
        .cloned()
        .unwrap_or(if name == "chart.point.now" {
            RuntimeValue::Float(bar.close)
        } else {
            RuntimeValue::Na
        });
    let id = allocate(state);
    let mut fields = HashMap::new();
    fields.insert("index".to_owned(), index.unwrap_or(RuntimeValue::Na));
    fields.insert("time".to_owned(), time.unwrap_or(RuntimeValue::Na));
    fields.insert("price".to_owned(), price);
    state.instances.insert(
        id,
        Object {
            type_name: CHART_POINT_TYPE.to_owned(),
            fields,
        },
    );
    Ok(RuntimeValue::Ref(RefKind::Object, id))
}

// MARK: Copying

/// `line.copy`, `label.copy` and `box.copy`: a new object with the same properties, its own id and
/// the same cap on how many a script keeps. `na` for a missing or deleted original.
fn copy_drawing(
    name: &str,
    args: &[Arg],
    state: &mut RuntimeState,
    limits: &DrawingLimits,
) -> Result<RuntimeValue, Diagnostic> {
    let target = args.first().map(|arg| &arg.value);
    let copied = match (name, target) {
        ("line.copy", Some(RuntimeValue::Ref(RefKind::Line, id))) => {
            let fresh = allocate(state);
            let copy = clone_into(&mut state.lines, *id, fresh, |line, id| line.id = id);
            cap(&mut state.lines, limits.max_lines);
            copy.then_some(RuntimeValue::Ref(RefKind::Line, fresh))
        }
        ("label.copy", Some(RuntimeValue::Ref(RefKind::Label, id))) => {
            let fresh = allocate(state);
            let copy = clone_into(&mut state.labels, *id, fresh, |label, id| label.id = id);
            cap(&mut state.labels, limits.max_labels);
            copy.then_some(RuntimeValue::Ref(RefKind::Label, fresh))
        }
        ("box.copy", Some(RuntimeValue::Ref(RefKind::Box, id))) => {
            let fresh = allocate(state);
            let copy = clone_into(&mut state.boxes, *id, fresh, |boxed, id| boxed.id = id);
            cap(&mut state.boxes, limits.max_boxes);
            copy.then_some(RuntimeValue::Ref(RefKind::Box, fresh))
        }
        _ => None,
    };
    Ok(copied.unwrap_or(RuntimeValue::Na))
}

/// Stores a copy of `objects[id]` as `fresh`; false when there is no `id`.
fn clone_into<T: Clone>(
    objects: &mut FastMap<usize, T>,
    id: usize,
    fresh: usize,
    set_id: impl FnOnce(&mut T, usize),
) -> bool {
    let Some(mut copy) = objects.get(&id).cloned() else {
        return false;
    };
    set_id(&mut copy, fresh);
    objects.insert(fresh, copy);
    true
}

/// `chart.point.copy`: a new point with the same `time`, `index` and `price`.
fn copy_chart_point(args: &[Arg], state: &mut RuntimeState) -> Result<RuntimeValue, Diagnostic> {
    let Some(RuntimeValue::Ref(RefKind::Object, id)) = args.first().map(|arg| &arg.value) else {
        return Ok(RuntimeValue::Na);
    };
    if !state
        .instances
        .get(id)
        .is_some_and(|object| object.type_name == CHART_POINT_TYPE)
    {
        return Ok(RuntimeValue::Na);
    }
    let fresh = allocate(state);
    clone_into(&mut state.instances, *id, fresh, |_, _| {});
    Ok(RuntimeValue::Ref(RefKind::Object, fresh))
}

// MARK: Price and x-location

/// `line.get_price(id, x)`: the price of the line, extended both ways, at bar index `x`. `na` for a
/// vertical line, which has no price along it. Only a line placed by bar index has one: the
/// reference makes it an error for a line placed by time.
fn line_price(
    args: &[Arg],
    state: &RuntimeState,
    range: SourceRange,
) -> Result<RuntimeValue, Diagnostic> {
    let Some(RuntimeValue::Ref(RefKind::Line, id)) = args.first().map(|arg| &arg.value) else {
        return Ok(RuntimeValue::Na);
    };
    let Some(line) = state.lines.get(id) else {
        return Ok(RuntimeValue::Na);
    };
    if line.time_anchored {
        return Err(runtime_error(
            "PINE4038",
            "line.get_price can only be called for a line created with xloc.bar_index.".to_owned(),
            range,
        ));
    }
    let Some(x) = args.get(1).and_then(|arg| arg.value.number()) else {
        return Ok(RuntimeValue::Na);
    };
    if !line.is_complete() || line.x1 == line.x2 {
        return Ok(RuntimeValue::Na);
    }
    let slope = (line.y2 - line.y1) / (line.x2 - line.x1) as f64;
    Ok(RuntimeValue::Float(line.y1 + slope * (x - line.x1 as f64)))
}

/// `line.set_xloc(id, x1, x2, xloc)`, `box.set_xloc(id, left, right, xloc)` and
/// `label.set_xloc(id, x, xloc)`: new x coordinates read as bar indexes or times as `xloc` says,
/// which is also how the drawing is placed from then on.
fn set_xloc(
    name: &str,
    args: &[Arg],
    state: &mut RuntimeState,
    bar: &Kline,
    bar_seconds: f64,
) -> Result<RuntimeValue, Diagnostic> {
    let value = |position: usize| {
        args.get(position)
            .map_or(RuntimeValue::Na, |arg| arg.value.clone())
    };
    let target = value(0);
    let has_two_x = name != "label.set_xloc";
    let xloc = value(if has_two_x { 3 } else { 2 });
    let timed = xloc.text_value() == Some("xloc.bar_time");
    let resolve =
        |state: &RuntimeState, x: RuntimeValue| x_index(x, timed, bar, bar_seconds, state);
    let (first, second) = (resolve(state, value(1)), resolve(state, value(2)));
    match target {
        RuntimeValue::Ref(RefKind::Line, id) => {
            if let Some(line) = state.lines.get_mut(&id) {
                line.x1 = first.int_value().unwrap_or(line.x1);
                line.x2 = second.int_value().unwrap_or(line.x2);
                line.time_anchored = timed;
            }
        }
        RuntimeValue::Ref(RefKind::Box, id) => {
            if let Some(boxed) = state.boxes.get_mut(&id) {
                boxed.left = first.int_value().unwrap_or(boxed.left);
                boxed.right = second.int_value().unwrap_or(boxed.right);
                boxed.time_anchored = timed;
            }
        }
        RuntimeValue::Ref(RefKind::Label, id) => {
            if let Some(label) = state.labels.get_mut(&id) {
                label.x = first.int_value().unwrap_or(label.x);
                label.time_anchored = timed;
            }
        }
        _ => {}
    }
    Ok(RuntimeValue::Void)
}

// MARK: Mutation

/// `line.set_*`, `label.set_*`, `box.set_*`, `table.set_*`, `linefill.*` getters and `*.delete`.
fn mutate_drawing(
    name: &str,
    args: &[Arg],
    state: &mut RuntimeState,
    bar: &Kline,
    bar_seconds: f64,
    range: SourceRange,
) -> Result<RuntimeValue, Diagnostic> {
    let values: Vec<RuntimeValue> = args.iter().map(|arg| arg.value.clone()).collect();
    let target = values.first().cloned().unwrap_or(RuntimeValue::Na);
    let a = values.get(1).cloned().unwrap_or(RuntimeValue::Na);
    let b = values.get(2).cloned().unwrap_or(RuntimeValue::Na);
    let member = name.rsplit('.').next().unwrap_or(name);
    let namespace = name.split('.').next().unwrap_or(name);
    if name == "table.delete" {
        if let RuntimeValue::Ref(RefKind::Table, id) = target {
            state.tables.remove(&id);
        }
        return Ok(RuntimeValue::Void);
    }
    if namespace == "table" && member.starts_with("cell_set_") {
        return set_cell_setter(member, args, state, range);
    }
    match namespace {
        "line" => with_line(target, member, a, b, state, bar, bar_seconds, range),
        "label" => with_label(target, member, a, b, state, bar, bar_seconds, range),
        "box" => with_box(target, member, a, b, state, bar, bar_seconds, range),
        "table" => with_table(target, member, a, state, range),
        "linefill" => with_linefill(target, member, a, state, range),
        _ => Err(unknown_function(name, range)),
    }
}

fn na_or_void(member: &str) -> RuntimeValue {
    if member.starts_with("get_") {
        RuntimeValue::Na
    } else {
        RuntimeValue::Void
    }
}

#[allow(clippy::too_many_arguments)]
fn with_line(
    target: RuntimeValue,
    member: &str,
    a: RuntimeValue,
    b: RuntimeValue,
    state: &mut RuntimeState,
    bar: &Kline,
    bar_seconds: f64,
    range: SourceRange,
) -> Result<RuntimeValue, Diagnostic> {
    let RuntimeValue::Ref(RefKind::Line, id) = target else {
        return Ok(na_or_void(member));
    };
    let Some(mut line) = state.lines.get(&id).cloned() else {
        return Ok(na_or_void(member));
    };
    if member == "delete" {
        state.lines.remove(&id);
        return Ok(RuntimeValue::Void);
    }
    let result = match member {
        "set_first_point" | "set_second_point" => {
            let Some((x, y)) = set_point(&a, line.time_anchored, bar, bar_seconds, state) else {
                return Ok(RuntimeValue::Void);
            };
            let setter = if member == "set_first_point" {
                "set_xy1"
            } else {
                "set_xy2"
            };
            mutate_line(&mut line, setter, &x, &y, range)?
        }
        _ => {
            let x = x_index(a, line.time_anchored, bar, bar_seconds, state);
            mutate_line(&mut line, member, &x, &b, range)?
        }
    };
    state.lines.insert(id, line);
    Ok(result)
}

#[allow(clippy::too_many_arguments)]
fn with_label(
    target: RuntimeValue,
    member: &str,
    a: RuntimeValue,
    b: RuntimeValue,
    state: &mut RuntimeState,
    bar: &Kline,
    bar_seconds: f64,
    range: SourceRange,
) -> Result<RuntimeValue, Diagnostic> {
    let RuntimeValue::Ref(RefKind::Label, id) = target else {
        return Ok(na_or_void(member));
    };
    let Some(mut label) = state.labels.get(&id).cloned() else {
        return Ok(na_or_void(member));
    };
    if member == "delete" {
        state.labels.remove(&id);
        return Ok(RuntimeValue::Void);
    }
    let result = if member == "set_point" {
        let Some((x, y)) = set_point(&a, label.time_anchored, bar, bar_seconds, state) else {
            return Ok(RuntimeValue::Void);
        };
        mutate_label(&mut label, "set_xy", &x, &y, range)?
    } else {
        let x = x_index(a, label.time_anchored, bar, bar_seconds, state);
        mutate_label(&mut label, member, &x, &b, range)?
    };
    state.labels.insert(id, label);
    Ok(result)
}

#[allow(clippy::too_many_arguments)]
fn with_box(
    target: RuntimeValue,
    member: &str,
    a: RuntimeValue,
    b: RuntimeValue,
    state: &mut RuntimeState,
    bar: &Kline,
    bar_seconds: f64,
    range: SourceRange,
) -> Result<RuntimeValue, Diagnostic> {
    let RuntimeValue::Ref(RefKind::Box, id) = target else {
        return Ok(na_or_void(member));
    };
    let Some(mut boxed) = state.boxes.get(&id).cloned() else {
        return Ok(na_or_void(member));
    };
    if member == "delete" {
        state.boxes.remove(&id);
        return Ok(RuntimeValue::Void);
    }
    let result = match member {
        "set_top_left_point" | "set_bottom_right_point" => {
            let Some((x, y)) = set_point(&a, boxed.time_anchored, bar, bar_seconds, state) else {
                return Ok(RuntimeValue::Void);
            };
            let setter = if member == "set_top_left_point" {
                "set_lefttop"
            } else {
                "set_rightbottom"
            };
            mutate_box(&mut boxed, setter, &x, &y, range)?
        }
        _ => {
            let x = x_index(a, boxed.time_anchored, bar, bar_seconds, state);
            mutate_box(&mut boxed, member, &x, &b, range)?
        }
    };
    state.boxes.insert(id, boxed);
    Ok(result)
}

fn with_table(
    target: RuntimeValue,
    member: &str,
    a: RuntimeValue,
    state: &mut RuntimeState,
    range: SourceRange,
) -> Result<RuntimeValue, Diagnostic> {
    let RuntimeValue::Ref(RefKind::Table, id) = target else {
        return Ok(na_or_void(member));
    };
    let Some(mut table) = state.tables.get(&id).cloned() else {
        return Ok(na_or_void(member));
    };
    if member == "delete" {
        state.tables.remove(&id);
        return Ok(RuntimeValue::Void);
    }
    let result = mutate_table(&mut table, member, &a, range)?;
    state.tables.insert(id, table);
    Ok(result)
}

fn with_linefill(
    target: RuntimeValue,
    member: &str,
    a: RuntimeValue,
    state: &mut RuntimeState,
    range: SourceRange,
) -> Result<RuntimeValue, Diagnostic> {
    let RuntimeValue::Ref(RefKind::Linefill, id) = target else {
        return Ok(na_or_void(member));
    };
    let Some(mut fill) = state.linefills.get(&id).cloned() else {
        return Ok(na_or_void(member));
    };
    if member == "delete" {
        state.linefills.remove(&id);
        return Ok(RuntimeValue::Void);
    }
    let result = mutate_linefill(&mut fill, member, &a, range)?;
    state.linefills.insert(id, fill);
    Ok(result)
}

/// The bar index and price of the `chart.point` in `value`, as runtime values.
fn set_point(
    value: &RuntimeValue,
    anchored_to_time: bool,
    bar: &Kline,
    bar_seconds: f64,
    state: &RuntimeState,
) -> Option<(RuntimeValue, RuntimeValue)> {
    let (index, price) = point_coordinates(value, anchored_to_time, bar, bar_seconds, state)?;
    Some((RuntimeValue::Int(index), RuntimeValue::Float(price)))
}

fn mutate_line(
    line: &mut LineOutput,
    member: &str,
    a: &RuntimeValue,
    b: &RuntimeValue,
    range: SourceRange,
) -> Result<RuntimeValue, Diagnostic> {
    match member {
        "set_x1" => line.x1 = a.int_value().unwrap_or(line.x1),
        "set_x2" => line.x2 = a.int_value().unwrap_or(line.x2),
        "set_y1" => line.y1 = a.number().unwrap_or(line.y1),
        "set_y2" => line.y2 = a.number().unwrap_or(line.y2),
        "set_xy1" => {
            line.x1 = a.int_value().unwrap_or(line.x1);
            line.y1 = b.number().unwrap_or(line.y1);
        }
        "set_xy2" => {
            line.x2 = a.int_value().unwrap_or(line.x2);
            line.y2 = b.number().unwrap_or(line.y2);
        }
        "set_color" => line.color = color_value(Some(a), None).unwrap_or(0),
        "set_width" => line.width = a.int_value().unwrap_or(line.width as i64).max(0) as usize,
        "set_style" => {
            if let Some(style) = a.text_value().and_then(LineStyle::from_name) {
                line.style = style;
            }
        }
        "set_extend" => {
            if let Some(extend) = a.text_value().and_then(LineExtend::from_name) {
                line.extend = extend;
            }
        }
        "get_x1" => return Ok(known_index(line.x1)),
        "get_x2" => return Ok(known_index(line.x2)),
        "get_y1" => return Ok(known_price(line.y1)),
        "get_y2" => return Ok(known_price(line.y2)),
        _ => return Err(unknown_function_member("line", member, range)),
    }
    Ok(RuntimeValue::Void)
}

fn mutate_label(
    label: &mut LabelOutput,
    member: &str,
    a: &RuntimeValue,
    b: &RuntimeValue,
    range: SourceRange,
) -> Result<RuntimeValue, Diagnostic> {
    match member {
        "set_x" => label.x = a.int_value().unwrap_or(label.x),
        "set_y" => label.y = a.number().unwrap_or(label.y),
        "set_xy" => {
            label.x = a.int_value().unwrap_or(label.x);
            label.y = b.number().unwrap_or(label.y);
        }
        "set_text" => label.text = a.text_value().unwrap_or("").to_owned(),
        "set_color" => label.color = color_value(Some(a), None),
        "set_textcolor" => label.text_color = color_value(Some(a), None).unwrap_or(0),
        "set_style" => {
            if let Some(text) = a.text_value() {
                label.style = LabelStyle::from_name(text).unwrap_or(LabelStyle::LabelCenter);
            }
        }
        "set_size" => label.size = size_of(Some(a), label.size),
        "set_tooltip" => label.tooltip = a.text_value().map(str::to_owned),
        "set_textalign" => {
            label.text_align = parse(a.text_value(), label.text_align, label.text_align);
        }
        "set_yloc" => label.yloc = parse(a.text_value(), label.yloc, label.yloc),
        "set_text_font_family" | "set_text_formatting" => {}
        "get_x" => return Ok(known_index(label.x)),
        "get_y" => return Ok(known_price(label.y)),
        "get_text" => return Ok(RuntimeValue::String(label.text.clone())),
        _ => return Err(unknown_function_member("label", member, range)),
    }
    Ok(RuntimeValue::Void)
}

fn mutate_box(
    boxed: &mut BoxOutput,
    member: &str,
    a: &RuntimeValue,
    b: &RuntimeValue,
    range: SourceRange,
) -> Result<RuntimeValue, Diagnostic> {
    match member {
        "set_left" => boxed.left = a.int_value().unwrap_or(boxed.left),
        "set_right" => boxed.right = a.int_value().unwrap_or(boxed.right),
        "set_top" => boxed.top = a.number().unwrap_or(boxed.top),
        "set_bottom" => boxed.bottom = a.number().unwrap_or(boxed.bottom),
        "set_lefttop" => {
            boxed.left = a.int_value().unwrap_or(boxed.left);
            boxed.top = b.number().unwrap_or(boxed.top);
        }
        "set_rightbottom" => {
            boxed.right = a.int_value().unwrap_or(boxed.right);
            boxed.bottom = b.number().unwrap_or(boxed.bottom);
        }
        "set_bgcolor" => boxed.background_color = color_value(Some(a), None),
        "set_border_color" => boxed.border_color = color_value(Some(a), None),
        "set_border_width" => {
            boxed.border_width = a.int_value().unwrap_or(boxed.border_width as i64).max(0) as usize
        }
        "set_border_style" => {
            boxed.border_style = parse(a.text_value(), boxed.border_style, boxed.border_style);
        }
        "set_extend" => {
            if let Some(extend) = a.text_value().and_then(LineExtend::from_name) {
                boxed.extend = extend;
            }
        }
        "set_text" => boxed.text = a.text_value().unwrap_or("").to_owned(),
        "set_text_color" => boxed.text_color = color_value(Some(a), None).unwrap_or(0),
        "set_text_size" => boxed.text_size = size_of(Some(a), boxed.text_size),
        "set_text_halign" => {
            boxed.text_horizontal_align = parse(
                a.text_value(),
                boxed.text_horizontal_align,
                boxed.text_horizontal_align,
            );
        }
        "set_text_valign" => {
            boxed.text_vertical_align = parse(
                a.text_value(),
                boxed.text_vertical_align,
                boxed.text_vertical_align,
            );
        }
        "set_text_wrap" | "set_text_font_family" | "set_text_formatting" => {}
        "get_left" => return Ok(known_index(boxed.left)),
        "get_right" => return Ok(known_index(boxed.right)),
        "get_top" => return Ok(known_price(boxed.top)),
        "get_bottom" => return Ok(known_price(boxed.bottom)),
        _ => return Err(unknown_function_member("box", member, range)),
    }
    Ok(RuntimeValue::Void)
}

fn mutate_table(
    table: &mut TableOutput,
    member: &str,
    a: &RuntimeValue,
    range: SourceRange,
) -> Result<RuntimeValue, Diagnostic> {
    match member {
        "set_position" => {
            table.position = parse(a.text_value(), table.position, table.position);
        }
        "set_bgcolor" => table.background_color = color_value(Some(a), None),
        "set_border_color" => table.border_color = color_value(Some(a), None),
        "set_frame_color" => table.frame_color = color_value(Some(a), None),
        "set_border_width" => {
            table.border_width = a.int_value().unwrap_or(table.border_width as i64).max(0) as usize
        }
        "set_frame_width" => {
            table.frame_width = a.int_value().unwrap_or(table.frame_width as i64).max(0) as usize
        }
        _ => return Err(unknown_function_member("table", member, range)),
    }
    Ok(RuntimeValue::Void)
}

fn mutate_linefill(
    fill: &mut LinefillOutput,
    member: &str,
    a: &RuntimeValue,
    range: SourceRange,
) -> Result<RuntimeValue, Diagnostic> {
    match member {
        "set_color" => {
            if let RuntimeValue::Color(color) = a {
                fill.color = *color;
            }
        }
        "get_line1" => return Ok(RuntimeValue::Ref(RefKind::Line, fill.line1)),
        "get_line2" => return Ok(RuntimeValue::Ref(RefKind::Line, fill.line2)),
        _ => return Err(unknown_function_member("linefill", member, range)),
    }
    Ok(RuntimeValue::Void)
}

fn unknown_function_member(namespace: &str, member: &str, range: SourceRange) -> Diagnostic {
    runtime_error(
        "PINE4007",
        format!("Unknown or unsupported function '{namespace}.{member}'."),
        range,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use RuntimeValue::{Float, Int, Na, String as Str};

    fn bar(open_time: i64, close: f64) -> Kline {
        Kline {
            open_time,
            close_time: open_time + 60_000,
            open: close,
            high: close,
            low: close,
            close,
            volume: 0.0,
        }
    }

    fn arg(value: RuntimeValue) -> Arg {
        Arg { name: None, value }
    }

    fn named(name: &str, value: RuntimeValue) -> Arg {
        Arg {
            name: Some(name.to_owned()),
            value,
        }
    }

    fn run(state: &mut RuntimeState, name: &str, args: &[Arg]) -> Result<RuntimeValue, Diagnostic> {
        call(
            name,
            args,
            state,
            &bar(0, 0.0),
            60.0,
            &DrawingLimits::default(),
            SourceRange::ZERO,
        )
    }

    fn point(state: &mut RuntimeState, index: i64, price: f64) -> RuntimeValue {
        let id = allocate(state);
        let mut fields = HashMap::new();
        fields.insert("index".to_owned(), Int(index));
        fields.insert("time".to_owned(), Na);
        fields.insert("price".to_owned(), Float(price));
        state.instances.insert(
            id,
            Object {
                type_name: CHART_POINT_TYPE.to_owned(),
                fields,
            },
        );
        RuntimeValue::Ref(RefKind::Object, id)
    }

    #[test]
    fn line_new_plain_and_point_forms() {
        let mut state = RuntimeState::default();
        let line = run(
            &mut state,
            "line.new",
            &[arg(Int(1)), arg(Float(10.0)), arg(Int(3)), arg(Float(20.0))],
        )
        .unwrap();
        let RuntimeValue::Ref(RefKind::Line, id) = line else {
            panic!("{line:?}")
        };
        assert_eq!(state.lines[&id].x1, 1);
        assert_eq!(state.lines[&id].y2, 20.0);
        assert_eq!(state.lines[&id].color, DEFAULT_COLOR);

        let first = point(&mut state, 5, 50.0);
        let second = point(&mut state, 9, 90.0);
        let line2 = run(&mut state, "line.new", &[arg(first), arg(second)]).unwrap();
        let RuntimeValue::Ref(RefKind::Line, id2) = line2 else {
            panic!("{line2:?}")
        };
        assert_eq!(state.lines[&id2].x1, 5);
        assert_eq!(state.lines[&id2].y2, 90.0);
    }

    #[test]
    fn line_setters_and_getters() {
        let mut state = RuntimeState::default();
        let line = run(
            &mut state,
            "line.new",
            &[arg(Int(1)), arg(Float(10.0)), arg(Int(3)), arg(Float(20.0))],
        )
        .unwrap();
        run(&mut state, "line.set_x2", &[arg(line.clone()), arg(Int(7))]).unwrap();
        assert_eq!(
            run(&mut state, "line.get_x2", &[arg(line.clone())]).unwrap(),
            Int(7)
        );
        assert_eq!(
            run(&mut state, "line.get_y1", &[arg(line.clone())]).unwrap(),
            Float(10.0)
        );
        run(
            &mut state,
            "line.set_xy1",
            &[arg(line.clone()), arg(Int(2)), arg(Float(15.0))],
        )
        .unwrap();
        assert_eq!(
            run(&mut state, "line.get_x1", &[arg(line.clone())]).unwrap(),
            Int(2)
        );
        run(&mut state, "line.delete", &[arg(line.clone())]).unwrap();
        assert_eq!(run(&mut state, "line.get_x1", &[arg(line)]).unwrap(), Na);
    }

    #[test]
    fn label_style_defaults_and_keeps_existing_on_unknown() {
        let mut state = RuntimeState::default();
        let label = run(
            &mut state,
            "label.new",
            &[arg(Int(1)), arg(Float(10.0)), arg(Str("hi".into()))],
        )
        .unwrap();
        let RuntimeValue::Ref(RefKind::Label, id) = label.clone() else {
            panic!("{label:?}")
        };
        assert_eq!(state.labels[&id].style, LabelStyle::LabelDown);
        assert_eq!(state.labels[&id].text, "hi");
        // An unknown style name falls back to labelCenter, matching Pine.
        run(
            &mut state,
            "label.set_style",
            &[arg(label.clone()), arg(Str("label.style_nope".into()))],
        )
        .unwrap();
        assert_eq!(state.labels[&id].style, LabelStyle::LabelCenter);
        // `na` leaves the current style alone.
        run(&mut state, "label.set_style", &[arg(label), arg(Na)]).unwrap();
        assert_eq!(state.labels[&id].style, LabelStyle::LabelCenter);
    }

    #[test]
    fn box_and_table_round_trip() {
        let mut state = RuntimeState::default();
        let boxed = run(
            &mut state,
            "box.new",
            &[arg(Int(1)), arg(Float(10.0)), arg(Int(3)), arg(Float(20.0))],
        )
        .unwrap();
        let RuntimeValue::Ref(RefKind::Box, box_id) = boxed else {
            panic!("{boxed:?}")
        };
        assert_eq!(state.boxes[&box_id].border_color, Some(DEFAULT_COLOR));
        assert_eq!(state.boxes[&box_id].background_color, Some(DEFAULT_COLOR));

        let table = run(
            &mut state,
            "table.new",
            &[arg(Int(0)), arg(Int(2)), arg(Int(2))],
        )
        .unwrap();
        let RuntimeValue::Ref(RefKind::Table, table_id) = table else {
            panic!("{table:?}")
        };
        assert_eq!(state.tables[&table_id].columns, 2);
        assert_eq!(state.tables[&table_id].rows, 2);
        assert_eq!(state.tables[&table_id].position, TablePosition::TopRight);
        run(
            &mut state,
            "table.cell",
            &[
                arg(RuntimeValue::Ref(RefKind::Table, table_id)),
                arg(Int(0)),
                arg(Int(0)),
                arg(Str("x".into())),
            ],
        )
        .unwrap();
        assert_eq!(state.tables[&table_id].cells.len(), 1);
        assert_eq!(state.tables[&table_id].cells[0].text, "x");
    }

    #[test]
    fn table_cell_out_of_bounds_is_pine4013() {
        let mut state = RuntimeState::default();
        let table = run(
            &mut state,
            "table.new",
            &[arg(Int(0)), arg(Int(1)), arg(Int(1))],
        )
        .unwrap();
        let RuntimeValue::Ref(RefKind::Table, table_id) = table else {
            panic!()
        };
        let error = run(
            &mut state,
            "table.cell",
            &[
                arg(RuntimeValue::Ref(RefKind::Table, table_id)),
                arg(Int(3)),
                arg(Int(0)),
                arg(Str("x".into())),
            ],
        )
        .unwrap_err();
        assert_eq!(error.code, "PINE4013");
    }

    #[test]
    fn chart_point_from_index_and_now() {
        let mut state = RuntimeState {
            bar_index: 4,
            ..RuntimeState::default()
        };
        let p = run(
            &mut state,
            "chart.point.from_index",
            &[arg(Int(2)), arg(Float(5.0))],
        )
        .unwrap();
        let RuntimeValue::Ref(RefKind::Object, id) = p else {
            panic!("{p:?}")
        };
        assert_eq!(state.instances[&id].fields["index"], Int(2));
        assert_eq!(state.instances[&id].fields["price"], Float(5.0));

        let now = run(&mut state, "chart.point.now", &[]).unwrap();
        let RuntimeValue::Ref(RefKind::Object, now_id) = now else {
            panic!("{now:?}")
        };
        assert_eq!(state.instances[&now_id].fields["index"], Int(4));
    }

    #[test]
    fn polyline_collects_known_points() {
        let mut state = RuntimeState::default();
        let a = point(&mut state, 1, 10.0);
        let b = point(&mut state, 2, 20.0);
        let array = super::super::array_builtins::new_array(&mut state, vec![a, b, Na]);
        let polyline = run(&mut state, "polyline.new", &[arg(array)]).unwrap();
        let RuntimeValue::Ref(RefKind::Polyline, id) = polyline else {
            panic!("{polyline:?}")
        };
        assert_eq!(state.polylines[&id].points.len(), 2);
        assert_eq!(state.polylines[&id].line_color, Some(DEFAULT_COLOR));
        assert_eq!(state.polylines[&id].fill_color, None);
    }

    #[test]
    fn handles_covers_namespaces() {
        assert!(
            handles("line.new")
                && handles("line.set_x1")
                && handles("line.get_x1")
                && handles("line.delete")
        );
        assert!(
            handles("label.new")
                && handles("box.new")
                && handles("table.new")
                && handles("table.cell")
        );
        assert!(handles("linefill.new") && handles("polyline.new") && handles("polyline.delete"));
        assert!(handles("chart.point") && handles("chart.point.now"));
        assert!(!handles("array.get") && !handles("plot") && !handles("line.nope_member"));
    }
}
