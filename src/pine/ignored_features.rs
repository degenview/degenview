//! Warnings about TradingView features the engine accepts and then ignores, ported from
//! `PineIgnoredFeatureValidator.swift` + `PineIgnoredFeatures.swift`.

use std::collections::HashMap;

use super::ast::{Argument, Expression, Statement};
use super::traversal::{for_each_call_with_args, for_each_identifier, for_each_string_literal};
use crate::pine::{Diagnostic, DiagnosticCategory, DiagnosticSeverity, SourceRange};

/// Arguments, by function. A declaration (`indicator`, `strategy`, `library`) is keyed by its name.
fn ignored_arguments() -> HashMap<&'static str, HashMap<&'static str, &'static str>> {
    let mut table: HashMap<&'static str, HashMap<&'static str, &'static str>> = HashMap::new();
    table.insert("indicator", declaration());
    table.insert("library", declaration());
    let mut strategy = declaration();
    strategy.insert(
        "margin_long",
        "There is no margin model, so no margin call ever closes a position.",
    );
    strategy.insert(
        "margin_short",
        "There is no margin model, so no margin call ever closes a position.",
    );
    strategy.insert(
        "use_bar_magnifier",
        "Orders fill on the chart's own bars; intrabar data does not refine the fills.",
    );
    strategy.insert(
        "backtest_fill_limits_assumption",
        "A limit order fills as soon as price reaches it, with no extra ticks required.",
    );
    strategy.insert(
        "risk_free_rate",
        "The report's risk-adjusted ratios do not use a risk-free rate.",
    );
    table.insert("strategy", strategy);
    table.insert("strategy.entry", entry());
    table.insert("strategy.order", entry());
    table.insert("strategy.exit", exit());
    table.insert("strategy.close", close());
    table.insert("strategy.close_all", close());
    table.insert("plot", plot());
    table.insert("plotarrow", plotarrow());
    for function in std::iter::once("input").chain(super::input_arguments::FUNCTIONS) {
        table.insert(function, input());
    }
    for rule in [
        "strategy.risk.max_drawdown",
        "strategy.risk.max_intraday_loss",
        "strategy.risk.max_intraday_filled_orders",
        "strategy.risk.max_cons_loss_days",
    ] {
        table.insert(
            rule,
            HashMap::from([("alert_message", "Order fill alerts are not sent.")]),
        );
    }
    table.insert("hline", hline());
    table.insert("bgcolor", color_series());
    table.insert("barcolor", color_series());
    table.insert("fill", fill());
    table.insert("plotcandle", plotcandle());
    table.insert("box.new", box_new());
    table.insert("table.cell", table_cell());
    let mut request_security = request();
    request_security.insert("calc_bars_count", "The whole history is always requested.");
    table.insert("request.security", request_security);
    table.insert("request.security_lower_tf", request());
    table.insert("plotshape", marker());
    table.insert("plotchar", marker());
    table
}

fn table_cell() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        ("tooltip", "Cell tooltips are not shown."),
        (
            "text_font_family",
            "Cell text is always drawn in the chart's own font.",
        ),
        (
            "text_formatting",
            "Cell text is never drawn bold or italic.",
        ),
    ])
}

fn declaration() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        (
            "explicit_plot_zorder",
            "Plots and drawings are painted in the app's own order.",
        ),
        (
            "scale",
            "The chart has one value axis, so the scale is not changed.",
        ),
        (
            "calc_bars_count",
            "The script always runs over the whole history.",
        ),
        (
            "timeframe",
            "The script always runs on the chart's own timeframe.",
        ),
        (
            "timeframe_gaps",
            "The script always runs on the chart's own timeframe.",
        ),
    ])
}

fn entry() -> HashMap<&'static str, &'static str> {
    HashMap::from([("alert_message", "Order fill alerts are not sent.")])
}

fn exit() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        ("alert_message", "Order fill alerts are not sent."),
        ("alert_profit", "Order fill alerts are not sent."),
        ("alert_loss", "Order fill alerts are not sent."),
        ("alert_trailing", "Order fill alerts are not sent."),
        (
            "oca_name",
            "Orders are never grouped; each fills on its own.",
        ),
    ])
}

fn close() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        ("alert_message", "Order fill alerts are not sent."),
        (
            "immediately",
            "The position closes on the next bar's open, as with any market order.",
        ),
    ])
}

