//! Signature help: for the call the caret is in, `ta.sma(source, length) → series float` with the
//! argument under the caret marked. A port of the spirit of `PineSignatureResolver`, covering the
//! builtins the editor offers; every name here is checked against the completion catalogue.

use super::edit_ops::{self, CallAt};
use crate::pine::{array_builtins, map_builtins, matrix_builtins, ta_call};

/// What to show: the call, its parameters, and which one the caret is on.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Help {
    pub name: String,
    pub parameters: Vec<String>,
    pub returns: &'static str,
    pub active: Option<usize>,
}

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
        "defval, title, minval, maxval, step, tooltip, inline, group, confirm, options",
        "input int",
    ),
    (
        "input.float",
        "defval, title, minval, maxval, step, tooltip, inline, group, confirm, options",
        "input float",
    ),
    (
        "input.bool",
        "defval, title, tooltip, inline, group, confirm",
        "input bool",
    ),
    (
        "input.string",
        "defval, title, options, tooltip, inline, group, confirm",
        "input string",
    ),
    (
        "input.color",
        "defval, title, tooltip, inline, group, confirm",
        "input color",
    ),
    (
        "input.source",
        "defval, title, tooltip, inline, group",
        "series float",
    ),
    (
        "input.time",
        "defval, title, tooltip, inline, group, confirm",
        "input int",
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
    ("chart.point.new", "time, index, price", "chart.point"),
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
        "line.new",
        "x1, y1, x2, y2, xloc, extend, color, style, width",
        "series line",
    ),
    ("line.delete", "id", ""),
    ("line.copy", "id", "series line"),
    ("line.get_price", "id, x", "series float"),
    ("line.set_xloc", "id, x1, x2, xloc", ""),
    (
        "label.new",
        "x, y, text, xloc, yloc, color, style, textcolor, size, textalign, tooltip",
        "series label",
    ),
    ("label.delete", "id", ""),
    ("label.copy", "id", "series label"),
    ("label.set_xloc", "id, x, xloc", ""),
    ("label.set_yloc", "id, yloc", ""),
    (
        "box.new",
        "left, top, right, bottom, border_color, border_width, border_style, extend, xloc, bgcolor",
        "series box",
    ),
    ("table.cell_set_text", "table_id, column, row, text", ""),
    (
        "table.cell_set_text_color",
        "table_id, column, row, text_color",
        "",
    ),
    (
        "table.cell_set_bgcolor",
        "table_id, column, row, bgcolor",
        "",
    ),
    ("table.cell_set_width", "table_id, column, row, width", ""),
    ("table.cell_set_height", "table_id, column, row, height", ""),
    (
        "table.cell_set_text_halign",
        "table_id, column, row, text_halign",
        "",
    ),
    (
        "table.cell_set_text_valign",
        "table_id, column, row, text_valign",
        "",
    ),
    (
        "table.cell_set_text_size",
        "table_id, column, row, text_size",
        "",
    ),
    (
        "table.cell_set_tooltip",
        "table_id, column, row, tooltip",
        "",
    ),
    (
        "table.cell_set_text_font_family",
        "table_id, column, row, text_font_family",
        "",
    ),
    (
        "table.cell_set_text_formatting",
        "table_id, column, row, text_formatting",
        "",
    ),
    ("box.copy", "id", "series box"),
    ("box.set_xloc", "id, left, right, xloc", ""),
    ("chart.point.copy", "id", "chart.point"),
    (
        "table.new",
        "position, columns, rows, bgcolor, frame_color, frame_width, border_color, border_width",
        "series table",
    ),
    (
        "table.cell",
        "table_id, column, row, text, width, height, text_color, text_halign, text_valign, text_size, bgcolor, tooltip, text_font_family, text_formatting",
        "",
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
        "id, from_entry, qty, qty_percent, profit, limit, loss, stop, comment",
        "",
    ),
    ("runtime.error", "message", ""),
];

