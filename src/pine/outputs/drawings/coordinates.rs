//! Where a drawing is: bar indexes and times (`xloc`), `chart.point` coordinates, and
//! `line.get_price` and `*.set_xloc`.

use std::collections::HashMap;

use super::{Arg, CHART_POINT_TYPE, runtime_error};
use crate::model::Kline;
use crate::pine::outputs::{MISSING_INDEX, index_is_known, price_is_known};
use crate::pine::runtime::state::RuntimeState;
use crate::pine::runtime::value::{RefKind, RuntimeValue};
use crate::pine::syntax::token::{Diagnostic, SourceRange};

/// `value` as a bar index when the drawing is anchored to time, else unchanged.
pub(super) fn x_index(
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

pub(super) fn is_time_anchored(bound: &HashMap<String, RuntimeValue>) -> bool {
    bound.get("xloc").and_then(RuntimeValue::text_value) == Some("xloc.bar_time")
}

/// The bar index a time-anchored coordinate stands for, mirroring `PineRuntimeSession.barIndex`.
pub(super) fn bar_index_for_time(
    millis: i64,
    bar: &Kline,
    bar_seconds: f64,
    state: &RuntimeState,
) -> i64 {
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
pub(super) fn is_chart_point(value: &RuntimeValue, state: &RuntimeState) -> bool {
    matches!(value, RuntimeValue::Ref(RefKind::Object, id)
        if state.instances.get(id).is_some_and(|object| object.type_name == CHART_POINT_TYPE))
}

/// The bar index and price of a `chart.point`, resolving its `time` when time-anchored.
pub(super) fn point_coordinates(
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

pub(super) fn known_index(index: i64) -> RuntimeValue {
    if index_is_known(index) {
        RuntimeValue::Int(index)
    } else {
        RuntimeValue::Na
    }
}

pub(super) fn known_price(price: f64) -> RuntimeValue {
    if price_is_known(price) {
        RuntimeValue::Float(price)
    } else {
        RuntimeValue::Na
    }
}

/// `line.get_price(id, x)`: the price of the line, extended both ways, at bar index `x`. `na` for a
/// vertical line, which has no price along it. Only a line placed by bar index has one: the
/// reference makes it an error for a line placed by time.
pub(super) fn line_price(
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
pub(super) fn set_xloc(
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
