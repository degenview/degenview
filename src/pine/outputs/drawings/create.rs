//! Making drawings: `line.new`, `label.new`, `box.new`, `linefill.new`, `polyline.new` and
//! `polyline.delete`, `chart.point.*`, and `*.copy`.

use std::collections::HashMap;

use super::coordinates::{bar_index_for_time, is_time_anchored, point_coordinates};
use super::names::size_of;
use super::{
    Arg, CHART_POINT_TYPE, DEFAULT_COLOR, DrawingLimits, OPAQUE_BLACK, allocate, bind,
    bind_overload, cap, parse, text, width_or,
};
use crate::model::Kline;
use crate::pine::outputs::{
    BoxOutput, LabelOutput, LabelStyle, LineExtend, LineOutput, LineStyle, LinefillOutput,
    MISSING_INDEX, PolylineOutput, PolylinePoint, Size, TextAlign, YLocation, index_is_known,
    price_is_known,
};
use crate::pine::runtime::containers::Object;
use crate::pine::runtime::fast_hash::FastMap;
use crate::pine::runtime::state::RuntimeState;
use crate::pine::runtime::value::{RefKind, RuntimeValue, color_value};
use crate::pine::syntax::token::Diagnostic;

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

pub(super) fn new_line(
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

pub(super) fn new_label(
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

pub(super) fn new_box(
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

pub(super) fn new_linefill(
    args: &[Arg],
    state: &mut RuntimeState,
) -> Result<RuntimeValue, Diagnostic> {
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

pub(super) fn new_polyline(
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

pub(super) fn delete_polyline(
    args: &[Arg],
    state: &mut RuntimeState,
) -> Result<RuntimeValue, Diagnostic> {
    if let Some(RuntimeValue::Ref(RefKind::Polyline, id)) = args.first().map(|arg| &arg.value) {
        state.polylines.remove(id);
    }
    Ok(RuntimeValue::Void)
}

pub(super) fn chart_point(
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

/// `line.copy`, `label.copy` and `box.copy`: a new object with the same properties, its own id and
/// the same cap on how many a script keeps. `na` for a missing or deleted original.
pub(super) fn copy_drawing(
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
pub(super) fn copy_chart_point(
    args: &[Arg],
    state: &mut RuntimeState,
) -> Result<RuntimeValue, Diagnostic> {
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
