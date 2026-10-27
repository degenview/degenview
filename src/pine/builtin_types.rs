//! What the type checker knows about builtin identifiers and functions, ported from
//! `PineBuiltinTypes.swift`. Anything unlisted is `unknown`, which the checker lets through.

use super::ast::{Argument, Qualifier};
use super::type_checker::Inferred;
use crate::pine::ValueType;

// MARK: Identifiers

const FLOAT_SERIES: &[&str] = &[
    "open",
    "high",
    "low",
    "close",
    "volume",
    "hl2",
    "hlc3",
    "ohlc4",
    "hlcc4",
    "ta.tr",
    "strategy.position_size",
    "strategy.position_avg_price",
    "strategy.equity",
    "strategy.netprofit",
    "strategy.openprofit",
    "strategy.initial_capital",
    "strategy.grossprofit",
    "strategy.grossloss",
];

const INT_SERIES: &[&str] = &[
    "time",
    "time_close",
    "bar_index",
    "last_bar_index",
    "last_bar_time",
    "timenow",
    "time_tradingday",
    "chart.left_visible_bar_time",
    "chart.right_visible_bar_time",
    "year",
    "month",
    "dayofmonth",
    "hour",
    "minute",
    "second",
    "dayofweek",
    "strategy.closedtrades",
    "strategy.opentrades",
    "strategy.wintrades",
    "strategy.losstrades",
];

const SIMPLE_STRINGS: &[&str] = &[
    "syminfo.ticker",
    "syminfo.tickerid",
    "syminfo.currency",
    "syminfo.type",
    "syminfo.root",
    "syminfo.prefix",
    "syminfo.timezone",
    "syminfo.basecurrency",
];

const SIMPLE_FLOATS: &[&str] = &["syminfo.mintick", "syminfo.pointvalue"];
const SIMPLE_COLORS: &[&str] = &["chart.fg_color", "chart.bg_color"];

const INT_CONSTANTS: &[&str] = &[
    "display.none",
    "display.pane",
    "display.data_window",
    "display.price_scale",
    "display.status_line",
    "display.all",
    "dayofweek.sunday",
    "dayofweek.monday",
    "dayofweek.tuesday",
    "dayofweek.wednesday",
    "dayofweek.thursday",
    "dayofweek.friday",
    "dayofweek.saturday",
];

const FLOAT_CONSTANTS: &[&str] = &["math.pi", "math.e", "math.phi", "math.rphi"];

const STRING_CONSTANTS: &[&str] = &[
    "strategy.long",
    "strategy.short",
    "strategy.cash",
    "strategy.fixed",
    "strategy.percent_of_equity",
    "strategy.commission.percent",
    "strategy.commission.cash_per_order",
    "strategy.commission.cash_per_contract",
    "strategy.oca.none",
    "strategy.oca.cancel",
    "strategy.oca.reduce",
    "format.inherit",
    "format.price",
    "format.volume",
    "format.percent",
    "format.mintick",
    "order.ascending",
    "order.descending",
    "strategy.direction.long",
    "strategy.direction.short",
    "strategy.direction.all",
    "alert.freq_all",
    "alert.freq_once_per_bar",
    "alert.freq_once_per_bar_close",
];

const COLOR_CONSTANTS: &[&str] = &[
    "color.aqua",
    "color.black",
    "color.blue",
    "color.fuchsia",
    "color.gray",
    "color.green",
    "color.lime",
    "color.maroon",
    "color.navy",
    "color.olive",
    "color.orange",
    "color.purple",
    "color.red",
    "color.silver",
    "color.teal",
    "color.white",
    "color.yellow",
];

pub fn identifier(name: &str) -> Inferred {
    if INT_CONSTANTS.contains(&name) {
        return Inferred::Known(ValueType::Int, Some(Qualifier::Constant));
    }
    if FLOAT_CONSTANTS.contains(&name) {
        return Inferred::Known(ValueType::Float, Some(Qualifier::Constant));
    }
    if STRING_CONSTANTS.contains(&name) {
        return Inferred::Known(ValueType::String, Some(Qualifier::Constant));
    }
    if COLOR_CONSTANTS.contains(&name) {
        return Inferred::Known(ValueType::Color, Some(Qualifier::Constant));
    }
    if FLOAT_SERIES.contains(&name) {
        return Inferred::Known(ValueType::Float, Some(Qualifier::Series));
    }
    if INT_SERIES.contains(&name) {
        return Inferred::Known(ValueType::Int, Some(Qualifier::Series));
    }
    if SIMPLE_STRINGS.contains(&name) {
        return Inferred::Known(ValueType::String, Some(Qualifier::Simple));
    }
    if SIMPLE_FLOATS.contains(&name) {
        return Inferred::Known(ValueType::Float, Some(Qualifier::Simple));
    }
    if SIMPLE_COLORS.contains(&name) {
        return Inferred::Known(ValueType::Color, Some(Qualifier::Simple));
    }
    if name.starts_with("barstate.") {
        return Inferred::Known(ValueType::Bool, Some(Qualifier::Series));
    }
    Inferred::Unknown
}

// MARK: Functions

/// The type `input.*` returns, or None for functions that are not typed inputs.
pub fn input_result(name: &str) -> Option<ValueType> {
    match name {
        "input.int" | "input.time" => Some(ValueType::Int),
        "input.float" | "input.source" => Some(ValueType::Float),
        "input.bool" => Some(ValueType::Bool),
        "input.string" | "input.session" => Some(ValueType::String),
        "input.color" => Some(ValueType::Color),
        _ => None,
    }
}

