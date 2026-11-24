//! The parameters and result of the builtin functions, by name: what the editor shows as signature
//! help and what the checker needs to tell which parameter a positional argument fills. The engine's
//! own families keep their parameter lists next to their code (`array::parameters`, `ta::call`,
//! `drawings::parameters`); this adds the table of the functions that have none.

use crate::pine::builtins::{array, map, matrix, ta::call};
use crate::pine::outputs::drawings;

/// `(name, "param, param", returns)`. A trailing `…` marks a variadic tail.
const SIGNATURES: &[(&str, &str, &str)] = &[
    (
        "indicator",
        "title, shorttitle, overlay, format, precision, max_bars_back",
        "",
    ),
    (
        "strategy",
        "title, shorttitle, overlay, initial_capital, default_qty_type, default_qty_value, commission_type, commission_value, pyramiding",
        "",
    ),
    (
        "plot",
        "series, title, color, linewidth, style, trackprice, histbase, offset, join, editable, show_last, display",
        "plot",
    ),
    (
        "hline",
        "price, title, color, linestyle, linewidth, editable, display",
        "hline",
    ),
    (
        "plotshape",
        "series, title, style, location, color, offset, text, textcolor, size, display",
        "",
    ),
    (
        "plotchar",
        "series, title, char, location, color, offset, text, textcolor, size, display",
        "",
    ),
    (
        "ta.pivot_point_levels",
        "type, anchor, developing",
        "array<float>",
    ),
    (
        "plotarrow",
        "series, title, colorup, colordown, offset, minheight, maxheight, editable, show_last, display, format, precision, force_overlay",
        "",
    ),
    (
        "plotcandle",
        "open, high, low, close, title, color, wickcolor, bordercolor, display",
        "",
    ),
    (
        "plotbar",
        "open, high, low, close, title, color, editable, show_last, display",
        "",
    ),
    (
        "bgcolor",
        "color, offset, editable, show_last, title, display",
        "",
    ),
    (
        "barcolor",
        "color, offset, editable, show_last, title, display",
        "",
    ),
    (
        "fill",
        "plot1, plot2, color, title, editable, fillgaps, display",
        "",
    ),
    ("alert", "message, freq", ""),
    ("alertcondition", "condition, title, message", ""),
    (
        "input.int",
        "defval, title, minval, maxval, step, tooltip, inline, group, confirm, display, active",
        "input int",
    ),
    (
        "input.float",
        "defval, title, minval, maxval, step, tooltip, inline, group, confirm, display, active",
        "input float",
    ),
    (
        "input.bool",
        "defval, title, tooltip, inline, group, confirm, display, active",
        "input bool",
    ),
    (
        "input.string",
        "defval, title, options, tooltip, inline, group, confirm, display, active",
        "input string",
    ),
    (
        "input.color",
        "defval, title, tooltip, inline, group, confirm, display, active",
        "input color",
    ),
    (
        "input.source",
        "defval, title, tooltip, inline, group, display, active, confirm",
        "series float",
    ),
    (
        "input.time",
        "defval, title, tooltip, inline, group, confirm, display, active",
        "input int",
    ),
    (
        "input.price",
        "defval, title, tooltip, inline, group, confirm, display, active",
        "input float",
    ),
    (
        "input.symbol",
        "defval, title, tooltip, inline, group, confirm, display, active",
        "input string",
    ),
    (
        "input.timeframe",
        "defval, title, options, tooltip, inline, group, confirm, display, active",
        "input string",
    ),
    (
        "input.session",
        "defval, title, options, tooltip, inline, group, confirm, display, active",
        "input string",
    ),
    (
        "input.text_area",
        "defval, title, tooltip, group, confirm, display, active",
        "input string",
    ),
    (
        "input.enum",
        "defval, title, options, tooltip, inline, group, confirm, display, active",
        "input enum",
    ),
    ("nz", "source, replacement", "series float"),
    ("na", "x", "bool"),
    ("fixnan", "source", "series float"),
    ("ta.sma", "source, length", "series float"),
    ("ta.ema", "source, length", "series float"),
    ("ta.rma", "source, length", "series float"),
    ("ta.wma", "source, length", "series float"),
    ("ta.hma", "source, length", "series float"),
    ("ta.rsi", "source, length", "series float"),
    ("ta.stdev", "source, length", "series float"),
    ("ta.variance", "source, length", "series float"),
    ("ta.median", "source, length", "series float"),
    ("ta.highest", "source, length", "series float"),
    ("ta.lowest", "source, length", "series float"),
    ("ta.change", "source, length", "series float"),
    ("ta.mom", "source, length", "series float"),
    ("ta.roc", "source, length", "series float"),
    ("ta.cci", "source, length", "series float"),
    ("ta.linreg", "source, length, offset", "series float"),
    ("ta.crossover", "source1, source2", "series bool"),
    ("ta.crossunder", "source1, source2", "series bool"),
    ("ta.cross", "source1, source2", "series bool"),
    ("ta.rising", "source, length", "series bool"),
    ("ta.falling", "source, length", "series bool"),
    ("ta.atr", "length", "series float"),
    ("ta.tr", "handle_na", "series float"),
    (
        "ta.bb",
        "series, length, mult",
        "[series float, series float, series float]",
    ),
    (
        "ta.macd",
        "source, fastlen, slowlen, siglen",
        "[series float, series float, series float]",
    ),
    (
        "ta.dmi",
        "diLength, adxSmoothing",
        "[series float, series float, series float]",
    ),
    (
        "ta.pivothigh",
        "source, leftbars, rightbars",
        "series float",
    ),
    ("ta.pivotlow", "source, leftbars, rightbars", "series float"),
    ("ta.cum", "source", "series float"),
    ("ta.correlation", "source1, source2, length", "series float"),
    ("ta.barssince", "condition", "series int"),
    ("ta.vwap", "source, anchor, stdev_mult", "series float"),
    ("ta.sar", "start, inc, max", "series float"),
    (
        "ta.supertrend",
        "factor, atrPeriod",
        "[series float, series int]",
    ),
    ("math.abs", "number", "series float"),
    ("math.sqrt", "number", "series float"),
    ("math.pow", "base, exponent", "series float"),
    ("math.round", "number, precision", "series float"),
    ("math.floor", "number", "series int"),
    ("math.ceil", "number", "series int"),
    ("math.max", "number0, number1, …", "series float"),
    ("math.min", "number0, number1, …", "series float"),
    ("math.avg", "number0, number1, …", "series float"),
    ("math.sum", "source, length", "series float"),
    ("math.log", "number", "series float"),
    ("math.exp", "number", "series float"),
    ("str.tostring", "value, format", "series string"),
    ("str.tonumber", "string", "series float"),
    ("str.format", "formatString, arg0, …", "series string"),
    ("str.format_time", "time, format, timezone", "series string"),
    ("str.contains", "source, str", "series bool"),
    ("str.length", "string", "series int"),
    ("str.upper", "string", "series string"),
    ("str.lower", "string", "series string"),
    (
        "str.substring",
        "source, begin_pos, end_pos",
        "series string",
    ),
    (
        "str.replace_all",
        "source, target, replacement",
        "series string",
    ),
    ("str.split", "string, separator", "array<string>"),
    (
        "str.replace",
        "source, target, replacement, occurrence",
        "series string",
    ),
    ("str.pos", "source, str", "series int"),
    ("max_bars_back", "var, num", ""),
    (
        "ticker.new",
        "prefix, ticker, session, adjustment, backadjustment, settlement_as_close",
        "simple string",
    ),
    ("ticker.standard", "symbol", "simple string"),
    (
        "ticker.modify",
        "tickerid, session, adjustment, backadjustment, settlement_as_close",
        "simple string",
    ),
    ("ticker.inherit", "from_tickerid, symbol", "simple string"),
    ("log.info", "formatString, arg0, …", ""),
    ("log.warning", "formatString, arg0, …", ""),
    ("log.error", "formatString, arg0, …", ""),
    (
        "request.currency_rate",
        "from, to, ignore_invalid_currency",
        "series float",
    ),
    (
        "request.footprint",
        "ticks_per_row, va_percent, imbalance_percent",
        "series footprint",
    ),
    ("footprint.buy_volume", "id", "series float"),
    ("footprint.sell_volume", "id", "series float"),
    ("footprint.total_volume", "id", "series float"),
    ("footprint.delta", "id", "series float"),
    ("footprint.poc", "id", "volume_row"),
    ("footprint.vah", "id", "volume_row"),
    ("footprint.val", "id", "volume_row"),
    ("footprint.rows", "id", "array<volume_row>"),
    ("footprint.get_row_by_price", "id, price", "volume_row"),
    ("volume_row.up_price", "id", "series float"),
    ("volume_row.down_price", "id", "series float"),
    ("volume_row.buy_volume", "id", "series float"),
    ("volume_row.sell_volume", "id", "series float"),
    ("volume_row.total_volume", "id", "series float"),
    ("volume_row.delta", "id", "series float"),
    ("volume_row.has_buy_imbalance", "id", "series bool"),
    ("volume_row.has_sell_imbalance", "id", "series bool"),
    ("color.new", "color, transp", "series color"),
    ("color.rgb", "red, green, blue, transp", "series color"),
    (
        "color.from_gradient",
        "value, bottom_value, top_value, bottom_color, top_color",
        "series color",
    ),
    ("color.r", "color", "series float"),
    ("color.g", "color", "series float"),
    ("color.b", "color", "series float"),
    ("color.t", "color", "series float"),
    ("timeframe.in_seconds", "timeframe", "series int"),
    ("timeframe.change", "timeframe", "series bool"),
    ("timeframe.from_seconds", "seconds", "series string"),
    (
        "request.security",
        "symbol, timeframe, expression, gaps, lookahead, ignore_invalid_symbol, currency",
        "series",
    ),
    (
        "strategy.entry",
        "id, direction, qty, limit, stop, oca_name, oca_type, comment, alert_message",
        "",
    ),
    (
        "strategy.close",
        "id, comment, qty, qty_percent, alert_message, immediately",
        "",
    ),
    (
        "strategy.exit",
        "id, from_entry, qty, qty_percent, profit, limit, loss, stop, trail_price, trail_points, trail_offset, oca_name, comment, comment_profit, comment_loss, comment_trailing, alert_message, alert_profit, alert_loss, alert_trailing",
        "",
    ),
    ("runtime.error", "message", ""),
];

