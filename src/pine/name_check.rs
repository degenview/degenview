//! Compile-time check for names that resolve to nothing. The runtime only finds an undefined
//! variable (`PINE4008`) or function (`PINE4007`) when the line runs, so `plot(nosuchvariable)`
//! used to compile with no diagnostics. This pass reports the same mistakes without running.
//!
//! A report is an error: the script would fail the run at that name anyway, and showing it as a
//! warning first made the Problems tab flip from yellow to red once the run failed.
//!
//! That includes a member of a namespace whose functions are all listed (`array.resize`,
//! `UNKNOWN_MEMBER`): it was a warning first, but a script with one cannot run correctly, and the
//! chart then ran it up to the line and failed there.
//!
//! It is deliberately lenient, because a false report on a script that runs is worse than a
//! missed typo: a name counts as defined when it is declared *anywhere* in the script (scope and
//! declaration order are not checked), and a dotted name whose head is a builtin namespace is left
//! to the runtime to judge. What it reports is a name that no declaration, import, builtin table
//! or namespace could possibly supply.

use std::collections::HashSet;

use super::ast::{Expression, Statement};
use super::traversal::{expressions, for_each_call, for_each_identifier, nested_blocks};
use super::{
    Diagnostic, DiagnosticCategory, SourceRange, alert_builtins, array_builtins, builtins,
    compiler, drawings, map_builtins, math, matrix_builtins, misc_builtins, strategy_builtins,
    strings, symbol_catalog, ta, ta_extra, visuals,
};

pub const UNDEFINED_VARIABLE: &str = "PINE3050";
pub const UNDEFINED_FUNCTION: &str = "PINE3051";
/// A call to a name in `array`, `map`, `matrix`, `math` or `str` that DegenView does not run.
pub const UNKNOWN_MEMBER: &str = "PINE3053";

/// Reports each variable read and function call whose name nothing defines. `import_aliases` are
/// the aliases of the script's `import` lines; `methods` the names declared with `method`.
pub fn validate(
    statements: &[Statement],
    import_aliases: &HashSet<String>,
    methods: &HashSet<String>,
) -> Vec<Diagnostic> {
    let mut declared: HashSet<String> = import_aliases.iter().chain(methods).cloned().collect();
    collect_declared(statements, &mut declared);

    let mut diagnostics = Vec::new();
    for_each_identifier(statements, &mut |name, range| {
        if !variable_is_defined(name, &declared) {
            diagnostics.push(error(
                UNDEFINED_VARIABLE,
                format!("Undefined variable '{name}'."),
                range,
            ));
        }
    });
    for_each_call(statements, &mut |name, range| {
        if !function_is_defined(name, &declared) {
            diagnostics.push(error(
                UNDEFINED_FUNCTION,
                format!("Undefined function '{name}'."),
                range,
            ));
        } else if let Some(message) = unknown_member(name, &declared) {
            diagnostics.push(error(UNKNOWN_MEMBER, message, range));
        }
    });
    diagnostics.sort_by_key(|diagnostic| diagnostic.range.start.offset);
    diagnostics
}

fn error(code: &str, message: String, range: SourceRange) -> Diagnostic {
    Diagnostic::error(code, DiagnosticCategory::Semantic, message, range)
}

/// The message for a call into a namespace whose every function is listed, when `name` is not one.
///
/// Only the namespaces below qualify: every function the runtime accepts is in the lists named
/// here, so a name missing from them fails when the line runs. The others (`request`, `color`, the
/// drawing namespaces) are dispatched from several places and are left to the runtime. A script's own
/// `array` or `str` (a variable, an import alias) is left alone, as it is for `log`.
fn unknown_member(name: &str, declared: &HashSet<String>) -> Option<String> {
    let (namespace, _) = name.split_once('.')?;
    if name.matches('.').count() != 1 || declared.contains(namespace) || declared.contains(name) {
        return None;
    }
    let (listed, known): (bool, Vec<String>) = match namespace {
        "array" => (
            array_builtins::handles(name),
            array_builtins::KNOWN
                .iter()
                .map(|n| (*n).to_owned())
                .collect(),
        ),
        "map" => (
            map_builtins::handles(name),
            map_builtins::NAMES
                .iter()
                .map(|n| (*n).to_owned())
                .collect(),
        ),
        "matrix" => (
            matrix_builtins::handles(name),
            matrix_builtins::MEMBERS
                .iter()
                .map(|member| format!("matrix.{member}"))
                .collect(),
        ),
        "ta" => (
            ta_is_listed(name),
            ta::SUPPORTED
                .iter()
                .chain(&ta_extra::FUNCTIONS)
                .chain(&TA_INLINE)
                .map(|n| (*n).to_owned())
                .chain(["ta.linreg".to_owned()])
                .collect(),
        ),
        "math" => (
            math::SUPPORTED.contains(&name),
            math::SUPPORTED.iter().map(|n| (*n).to_owned()).collect(),
        ),
        "str" => (
            strings::SUPPORTED.contains(&name) || STR_INLINE.contains(&name),
            strings::SUPPORTED
                .iter()
                .chain(&STR_INLINE)
                .map(|n| (*n).to_owned())
                .collect(),
        ),
        _ => return None,
    };
    if listed {
        return None;
    }
    let mut message = format!("'{name}' is not a function DegenView runs.");
    if let Some(close) = closest(name, &known) {
        message.push_str(&format!(" Did you mean '{close}'?"));
    }
    Some(message)
}

