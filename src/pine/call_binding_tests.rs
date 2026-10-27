//! `call_binding` against the chain it shortcuts: a routed run must give exactly what the chain gives.
//!
//! `Session::ignore_routes` (tests only) makes a session take the chain for every call, so each test
//! runs a script both ways and compares everything the script produced.

use std::collections::HashMap;

use super::ast::{Expression, Route, Statement};
use super::bench_tests::{self as bench, bars, limits, symbol};
use super::call_binding::{bind, route_of};
use super::compiler::{self, LibraryResolver, NoLibraries};
use super::session::{RunError, Session};
use super::traversal::{children, expressions, nested_blocks};

const MINUTE: i64 = 60_000;

/// What a run of `source` produced, or how it failed, as one comparable string.
fn outcome(source: &str, resolver: &dyn LibraryResolver, ignore_routes: bool) -> String {
    let program = compiler::compile(source, limits(), resolver);
    let mut session = Session::new(
        program,
        HashMap::new(),
        limits(),
        Some(0.01),
        symbol(),
        None,
        None,
    );
    session.ignore_routes = ignore_routes;
    match session.evaluate(&bars(40, MINUTE)) {
        Ok(result) => format!("{:?}", result.output),
        Err(RunError::Script(diagnostic)) => format!("{diagnostic:?}"),
        Err(other) => format!("{other:?}"),
    }
}

fn same_both_ways(source: &str) -> String {
    let routed = outcome(source, &NoLibraries, false);
    let chained = outcome(source, &NoLibraries, true);
    assert_eq!(
        routed, chained,
        "routed and chained runs differ for:\n{source}"
    );
    routed
}

/// Every call in `statements` with its route, in source order.
fn calls(statements: &[Statement]) -> Vec<(String, Option<Route>)> {
    fn in_expression(expression: &Expression, out: &mut Vec<(String, Option<Route>)>) {
        if let Expression::Call { name, route, .. } = expression {
            out.push((name.clone(), *route));
        }
        if let Expression::Statement(statement, _) = expression {
            in_statements(std::slice::from_ref(statement.as_ref()), out);
        }
        for child in children(expression) {
            in_expression(child, out);
        }
    }
    fn in_statements(statements: &[Statement], out: &mut Vec<(String, Option<Route>)>) {
        for statement in statements {
            for expression in expressions(statement) {
                in_expression(expression, out);
            }
            for block in nested_blocks(statement) {
                in_statements(block, out);
            }
        }
    }
    let mut out = Vec::new();
    in_statements(statements, &mut out);
    out
}

fn route_in(source: &str, name: &str) -> Option<Route> {
    let mut program = compiler::compile(source, limits(), &NoLibraries);
    bind(&mut program);
    calls(&program.statements)
        .into_iter()
        .find(|(call, _)| call == name)
        .unwrap_or_else(|| panic!("no call to {name} in:\n{source}"))
        .1
}

#[test]
fn names_route_to_the_family_that_evaluates_them() {
    let expected = [
        ("nz", Some(Route::Scalar)),
        ("str.tostring", Some(Route::Scalar)),
        ("year", Some(Route::Scalar)),
        ("array.push", Some(Route::Collection)),
        ("array.new_float", Some(Route::Collection)),
        ("map.get", Some(Route::Collection)),
        ("matrix.set", Some(Route::Collection)),
        ("math.max", Some(Route::Math)),
        ("math.sum", Some(Route::TaTable)),
        ("str.length", Some(Route::Strings)),
        ("str.split", None),
        ("ta.sma", Some(Route::TaTable)),
        ("ta.highest", Some(Route::TaTable)),
        ("ta.alma", Some(Route::TaExtra)),
        ("ta.atr", Some(Route::TaInline)),
        ("ta.linreg", Some(Route::TaInline)),
        ("fixnan", Some(Route::TaInline)),
        ("line.new", Some(Route::Drawings)),
        ("line.set_x1", Some(Route::Drawings)),
        ("chart.point.new", None),
        ("plot", Some(Route::Visuals)),
        ("strategy.entry", Some(Route::Strategy)),
        ("strategy.closedtrades.profit", Some(Route::StrategyTrade)),
        ("alert", Some(Route::Alert)),
        ("input.int", Some(Route::Input)),
        ("indicator", Some(Route::Input)),
        ("request.security", None),
        ("log.info", None),
        ("ticker.new", None),
        ("array.nosuch", None),
        ("my_function", None),
    ];
    for (name, route) in expected {
        assert_eq!(route_of(name), route, "{name}");
    }
}

#[test]
fn every_registered_function_without_a_route_is_known_to_be_one() {
    // A function added to the interpreter and not routed still runs, through the chain, only
    // slower. This list makes leaving one out a decision rather than an accident.
    let unrouted: Vec<&str> = super::registry::all_functions()
        .iter()
        .map(String::as_str)
        .filter(|name| route_of(name).is_none())
        .collect();
    let allowed = |name: &str| {
        name.starts_with("request.")
            || name.starts_with("log.")
            || name.starts_with("ticker.")
            || name.starts_with("footprint.")
            || name.starts_with("volume_row.")
            || name.starts_with("chart.point")
            || matches!(
                name,
                "str.split"
                    | "max_bars_back"
                    | "strategy.convert_to_account"
                    | "strategy.convert_to_symbol"
            )
    };
    let surprising: Vec<&&str> = unrouted.iter().filter(|name| !allowed(name)).collect();
    assert!(
        surprising.is_empty(),
        "registered, runs, but has no route: {surprising:?}"
    );
}

