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
         e = \"x\".getType()",
        1,
    );
    assert_eq!(text(&session, "a"), "int");
    assert_eq!(text(&session, "b"), "float");
    assert_eq!(text(&session, "c"), "bool");
    assert_eq!(text(&session, "d"), "color");
    assert_eq!(text(&session, "e"), "string");
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