/// `ta.*` functions the session evaluates inline, outside `ta::SUPPORTED` and `ta_extra::FUNCTIONS`.
/// A test reads the engine's source for every `ta.` name it mentions, so one added there and not
/// here fails it rather than reaching a script as a false error.
const TA_INLINE: [&str; 12] = [
    "ta.barssince",
    "ta.vwap",
    "ta.dmi",
    "ta.cum",
    "ta.correlation",
    "ta.tr",
    "ta.atr",
    "ta.bb",
    "ta.pivothigh",
    "ta.pivotlow",
    "ta.sar",
    "ta.supertrend",
];

fn ta_is_listed(name: &str) -> bool {
    ta::is_supported(name) || ta_extra::FUNCTIONS.contains(&name) || TA_INLINE.contains(&name)
}

/// `str.*` functions the session evaluates inline rather than from `strings::SUPPORTED`.
const STR_INLINE: [&str; 1] = ["str.format_time"];

/// The listed name nearest to `name`, if one is a typo away (two edits or fewer).
fn closest<'a>(name: &str, known: &'a [String]) -> Option<&'a str> {
    known
        .iter()
        .map(|candidate| (edit_distance(name, candidate), candidate))
        .filter(|(distance, _)| *distance <= 2)
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, candidate)| candidate.as_str())
}

/// Levenshtein distance between two names.
fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (diagonal + usize::from(ca != *cb))
                .min(row[j] + 1)
                .min(above + 1);
            diagonal = above;
        }
    }
    row[b.len()]
}

/// Every name the script itself introduces, at any depth: variables, tuple parts, loop counters,
/// functions and their parameters, types and enums.
fn collect_declared(statements: &[Statement], declared: &mut HashSet<String>) {
    for statement in statements {
        match statement {
            Statement::Declaration { name, .. } => {
                declared.insert(name.clone());
            }
            Statement::TupleDeclaration { names, .. } => declared.extend(names.iter().cloned()),
            Statement::ForRange { variable, .. } => {
                declared.insert(variable.clone());
            }
            Statement::ForIn { index, value, .. } => {
                declared.extend(index.iter().cloned());
                declared.insert(value.clone());
            }
            Statement::Function {
                name, parameters, ..
            } => {
                declared.insert(name.clone());
                declared.extend(parameters.iter().map(|parameter| parameter.name.clone()));
            }
            Statement::TypeDeclaration { name, .. } | Statement::EnumDeclaration { name, .. } => {
                declared.insert(name.clone());
            }
            _ => {}
        }
        for expression in expressions(statement) {
            collect_declared_in_expression(expression, declared);
        }
        for block in nested_blocks(statement) {
            collect_declared(block, declared);
        }
    }
}

/// `x = if …` and `x = switch …` carry whole statements inside an expression.
fn collect_declared_in_expression(expression: &Expression, declared: &mut HashSet<String>) {
    if let Expression::Statement(statement, _) = expression {
        collect_declared(std::slice::from_ref(statement.as_ref()), declared);
    }
    for child in super::traversal::children(expression) {
        collect_declared_in_expression(child, declared);
    }
}

fn head(name: &str) -> &str {
    name.split('.').next().unwrap_or(name)
}

/// Whether the head of a dotted name can supply it: a declared name (`levels.push`, `zone.top`,
/// `lib.value`, an enum member), a builtin variable a method is called on (`close.scaled(2)`) or a
/// builtin namespace, whose members the runtime judges.
fn head_is_defined(name: &str, declared: &HashSet<String>) -> bool {
    let head = head(name);
    declared.contains(head)
        || symbol_catalog::is_variable(head)
        || compiler::is_builtin_namespace(head)
        || symbol_catalog::OBJECT_TYPES.contains(&head)
        || OTHER_BUILTIN_HEADS.contains(&head)
}