/// The parameters and return type of `name`, from the table or the engine's own collection
/// parameter lists (`array.push(id, value)`).
pub fn signature_of(name: &str) -> Option<(Vec<String>, &'static str)> {
    if let Some((_, params, returns)) = SIGNATURES.iter().find(|(n, _, _)| *n == name) {
        return Some((params.split(", ").map(str::to_owned).collect(), returns));
    }
    if let Some(parameters) = ta_call::parameters(name) {
        let returns = if name == "ta.kc" {
            "[series float, series float, series float]"
        } else {
            "series float"
        };
        return Some((
            parameters.iter().map(|p| (*p).to_owned()).collect(),
            returns,
        ));
    }
    let collection = if array_builtins::handles(name) {
        array_builtins::parameters(name)
    } else if map_builtins::handles(name) {
        map_builtins::parameters(name)
    } else if matrix_builtins::handles(name) {
        matrix_builtins::parameters(name)
    } else {
        return None;
    };
    Some((collection.iter().map(|p| (*p).to_owned()).collect(), ""))
}

fn active_parameter(parameters: &[String], call: &CallAt) -> Option<usize> {
    if let Some(keyword) = &call.keyword
        && let Some(index) = parameters.iter().position(|p| p == keyword)
    {
        return Some(index);
    }
    if call.argument < parameters.len() {
        Some(call.argument)
    } else if parameters.last().is_some_and(|last| last.ends_with('…')) {
        Some(parameters.len() - 1)
    } else {
        None
    }
}

/// Signature help for the call around `caret`, when the callee is a known builtin.
pub fn help(source: &str, caret: usize) -> Option<Help> {
    let call = edit_ops::enclosing_call(source, caret)?;
    let (parameters, returns) = signature_of(&call.name)?;
    let active = active_parameter(&parameters, &call);
    Some(Help {
        name: call.name,
        parameters,
        returns,
        active,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script_manager::completion;

    #[test]
    fn every_signature_names_something_the_editor_offers() {
        let offered: Vec<&str> = completion::builtins()
            .iter()
            .map(|i| i.label.as_str())
            .collect();
        let strangers: Vec<&str> = SIGNATURES
            .iter()
            .map(|(name, _, _)| *name)
            .filter(|name| !offered.contains(name))
            .collect();
        assert!(strangers.is_empty(), "{strangers:?}");
        // No duplicates either.
        let mut names: Vec<&str> = SIGNATURES.iter().map(|(n, _, _)| *n).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len());
    }

    #[test]
    fn newer_builtins_have_help_from_the_engine_tables() {
        let at = |text: &str| help(text, text.chars().count()).unwrap();
        let stoch = at("ta.stoch(close, high, ");
        assert_eq!(stoch.parameters, ["source", "high", "low", "length"]);
        assert_eq!((stoch.returns, stoch.active), ("series float", Some(2)));
        assert_eq!(
            at("ta.kc(close, 20, ").returns,
            "[series float, series float, series float]"
        );
        assert_eq!(at("str.replace(s, ").active, Some(1));
        assert_eq!(at("array.fill(a, 0, ").active, Some(2));
    }

    #[test]
    fn the_active_parameter_follows_the_caret_and_keywords() {
        let at = |text: &str| help(text, text.chars().count());
        let sma = at("ta.sma(close, ").unwrap();
        assert_eq!(sma.parameters, ["source", "length"]);
        assert_eq!((sma.returns, sma.active), ("series float", Some(1)));
        assert_eq!(at("ta.sma(").unwrap().active, Some(0));
        assert_eq!(
            at("ta.sma(close, 14, ").unwrap().active,
            None,
            "past the last parameter"
        );
        assert_eq!(at("plot(close, title = ").unwrap().active, Some(1));
        assert_eq!(
            at("plot(close, linewidth = 2, color = ").unwrap().active,
            Some(2),
            "a keyword wins over position"
        );
        assert_eq!(
            at("math.max(1, 2, 3, ").unwrap().active,
            Some(2),
            "a variadic tail repeats"
        );
        assert!(at("myFunc(").is_none(), "not a builtin");
        assert!(at("(1 + ").is_none());
    }

    #[test]
    fn collections_use_the_engines_own_parameter_names() {
        let push = help("array.push(", 11).unwrap();
        assert_eq!(push.parameters.first().map(String::as_str), Some("id"));
        assert!(push.parameters.len() >= 2);
    }
}