/// The names in the table, for the editor's check that each one is offered.
#[cfg(test)]
pub fn tabled_names() -> impl Iterator<Item = &'static str> {
    SIGNATURES.iter().map(|(name, _, _)| *name)
}

/// The parameter names of `name` in Pine's order, or None for a function without a list.
pub fn parameters(name: &str) -> Option<Vec<&'static str>> {
    if let Some((_, parameters, _)) = SIGNATURES.iter().find(|(n, _, _)| *n == name) {
        return Some(parameters.split(", ").collect());
    }
    if let Some(parameters) = call::parameters(name) {
        return Some(parameters.to_vec());
    }
    let collection = if array::handles(name) {
        array::parameters(name)
    } else if map::handles(name) {
        map::parameters(name)
    } else if matrix::handles(name) {
        matrix::parameters(name)
    } else {
        drawings::parameters(name)
    };
    (!collection.is_empty()).then(|| collection.to_vec())
}

/// What `name` returns, as the editor writes it; empty for a function that returns nothing or has
/// no description of its result.
pub fn returns(name: &str) -> &'static str {
    if let Some((_, _, returns)) = SIGNATURES.iter().find(|(n, _, _)| *n == name) {
        return returns;
    }
    if call::parameters(name).is_some() {
        return if name == "ta.kc" {
            "[series float, series float, series float]"
        } else {
            "series float"
        };
    }
    drawing_returns(name)
}

fn drawing_returns(name: &str) -> &'static str {
    match name {
        "chart.point"
        | "chart.point.new"
        | "chart.point.from_index"
        | "chart.point.from_time"
        | "chart.point.now"
        | "chart.point.copy" => "chart.point",
        "line.new" | "line.copy" | "linefill.get_line1" | "linefill.get_line2" => "series line",
        "label.new" | "label.copy" => "series label",
        "box.new" | "box.copy" => "series box",
        "linefill.new" => "series linefill",
        "polyline.new" => "series polyline",
        "table.new" => "series table",
        "line.get_price" | "line.get_y1" | "line.get_y2" | "label.get_y" | "box.get_top"
        | "box.get_bottom" => "series float",
        "line.get_x1" | "line.get_x2" | "label.get_x" | "box.get_left" | "box.get_right" => {
            "series int"
        }
        "label.get_text" => "series string",
        _ => "",
    }
}

/// The parameters and return type of `name`, for signature help.
pub fn signature_of(name: &str) -> Option<(Vec<String>, &'static str)> {
    let parameters = parameters(name)?;
    Some((
        parameters.iter().map(|p| (*p).to_owned()).collect(),
        returns(name),
    ))
}
