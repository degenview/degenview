//! The builtin functions DegenView runs, in one place, for the compiler to ask.
//!
//! Each module that evaluates a family of builtins keeps its own list (`array_builtins::KNOWN`,
//! `math::SUPPORTED`, `drawings::NAMESPACES`, …) and stays the truth the interpreter dispatches from.
//! This module gathers those lists, plus the few names the session evaluates inline, into one lookup,
//! so that a check which needs "is this a builtin?" does not have to know about ten of them.
//!
//! A namespace is **closed** when every function the runtime accepts in it is registered: a call to
//! another name there is a compile error (`name_check`), because it would fail the run at that line.
//! Everything else is open, and left to the runtime. A namespace is added to [`CLOSED`] only when
//! both guards in this module's tests pass: every registered name runs (registry ⊆ runtime), and every
//! `"ns.member"` the engine's source mentions is registered (runtime ⊆ registry). A false error on a
//! script that runs is worse than a missed typo, so a namespace that fails a guard stays open.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use super::drawings::FromName;
use super::outputs::LabelStyle;
use super::{
    array_builtins, builtins, compiler, drawings, footprint_call, input_arguments, map_builtins,
    math, matrix_builtins, misc_builtins, strategy_builtins, strings, symbol_catalog, ta, ta_extra,
};

/// The namespaces whose functions are all registered.
///
/// `request` is not here: an unknown `request.*` already has its own error (`PINE9003`), and a second
/// one would list the same mistake twice. `log` is checked by `name_check` itself.
pub const CLOSED: &[&str] = &[
    "array",
    "map",
    "matrix",
    "math",
    "str",
    "ta",
    "line",
    "label",
    "box",
    "table",
    "linefill",
    "polyline",
    "input",
    "alert",
    "strategy",
    "color",
    "ticker",
    "timeframe",
    "runtime",
    "chart",
    "barstate",
    "session",
    "syminfo",
];

/// The namespaces whose *values* (variables and constants) are all known as well, so that reading
/// another name there is an error (`barstate.nosuch`). A subset of [`CLOSED`]: Pine adds variables
/// to `syminfo` and `strategy` over time, and a community script reading one that is not listed
/// (`syminfo.cftc_code`) must still compile, so those two, and `runtime`, stay open for values.
pub const CLOSED_VALUES: &[&str] = &[
    "array",
    "map",
    "matrix",
    "math",
    "str",
    "ta",
    "line",
    "label",
    "box",
    "table",
    "linefill",
    "polyline",
    "input",
    "alert",
    "color",
    "ticker",
    "timeframe",
    "chart",
    "barstate",
    "session",
];

/// Functions of the remaining families, which the session evaluates in `scalar_call` and
/// `misc_call` rather than from a table of their own.
const OTHER_FUNCTIONS: [&str; 32] = [
    "color.new",
    "color.rgb",
    "color.from_gradient",
    "color.r",
    "color.g",
    "color.b",
    "color.t",
    "ticker.new",
    "ticker.standard",
    "ticker.modify",
    "ticker.inherit",
    "ticker.heikinashi",
    "ticker.renko",
    "ticker.kagi",
    "ticker.linebreak",
    "ticker.pointfigure",
    "timeframe.in_seconds",
    "timeframe.change",
    "timeframe.from_seconds",
    "runtime.error",
    "request.security",
    "request.security_lower_tf",
    "request.currency_rate",
    "request.footprint",
    "chart.point",
    "max_bars_back",
    "fixnan",
    "alert",
    "alertcondition",
    "log.info",
    "log.warning",
    "log.error",
];

/// `ta.*` functions the session evaluates inline, outside `ta::SUPPORTED` and `ta_extra::FUNCTIONS`.
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

/// `str.*` functions the session evaluates inline rather than from `strings::SUPPORTED`.
const STR_INLINE: [&str; 1] = ["str.format_time"];

/// Whether `namespace` is closed: an unregistered function in it is an error.
pub fn is_closed(namespace: &str) -> bool {
    CLOSED.contains(&namespace)
}

/// Whether reading an unregistered value in `namespace` is an error.
pub fn is_closed_for_values(namespace: &str) -> bool {
    CLOSED_VALUES.contains(&namespace)
}