fn plot() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        (
            "offset",
            "Partly. The plot is shifted, but values that land past the last bar are not drawn.",
        ),
        ("trackprice", "No price line is drawn across the chart."),
        ("join", "Isolated points are not connected."),
        ("editable", "Plot settings cannot be edited from the chart."),
        (
            "show_last",
            "The plot is drawn on every bar, not only the last ones.",
        ),
        ("format", "The value is formatted by the chart's own rules."),
        (
            "precision",
            "The value is formatted by the chart's own rules.",
        ),
        ("linestyle", "The line is always solid."),
    ])
}

fn input() -> HashMap<&'static str, &'static str> {
    HashMap::from([(
        "display",
        "The input is not listed in the status line or the Data Window.",
    )])
}

fn plotarrow() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        (
            "offset",
            "Partly. The arrows are shifted, but those that land past the last bar are not drawn.",
        ),
        ("editable", "Plot settings cannot be edited from the chart."),
        ("format", "The value is formatted by the chart's own rules."),
        (
            "precision",
            "The value is formatted by the chart's own rules.",
        ),
    ])
}

fn hline() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        ("linestyle", "The line is always solid."),
        (
            "linewidth",
            "The line is always drawn at the default width.",
        ),
        ("editable", "The line cannot be edited from the chart."),
    ])
}

fn color_series() -> HashMap<&'static str, &'static str> {
    HashMap::from([("editable", "The color cannot be edited from the chart.")])
}

fn fill() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        ("fillgaps", "Gaps in the plots are not bridged."),
        ("editable", "The fill cannot be edited from the chart."),
        (
            "show_last",
            "The fill is drawn on every bar, not only the last ones.",
        ),
    ])
}

fn plotcandle() -> HashMap<&'static str, &'static str> {
    HashMap::from([(
        "offset",
        "The candles are not shifted; each is drawn on the bar that produced it.",
    )])
}

fn box_new() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        ("text_wrap", "Box text is never wrapped."),
        ("text_font_family", "Box text uses the default font."),
        (
            "text_formatting",
            "Box text has no bold or italic formatting.",
        ),
    ])
}

fn request() -> HashMap<&'static str, &'static str> {
    HashMap::from([("currency", "Prices are not converted to another currency.")])
}

fn marker() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        ("text", "The text next to the marker is not drawn."),
        ("textcolor", "The text next to the marker is not drawn."),
        (
            "offset",
            "The markers are not shifted; each is drawn on the bar that produced it.",
        ),
        (
            "transp",
            "Transparency is not applied; use `color.new(color, transparency)` instead.",
        ),
        ("editable", "The markers cannot be edited from the chart."),
        (
            "show_last",
            "The markers are drawn on every bar, not only the last ones.",
        ),
    ])
}

/// Whole functions that do nothing, by name prefix. None today: the risk rules are enforced.
fn function_prefixes() -> [(&'static str, &'static str); 0] {
    []
}

/// Functions the engine accepts but cannot honour: what they return instead, by exact name.
fn stubbed_functions() -> Vec<(&'static str, &'static str)> {
    let feed = "The data feed is not available here, so it always returns `na`.";
    let mut table: Vec<(&'static str, &'static str)> = super::misc_builtins::UNAVAILABLE_FEEDS
        .iter()
        .map(|name| (*name, feed))
        .collect();
    table.retain(|(name, _)| *name != "request.currency_rate");
    table.push((
        "request.currency_rate",
        "Exchange rates are not available here: it returns 1 for the same currency and `na` otherwise.",
    ));
    for name in super::misc_builtins::UNIMPLEMENTED_FUNCTIONS {
        table.push((name, "It is not implemented, so it always returns `na`."));
    }
    for name in super::misc_builtins::CURRENCY_CONVERSIONS {
        table.push((
            name,
            "No exchange rates are available: the amount is returned unchanged, which is right only while the account and the symbol share a currency.",
        ));
    }
    table.push(("table.cell_set_tooltip", "Cell tooltips are not shown."));
    table.push((
        "table.cell_set_text_font_family",
        "Cell text is always drawn in the chart's own font.",
    ));
    table.push((
        "table.cell_set_text_formatting",
        "Cell text is never drawn bold or italic.",
    ));
    table.push((
        "ticker.modify",
        "Session and adjustment modifiers are not applied; the symbol is used as given.",
    ));
    table.push((
        "ticker.inherit",
        "Modifiers are not modelled; the new symbol is used as given.",
    ));
    table
}

/// Functions that work, with a limit the script's author should know: what is different, by exact
/// name. The warning reads "only partly supported", as an argument with a limit does.
fn partly_supported_functions() -> [(&'static str, &'static str); 1] {
    [(
        "request.footprint",
        "It is built from 1-minute or longer intrabar candles (no tick data), so it is coarser than \
         TradingView's. It is `na` on charts of 1 minute or less, on price-based charts, and for a bar \
         that would need more than 1000 rows (use a larger `ticks_per_row`).",
    )]
}

