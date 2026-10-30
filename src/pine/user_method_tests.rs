//! User-defined methods end to end: the examples on TradingView's Methods, User-defined functions
//! and Objects pages (adapted to a few bars), declaration and call diagnostics, and the history a
//! method keeps per call site. Each test compiles a small original script and reads the variables
//! the last bar left behind.

use std::collections::HashMap;

use crate::model::Kline;
use crate::pine::compiler::{self, NoLibraries};
use crate::pine::program::SymbolInfo;
use crate::pine::runtime_value::RuntimeValue;
use crate::pine::session::Session;
use crate::pine::token::{Diagnostic, DiagnosticSeverity, Limits};

fn bars(count: usize) -> Vec<Kline> {
    (0..count)
        .map(|index| {
            let price = 100.0 + index as f64;
            Kline {
                open_time: index as i64 * 60_000,
                close_time: (index as i64 + 1) * 60_000,
                open: price,
                high: price + 2.0,
                low: price - 2.0,
                close: price + 1.0,
                volume: 10.0,
            }
        })
        .collect()
}

fn source(body: &str) -> String {
    format!("//@version=6\nindicator(\"T\")\n{body}")
}

fn run(body: &str, count: usize) -> Session {
    let program = compiler::compile(&source(body), Limits::default(), &NoLibraries);
    assert!(program.is_valid(), "{:?}", program.diagnostics);
    let mut session = Session::new(
        program,
        HashMap::new(),
        Limits::default(),
        None,
        SymbolInfo::default(),
        None,
        None,
    );
    if let Err(error) = session.evaluate(&bars(count)) {
        panic!("{error:?}");
    }
    session
}

fn errors(body: &str) -> Vec<Diagnostic> {
    compiler::compile(&source(body), Limits::default(), &NoLibraries)
        .diagnostics
        .into_iter()
        .filter(|d| d.severity == DiagnosticSeverity::Error)
        .collect()
}

fn value(session: &Session, name: &str) -> RuntimeValue {
    session
        .working
        .variables
        .get(name)
        .cloned()
        .unwrap_or_else(|| panic!("no variable {name}"))
}

fn float(session: &Session, name: &str) -> f64 {
    match value(session, name) {
        RuntimeValue::Float(number) => number,
        RuntimeValue::Int(number) => number as f64,
        other => panic!("{name} is {other:?}"),
    }
}

fn text(session: &Session, name: &str) -> String {
    match value(session, name) {
        RuntimeValue::String(text) => text,
        other => panic!("{name} is {other:?}"),
    }
}

// MARK: Methods page

#[test]
fn a_method_returning_its_array_chains() {
    let session = run(
        "method maintainQueue(array<float> srcArray, float value, bool takeSample = true) =>\n\
         \x20   if takeSample\n\
         \x20       srcArray.push(value)\n\
         \x20       srcArray.shift()\n\
         \x20   srcArray\n\
         method calcBB(array<float> srcArray, float mult, bool calculate = true) =>\n\
         \x20   var float mean = na\n\
         \x20   var float dev = na\n\
         \x20   if calculate\n\
         \x20       mean := srcArray.avg()\n\
         \x20       dev := mult * srcArray.stdev()\n\
         \x20   [mean, mean + dev, mean - dev]\n\
         var array<float> queue = array.new<float>(3, 0.0)\n\
         [basis, upper, lower] = queue.maintainQueue(close, true).calcBB(2.0, true)\n\
         size = queue.size()",
        4,
    );
    // The queue holds the last three closes: 102, 103, 104.
    assert_eq!(value(&session, "size"), RuntimeValue::Int(3));
    assert!((float(&session, "basis") - 103.0).abs() < 1e-9);
    let dev = (float(&session, "upper") - 103.0).abs();
    assert!((dev - (float(&session, "basis") - float(&session, "lower"))).abs() < 1e-9);
    assert!(dev > 0.0);
}