/// Whether `name` is a builtin function DegenView runs.
pub fn is_function(name: &str) -> bool {
    // The `array.new_<type>` family is matched by prefix, not listed.
    registry().functions.contains(name) || array_builtins::handles(name)
}

/// The collections of every drawing a script has made (`line.all`).
pub const DRAWING_COLLECTIONS: [&str; 6] = [
    "line.all",
    "label.all",
    "box.all",
    "table.all",
    "linefill.all",
    "polyline.all",
];

/// Whether `name` is a builtin variable or constant: a value a script reads, not a function it
/// calls. Unlike the functions these are not listed in one place (`builtins::color` and
/// `builtins::constant` are `match`es), so this asks each of the places that can supply one.
pub fn is_value(name: &str) -> bool {
    DRAWING_COLLECTIONS.contains(&name)
        || LabelStyle::from_name(name).is_some()
        || symbol_catalog::is_variable(name)
        || symbol_catalog::is_unimplemented_variable(name)
        || symbol_catalog::is_reserved_constant(name)
        || builtins::color(name).is_some()
        || builtins::constant(name).is_some()
        || compiler::is_named_constant(name)
        || ta_extra::VARIABLES.contains(&name)
}

/// The registered functions of `namespace`, for suggesting the name that was meant.
pub fn members(namespace: &str) -> &'static [String] {
    registry()
        .by_namespace
        .get(namespace)
        .map_or(&[], Vec::as_slice)
}

/// Every registered function, sorted: what the editor offers.
pub fn all_functions() -> &'static [String] {
    &registry().sorted
}

struct Registry {
    functions: HashSet<String>,
    sorted: Vec<String>,
    by_namespace: HashMap<&'static str, Vec<String>>,
}

fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(build)
}