/// Pine namespaces and object types DegenView has no table for, or whose members are only named
/// constants (`size.small`, `display.none`). Scripts using the unsupported ones are reported
/// elsewhere (`PINE9003`, `PINE7001`–`PINE7006`), so this pass leaves their members alone.
const OTHER_BUILTIN_HEADS: &[&str] = &[
    "footprint",
    "volume_row",
    "earnings",
    "dividends",
    "splits",
    "adjustment",
    "backadjustment",
    "barmerge",
    "currency",
    "dayofweek",
    "display",
    "extend",
    "font",
    "format",
    "hline",
    "location",
    "order",
    "plot",
    "position",
    "scale",
    "settlement_as_close",
    "shape",
    "size",
    "text",
    "xloc",
    "yloc",
];

fn variable_is_defined(name: &str, declared: &HashSet<String>) -> bool {
    declared.contains(name)
        || head_is_defined(name, declared)
        || symbol_catalog::is_variable(name)
        || symbol_catalog::is_unimplemented_variable(name)
        || symbol_catalog::is_reserved_constant(name)
        || symbol_catalog::is_function(name)
        || builtins::color(name).is_some()
        || builtins::constant(name).is_some()
        || compiler::is_named_constant(name)
        || matches!(name, "true" | "false" | "na")
        // Pine variables the runtime has no value for; a script reads them only in realtime or
        // tick branches (`ta` library: `timeframe.isticks`), so a script that never gets there runs.
        || matches!(name, "ask" | "bid")
}

fn function_is_defined(name: &str, declared: &HashSet<String>) -> bool {
    // `log` is a namespace of exactly three functions: that it is a namespace does not make
    // `log.hi` one. A script's own `log` (an import alias, say) is left alone.
    if name.starts_with("log.") && !declared.contains("log") && !declared.contains(name) {
        return crate::pine::logs::LogLevel::from_function(name).is_some();
    }
    declared.contains(name)
        || head_is_defined(name, declared)
        || symbol_catalog::FUNCTIONS.contains(&name)
        || symbol_catalog::is_unimplemented_function(name)
        || symbol_catalog::OBJECT_TYPES.contains(&name)
        || UNDOTTED_BUILTINS.contains(&name)
        || visuals::handles(name)
        || drawings::handles(name)
        || strategy_builtins::handles(name)
        || alert_builtins::handles(name)
        || array_builtins::handles(name)
        || map_builtins::handles(name)
        || matrix_builtins::handles(name)
        || math::SUPPORTED.contains(&name)
        || strings::SUPPORTED.contains(&name)
        || ta::is_supported(name)
        || ta_extra::FUNCTIONS.contains(&name)
        || misc_builtins::UNAVAILABLE_FEEDS.contains(&name)
        || misc_builtins::UNIMPLEMENTED_FUNCTIONS.contains(&name)
}

