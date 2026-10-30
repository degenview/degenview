//! Signature help: for the call the caret is in, `ta.sma(source, length) → series float` with the
//! argument under the caret marked. A port of the spirit of `PineSignatureResolver`, covering the
//! builtins the editor offers; every name here is checked against the completion catalogue.

use super::edit_ops::{self, CallAt};
use super::symbols::{self, Kind, SymbolTable};
use crate::pine::{array_builtins, map_builtins, matrix_builtins, symbol_catalog, ta_call};

/// What to show: the call, its parameters, and which one the caret is on.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Help {
    pub name: String,
    /// Each parameter as it is shown: a builtin's name, or a user parameter as the script writes it
    /// (`float factor = 2.0`).
    pub parameters: Vec<String>,
    /// The name of each parameter, which a `name =` keyword in the call matches.
    pub names: Vec<String>,
    /// Empty for a function the script declares: its type needs the checker, which does not run
    /// while typing.
    pub returns: String,
    pub active: Option<usize>,
    /// How many other declarations of this name there are (a function declared for several types).
    pub overloads: usize,
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
        "id, from_entry, qty, qty_percent, profit, limit, loss, stop, trail_price, trail_points, trail_offset, oca_name, comment, comment_profit, comment_loss, comment_trailing, alert_message, alert_profit, alert_loss, alert_trailing",
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

fn active_parameter(names: &[String], call: &CallAt) -> Option<usize> {
    if let Some(keyword) = &call.keyword
        && let Some(index) = names.iter().position(|p| p == keyword)
    {
        return Some(index);
    }
    if call.argument < names.len() {
        Some(call.argument)
    } else if names.last().is_some_and(|last| last.ends_with('…')) {
        Some(names.len() - 1)
    } else {
        None
    }
}

/// [`help_with`] for a caller that has no symbol table of the text.
pub fn help(source: &str, caret: usize) -> Option<Help> {
    help_with(source, caret, &symbols::analyze(source))
}

/// Signature help for the call around `caret`: a function or method the script declares, or a
/// builtin. `table` is the symbols of `source`.
pub fn help_with(source: &str, caret: usize, table: &SymbolTable) -> Option<Help> {
    let call = edit_ops::enclosing_call(source, caret)?;
    if let Some(help) = user_help(&call, caret, table) {
        return Some(help);
    }
    let (parameters, returns) = signature_of(&call.name)?;
    let active = active_parameter(&parameters, &call);
    Some(Help {
        name: call.name,
        names: parameters.clone(),
        parameters,
        returns: returns.to_owned(),
        active,
        overloads: 0,
    })
}

/// The declarations a method call `path.member(…)` can mean, or None when it is not a call to a
/// method the script declares (a builtin namespace, a library, a value of unknown kind).
fn receiver_methods<'a>(
    table: &'a SymbolTable,
    caret: usize,
    path: &str,
    member: &str,
) -> Option<Vec<&'a symbols::Symbol>> {
    // Without a known receiver type, the methods of that name stand for the call only when they all
    // belong to one type: guessing between types would show the wrong parameters.
    let by_name = || {
        let all = table.methods_named(member, None);
        let one_type = all.windows(2).all(|pair| {
            pair[0].receiver.as_deref().map(symbols::short_name)
                == pair[1].receiver.as_deref().map(symbols::short_name)
        });
        (!all.is_empty() && one_type).then_some(all)
    };
    if path.is_empty() {
        // A method called on something that is not a plain path: `f().member(…)`.
        return by_name();
    }
    let head = path.split('.').next().unwrap_or(path);
    match table.value_at(head, caret) {
        Some(symbol) if symbol.kind == Kind::ImportAlias => None,
        Some(_) => match table.type_of_path(path, caret)? {
            Some(type_name) => {
                let methods = table.methods_named(member, Some(&type_name));
                (!methods.is_empty()).then_some(methods)
            }
            None => by_name(),
        },
        // A builtin series (`close.scaled(…)`); a namespace (`ta.sma(…)`) is a builtin's.
        None if !path.contains('.') && symbol_catalog::is_variable(head) => by_name(),
        None => None,
    }
}