fn build() -> Registry {
    let mut names: Vec<String> = Vec::new();
    let mut add = |name: &str| names.push(name.to_owned());

    for name in array_builtins::KNOWN
        .iter()
        .chain(&map_builtins::NAMES)
        .chain(&math::SUPPORTED)
        .chain(&strings::SUPPORTED)
        .chain(&ta::SUPPORTED)
        .chain(&ta_extra::FUNCTIONS)
        .chain(&input_arguments::FUNCTIONS)
        .chain(&strategy_builtins::NAMES)
        .chain(&misc_builtins::UNAVAILABLE_FEEDS)
        .chain(&misc_builtins::UNIMPLEMENTED_FUNCTIONS)
        .chain(&misc_builtins::CURRENCY_CONVERSIONS)
        .chain(&TA_INLINE)
        .chain(&STR_INLINE)
        .chain(&OTHER_FUNCTIONS)
        .chain(drawings::CHART_POINT_NAMES)
    {
        add(name);
    }
    // `ta.linreg` is handled by the session before `ta::SUPPORTED`, which does not list it.
    add("ta.linreg");
    for member in matrix_builtins::MEMBERS {
        add(&format!("matrix.{member}"));
    }
    for (namespace, members) in drawings::NAMESPACES {
        for member in members {
            add(&format!("{namespace}.{member}"));
        }
    }
    for member in footprint_call::FOOTPRINT_FUNCTIONS {
        add(&format!("footprint.{member}"));
    }
    for member in footprint_call::ROW_FUNCTIONS {
        add(&format!("volume_row.{member}"));
    }
    for property in strategy_builtins::CLOSED_TRADE_PROPERTIES {
        add(&format!("strategy.closedtrades.{property}"));
    }
    for property in strategy_builtins::OPEN_TRADE_PROPERTIES {
        add(&format!("strategy.opentrades.{property}"));
    }

    let mut by_namespace: HashMap<&'static str, Vec<String>> = HashMap::new();
    for namespace in CLOSED {
        let prefix = format!("{namespace}.");
        let mut members: Vec<String> = names
            .iter()
            .filter(|name| name.starts_with(&prefix))
            .cloned()
            .collect();
        members.sort();
        members.dedup();
        by_namespace.insert(namespace, members);
    }
    let functions: HashSet<String> = names.into_iter().collect();
    let mut sorted: Vec<String> = functions.iter().cloned().collect();
    sorted.sort();
    Registry {
        functions,
        sorted,
        by_namespace,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::bench_tests::{bars, limits, symbol};
    use crate::pine::compiler::{self, NoLibraries};
    use crate::pine::session::{RunError, Session};
    use crate::pine::symbol_catalog;

    /// How a one-line script fails when it runs, if it does.
    fn run_error(statement: &str) -> Option<crate::pine::Diagnostic> {
        let source = format!("//@version=6\nindicator(\"T\")\n{statement}\n");
        let program = compiler::compile(&source, limits(), &NoLibraries);
        let mut session = Session::new(
            program,
            std::collections::HashMap::new(),
            limits(),
            Some(0.01),
            symbol(),
            None,
            None,
        );
        match session.evaluate(&bars(3, 60_000)) {
            Err(RunError::Script(diagnostic)) => Some(diagnostic),
            _ => None,
        }
    }

    /// Guard 1, registry ⊆ runtime: every registered function reaches the interpreter's own
    /// handling. Called with no arguments it may fail for many reasons (missing arguments, a
    /// strategy call in an indicator), but not as an unknown function.
    #[test]
    fn every_registered_function_is_one_the_runtime_knows() {
        let mut unknown = Vec::new();
        for name in all_functions() {
            if run_error(&format!("x = {name}()")).is_some_and(|d| d.code == "PINE4007") {
                unknown.push(name.clone());
            }
        }
        assert!(
            unknown.is_empty(),
            "registered but the runtime says unknown: {unknown:?}"
        );
    }

    /// Guard 1 for values: each variable the catalogue lists in a closed namespace reads without
    /// being undefined.
    #[test]
    fn every_listed_variable_is_one_the_runtime_knows() {
        let mut unknown = Vec::new();
        for name in symbol_catalog::VARIABLES {
            let namespace = name.split('.').next().unwrap();
            if !is_closed_for_values(namespace) || !name.contains('.') {
                continue;
            }
            if run_error(&format!("x = {name}")).is_some_and(|d| d.code == "PINE4008") {
                unknown.push(*name);
            }
        }
        assert!(
            unknown.is_empty(),
            "listed but the runtime says undefined: {unknown:?}"
        );
    }

    /// The `"ns.member"` string literals in the non-test source under `src/pine`.
    fn mentioned_names() -> Vec<(String, String)> {
        fn visit(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                if path.is_dir() {
                    if name != "corpus" && name != "tests" {
                        visit(&path, out);
                    }
                } else if name.ends_with(".rs") && !name.ends_with("tests.rs") && name != "tests.rs"
                {
                    out.push(path);
                }
            }
        }
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/pine");
        let mut files = Vec::new();
        visit(&root, &mut files);
        let mut found = Vec::new();
        for file in files {
            let text = std::fs::read_to_string(&file).unwrap();
            let code = text.split("#[cfg(test)]").next().unwrap_or(&text);
            let file_name = file.file_name().unwrap().to_string_lossy().into_owned();
            for piece in code.split('"').skip(1).step_by(2) {
                let is_name = !piece.is_empty()
                    && piece.contains('.')
                    && piece.chars().all(|c| {
                        c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.'
                    })
                    && !piece.starts_with('.')
                    && !piece.ends_with('.')
                    && !piece.ends_with('_');
                if is_name {
                    found.push((piece.to_owned(), file_name.clone()));
                }
            }
        }
        found
    }

    /// Guard 2, runtime ⊆ registry: a name the engine's source mentions in a closed namespace is a
    /// registered function, a variable or constant, or on this short list with the reason.
    const NOT_FUNCTIONS: &[(&str, &str)] = &[];

    #[test]
    fn every_name_the_engine_mentions_in_a_closed_namespace_is_registered() {
        let mut missing = Vec::new();
        let mut seen = 0;
        for (name, file) in mentioned_names() {
            let namespace = name.split('.').next().unwrap();
            if !is_closed(namespace) {
                continue;
            }
            seen += 1;
            if !is_function(&name)
                && !is_value(&name)
                && !NOT_FUNCTIONS.iter().any(|(allowed, _)| *allowed == name)
            {
                missing.push(format!("{name} ({file})"));
            }
        }
        assert!(seen > 150, "the scan found only {seen} names");
        missing.sort();
        missing.dedup();
        assert!(
            missing.is_empty(),
            "mentioned but not registered: {missing:#?}"
        );
    }
}