/// Callable names without a namespace that the runtime dispatches inline rather than from a table.
const UNDOTTED_BUILTINS: &[&str] = &["time", "time_close", "max_bars_back", "fixnan"];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::DiagnosticSeverity;
    use crate::pine::compiler::compile;
    use crate::pine::{Limits, compiler::NoLibraries};

    fn codes(body: &str) -> Vec<String> {
        let source = format!("//@version=6\nindicator(\"T\")\n{body}");
        compile(&source, Limits::default(), &NoLibraries)
            .diagnostics
            .into_iter()
            .filter(|d| d.code == UNDEFINED_VARIABLE || d.code == UNDEFINED_FUNCTION)
            .map(|d| format!("{}:{}", d.code, d.message))
            .collect()
    }

    fn unknown_members(body: &str) -> Vec<Diagnostic> {
        let source = format!("//@version=6\nindicator(\"T\")\n{body}");
        compile(&source, Limits::default(), &NoLibraries)
            .diagnostics
            .into_iter()
            .filter(|d| d.code == UNKNOWN_MEMBER)
            .collect()
    }

    #[test]
    fn an_unknown_array_function_is_an_error_on_the_whole_name() {
        let body = "var a = array.new_float()\nif barstate.islast\n    array.resize(a, 5)\n";
        let found = unknown_members(body);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].severity, DiagnosticSeverity::Error);
        assert!(
            found[0]
                .message
                .starts_with("'array.resize' is not a function")
        );
        let source = format!("//@version=6\nindicator(\"T\")\n{body}");
        let range = found[0].range;
        assert_eq!(
            &source[range.start.offset..range.end.offset],
            "array.resize"
        );
    }

    #[test]
    fn a_typo_of_a_listed_function_names_the_one_meant() {
        let hint = |body: &str| unknown_members(body).remove(0).message;
        assert!(
            hint("a = array.new_float()\narray.pusj(a, 1)").ends_with("Did you mean 'array.push'?")
        );
        assert!(hint("plot(math.sqr(close))").ends_with("Did you mean 'math.sqrt'?"));
        assert!(hint("x = str.upperr(\"a\")").ends_with("Did you mean 'str.upper'?"));
        assert!(
            hint("m = map.new<string, float>()\nmap.putt(m, \"a\", 1.0)")
                .ends_with("Did you mean 'map.put'?")
        );
        // Nothing near it: no guess.
        assert!(!hint("a = array.new_float()\narray.resize1(a, 5)").contains("Did you mean"));
    }

    #[test]
    fn listed_functions_and_generic_forms_are_not_warned() {
        let body = "\
a = array.new<float>(3)
b = array.new_float()
c = array.from(1.0, 2.0)
array.push(b, 1.0)
m = map.new<string, float>()
map.put(m, \"k\", 1.0)
x = matrix.new<float>(2, 2, 0.0)
y = math.max(1, 2) + math.sqrt(4) + math.sum(close, 3)
s = str.format(\"{0}\", 1) + str.tostring(2)
";
        assert!(
            unknown_members(body).is_empty(),
            "{:?}",
            unknown_members(body)
        );
    }

    #[test]
    fn an_unknown_ta_function_is_an_error_with_a_hint_for_a_typo() {
        let found = unknown_members("x = ta.covariance(close, open, 5)");
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].severity, DiagnosticSeverity::Error);
        assert!(
            found[0]
                .message
                .starts_with("'ta.covariance' is not a function")
        );
        let hint = unknown_members("plot(ta.smaa(close, 3))").remove(0).message;
        assert!(hint.ends_with("Did you mean 'ta.sma'?"), "{hint}");
        // Used by the script, listed or inline: all fine.
        let fine = "a = ta.sma(close, 3) + ta.atr(14) + ta.correlation(close, open, 5)\n\
                    [m, s, h] = ta.macd(close, 12, 26, 9)\nb = ta.linreg(close, 5, 0)\n\
                    c = ta.stoch(close, high, low, 14)\nd = ta.tr(true)";
        assert!(
            unknown_members(fine).is_empty(),
            "{:?}",
            unknown_members(fine)
        );
    }

    /// Every `ta.` name between quotes in the engine's source is a function or a variable the check
    /// knows, so a name added to the interpreter without being listed fails here.
    #[test]
    fn every_ta_name_the_engine_mentions_is_known() {
        let files = [
            include_str!("session.rs"),
            include_str!("ta.rs"),
            include_str!("ta_call.rs"),
            include_str!("ta_extra.rs"),
            include_str!("market_values.rs"),
        ];
        let mut unknown = Vec::new();
        let mut seen = 0;
        for text in files {
            let code = text.split("#[cfg(test)]").next().unwrap_or(text);
            for piece in code.split("\"ta.").skip(1) {
                let member: String = piece
                    .chars()
                    .take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_')
                    .collect();
                if member.is_empty() || !piece[member.len()..].starts_with('"') {
                    continue;
                }
                seen += 1;
                let name = format!("ta.{member}");
                if !ta_is_listed(&name) && !ta_extra::VARIABLES.contains(&name.as_str()) {
                    unknown.push(name);
                }
            }
        }
        assert!(seen > 40, "the scan found only {seen} names");
        assert!(unknown.is_empty(), "{unknown:?}");
    }

    #[test]
    fn other_namespaces_are_left_to_the_runtime() {
        assert!(unknown_members("c = color.nosuch(1)").is_empty());
        assert!(unknown_members("t = request.nosuch(1)").is_empty());
    }

    #[test]
    fn an_undefined_variable_is_reported() {
        assert_eq!(
            codes("plot(nosuchvariable)"),
            ["PINE3050:Undefined variable 'nosuchvariable'."]
        );
    }

    #[test]
    fn log_has_three_functions_and_no_others() {
        assert!(codes("log.info(\"a\")\nlog.warning(\"b\")\nlog.error(\"c\")").is_empty());
        assert_eq!(
            codes("log.hi(\"hiii\")"),
            ["PINE3051:Undefined function 'log.hi'."]
        );
        assert_eq!(codes("log.infos(\"x\")").len(), 1);
    }

    #[test]
    fn an_import_named_log_keeps_its_own_members() {
        let mut libraries = std::collections::HashMap::new();
        libraries.insert(
            "user/Mine/1".to_owned(),
            "//@version=6\nlibrary(\"Mine\")\nexport hi(string text) => text\n".to_owned(),
        );
        struct Libraries(std::collections::HashMap<String, String>);
        impl crate::pine::compiler::LibraryResolver for Libraries {
            fn source_for_library(&self, path: &str) -> Option<String> {
                self.0.get(path).cloned()
            }
        }
        let source =
            "//@version=6\nindicator(\"T\")\nimport user/Mine/1 as log\nx = log.hi(\"a\")\n";
        let program = compile(source, Limits::default(), &Libraries(libraries));
        assert!(
            program
                .diagnostics
                .iter()
                .all(|d| d.code != UNDEFINED_FUNCTION),
            "{:?}",
            program.diagnostics
        );
    }

    #[test]
    fn an_undefined_function_is_reported() {
        assert_eq!(
            codes("plot(undefined_fn(close))"),
            ["PINE3051:Undefined function 'undefined_fn'."]
        );
    }

    #[test]
    fn a_dotted_name_under_an_unknown_head_is_reported() {
        assert_eq!(
            codes("plot(nosuch.value)\nnosuch.run()").len(),
            2,
            "a variable and a call"
        );
    }

    #[test]
    fn declared_names_are_defined() {
        let body = "\
len = input.int(14, \"Length\")
var float total = 0.0
[a, b] = [close, open]
f(x, y = 1) =>
    z = x + y
    z * 2
method scaled(float self, float k) => self * k
type Zone
    float top
    float bottom
enum Mode
    fast
    slow
zone = Zone.new(high, low)
total += f(a) + zone.top + close.scaled(2.0)
for i = 0 to 3
    total += i
for [j, v] in array.from(1.0, 2.0)
    total += j + v
levels = array.new_float()
levels.push(total)
mode = Mode.fast
plot(total + b)";
        assert_eq!(codes(body), Vec::<String>::new());
    }

    #[test]
    fn names_declared_inside_a_value_statement_are_defined() {
        let body = "\
x = if close > open
    inner = close
    inner
else
    open
plot(x)";
        assert_eq!(codes(body), Vec::<String>::new());
    }

    #[test]
    fn builtins_are_defined() {
        let body = "\
plot(ta.sma(close, 3) + math.abs(open) + nz(volume) + bar_index + hl2)
plot(syminfo.mintick + timeframe.multiplier + time + year(time))
if barstate.islast and not na(close)
    label.new(bar_index, high, str.tostring(close), color = color.red, style = label.style_label_down)
hline(0, linestyle = hline.style_dashed)
plotshape(close > open, style = shape.triangleup, location = location.top, size = size.small)
alertcondition(close > open, \"c\", \"m\")
plot(close, display = display.none, linewidth = 2)";
        assert_eq!(codes(body), Vec::<String>::new());
    }

    #[test]
    fn pine_variables_without_a_runtime_value_are_defined() {
        // The `ta` library reads these in a `timeframe.isticks` branch only.
        let body = "if timeframe.isticks\n    plot(math.avg(ask, bid))";
        assert_eq!(codes(body), Vec::<String>::new());
    }

    #[test]
    fn every_catalog_variable_is_defined() {
        let mut body = String::new();
        for name in symbol_catalog::VARIABLES {
            body.push_str(&format!("v_{} = {name}\n", name.replace('.', "_")));
        }
        assert_eq!(codes(&body), Vec::<String>::new());
    }

    #[test]
    fn every_catalog_function_is_defined() {
        let mut body = String::new();
        for name in symbol_catalog::FUNCTIONS {
            body.push_str(&format!("{name}()\n"));
        }
        assert_eq!(codes(&body), Vec::<String>::new());
    }

    #[test]
    fn library_aliases_are_defined() {
        let source = "//@version=6\nindicator(\"T\")\nimport someone/Lib/1 as lib\nplot(lib.value)\nlib.run(close)";
        let found: Vec<_> = compile(source, Limits::default(), &NoLibraries)
            .diagnostics
            .into_iter()
            .filter(|d| d.code == UNDEFINED_VARIABLE || d.code == UNDEFINED_FUNCTION)
            .collect();
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn the_report_is_an_error_with_the_name_range() {
        let source = "//@version=6\nindicator(\"T\")\nplot(nosuchvariable)";
        let program = compile(source, Limits::default(), &NoLibraries);
        let found = program
            .diagnostics
            .iter()
            .find(|d| d.code == UNDEFINED_VARIABLE)
            .expect("reported");
        assert_eq!(found.severity, crate::pine::DiagnosticSeverity::Error);
        assert_eq!(found.range.start.line, 3);
        assert!(!program.is_valid(), "the script is refused, not run");
    }
}
