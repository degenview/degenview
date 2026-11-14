//! Changing and reading drawings: `*.set_*`, `*.get_*` and `*.delete`.

use super::coordinates::{known_index, known_price, point_coordinates, x_index};
use super::names::size_of;
use super::table::set_cell_setter;
use super::{Arg, FromName, parse, runtime_error, unknown_function};
use crate::model::Kline;
use crate::pine::outputs::{
    BoxOutput, LabelOutput, LabelStyle, LineExtend, LineOutput, LineStyle, LinefillOutput,
    TableOutput,
};
use crate::pine::runtime::state::RuntimeState;
use crate::pine::runtime::value::{RefKind, RuntimeValue, color_value};
use crate::pine::syntax::token::{Diagnostic, SourceRange};

/// `line.set_*`, `label.set_*`, `box.set_*`, `table.set_*`, `linefill.*` getters and `*.delete`.
pub(super) fn mutate_drawing(
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