fn series_result(name: &str) -> Option<ValueType> {
    match name {
        "line.new" => Some(ValueType::Line),
        "label.new" => Some(ValueType::Label),
        "box.new" => Some(ValueType::Box),
        "table.new" => Some(ValueType::Table),
        "linefill.new" => Some(ValueType::Linefill),
        "polyline.new" => Some(ValueType::Polyline),
        "array.from" | "array.copy" | "array.slice" | "ta.pivot_point_levels" => {
            Some(ValueType::Array)
        }
        _ if is_ta_float(name) => Some(ValueType::Float),
        _ if is_ta_bool(name) => Some(ValueType::Bool),
        _ => None,
    }
}

fn is_ta_float(name: &str) -> bool {
    matches!(
        name,
        "ta.sma"
            | "ta.ema"
            | "ta.rma"
            | "ta.wma"
            | "ta.rsi"
            | "ta.atr"
            | "ta.tr"
            | "ta.stdev"
            | "ta.highest"
            | "ta.lowest"
            | "ta.mom"
            | "ta.roc"
            | "ta.cum"
            | "ta.pivothigh"
            | "ta.pivotlow"
            | "ta.median"
            | "ta.range"
            | "ta.variance"
            | "ta.dev"
            | "ta.swma"
            | "ta.cmo"
            | "ta.cci"
            | "ta.hma"
            | "ta.percentrank"
            | "ta.correlation"
            | "ta.linreg"
            | "ta.rci"
    )
}

fn is_ta_bool(name: &str) -> bool {
    matches!(
        name,
        "ta.cross"
            | "ta.crossover"
            | "ta.crossunder"
            | "ta.rising"
            | "ta.falling"
            | "timeframe.change"
    )
}

fn propagating_result(name: &str) -> Option<ValueType> {
    match name {
        "int" | "timestamp" => Some(ValueType::Int),
        "float" => Some(ValueType::Float),
        "bool" | "na" => Some(ValueType::Bool),
        "color.new" | "color.rgb" | "color.from_gradient" => Some(ValueType::Color),
        "color.r" | "color.g" | "color.b" | "color.t" => Some(ValueType::Float),
        "str.length" => Some(ValueType::Int),
        "str.tonumber" => Some(ValueType::Float),
        "math.floor" | "math.ceil" => Some(ValueType::Int),
        "str.tostring"
        | "timeframe.from_seconds"
        | "str.format"
        | "str.upper"
        | "str.lower"
        | "str.trim"
        | "str.replace_all"
        | "str.substring" => Some(ValueType::String),
        "str.contains" | "str.startswith" | "str.endswith" => Some(ValueType::Bool),
        "math.sqrt"
        | "math.pow"
        | "math.log"
        | "math.log10"
        | "math.exp"
        | "math.sin"
        | "math.cos"
        | "math.tan"
        | "math.asin"
        | "math.acos"
        | "math.atan"
        | "math.avg"
        | "math.todegrees"
        | "math.toradians"
        | "math.round_to_mintick" => Some(ValueType::Float),
        _ => None,
    }
}

/// Result of calling `name`, given the inferred argument types and their combined qualifier.
pub fn call(
    name: &str,
    arguments: &[Argument],
    values: &[Inferred],
    qualifier: Option<Qualifier>,
) -> Inferred {
    if let Some(kind) = series_result(name) {
        return Inferred::Known(kind, Some(Qualifier::Series));
    }
    if let Some(kind) = propagating_result(name) {
        return Inferred::Known(kind, qualifier);
    }
    match name {
        "nz" | "math.abs" => {
            if let Some(kind) = values.first().and_then(|value| value.kind())
                && matches!(kind, ValueType::Int | ValueType::Float)
            {
                return Inferred::Known(kind, qualifier);
            }
            Inferred::Unknown
        }
        "math.round" => {
            let kind = if arguments.len() > 1 {
                ValueType::Float
            } else {
                ValueType::Int
            };
            Inferred::Known(kind, qualifier)
        }
        "math.max" | "math.min" => {
            let kinds: Vec<ValueType> = values.iter().filter_map(|value| value.kind()).collect();
            if kinds.len() == values.len() && !kinds.is_empty() && kinds.iter().all(is_numeric) {
                let kind = if kinds.iter().all(|k| *k == ValueType::Int) {
                    ValueType::Int
                } else {
                    ValueType::Float
                };
                Inferred::Known(kind, qualifier)
            } else {
                Inferred::Unknown
            }
        }
        _ if name.starts_with("array.new_") => {
            Inferred::Known(ValueType::Array, Some(Qualifier::Series))
        }
        _ => Inferred::Unknown,
    }
}

fn is_numeric(kind: &ValueType) -> bool {
    matches!(kind, ValueType::Int | ValueType::Float)
}

/// The source-spelling of a value type, for diagnostics (`int`, `float`, …).
pub fn value_type_name(kind: ValueType) -> &'static str {
    match kind {
        ValueType::Int => "int",
        ValueType::Float => "float",
        ValueType::Bool => "bool",
        ValueType::String => "string",
        ValueType::Color => "color",
        ValueType::Line => "line",
        ValueType::Label => "label",
        ValueType::Box => "box",
        ValueType::Table => "table",
        ValueType::Linefill => "linefill",
        ValueType::Polyline => "polyline",
        ValueType::Array => "array",
        ValueType::Object => "object",
        ValueType::Map => "map",
        ValueType::Matrix => "matrix",
        ValueType::Time => "time",
    }
}