/// Argument values that make the engine draw something else, keyed function → argument → constant.
/// None today: every constant the reference lists for these arguments is drawn as it says.
fn ignored_values()
-> HashMap<&'static str, HashMap<&'static str, HashMap<&'static str, &'static str>>> {
    HashMap::new()
}

pub fn validate(statements: &[Statement]) -> Vec<Diagnostic> {
    let own: std::collections::HashSet<&str> = statements
        .iter()
        .filter_map(|statement| match statement {
            Statement::Function { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect();
    let arguments = ignored_arguments();
    let values = ignored_values();
    let prefixes = function_prefixes();
    let stubbed = stubbed_functions();
    let partly = partly_supported_functions();

    let mut diagnostics = Vec::new();
    for_each_call_with_args(statements, &mut |name, call_arguments, range| {
        if own.contains(name) {
            return;
        }
        if let Some((_, explanation)) = prefixes.iter().find(|(p, _)| name.starts_with(p)) {
            diagnostics.push(warning(
                "PINE7002",
                format!("{name}() is ignored. {explanation}"),
                spanning(name, range),
            ));
        }
        if let Some((_, explanation)) = stubbed.iter().find(|(stub, _)| *stub == name) {
            diagnostics.push(warning(
                "PINE7005",
                format!("{name}() is not supported. {explanation}"),
                spanning(name, range),
            ));
        }
        if let Some((_, limit)) = partly.iter().find(|(partial, _)| *partial == name) {
            diagnostics.push(warning(
                "PINE7001",
                format!("{name}() is only partly supported. {limit}"),
                spanning(name, range),
            ));
        }
        for argument in call_arguments {
            let Some(argument_name) = argument.name.as_deref() else {
                continue;
            };
            if let Some(explanation) = arguments
                .get(name)
                .and_then(|table| table.get(argument_name))
            {
                // An explanation that starts "Partly." belongs to an argument that works with a limit.
                let message = match explanation.strip_prefix("Partly. ") {
                    Some(limit) => {
                        format!("{name}(): `{argument_name}` is only partly supported. {limit}")
                    }
                    None => format!("{name}(): `{argument_name}` is ignored. {explanation}"),
                };
                diagnostics.push(warning(
                    "PINE7001",
                    message,
                    argument.name_range.unwrap_or(argument.value.range()),
                ));
            }
            if let Expression::Identifier(constant, value_range) = &argument.value
                && let Some(explanation) = values
                    .get(name)
                    .and_then(|table| table.get(argument_name))
                    .and_then(|table| table.get(constant.as_str()))
            {
                diagnostics.push(warning(
                    "PINE7004",
                    format!(
                        "{name}(): `{argument_name} = {constant}` is not supported. {explanation}"
                    ),
                    spanning(constant, *value_range),
                ));
            }
        }
    });
    for_each_identifier(statements, &mut |name, range| {
        if (super::symbol_catalog::is_unimplemented_variable(name) || name == "session.extended")
            && !own.contains(name)
        {
            diagnostics.push(warning(
                "PINE7003",
                format!(
                    "`{name}` is not supported. It is not modelled here, so it is always `na`."
                ),
                spanning(name, range),
            ));
        }
    });
    // A zone the built-in table lacks reads as UTC, which moves every session and calendar part.
    for_each_string_literal(statements, &mut |text, range| {
        if super::timezone::looks_like_iana(text) && super::timezone::resolve(text).is_none() {
            diagnostics.push(warning(
                "PINE7007",
                format!(
                    "Time zone '{text}' is not in the built-in table, so it is read as UTC. Use a UTC offset such as UTC-5."
                ),
                range,
            ));
        }
    });
    diagnostics.sort_by_key(|d| d.range.start.offset);
    diagnostics
}

/// The parser records only the first token of a dotted name; the underline should cover all of it.
fn spanning(name: &str, range: SourceRange) -> SourceRange {
    let length = name.encode_utf16().count();
    if range.end.offset - range.start.offset >= length {
        return range;
    }
    let mut end = range.start;
    end.column += length;
    end.offset += length;
    SourceRange::new(range.start, end)
}

pub(super) fn warning(code: &str, message: String, range: SourceRange) -> Diagnostic {
    Diagnostic {
        code: code.to_owned(),
        severity: DiagnosticSeverity::Warning,
        category: DiagnosticCategory::Unsupported,
        message,
        range,
    }
}