#[test]
fn overloads_by_primitive_receiver_type_report_na() {
    let session = run(
        "method getType(int this) =>\n    na(this) ? \"int (na)\" : \"int\"\n\
         method getType(float this) =>\n    na(this) ? \"float (na)\" : \"float\"\n\
         method getType(bool this) =>\n    \"bool\"\n\
         method getType(color this) =>\n    na(this) ? \"color (na)\" : \"color\"\n\
         method getType(string this) =>\n    na(this) ? \"string (na)\" : \"string\"\n\
         a = (1).getType()\n\
         b = (1.5).getType()\n\
         c = true.getType()\n\
         red = color.red\n\
         d = red.getType()\n\
         e = \"x\".getType()\n\
         f = float(na).getType()\n\
         g = int(na).getType()",
        1,
    );
    assert_eq!(text(&session, "a"), "int");
    assert_eq!(text(&session, "b"), "float");
    assert_eq!(text(&session, "c"), "bool");
    assert_eq!(text(&session, "d"), "color");
    assert_eq!(text(&session, "e"), "string");
    assert_eq!(text(&session, "f"), "float (na)");
    assert_eq!(text(&session, "g"), "int (na)");
}

#[test]
fn a_five_argument_fill_overload_wins_only_when_all_arguments_are_given() {
    let session = run(
        "method fill(array<float> srcArray, float innerValue, float outerValue, float lowerBound, float upperBound) =>\n\
         \x20   for i = 0 to srcArray.size() - 1\n\
         \x20       float element = srcArray.get(i)\n\
         \x20       bool inside = (na(lowerBound) or element >= lowerBound) and (na(upperBound) or element <= upperBound)\n\
         \x20       srcArray.set(i, inside ? innerValue : outerValue)\n\
         \x20   srcArray\n\
         a = array.from(1.0, 2.0, 3.0, 4.0)\n\
         b = array.from(1.0, 2.0, 3.0, 4.0)\n\
         a.fill(9.0)\n\
         b.fill(1.0, 0.0, 2.0, 3.0)\n\
         builtin = a.sum()\n\
         user = b.sum()",
        1,
    );
    // The builtin one-argument `fill` still runs for `a`; the user overload runs for `b`.
    assert_eq!(float(&session, "builtin"), 36.0);
    assert_eq!(float(&session, "user"), 2.0);
}

#[test]
fn copy_fill_avg_chain_estimates_a_distribution() {
    let session = run(
        "method fill(array<float> srcArray, float innerValue, float outerValue, float lowerBound, float upperBound) =>\n\
         \x20   for i = 0 to srcArray.size() - 1\n\
         \x20       float element = srcArray.get(i)\n\
         \x20       srcArray.set(i, element >= lowerBound and element <= upperBound ? innerValue : outerValue)\n\
         \x20   srcArray\n\
         method featureScale(array<float> srcArray) =>\n\
         \x20   float low = srcArray.min()\n\
         \x20   float span = srcArray.range()\n\
         \x20   array<float> scaled = array.new<float>()\n\
         \x20   for element in srcArray\n\
         \x20       scaled.push((element - low) / span)\n\
         \x20   scaled\n\
         values = array.from(2.0, 4.0, 6.0, 8.0)\n\
         scaled = values.featureScale()\n\
         cdf = values.copy().fill(1.0, 0.0, 0.0, 5.0).avg()\n\
         last = scaled.last()",
        1,
    );
    assert_eq!(float(&session, "last"), 1.0);
    // 2 and 4 are at or below 5.
    assert_eq!(float(&session, "cdf"), 0.5);
}

// MARK: Objects page

#[test]
fn a_method_mutates_the_object_it_is_called_on() {
    let session = run(
        "type Counter\n    int value = 0\n\
         method increment(Counter this) =>\n    this.value += 1\n    this\n\
         var Counter counter = Counter.new()\n\
         counter.increment().increment()\n\
         total = counter.value",
        3,
    );
    // Two increments a bar, three bars.
    assert_eq!(value(&session, "total"), RuntimeValue::Int(6));
}

