//! `call_binding` against the chain it shortcuts: a routed run must give exactly what the chain gives.
//!
//! `Session::ignore_routes` (tests only) makes a session take the chain for every call, so each test
//! runs a script both ways and compares everything the script produced.

use std::collections::HashMap;

use super::total::{self, is_total};
use super::{INLINE, MORE_SCALAR_NAMES, SCALAR_NAMES, bind, route_of};
use crate::pine::builtins::ta::extra;
use crate::pine::compiler::{self, LibraryResolver, NoLibraries};
use crate::pine::runtime::session::{RunError, Session};
use crate::pine::syntax::ast::{Expression, Route, Statement};
use crate::pine::syntax::traversal::{children, expressions, nested_blocks};
use crate::pine::tests::bench::{self as bench, bars, limits, symbol};

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
    let unrouted: Vec<&str> = crate::pine::compiler::registry::all_functions()
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

/// A quarter of the registry, so the four quarters run in parallel instead of one long loop.
fn registered_functions_run_the_same_both_ways(quarter: usize) {
    for name in crate::pine::compiler::registry::all_functions()
        .iter()
        .enumerate()
        .filter(|(index, _)| index % 4 == quarter)
        .map(|(_, name)| name)
    {
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
fn every_registered_function_runs_the_same_both_ways_1_of_4() {
    registered_functions_run_the_same_both_ways(0);
}

#[test]
fn every_registered_function_runs_the_same_both_ways_2_of_4() {
    registered_functions_run_the_same_both_ways(1);
}

#[test]
fn every_registered_function_runs_the_same_both_ways_3_of_4() {
    registered_functions_run_the_same_both_ways(2);
}

#[test]
fn every_registered_function_runs_the_same_both_ways_4_of_4() {
    registered_functions_run_the_same_both_ways(3);
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

// MARK: Calls whose history a lazy operand reads

/// The program as the session sees it: lazy histories hoisted, then calls bound.
fn hoisted(
    source: &str,
    resolver: &dyn LibraryResolver,
) -> crate::pine::compiler::program::CompiledProgram {
    let mut program = compiler::compile(source, limits(), resolver);
    assert!(program.is_valid(), "{:?}", program.diagnostics);
    super::hoist::lazy_history(&mut program);
    bind(&mut program);
    program
}

/// The synthetic declarations in `statements`, at any depth, as (name, the called function, its
/// call site, its route).
fn synthetic(statements: &[Statement]) -> Vec<(String, String, usize, Option<Route>)> {
    let mut found = Vec::new();
    for statement in statements {
        if let Statement::Declaration { name, value, .. } = statement
            && statement.is_synthetic()
        {
            let Expression::Call {
                name: callee,
                site,
                route,
                ..
            } = value
            else {
                panic!("a synthetic declaration holds a call: {value:?}");
            };
            found.push((name.clone(), callee.clone(), *site, *route));
        }
        for block in nested_blocks(statement) {
            found.extend(synthetic(block));
        }
    }
    found
}

/// The callee names of the synthetic declarations of `source`, in order.
fn hoisted_calls(source: &str) -> Vec<String> {
    let program = hoisted(
        &format!("//@version=6\nindicator(\"T\")\n{source}\n"),
        &NoLibraries,
    );
    synthetic(&program.statements)
        .into_iter()
        .map(|(_, callee, _, _)| callee)
        .collect()
}

#[test]
fn a_call_with_a_history_in_a_ternary_arm_moves_ahead_of_its_statement_unchanged() {
    let source = "//@version=6\nindicator(\"T\")\nx = bar_index % 2 == 0 ? ta.sma(close, 3)[1] : na\nplot(x)\n";
    let program = hoisted(source, &NoLibraries);
    let moved = synthetic(&program.statements);
    assert_eq!(moved.len(), 1);
    let (name, callee, _, route) = &moved[0];
    assert_eq!(callee, "ta.sma");
    assert_eq!(
        *route,
        Some(Route::TaTable),
        "the moved call is routed like any other"
    );
    // Before the statement it came from, which now reads the variable's history.
    let [_, first, second, _] = &program.statements[..] else {
        panic!("{:?}", program.statements);
    };
    assert!(first.is_synthetic());
    let Statement::Declaration { value, .. } = second else {
        panic!("{second:?}");
    };
    let Expression::Ternary { when_true, .. } = value else {
        panic!("{value:?}");
    };
    let Expression::History { base, .. } = when_true.as_ref() else {
        panic!("{when_true:?}");
    };
    assert_eq!(**base, Expression::Identifier(name.clone(), base.range()));
}

#[test]
fn the_moved_call_keeps_its_call_site() {
    let source = "//@version=6\nindicator(\"T\")\nx = bar_index % 2 == 0 ? ta.sma(close, 3)[1] : na\nplot(ta.ema(close, 3))\n";
    let sites = |program: &crate::pine::compiler::program::CompiledProgram| {
        let mut sites = Vec::new();
        crate::pine::syntax::traversal::for_each_call_with_args(
            &program.statements,
            &mut |name, _, range| {
                sites.push((name.to_owned(), range.start.offset));
            },
        );
        sites.sort();
        sites
    };
    let before = sites(&compiler::compile(source, limits(), &NoLibraries));
    let after = sites(&hoisted(source, &NoLibraries));
    assert_eq!(
        before, after,
        "no call is added, dropped or moved in the source"
    );
}

#[test]
fn only_a_history_in_a_lazy_operand_moves_a_call() {
    // Eager positions run on every bar the statement does: nothing to move.
    assert!(hoisted_calls("x = ta.sma(close, 3)[1]").is_empty());
    assert!(hoisted_calls("x = ta.sma(close, 3)[1] > 0 ? 1.0 : 2.0").is_empty());
    assert!(hoisted_calls("x = ta.sma(close, 3)[1] > 0 and close > 0").is_empty());
    assert!(hoisted_calls("x = close > 0 ? ta.sma(close, 3) : na").is_empty());
    assert!(hoisted_calls("x = close > 0 ? close[1] : na").is_empty());
    // The lazy ones.
    assert_eq!(
        hoisted_calls("x = close > 0 ? ta.sma(close, 3)[1] : na"),
        ["ta.sma"]
    );
    assert_eq!(
        hoisted_calls("x = close > 0 ? na : ta.ema(close, 3)[2]"),
        ["ta.ema"]
    );
    assert_eq!(
        hoisted_calls("x = close > 0 and ta.sma(close, 3)[1] > 0"),
        ["ta.sma"]
    );
    assert_eq!(
        hoisted_calls("x = close < 0 or ta.sma(close, 3)[1] > 0"),
        ["ta.sma"]
    );
    assert_eq!(
        hoisted_calls("x = if close > 0\n    ta.sma(close, 3)[1]\nelse\n    na"),
        ["ta.sma"]
    );
    assert_eq!(
        hoisted_calls("x = switch\n    close > 0 => ta.sma(close, 3)[1]\n    => na"),
        ["ta.sma"]
    );
    assert_eq!(
        hoisted_calls(
            "x = switch\n    close > 0 => 1.0\n    ta.sma(close, 3)[1] > 0 => 2.0\n    => na"
        ),
        ["ta.sma"]
    );
}

#[test]
fn two_histories_make_two_variables() {
    let program = hoisted(
        "//@version=6\nindicator(\"T\")\nx = close > 0 ? ta.sma(close, 3)[1] + ta.sma(close, 3)[2] : na\n",
        &NoLibraries,
    );
    let moved = synthetic(&program.statements);
    assert_eq!(moved.len(), 2);
    assert_ne!(moved[0].0, moved[1].0);
    assert_ne!(
        moved[0].2, moved[1].2,
        "each keeps the state of its own call site"
    );
}

#[test]
fn a_call_inside_the_moved_calls_arguments_that_is_lazy_moves_first() {
    assert_eq!(
        hoisted_calls("x = close > 0 ? ta.sma(close > 1 ? ta.ema(close, 2)[1] : close, 3)[1] : na"),
        ["ta.ema", "ta.sma"]
    );
}

#[test]
fn a_call_that_could_fail_or_change_something_stays_where_it_is() {
    for body in [
        "a = array.new_float(1)\nx = array.size(a) > 0 ? array.get(a, 0)[1] : na",
        "a = array.new_float(1)\nx = array.size(a) > 0 and array.get(a, 0)[1] > 0",
        "x = close > 0 ? str.length(\"ab\")[1] : na",
        "x = close > 0 ? ta.pivot_point_levels(\"Traditional\")[1] : na",
        "x = close > 0 ? math.random(0, 1)[1] : na",
        "x = close > 0 ? request.security(syminfo.tickerid, \"D\", close)[1] : na",
        "x = close > 0 ? request.security(syminfo.tickerid, \"D\", close > 0 ? ta.sma(close, 3)[1] : na) : na",
        "x = close > 0 ? ta.sma(array.get(array.new_float(1), 0), 3)[1] : na",
        "type P\n    float v\np = P.new(1.0)\nx = close > 0 ? p.v[1] : na",
        "x = close > 0 ? nz(plot(close))[1] : na",
        // An offset only the guard keeps non-negative: `close[lag]` raises for a negative `lag`.
        "lag = bar_index - 1\nx = lag >= 0 ? math.abs(close[lag])[1] : na",
        "lag = bar_index - 1\nx = lag >= 0 ? ta.sma(close[lag], 3)[1] : na",
        "lag = bar_index - 1\nx = lag >= 0 ? ta.sma(close[-1], 3)[1] : na",
        "f(l) =>\n    math.abs(close[l])\nlag = bar_index - 1\nx = lag >= 0 ? f(lag)[1] : na",
        // Builtins that can raise, in families whose other members cannot.
        "x = close > 0 ? runtime.error(\"x\")[1] : na",
        "x = close > 0 ? time(\"nonsense\")[1] : na",
        "x = close > 0 ? time_close(\"nonsense\")[1] : na",
    ] {
        assert!(hoisted_calls(body).is_empty(), "{body}");
    }
}

#[test]
fn a_user_function_moves_only_when_it_only_computes() {
    let pure = "f() =>\n    ta.sma(close, 3) + math.abs(close)\n\
                x = close > 0 ? f()[1] : na";
    assert_eq!(hoisted_calls(pure), ["f"]);
    let chain = "g() =>\n    ta.sma(close, 3)\nf() =>\n    g() * 2\n\
                 x = close > 0 ? f()[1] : na";
    assert_eq!(hoisted_calls(chain), ["f"]);
    // A history at a literal offset cannot fail, in the call's argument or in a function's body.
    assert_eq!(
        hoisted_calls("x = close > 0 ? ta.sma(close[1], 3)[1] : na"),
        ["ta.sma"]
    );
    assert_eq!(
        hoisted_calls("f() =>\n    close[2]\nx = close > 0 ? f()[1] : na"),
        ["f"]
    );
    for impure in [
        "var a = array.new_float()\nf() =>\n    array.push(a, close)\n    1.0\n",
        "f() =>\n    plot(close)\n    1.0\n",
        "f() =>\n    array.get(array.new_float(0), 0)\n",
        "f() =>\n    request.security(syminfo.tickerid, \"D\", close)\n",
        "f() =>\n    na(close) ? 1.0 : label.new(bar_index, close).get_x()\n",
        "g() =>\n    plot(close)\n    1.0\nf() =>\n    g()\n",
        "f() =>\n    i = 0\n    while i < 3\n        i += 1\n    i\n",
        "f() =>\n    runtime.error(\"x\")\n    1.0\n",
        "f() =>\n    time(\"nonsense\")\n",
        // A loop is total only when its trip count is known: a zero step raises, and a bound the
        // data sets can run past the per-bar instruction budget.
        "f() =>\n    s = 0.0\n    for i = 0 to 3 by 0\n        s += 1.0\n    s\n",
        "f() =>\n    s = 0.0\n    for i = 0 to 3 by bar_index\n        s += 1.0\n    s\n",
        "f() =>\n    s = 0.0\n    for i = 0 to bar_index\n        s += 1.0\n    s\n",
        "f() =>\n    s = 0.0\n    for i = bar_index to 3\n        s += 1.0\n    s\n",
        // Known, but more than a bar may be asked to spend ahead of its guard (PINE8004 is at 20
        // million instructions): a long loop, loops that multiply, a function that calls one twice.
        "f() =>\n    s = 0.0\n    for i = 0 to 30000000\n        s += 1.0\n    s\n",
        "f() =>\n    s = 0.0\n    for i = 0 to 1 by 0.00000001\n        s += 1.0\n    s\n",
        "f() =>\n    s = 0.0\n    for i = 0 to 1000\n        for j = 0 to 1000\n            s += 1.0\n    s\n",
        "g() =>\n    s = 0.0\n    for i = 0 to 20000\n        s += 1.0\n    s\nf() =>\n    g() + g()\n",
        // A counter that cannot move: past 2^53, or by less than the gap between floats there.
        "f() =>\n    s = 0.0\n    for i = 1e20 to 1e20\n        s += 1.0\n    s\n",
        "f() =>\n    s = 0.0\n    for i = 9007199254740992 to 9007199254740994\n        s += 1.0\n    s\n",
        "f() =>\n    s = 0.0\n    for i = 1e15 to 1e15 by 0.01\n        s += 1.0\n    s\n",
        // Half a gap rounds back to the same float on every other value (ties to even).
        "f() =>\n    s = 0.0\n    for i = 4503599627370496 to 4503599627370497 by 0.5\n        s += 1.0\n    s\n",
        // What a call leaves out is evaluated for it: a default is part of the function.
        "f(v = math.random(0, 1)) =>\n    v\n",
        "f(v = time(\"nonsense\")) =>\n    v\n",
        "f(v = array.get(array.new_float(0), 0)) =>\n    v\n",
        "g() =>\n    plot(close)\n    1.0\nf(v = g()) =>\n    v\n",
    ] {
        let source = format!("{impure}x = close > 0 ? f()[1] : na");
        assert!(hoisted_calls(&source).is_empty(), "{source}");
    }
}

#[test]
fn a_call_in_a_loop_moves_only_while_the_loop_leaves_room_for_its_cost() {
    // The moved call runs once per pass of the loop around it, so its cost is multiplied by them.
    let helper = "g() =>\n    s = 0.0\n    for i = 0 to 20000\n        s += 1.0\n    s\n";
    assert_eq!(
        hoisted_calls(&format!(
            "{helper}for i = 0 to 0\n    r = close > 0 ? g()[1] : na"
        )),
        ["g"]
    );
    for loops in [
        "for i = 0 to 999\n    r = close > 0 ? g()[1] : na",
        "for i = 0 to bar_index\n    r = close > 0 ? g()[1] : na",
        "while bar_index < 3\n    r = close > 0 ? g()[1] : na\n    break",
        "for i = 0 to 9\n    for j = 0 to 9\n        r = close > 0 ? g()[1] : na",
    ] {
        let source = format!("{helper}{loops}");
        assert!(hoisted_calls(&source).is_empty(), "{source}");
    }
    // A cheap call has room for a few passes and none for a loop nobody can count.
    assert_eq!(
        hoisted_calls("for i = 0 to 9\n    r = close > 0 ? ta.sma(close, 3)[1] : na"),
        ["ta.sma"]
    );
    assert!(
        hoisted_calls("for i = 0 to bar_index\n    r = close > 0 ? ta.sma(close, 3)[1] : na")
            .is_empty()
    );
}

#[test]
fn a_function_with_a_total_default_or_a_loop_within_the_budget_is_run_ahead() {
    for helper in [
        "f(v = 2.0) =>\n    v\n",
        "f(v = ta.sma(close, 3)) =>\n    v\n",
        "g() =>\n    s = 0.0\n    for i = 0 to 20000\n        s += 1.0\n    s\nf() =>\n    g()\n",
    ] {
        let source = format!("{helper}x = close > 0 ? f()[1] : na");
        assert_eq!(hoisted_calls(&source), ["f"], "{source}");
    }
}

#[test]
fn a_loop_with_a_known_trip_count_is_total() {
    for body in [
        "for i = 0 to 3\n        s += 1.0",
        "for i = 0 to 3 by 1\n        s += 1.0",
        "for i = 3 to 0 by -1\n        s += 1.0",
        "for i = 0 to 10 by 2.5\n        s += 1.0",
    ] {
        let source = format!("f() =>\n    s = 0.0\n    {body}\n    s\nx = close > 0 ? f()[1] : na");
        assert_eq!(hoisted_calls(&source), ["f"], "{source}");
    }
}

// MARK: Which builtins may run ahead of their guard

#[test]
fn the_builtins_that_can_raise_or_draw_on_a_generator_are_not_total() {
    for name in [
        "runtime.error",
        "ta.pivot_point_levels",
        "time",
        "time_close",
        "math.random",
        "array.get",
        "str.tostring",
        "request.security",
        "plot",
        "no.such.function",
    ] {
        assert!(!is_total(name), "{name}");
    }
    for name in [
        "ta.sma", "ta.ema", "ta.atr", "math.abs", "math.max", "nz", "na", "fixnan",
    ] {
        assert!(is_total(name), "{name}");
    }
}

#[test]
fn every_name_that_is_total_is_a_builtin_the_compiler_knows() {
    for name in total::NAMES.iter().chain(total::REFUSED) {
        assert!(route_of(name).is_some(), "{name} has no route");
    }
}

#[test]
fn a_builtin_has_to_be_decided_before_it_may_run_ahead() {
    // Every name the families that can raise answer is listed as total or as refused: a builtin
    // added to one of them is neither until someone reads its evaluator, and until then it stays.
    let answered = INLINE
        .iter()
        .chain(&extra::FUNCTIONS)
        .chain(&SCALAR_NAMES)
        .chain(&MORE_SCALAR_NAMES)
        .chain(&crate::pine::time::PART_NAMES);
    for name in answered {
        let listed = total::NAMES.contains(name) as usize + total::REFUSED.contains(name) as usize;
        assert_eq!(
            listed, 1,
            "{name} must be in exactly one of NAMES and REFUSED"
        );
        assert_eq!(is_total(name), total::NAMES.contains(name), "{name}");
    }
}

#[test]
fn a_call_reading_what_its_own_statement_declares_stays_in_the_branch() {
    // `inner` exists only inside the arm: the call cannot run ahead of it.
    assert!(
        hoisted_calls(
            "x = if close > 0\n    inner = close * 2\n    ta.sma(inner, 3)[1]\nelse\n    na"
        )
        .is_empty()
    );
    // A call that reads nothing the arm makes can.
    assert_eq!(
        hoisted_calls(
            "x = if close > 0\n    inner = close * 2\n    ta.sma(close, 3)[1] + inner\nelse\n    na"
        ),
        ["ta.sma"]
    );
}

#[test]
fn a_move_stays_inside_the_block_or_function_it_was_written_in() {
    let program = hoisted(
        "//@version=6\nindicator(\"T\")\n\
         f(c) =>\n    c ? ta.sma(close, 3)[1] : na\n\
         total = 0.0\n\
         for i = 0 to 2\n    total += close > 0 ? ta.ema(close, 2)[1] : 0.0\n\
         if close > 0\n    total += close > 0 ? ta.rsi(close, 2)[1] : 0.0\n\
         plot(f(true) + total)\n",
        &NoLibraries,
    );
    // None at the top level: each sits before its statement in its own function, loop body or branch.
    assert!(
        program
            .statements
            .iter()
            .all(|statement| !statement.is_synthetic())
    );
    let in_function = nested_blocks(
        program
            .statements
            .iter()
            .find(|s| matches!(s, Statement::Function { .. }))
            .unwrap(),
    )[0];
    assert!(in_function[0].is_synthetic() && !in_function[1].is_synthetic());
    let names: Vec<String> = synthetic(&program.statements)
        .into_iter()
        .map(|(_, callee, _, _)| callee)
        .collect();
    assert_eq!(names, ["ta.sma", "ta.ema", "ta.rsi"]);
}

#[test]
fn a_library_function_is_hoisted_in_its_own_program() {
    struct Lib;
    impl LibraryResolver for Lib {
        fn source_for_library(&self, path: &str) -> Option<String> {
            (path == "user/Lazy/1").then(|| {
                "//@version=6\nlibrary(\"Lazy\")\nexport ahead(bool on) =>\n    on ? ta.sma(close, 3)[1] : na\n"
                    .to_owned()
            })
        }
    }
    let source =
        "//@version=6\nindicator(\"T\")\nimport user/Lazy/1 as lazy\nplot(lazy.ahead(close > 0))\n";
    let program = hoisted(source, &Lib);
    assert!(synthetic(&program.statements).is_empty());
    let library = program.imports[0]
        .library
        .as_deref()
        .expect("the library compiled");
    assert_eq!(synthetic(&library.statements).len(), 1);
    assert_eq!(outcome(source, &Lib, false), outcome(source, &Lib, true));
}

#[test]
fn a_library_function_that_calls_its_own_overload_is_still_hoisted() {
    // A library's calls are not resolved, so a call reaches every overload of its name, the
    // function it is in among them: that is not a recursion, and costs nothing.
    struct Lib;
    impl LibraryResolver for Lib {
        fn source_for_library(&self, path: &str) -> Option<String> {
            (path == "user/Lazy/1").then(|| {
                "//@version=6\nlibrary(\"Lazy\")\nf(float x) =>\n    x * 2\nf(float x, float y) =>\n    f(x) + y\n\
                 export ahead(bool on) =>\n    on ? f(close, 1.0)[1] : na\n"
                    .to_owned()
            })
        }
    }
    let source =
        "//@version=6\nindicator(\"T\")\nimport user/Lazy/1 as lazy\nplot(lazy.ahead(close > 0))\n";
    let program = hoisted(source, &Lib);
    let library = program.imports[0]
        .library
        .as_deref()
        .expect("the library compiled");
    assert_eq!(synthetic(&library.statements).len(), 1);
    assert_eq!(outcome(source, &Lib, false), outcome(source, &Lib, true));
}

#[test]
fn a_cost_found_while_another_function_is_being_counted_is_not_kept() {
    // A library's calls are resolved by name only, so counting `a` reaches `p(float, float)` which
    // reaches `a` again (a fallback, not a recursion). What `p(float, float)` costs there lacks `a`;
    // kept, it would hide that a call of it costs 90 thousand and is made a thousand times.
    struct Lib;
    impl LibraryResolver for Lib {
        fn source_for_library(&self, path: &str) -> Option<String> {
            (path == "user/Lazy/1").then(|| {
                "//@version=6\nlibrary(\"Lazy\")\np(float x) =>\n    x\n\
                 a(float x) =>\n    s = 0.0\n    for i = 0 to 29999\n        s += 1.0\n    s + p(x)\n\
                 p(float x, float y) =>\n    a(x) + y\n\
                 export ahead(bool on) =>\n    float r = na\n    for i = 0 to 999\n        r := on ? p(close, 1.0)[1] : na\n    r\n"
                    .to_owned()
            })
        }
    }
    let source =
        "//@version=6\nindicator(\"T\")\nimport user/Lazy/1 as lazy\nplot(lazy.ahead(close > 0))\n";
    let program = hoisted(source, &Lib);
    let library = program.imports[0]
        .library
        .as_deref()
        .expect("the library compiled");
    assert!(synthetic(&library.statements).is_empty());
}

#[test]
fn hoisted_scripts_run_the_same_both_ways() {
    for source in [
        "x = bar_index % 2 == 0 ? ta.sma(close, 3)[1] : na\nplot(x)",
        "f() =>\n    ta.ema(close, 3)\nx = bar_index % 3 == 0 ? f()[2] : na\nplot(x)",
        "hit = bar_index % 2 == 0 and ta.rsi(close, 3)[1] > 50\nplot(hit ? 1 : 0)",
        "x = switch\n    bar_index % 2 == 0 => ta.wma(close, 3)[1]\n    => 0.0\nplot(x)",
    ] {
        same_both_ways(&format!("//@version=6\nindicator(\"T\")\n{source}\n"));
    }
}

// MARK: The builtins that are total survive hostile arguments

/// Arguments that are likeliest to break a builtin that trusts its inputs: a missing value, zero and
/// negative lengths, huge ones, a series, a condition, an empty string and one that is no timeframe
/// or format.
const HOSTILE: [&str; 9] = [
    "na",
    "0",
    "-1",
    "1000000",
    "1e300",
    "close",
    "close > open",
    "\"\"",
    "\"x\"",
];

/// Every argument list of up to three `HOSTILE` values (the first of a list of three is a series or
/// `na`, which is what a builtin of three arguments reads first).
fn hostile_argument_lists() -> Vec<String> {
    let mut lists = vec![String::new()];
    lists.extend(HOSTILE.iter().map(|value| (*value).to_owned()));
    for first in HOSTILE {
        for second in HOSTILE {
            lists.push(format!("{first}, {second}"));
        }
    }
    for first in ["close", "na"] {
        for second in HOSTILE {
            for third in HOSTILE {
                lists.push(format!("{first}, {second}, {third}"));
            }
        }
    }
    lists
}

/// Calls `name` with each of `hostile_argument_lists`, as an assignment (or a tuple, for what returns
/// one), over a few bars. Returns how many calls compiled and the first run-time failure, if any.
fn hostile_calls(name: &str) -> (usize, Option<String>) {
    let data = bars(12, MINUTE);
    let mut compiled = 0;
    for list in hostile_argument_lists() {
        for target in ["x", "[a, b]", "[a, b, c]"] {
            let source = format!("//@version=6\nindicator(\"T\")\n{target} = {name}({list})\n");
            let program = compiler::compile(&source, limits(), &NoLibraries);
            if !program.is_valid() {
                continue;
            }
            compiled += 1;
            let mut session = Session::new(
                program,
                HashMap::new(),
                limits(),
                Some(0.01),
                symbol(),
                None,
                None,
            );
            if let Err(RunError::Script(diagnostic)) = session.evaluate(&data) {
                return (compiled, Some(format!("{source}=> {diagnostic:?}")));
            }
            break;
        }
    }
    (compiled, None)
}

#[test]
fn the_search_for_a_failing_builtin_finds_one() {
    // Without this the test below could pass because it looks at nothing.
    for name in [
        "runtime.error",
        "time",
        "time_close",
        "ta.pivot_point_levels",
    ] {
        assert!(hostile_calls(name).1.is_some(), "{name}");
    }
    assert!(hostile_calls("ta.sma").1.is_none());
}

#[test]
fn every_builtin_that_is_total_survives_hostile_arguments() {
    let names: Vec<&str> = crate::pine::compiler::registry::all_functions()
        .iter()
        .map(String::as_str)
        .filter(|name| is_total(name))
        .collect();
    assert!(
        names.len() > 30,
        "the registry lists the builtins that are total: {}",
        names.len()
    );
    let workers = std::thread::available_parallelism().map_or(2, usize::from);
    let results: Vec<(&str, usize, Option<String>)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|worker| {
                let names = &names;
                scope.spawn(move || {
                    names
                        .iter()
                        .skip(worker)
                        .step_by(workers)
                        .map(|name| {
                            let (compiled, failure) = hostile_calls(name);
                            (*name, compiled, failure)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().expect("a worker finished"))
            .collect()
    });
    let failures: Vec<&str> = results
        .iter()
        .filter_map(|(_, _, failure)| failure.as_deref())
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
    let untested: Vec<&str> = results
        .iter()
        .filter(|(_, compiled, _)| *compiled == 0)
        .map(|(name, ..)| *name)
        .collect();
    assert!(
        untested.is_empty(),
        "no call of these compiled, so nothing was checked: {untested:?}"
    );
}