#[test]
fn binding_reaches_calls_in_every_kind_of_statement_and_in_libraries() {
    let source = "//@version=6
indicator(\"T\")
f(x) =>
    math.abs(x)
type Box2
    float v = math.sqrt(4)
var a = array.new_float()
for i = 0 to 2
    array.push(a, math.max(i, 1))
if close > open
    plot(ta.sma(close, 3))
else
    plot(ta.ema(close, 3))
y = switch
    close > open => math.min(1, 2)
    => math.floor(3.5)
w = while false
    str.length(\"x\")
plot(f(1) + math.round(close)[1])
";
    let mut program = compiler::compile(source, limits(), &NoLibraries);
    bind(&mut program);
    for (name, route) in calls(&program.statements) {
        if name == "f" {
            assert_eq!(route, None, "a user function is not routed");
        } else {
            assert!(route.is_some(), "{name} was left unbound");
        }
    }
}

#[test]
fn the_workloads_agree_both_ways() {
    for (name, source, security) in [
        ("simple", bench::SIMPLE, false),
        ("many_ta", bench::MANY_TA, false),
        ("many_math", bench::MANY_MATH, false),
        ("many_str", bench::MANY_STR, false),
        ("plots_and_drawings", bench::PLOTS_AND_DRAWINGS, false),
        ("containers", bench::CONTAINERS, false),
        ("map_counting", bench::MAP_COUNTING, false),
        ("map_counting_strings", bench::MAP_COUNTING_STRINGS, false),
        ("nested_functions", bench::NESTED_FUNCTIONS, false),
        ("strategy", bench::STRATEGY, false),
        ("loops", bench::LOOPS, false),
        ("security", bench::SECURITY, true),
    ] {
        let data = bars(300, MINUTE);
        let run = |ignore: bool| {
            bench::run_script(source, &data, security, |session| {
                session.ignore_routes = ignore;
            })
        };
        let (routed, chained) = (run(false), run(true));
        assert_eq!(
            format!("{:?}", routed.output()),
            format!("{:?}", chained.output()),
            "{name}"
        );
    }
}

#[test]
fn every_registered_function_runs_the_same_both_ways() {
    for name in super::registry::all_functions() {
        let header = if name.starts_with("strategy.") {
            "strategy(\"T\")"
        } else {
            "indicator(\"T\")"
        };
        for arguments in ["", "close, 2", "close, close, 3", "\"a\", 1.5"] {
            same_both_ways(&format!(
                "//@version=6\n{header}\nx = {name}({arguments})\nplot(close)\n"
            ));
        }
    }
}

#[test]
fn a_user_method_with_a_builtins_name_still_wins() {
    let source = "//@version=6
indicator(\"T\")
method nz(float self, float other) => other * 2
plot(nz(1.0, 21.0))
";
    assert_eq!(route_in(source, "nz"), None);
    same_both_ways(source);
    // And it did run the user's method, not the builtin.
    assert!(same_both_ways(source).contains("42"));
}

#[test]
fn a_user_type_named_like_a_drawing_namespace_still_constructs() {
    let source = "//@version=6
indicator(\"T\")
type line
    float width = 1.0
l = line.new(width = 5.0)
plot(l.width)
";
    assert_eq!(route_in(source, "line.new"), None);
    assert!(same_both_ways(source).contains('5'));
}

#[test]
fn an_import_alias_named_like_a_namespace_shadows_only_what_the_library_exports() {
    struct One;
    impl LibraryResolver for One {
        fn source_for_library(&self, path: &str) -> Option<String> {
            (path == "user/Mine/1").then(|| {
                "//@version=6\nlibrary(\"Mine\")\nexport sma(float source, int length) => source * 1000\n"
                    .to_owned()
            })
        }
    }
    let source = "//@version=6
indicator(\"T\")
import user/Mine/1 as ta
plot(ta.sma(close, 3))
plot(math.max(close, open))
";
    let mut program = compiler::compile(source, limits(), &One);
    assert!(program.is_valid(), "{:?}", program.diagnostics);
    bind(&mut program);
    let found = calls(&program.statements);
    let route = |name: &str| found.iter().find(|(call, _)| call == name).unwrap().1;
    // The alias is captured, so every `ta.` call stays on the chain; `math.max` is not affected.
    assert_eq!(route("ta.sma"), None);
    assert_eq!(route("math.max"), Some(Route::Math));
    assert_eq!(outcome(source, &One, false), outcome(source, &One, true));
}

#[test]
fn a_variable_named_like_a_namespace_does_not_change_which_builtin_runs() {
    let source = "//@version=6
indicator(\"T\")
array = 3
a = array.new_float()
array.push(a, 4.0)
plot(array.get(a, 0) + array)
";
    same_both_ways(source);
}

#[test]
fn a_call_in_a_debugger_expression_is_not_bound_and_still_runs() {
    // A watch expression is parsed on its own and never passes through `bind`: its calls have no
    // route, and run the chain.
    let mut program = compiler::compile(
        "//@version=6\nindicator(\"T\")\nplot(math.max(1, 2))\n",
        limits(),
        &NoLibraries,
    );
    assert!(
        calls(&program.statements)
            .iter()
            .all(|(_, route)| route.is_none())
    );
    bind(&mut program);
    assert!(
        calls(&program.statements)
            .iter()
            .any(|(_, route)| route.is_some())
    );
}