#[test]
fn a_copy_is_shallow_and_a_deep_copy_method_copies_the_nested_object() {
    let session = run(
        "type Inner\n    int n\n\
         type Outer\n    string info\n    Inner inner\n\
         method deepCopy(Outer this) =>\n    Outer.new(this.info, this.inner.copy())\n\
         parent = Outer.new(\"p\", Inner.new(1))\n\
         shallow = parent.copy()\n\
         deep = parent.deepCopy()\n\
         parent.inner.n := 5\n\
         shallow_n = shallow.inner.n\n\
         deep_n = deep.inner.n",
        1,
    );
    assert_eq!(value(&session, "shallow_n"), RuntimeValue::Int(5));
    assert_eq!(value(&session, "deep_n"), RuntimeValue::Int(1));
}

// MARK: Per call-site history

#[test]
fn two_call_sites_of_one_method_keep_separate_history() {
    let session = run(
        "method delta(float value) =>\n    value - value[1]\n\
         a = close.delta()\n\
         b = (close * 10.0).delta()",
        5,
    );
    assert_eq!(float(&session, "a"), 1.0);
    assert_eq!(float(&session, "b"), 10.0);
}

#[test]
fn a_method_called_on_some_bars_sees_its_previous_call() {
    let session = run(
        "method delta(float value) =>\n    value - value[1]\n\
         r = bar_index % 2 == 0 ? close.delta() : na",
        5,
    );
    // Bars 0, 2, 4 run it: the previous call was two bars back.
    assert_eq!(float(&session, "r"), 2.0);
}

#[test]
fn a_call_in_a_loop_is_one_scope() {
    let session = run(
        "method bump(float value) =>\n    var float total = 0.0\n    total += value\n    total\n\
         float last = 0.0\n\
         for i = 1 to 3\n    last := 1.0.bump()\n",
        2,
    );
    // One scope for the written call: 3 calls a bar over 2 bars.
    assert_eq!(float(&session, "last"), 6.0);
}

#[test]
fn a_method_with_a_stateful_builtin_keeps_state_per_call_site() {
    let session = run(
        "method smooth(float source, int length) =>\n    ta.sma(source, length)\n\
         a = close.smooth(2)\n\
         b = open.smooth(3)",
        5,
    );
    assert_eq!(float(&session, "a"), 104.5);
    assert_eq!(float(&session, "b"), 103.0);
}

#[test]
fn a_method_calls_other_methods_and_functions() {
    let session = run(
        "double(float x) => x * 2.0\n\
         method add(float this, float amount) => this + amount\n\
         method twiceThenAdd(float this, float amount) => double(this).add(amount)\n\
         r = close.twiceThenAdd(1.0)",
        2,
    );
    assert_eq!(float(&session, "r"), 2.0 * 102.0 + 1.0);
}

// MARK: Declaration diagnostics

fn has_error(body: &str, needle: &str) -> bool {
    errors(body)
        .iter()
        .any(|d| d.message.to_lowercase().contains(needle))
}

#[test]
fn a_receiver_without_a_type_is_an_error() {
    assert!(has_error(
        "method calculate(value) =>\n    value * 2\n",
        "receiver"
    ));
}

#[test]
fn a_method_without_parameters_is_an_error() {
    assert!(has_error("method nothing() =>\n    1\n", "receiver"));
}

#[test]
fn a_method_declared_in_a_local_scope_is_an_error() {
    assert!(has_error(
        "if close > open\n    method inner(float x) =>\n        x\n",
        "global",
    ));
}

#[test]
fn the_same_signature_twice_is_an_error() {
    assert!(has_error(
        "method f(float x) =>\n    x\nmethod f(float y) =>\n    y * 2.0\n",
        "already declared",
    ));
}

#[test]
fn overloads_that_differ_only_in_optional_parameters_are_ambiguous() {
    assert!(has_error(
        "method f(float x, int n = 1) =>\n    x\nmethod f(float x, string s = \"a\") =>\n    x\n",
        "already declared",
    ));
}

#[test]
fn a_method_cannot_call_itself() {
    assert!(has_error(
        "method down(int n) =>\n    n <= 0 ? 0 : down(n - 1)\n",
        "recurs",
    ));
}