/// The parameters a call shows: all of them, or without the receiver for `value.name(…)`.
fn shown(declaration: &symbols::Symbol, method_syntax: bool) -> Vec<&symbols::ParamSig> {
    declaration
        .params
        .iter()
        .skip(usize::from(method_syntax))
        .collect()
}

/// Help for a function or method the script declares. A method called as `value.name(…)` is shown
/// without its receiver, the first parameter, which is the value before the dot.
fn user_help(call: &CallAt, caret: usize, table: &SymbolTable) -> Option<Help> {
    let method_syntax = call.name.contains('.');
    let candidates = match call.name.rsplit_once('.') {
        Some((path, member)) => receiver_methods(table, caret, path, member)?,
        None => table.callables_named(&call.name, caret),
    };
    // The first declaration that can take the argument being typed; the first one otherwise.
    let takes = |declaration: &symbols::Symbol| {
        let parameters = shown(declaration, method_syntax);
        call.argument < parameters.len()
            || call
                .keyword
                .as_ref()
                .is_some_and(|keyword| parameters.iter().any(|p| &p.name == keyword))
    };
    let chosen = candidates
        .iter()
        .find(|declaration| takes(declaration))
        .or(candidates.first())?;
    let parameters = shown(chosen, method_syntax);
    let names: Vec<String> = parameters.iter().map(|p| p.name.clone()).collect();
    let active = active_parameter(&names, call);
    Some(Help {
        name: call.name.trim_start_matches('.').to_owned(),
        parameters: parameters.iter().map(|p| p.text.clone()).collect(),
        names,
        returns: String::new(),
        active,
        overloads: candidates.len() - 1,
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
        assert_eq!(
            (stoch.returns.as_str(), stoch.active),
            ("series float", Some(2))
        );
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
        assert_eq!(
            (sma.returns.as_str(), sma.active),
            ("series float", Some(1))
        );
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

    // MARK: Functions and methods the script declares

    fn at_end(text: &str) -> Option<Help> {
        help(text, text.chars().count())
    }

    #[test]
    fn a_function_the_script_declares_has_help() {
        let found = at_end("half(x) => x / 2\nhalf(").unwrap();
        assert_eq!(found.name, "half");
        assert_eq!(found.parameters, ["x"]);
        assert_eq!(found.names, ["x"]);
        assert_eq!((found.active, found.overloads), (Some(0), 0));
        assert!(found.returns.is_empty(), "the type needs the checker");
    }

    #[test]
    fn parameters_are_shown_as_written_with_types_qualifiers_and_defaults() {
        let found =
            at_end("scale(float x, simple int length = 14, array<float> v) => 1\nscale(1, ")
                .unwrap();
        assert_eq!(
            found.parameters,
            ["float x", "simple int length = 14", "array<float> v"]
        );
        assert_eq!(found.names, ["x", "length", "v"]);
        assert_eq!(found.active, Some(1));
        // A parameter list wrapped over lines still reads as one line each.
        let wrapped = at_end("scale(float x,\n      float k = 2.0) => 1\nscale(").unwrap();
        assert_eq!(wrapped.parameters, ["float x", "float k = 2.0"]);
    }

    #[test]
    fn keywords_match_parameter_names_not_their_text() {
        let found = at_end("scale(float x, float k = 2.0) => 1\nscale(1, k = ").unwrap();
        assert_eq!(found.active, Some(1));
        let found = at_end("scale(float x, float k = 2.0) => 1\nscale(k = 3, ").unwrap();
        assert_eq!(found.active, Some(1), "a keyword wins over position");
    }

    #[test]
    fn a_method_called_with_a_dot_hides_its_receiver() {
        let zone = "type Zone\n    float top\n\
                    method grow(Zone this, float by, int times = 1) => this\n";
        for declaration in [
            "z = Zone.new(1.0)",
            "Zone z = na",
            "var Zone z = Zone.new(1.0)",
        ] {
            let text = format!("{zone}{declaration}\nz.grow(");
            let found = at_end(&text).unwrap_or_else(|| panic!("{declaration}"));
            assert_eq!(found.name, "z.grow");
            assert_eq!(found.parameters, ["float by", "int times = 1"]);
            assert_eq!(found.active, Some(0), "the receiver is not an argument");
        }
        let text = format!("{zone}z = Zone.new(1.0)\nz.grow(1.0, ");
        assert_eq!(at_end(&text).unwrap().active, Some(1));
    }

    #[test]
    fn a_method_on_a_builtin_series_or_self_has_help() {
        let found =
            at_end("method scaled(float this, float k) => this * k\nclose.scaled(").unwrap();
        assert_eq!(found.parameters, ["float k"]);
        let found = at_end(
            "type S\n    int n\nmethod forget(S self, int keep = 0) =>\n    self.n := 0\n    self.forget(",
        )
        .unwrap();
        assert_eq!(found.parameters, ["int keep = 0"]);
        // A method called on a call's result: the only method of that name.
        let found =
            at_end("method scaled(float this, float k) => this * k\nmake().scaled(").unwrap();
        assert_eq!(found.parameters, ["float k"]);
    }

    #[test]
    fn overloads_are_picked_by_the_receiver_type() {
        let methods = "type Dog\n    int legs\ntype Bird\n    int wings\n\
                       method describe(Dog this, string prefix) => 1\n\
                       method describe(Bird this, int times) => 2\n\
                       d = Dog.new(4)\nb = Bird.new(2)\nx = foo()\n";
        let dog = at_end(&format!("{methods}d.describe(")).unwrap();
        assert_eq!(
            (dog.parameters, dog.overloads),
            (vec!["string prefix".to_owned()], 0)
        );
        let bird = at_end(&format!("{methods}b.describe(")).unwrap();
        assert_eq!(bird.parameters, ["int times"]);
        // An untyped receiver could be either: nothing is guessed.
        assert!(at_end(&format!("{methods}x.describe(")).is_none());
    }

    #[test]
    fn overloads_of_one_type_are_picked_by_how_many_arguments_are_given() {
        let text = "method fill(array<float> a, float v) => a\n\
                    method fill(array<float> a, float inner, float outer, float lo, float hi) => a\n\
                    values = array.new_float()\n";
        let first = at_end(&format!("{text}values.fill(")).unwrap();
        assert_eq!((first.parameters.len(), first.overloads), (1, 1));
        let more = at_end(&format!("{text}values.fill(1.0, 0.0, ")).unwrap();
        assert_eq!(more.parameters.len(), 4);
        assert_eq!((more.active, more.overloads), (Some(2), 1));
    }

    #[test]
    fn a_method_called_as_a_function_shows_its_receiver() {
        let found =
            at_end("type Z\n    int a\nmethod grow(Z this, float by) => this\ngrow(").unwrap();
        assert_eq!(found.parameters, ["Z this", "float by"]);
    }

    #[test]
    fn builtins_are_not_taken_over_by_methods_of_the_same_name() {
        let found = at_end("method push(array<float> a) => a\narray.push(").unwrap();
        assert_eq!(found.parameters.first().map(String::as_str), Some("id"));
        assert_eq!(found.overloads, 0);
        assert!(at_end("ta.sma(close, ").is_some());
        // A library alias's members are the library's.
        assert!(at_end("import user/lib/1 as lib\nmethod f(float x) => x\nlib.f(").is_none());
        // Not declared anywhere.
        assert!(at_end("nothing(").is_none());
        assert!(
            at_end("method f(float x) => x\nx.f(").is_none(),
            "undeclared head"
        );
    }

    #[test]
    fn user_help_survives_crlf_and_astral_characters() {
        let found = at_end("// 🙂🙂\r\nscale(float x, float k) => 1\r\nscale(1, ").unwrap();
        assert_eq!(found.parameters, ["float x", "float k"]);
        assert_eq!(found.active, Some(1));
    }
}