#[test]
fn a_mutual_recursion_is_an_error() {
    assert!(has_error(
        "even(int n) =>\n    n == 0 ? true : odd(n - 1)\nodd(int n) =>\n    n == 0 ? false : even(n - 1)\n",
        "cannot be recursive",
    ));
}

#[test]
fn overloads_with_different_qualifiers_or_counts_coexist() {
    let found = errors(
        "method f(float x) =>\n    x\n\
         method f(float x, float y) =>\n    x + y\n\
         method f(int x) =>\n    float(x)\n\
         pick(simple int length) =>\n    length\n\
         pick(series int length) =>\n    length + 1\n\
         method g(array<float> v) =>\n    1.0\n\
         method g(array<int> v) =>\n    2.0\n",
    );
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn an_overload_may_call_an_earlier_overload() {
    let found = errors("method f(float x) =>\n    x\nmethod f(int x) =>\n    f(float(x))\n");
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn a_method_named_like_a_builtin_may_call_the_builtin() {
    let found = errors(
        "type Stack\n    array<float> items\n\
         method size(Stack this) =>\n    this.items.size()\n\
         method push(Stack this, float x) =>\n    this.items.push(x)\n",
    );
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn an_exported_method_declares_every_parameter_type() {
    let program = compiler::compile(
        "//@version=6\nlibrary(\"L\")\nexport method scaled(float this, factor) =>\n    this * factor\n",
        Limits::default(),
        &NoLibraries,
    );
    assert!(
        program
            .diagnostics
            .iter()
            .any(|d| d.code == "PINE3062" && d.message.contains("factor")),
        "{:?}",
        program.diagnostics
    );
}

// MARK: Call diagnostics

#[test]
fn an_unknown_method_on_a_known_receiver_is_a_compile_error() {
    assert!(has_error(
        "values = array.from(1.0, 2.0)\nx = values.nothingLikeThis()\n",
        "nothinglikethis",
    ));
}

#[test]
fn a_receiver_of_the_wrong_type_is_a_compile_error() {
    assert!(has_error(
        "method double(array<float> values) =>\n    values.sum() * 2.0\nx = \"text\".double()\n",
        "receiver of type string",
    ));
}

#[test]
fn an_array_of_the_wrong_element_type_is_not_a_receiver() {
    assert!(has_error(
        "method double(array<float> values) =>\n    values.sum() * 2.0\nx = array.from(\"a\", \"b\").double()\n",
        "receiver of type array<string>",
    ));
}

#[test]
fn a_wrong_argument_count_is_a_compile_error() {
    assert!(has_error(
        "method scale(float x, float k) =>\n    x * k\ny = close.scale(1.0, 2.0)\n",
        "wrong number of arguments",
    ));
}

#[test]
fn an_argument_of_the_wrong_type_is_a_compile_error() {
    assert!(has_error(
        "method scale(float x, float k) =>\n    x * k\ny = close.scale(\"two\")\n",
        "must be float, got string",
    ));
}

#[test]
fn a_named_argument_no_parameter_has_is_a_compile_error() {
    assert!(has_error(
        "method scale(float x, float k) =>\n    x * k\ny = close.scale(factor = 2.0)\n",
        "no parameter named 'factor'",
    ));
}

#[test]
fn a_series_argument_for_a_simple_parameter_is_a_compile_error() {
    assert!(has_error(
        "method pick(int x, simple int length) =>\n    length\ny = (1).pick(bar_index)\n",
        "simple",
    ));
}

#[test]
fn a_series_argument_selects_the_series_overload() {
    let program = compiler::compile(
        &source(
            "method pick(int x, simple int length) =>\n    1\n\
             method pick(int x, series int length) =>\n    2\n\
             a = (1).pick(5)\n\
             b = (1).pick(bar_index)\n",
        ),
        Limits::default(),
        &NoLibraries,
    );
    assert!(program.is_valid(), "{:?}", program.diagnostics);
    let mut chosen: Vec<(usize, usize)> = program.resolutions.into_iter().collect();
    chosen.sort_unstable();
    // The constant takes the simple overload, the bar index the series one.
    assert_eq!(chosen.iter().map(|(_, id)| *id).collect::<Vec<_>>(), [0, 1]);
}

#[test]
fn a_user_overload_of_a_builtin_name_applies_only_when_it_fits() {
    let found = errors(
        "method fill(array<float> a, float inner, float outer, float lo, float hi) =>\n    a\n\
         values = array.from(1.0, 2.0)\n\
         values.fill(0.0)\n\
         values.fill(1.0, 0.0, 0.0, 5.0)\n",
    );
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn a_method_calling_itself_through_method_syntax_is_recursion() {
    assert!(has_error(
        "method down(int this) =>\n    this <= 0 ? 0 : (this - 1).down()\n",
        "cannot be recursive",
    ));
}

#[test]
fn a_method_on_a_field_is_resolved_through_the_path() {
    let found = errors(
        "type Stack\n    array<float> items\n\
         method top(array<float> this) =>\n    this.last()\n\
         method peek(Stack this) =>\n    this.items.top()\n\
         s = Stack.new(array.from(1.0))\n\
         x = s.peek()\n",
    );
    assert!(found.is_empty(), "{found:?}");
    assert!(has_error(
        "type Stack\n    array<float> items\n\
         method peek(Stack this) =>\n    this.items.nothing()\n",
        "nothing",
    ));
}

#[test]
fn a_chain_resolves_each_call_from_the_type_before_it() {
    let found = errors(
        "method keep(array<float> a, float v) =>\n    a.push(v)\n    a\n\
         method total(array<float> a) =>\n    a.sum()\n\
         t = array.new_float().keep(1.0).keep(2.0).total()\n",
    );
    assert!(found.is_empty(), "{found:?}");
    assert!(has_error(
        "method keep(array<float> a, float v) =>\n    a.push(v)\n    a\n\
         t = array.new_float().keep(1.0).total()\n",
        "total",
    ));
}

#[test]
fn calls_the_checker_settles_are_recorded_by_site() {
    let program = compiler::compile(
        &source(
            "method twice(float this) =>\n    this * 2.0\n\
             a = close.twice()\n\
             b = twice(open)\n\
             c = unknown_receiver.twice()\n",
        ),
        Limits::default(),
        &NoLibraries,
    );
    // `unknown_receiver` is undeclared, so it is an error elsewhere; the first two still resolve.
    assert_eq!(program.resolutions.len(), 2, "{:?}", program.resolutions);
    assert!(program.resolutions.values().all(|&id| id == 0));
}

// MARK: Run-time selection

#[test]
fn the_overload_for_a_series_argument_runs_for_a_series_argument() {
    let session = run(
        "method pick(int x, simple int length) =>\n    1\n\
         method pick(int x, series int length) =>\n    2\n\
         a = (1).pick(5)\n\
         b = (1).pick(bar_index)",
        1,
    );
    assert_eq!(value(&session, "a"), RuntimeValue::Int(1));
    assert_eq!(value(&session, "b"), RuntimeValue::Int(2));
}

#[test]
fn a_method_on_a_namespaced_constant_runs() {
    let session = run(
        "method name(color this) =>\n    \"color\"\n\
         r = color.red.name()",
        1,
    );
    assert_eq!(text(&session, "r"), "color");
}

/// Runs `body` with every call bound to its target and with every call found by name.
fn both_ways(body: &str, count: usize, names: &[&str]) {
    let program = compiler::compile(&source(body), Limits::default(), &NoLibraries);
    assert!(program.is_valid(), "{:?}", program.diagnostics);
    let mut results = Vec::new();
    for ignore in [false, true] {
        let mut session = Session::new(
            program.clone(),
            HashMap::new(),
            Limits::default(),
            None,
            SymbolInfo::default(),
            None,
            None,
        );
        session.ignore_routes = ignore;
        if let Err(error) = session.evaluate(&bars(count)) {
            panic!("{error:?}");
        }
        results.push(
            names
                .iter()
                .map(|name| format!("{:?}", value(&session, name)))
                .collect::<Vec<_>>(),
        );
    }
    assert_eq!(results[0], results[1], "bound and by-name runs differ");
}

#[test]
fn bound_and_by_name_calls_agree() {
    both_ways(
        "type Counter\n    int value = 0\n\
         method bump(Counter this, int by = 1) =>\n    this.value += by\n    this\n\
         method delta(float value) =>\n    value - value[1]\n\
         method total(array<float> values) =>\n    values.sum()\n\
         double(float x) => x * 2.0\n\
         var Counter c = Counter.new()\n\
         c.bump().bump(by = 3)\n\
         d = close.delta()\n\
         e = (close * 2.0).delta()\n\
         t = array.from(1.0, 2.0, 3.0).total()\n\
         f = double(close)\n\
         v = c.value",
        6,
        &["d", "e", "t", "f", "v"],
    );
}

#[test]
fn a_receiver_the_checker_cannot_type_is_chosen_by_its_value() {
    let session = run(
        "type Dog\n    int legs\n\
         type Bird\n    int legs\n\
         method describe(Dog this) =>\n    \"dog\"\n\
         method describe(Bird this) =>\n    \"bird\"\n\
         tag(x) =>\n    x.describe()\n\
         a = tag(Dog.new(4))\n\
         b = tag(Bird.new(2))",
        1,
    );
    assert_eq!(text(&session, "a"), "dog");
    assert_eq!(text(&session, "b"), "bird");
}

#[test]
fn a_method_an_imported_library_exports_runs_on_a_receiver() {
    struct Library;
    impl compiler::LibraryResolver for Library {
        fn source_for_library(&self, path: &str) -> Option<String> {
            (path == "user/Mine/1").then(|| {
                "//@version=6\nlibrary(\"Mine\")\n\
                 export method scaled(float this, float k) =>\n    this * k\n"
                    .to_owned()
            })
        }
    }
    let program = compiler::compile(
        &source("import user/Mine/1 as mine\nr = close.scaled(2.0)\n"),
        Limits::default(),
        &Library,
    );
    assert!(program.is_valid(), "{:?}", program.diagnostics);
    let mut session = Session::new(
        program,
        HashMap::new(),
        Limits::default(),
        None,
        SymbolInfo::default(),
        None,
        None,
    );
    session.evaluate(&bars(3)).unwrap();
    assert_eq!(float(&session, "r"), 2.0 * 103.0);
}

// MARK: Realtime, strategies and alerts

/// What a controller produced after each update: every plot's values, the closed trades and the
/// alerts raised.
type Snapshot = (Vec<Vec<Option<f64>>>, usize, usize);

/// Feeds `source` three closed bars, then an open fourth that ticks three times, then the fifth bar
/// (which closes the fourth), and returns what each update produced.
fn ticked(source: &str) -> Vec<Snapshot> {
    use crate::pine::execution::ExecutionController;
    let mut controller = ExecutionController::new(
        source,
        HashMap::new(),
        SymbolInfo::default(),
        None,
        Limits::default(),
        &NoLibraries,
        None,
        None,
    )
    .expect("valid script");
    let mut candles = bars(3);
    let minute = 60_000;
    let mut out = Vec::new();
    let mut record = |update: crate::pine::execution::ExecutionUpdate| {
        assert!(update.error.is_none(), "{:?}", update.error);
        out.push((
            update
                .output
                .plots
                .iter()
                .map(|plot| plot.values.clone())
                .collect(),
            update
                .output
                .strategy
                .as_ref()
                .map_or(0, |report| report.trades.len()),
            update.alerts.len(),
        ));
    };
    record(controller.update(&candles, None, 3 * minute + 1));
    let mut live = bars(4).pop().expect("a fourth bar");
    live.close_time = 10 * minute;
    candles.push(live.clone());
    for (step, close) in [120.0, 80.0, 130.0].into_iter().enumerate() {
        live.close = close;
        *candles.last_mut().expect("live bar") = live.clone();
        record(controller.update(&candles, None, 4 * minute + 1_000 * (step as i64 + 1)));
    }
    // The fourth bar closes and the fifth opens.
    let mut closed = live.clone();
    closed.close_time = 4 * minute;
    *candles.last_mut().expect("live bar") = closed;
    let mut next = bars(5).pop().expect("a fifth bar");
    next.close_time = 11 * minute;
    candles.push(next);
    record(controller.update(&candles, None, 5 * minute + 1_000));
    out
}

fn script_for(body: &str) -> String {
    source(body)
}

#[test]
fn a_method_changes_state_across_ticks_as_its_function_twin_does() {
    let methods = "type Acc\n    float total = 0.0\n    int count = 0\n\
         method add(Acc this, float x) =>\n    this.total += x\n    this.count += 1\n    this\n\
         method keep(array<float> v, float x) =>\n    v.push(x)\n    v\n\
         var Acc acc = Acc.new()\n\
         var array<float> seen = array.new_float()\n\
         acc.add(close)\n\
         seen.keep(close)\n\
         plot(acc.count)\n\
         plot(seen.size())\n\
         plot(acc.total)\n";
    let functions = "type Acc\n    float total = 0.0\n    int count = 0\n\
         add(Acc this, float x) =>\n    this.total += x\n    this.count += 1\n    this\n\
         keep(array<float> v, float x) =>\n    v.push(x)\n    v\n\
         var Acc acc = Acc.new()\n\
         var array<float> seen = array.new_float()\n\
         add(acc, close)\n\
         keep(seen, close)\n\
         plot(acc.count)\n\
         plot(seen.size())\n\
         plot(acc.total)\n";
    let by_method = ticked(&script_for(methods));
    assert_eq!(by_method, ticked(&script_for(functions)));
    // The open bar counts once however often it ticks, and again only when it closes.
    let counts: Vec<f64> = by_method
        .iter()
        .map(|(plots, ..)| plots[0].last().copied().flatten().expect("a count"))
        .collect();
    assert_eq!(counts, [3.0, 4.0, 4.0, 4.0, 5.0]);
    let sizes: Vec<f64> = by_method
        .iter()
        .map(|(plots, ..)| plots[1].last().copied().flatten().expect("a size"))
        .collect();
    assert_eq!(sizes, [3.0, 4.0, 4.0, 4.0, 5.0]);
}

#[test]
fn a_method_in_a_strategy_condition_trades_as_its_function_twin_does() {
    let strategy = |methods: bool| {
        let (declare, test) = if methods {
            (
                "method crossed(float x, float y) => x > y\n",
                "close.crossed(open)",
            )
        } else {
            (
                "crossed(float x, float y) => x > y\n",
                "crossed(close, open)",
            )
        };
        format!(
            "//@version=6\nstrategy(\"S\", overlay = true)\n{declare}\
             if {test} and bar_index % 2 == 0\n    strategy.entry(\"L\", strategy.long)\n\
             if bar_index % 2 == 1\n    strategy.close(\"L\")\nplot(close)\n"
        )
    };
    let by_method = ticked(&strategy(true));
    assert_eq!(by_method, ticked(&strategy(false)));
    // Entered on bar 0, closed on bar 1: the strategy did trade.
    assert!(by_method.last().is_some_and(|(_, trades, _)| *trades > 0));
}

#[test]
fn a_method_in_an_alert_condition_alerts_as_its_function_twin_does() {
    let alerting = |methods: bool| {
        let (declare, test) = if methods {
            (
                "method rose(float x, float y) => x > y\n",
                "close.rose(open)",
            )
        } else {
            ("rose(float x, float y) => x > y\n", "rose(close, open)")
        };
        format!(
            "//@version=6\nindicator(\"A\")\n{declare}\
             if {test}\n    alert(\"up\", alert.freq_once_per_bar_close)\nplot(close)\n"
        )
    };
    let by_method = ticked(&alerting(true));
    assert_eq!(by_method, ticked(&alerting(false)));
    assert!(by_method.iter().any(|(.., alerts)| *alerts > 0));
}
