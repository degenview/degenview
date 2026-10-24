//! What a stopped script shows: variables by scope, the caller's view of shadowed names, containers,
//! series history, stale handles and watches, and that none of it changes the script.

use std::sync::{Arc, Mutex};

use crate::pine::bench_tests::{self, run_script};

use super::tests::{Probe, bars, runtime_with, with_header};
use super::*;

type Rows = Vec<(Group, String, String)>;
type Rows2 = (Vec<String>, Vec<String>);
type Marks = Vec<(String, bool)>;

/// What a stop looked like through the view, for every frame, innermost first.
#[derive(Clone, Debug, Default)]
struct Captured {
    frames: Vec<(String, Rows)>,
}

fn rows(groups: &[VariableGroup]) -> Rows {
    groups
        .iter()
        .flat_map(|group| {
            group
                .variables
                .iter()
                .map(|variable| (group.group, variable.name.clone(), variable.value.clone()))
        })
        .collect()
}

fn value_of(rows: &Rows, name: &str) -> Option<String> {
    rows.iter()
        .find(|(_, row, _)| row == name)
        .map(|(_, _, value)| value.clone())
}

fn group_of(rows: &Rows, name: &str) -> Option<Group> {
    rows.iter()
        .find(|(_, row, _)| row == name)
        .map(|(group, _, _)| *group)
}

fn capture_all() -> (Probe, Arc<Mutex<Vec<Captured>>>) {
    let captured: Arc<Mutex<Vec<Captured>>> = Arc::default();
    let sink = Arc::clone(&captured);
    let probe: Probe = Box::new(move |_, view| {
        let frames = view
            .stack()
            .into_iter()
            .map(|frame| (frame.label.clone(), rows(&view.variables(frame.index))))
            .collect();
        sink.lock().unwrap().push(Captured { frames });
    });
    (probe, captured)
}

fn at_lines(
    body: &str,
    count: usize,
    lines: &[usize],
    probe: Probe,
) -> crate::pine::session::Session {
    let source = with_header(body);
    let specs = lines
        .iter()
        .enumerate()
        .map(|(index, line)| BreakpointSpec::line(index as u64 + 1, *line))
        .collect();
    let (runtime, _) = runtime_with(
        &source,
        specs,
        false,
        &[],
        DebugControl::default(),
        Some(probe),
    );
    run_script(&source, &bars(count), false, |session| {
        session.attach_debugger(runtime);
    })
}

/// A function reads a global; line 3 is the first line of the body.
const GROUPS: &str = "length = 3
var total = 0.0
total += close
double(x) =>
    twice = x * 2
    twice + 1
result = double(close)
plot(result)
";

#[test]
fn variables_are_grouped_by_scope_with_their_current_values() {
    let (probe, captured) = capture_all();
    // Line 8 is `twice + 1`, inside the function, on the third bar.
    let breakpoint = vec![BreakpointSpec {
        hit: HitRule::Equals(3),
        ..BreakpointSpec::line(1, 8)
    }];
    let source = with_header(GROUPS);
    let (runtime, _) = runtime_with(
        &source,
        breakpoint,
        false,
        &[],
        DebugControl::default(),
        Some(probe),
    );
    run_script(&source, &bars(5), false, |session| {
        session.attach_debugger(runtime)
    });

    let captured = captured.lock().unwrap();
    assert_eq!(captured.len(), 1);
    let frames = &captured[0].frames;
    assert_eq!(
        frames
            .iter()
            .map(|(label, _)| label.as_str())
            .collect::<Vec<_>>(),
        ["double()", "Main Script"]
    );

    let inside = &frames[0].1;
    assert_eq!(group_of(inside, "x"), Some(Group::Local));
    assert_eq!(value_of(inside, "x").as_deref(), Some("103"));
    assert_eq!(group_of(inside, "twice"), Some(Group::Local));
    assert_eq!(value_of(inside, "twice").as_deref(), Some("206"));
    // A function sees the names declared before it, assigned on this bar.
    assert_eq!(group_of(inside, "length"), Some(Group::Global));
    assert_eq!(value_of(inside, "length").as_deref(), Some("3"));
    assert_eq!(group_of(inside, "total"), Some(Group::Persistent));
    assert_eq!(value_of(inside, "total").as_deref(), Some("306"));
    assert_eq!(group_of(inside, "close"), Some(Group::BuiltIn));
    assert_eq!(value_of(inside, "close").as_deref(), Some("103"));
    assert_eq!(value_of(inside, "bar_index").as_deref(), Some("2"));
    // `result` is declared after the function, and is not visible from inside it.
    assert_eq!(value_of(inside, "result"), None);
    assert!(
        !inside
            .iter()
            .any(|(_, _, value)| value.contains("not yet assigned")),
        "{inside:?}"
    );

    let caller = &frames[1].1;
    assert_eq!(
        group_of(caller, "x"),
        None,
        "a parameter is not the caller's"
    );
    assert_eq!(value_of(caller, "total").as_deref(), Some("306"));
    assert_eq!(value_of(caller, "twice"), None);
}

#[test]
fn a_caller_frame_shows_its_own_value_of_a_name_the_callee_shadows() {
    let body = "x = 5.0
f(x) =>
    x * 2
y = f(10.0)
plot(y)
";
    let (probe, captured) = capture_all();
    at_lines(body, 1, &[5], probe);
    let captured = captured.lock().unwrap();
    let frames = &captured[0].frames;
    assert_eq!(
        value_of(&frames[0].1, "x").as_deref(),
        Some("10"),
        "the callee's parameter"
    );
    assert_eq!(
        value_of(&frames[1].1, "x").as_deref(),
        Some("5"),
        "the caller's own x"
    );
}

#[test]
fn an_if_blocks_names_are_visible_inside_it_and_gone_after_it() {
    let body = "x = 0.0
if close > open
    inner = 1.0
    x := inner + 1
plot(x)
";
    let (probe, captured) = capture_all();
    at_lines(body, 1, &[6, 7], probe);
    let captured = captured.lock().unwrap();
    assert_eq!(captured.len(), 2);
    let inside = &captured[0].frames[0].1;
    assert_eq!(group_of(inside, "inner"), Some(Group::Local));
    assert_eq!(value_of(inside, "inner").as_deref(), Some("1"));
    assert_eq!(group_of(inside, "x"), Some(Group::Global));
    let after = &captured[1].frames[0].1;
    assert_eq!(
        value_of(after, "inner"),
        None,
        "the interpreter's table still holds it"
    );
    assert_eq!(value_of(after, "x").as_deref(), Some("2"));
}

#[test]
fn what_the_script_stores_in_containers_can_be_expanded_lazily() {
    let body = "var arr = array.new_float(0)
var counts = map.new<string, float>()
var grid = matrix.new<float>(2, 3, 0.0)
type Pos
    float entry
    bool active
pos = Pos.new(125.5, true)
array.push(arr, close)
array.push(arr, open)
map.put(counts, \"last\", close)
matrix.set(grid, 1, 2, close)
plot(array.size(arr))
";
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = Arc::clone(&seen);
    let probe: Probe = Box::new(move |_, view| {
        let frame = view.stack()[0].index;
        let groups = view.variables(frame);
        let find = |name: &str| {
            groups
                .iter()
                .flat_map(|group| &group.variables)
                .find(|variable| variable.name == name)
                .cloned()
                .unwrap_or_else(|| panic!("no {name}"))
        };
        let mut out = sink.lock().unwrap();

        let arr = find("arr");
        out.push(format!("arr: {} / {}", arr.type_name, arr.value));
        let page = view.expand(arr.expand.as_ref().unwrap(), 0, 10).unwrap();
        out.push(format!(
            "arr total {}: {:?}",
            page.total,
            page.items
                .iter()
                .map(|v| (v.name.clone(), v.value.clone()))
                .collect::<Vec<_>>()
        ));

        let counts = find("counts");
        out.push(format!("counts: {} / {}", counts.type_name, counts.value));
        let page = view.expand(counts.expand.as_ref().unwrap(), 0, 10).unwrap();
        out.push(format!(
            "counts entries: {:?}",
            page.items
                .iter()
                .map(|v| (v.name.clone(), v.value.clone()))
                .collect::<Vec<_>>()
        ));

        let grid = find("grid");
        out.push(format!("grid: {} / {}", grid.type_name, grid.value));
        let rows = view.expand(grid.expand.as_ref().unwrap(), 0, 10).unwrap();
        out.push(format!("grid rows: {}", rows.total));
        let row = view
            .expand(rows.items[1].expand.as_ref().unwrap(), 0, 10)
            .unwrap();
        out.push(format!(
            "grid row 1: {:?}",
            row.items
                .iter()
                .map(|v| v.value.clone())
                .collect::<Vec<_>>()
        ));

        let pos = find("pos");
        out.push(format!("pos: {} / {}", pos.type_name, pos.value));
        let page = view.expand(pos.expand.as_ref().unwrap(), 0, 10).unwrap();
        out.push(format!(
            "pos fields: {:?}",
            page.items
                .iter()
                .map(|v| (v.name.clone(), v.value.clone(), v.type_name.clone()))
                .collect::<Vec<_>>()
        ));
    });
    at_lines(body, 1, &[14], probe);
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0], "arr: array<float> / size 2");
    assert_eq!(
        seen[1],
        "arr total 2: [(\"[0]\", \"101\"), (\"[1]\", \"100\")]"
    );
    assert_eq!(seen[2], "counts: map<string, float> / size 1");
    assert_eq!(seen[3], "counts entries: [(\"\\\"last\\\"\", \"101\")]");
    assert_eq!(seen[4], "grid: matrix<float> / 2 × 3");
    assert_eq!(seen[5], "grid rows: 2");
    assert_eq!(seen[6], "grid row 1: [\"0\", \"0\", \"101\"]");
    assert!(
        seen[7].starts_with("pos: Pos / { active: true, entry: 125.5 }"),
        "{}",
        seen[7]
    );
    assert_eq!(
        seen[8],
        "pos fields: [(\"active\", \"true\", \"bool\"), (\"entry\", \"125.5\", \"float\")]"
    );
}

#[test]
fn a_large_collection_is_read_a_page_at_a_time() {
    let body = "var big = array.new_float(1000, 1.0)
plot(array.size(big))
";
    let seen: Arc<Mutex<Vec<(usize, usize, usize)>>> = Arc::default();
    let sink = Arc::clone(&seen);
    let probe: Probe = Box::new(move |_, view| {
        let frame = view.stack()[0].index;
        let big = view
            .variables(frame)
            .into_iter()
            .flat_map(|group| group.variables)
            .find(|variable| variable.name == "big")
            .unwrap();
        let reference = big.expand.unwrap();
        let first = view.expand(&reference, 0, 50).unwrap();
        let last = view.expand(&reference, 990, 50).unwrap();
        let greedy = view.expand(&reference, 0, 100_000).unwrap();
        sink.lock()
            .unwrap()
            .push((first.items.len(), last.items.len(), greedy.items.len()));
        assert_eq!(first.total, 1000);
        assert_eq!(last.items[0].name, "[990]");
    });
    at_lines(body, 1, &[4], probe);
    assert_eq!(*seen.lock().unwrap(), vec![(50, 10, PAGE_LIMIT)]);
}

#[test]
fn a_series_expands_to_the_bars_it_took_earlier_values_on() {
    let body = "gap = close + bar_index
plot(gap + close)
";
    let seen: Arc<Mutex<Vec<Rows2>>> = Arc::default();
    let sink = Arc::clone(&seen);
    let probe: Probe = Box::new(move |stop, view| {
        if stop.bar_index != 3 {
            return;
        }
        let frame = view.stack()[0].index;
        let groups = view.variables(frame);
        let find = |name: &str| {
            groups
                .iter()
                .flat_map(|group| &group.variables)
                .find(|variable| variable.name == name)
                .cloned()
                .unwrap()
        };
        let close = view
            .expand(find("close").expand.as_ref().unwrap(), 0, 4)
            .unwrap();
        let gap = view
            .expand(find("gap").expand.as_ref().unwrap(), 0, 3)
            .unwrap();
        let text = |page: &Page| {
            page.items
                .iter()
                .map(|item| format!("{}={}", item.name, item.value))
                .collect::<Vec<_>>()
        };
        sink.lock().unwrap().push((text(&close), text(&gap)));
        assert_eq!(close.total, 4, "the current value and three earlier bars");
    });
    at_lines(body, 5, &[4], probe);
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0].0, ["[0]=104", "[1]=103", "[2]=102", "[3]=101"]);
    assert_eq!(seen[0].1, ["[0]=107", "[1]=105", "[2]=103"]);
}

#[test]
fn an_input_has_no_history_and_a_constant_series_says_so_once() {
    let body = "length = input.int(3, \"Length\")
konst = 5.0
plot(length + konst + close)
";
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = Arc::clone(&seen);
    let probe: Probe = Box::new(move |stop, view| {
        if stop.bar_index != 3 {
            return;
        }
        let frame = view.stack()[0].index;
        let groups = view.variables(frame);
        let find = |name: &str| {
            groups
                .iter()
                .flat_map(|group| &group.variables)
                .find(|variable| variable.name == name)
                .cloned()
                .unwrap_or_else(|| panic!("no {name}"))
        };
        let mut out = sink.lock().unwrap();
        out.push(format!(
            "length expandable: {}",
            find("length").expand.is_some()
        ));
        let konst = view
            .expand(find("konst").expand.as_ref().unwrap(), 0, 10)
            .unwrap();
        out.push(format!(
            "konst: {} rows {:?}",
            konst.total,
            konst
                .items
                .iter()
                .map(|v| (
                    v.name.clone(),
                    v.value.clone(),
                    v.age.clone().unwrap_or_default()
                ))
                .collect::<Vec<_>>()
        ));
        let close = view
            .expand(find("close").expand.as_ref().unwrap(), 0, 10)
            .unwrap();
        out.push(format!(
            "close ages: {:?}",
            close
                .items
                .iter()
                .map(|v| v.age.clone().unwrap_or_default())
                .collect::<Vec<_>>()
        ));
        out.push(format!("close type kept: {}", close.items[1].type_name));
    });
    at_lines(body, 5, &[5], probe);
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0], "length expandable: false");
    assert_eq!(
        seen[1],
        "konst: 2 rows [(\"[0]\", \"5\", \"this bar\"), (\"[1…3]\", \"same value\", \"3 bars ago\")]"
    );
    assert_eq!(
        seen[2],
        "close ages: [\"this bar\", \"1 bar ago\", \"2 bars ago\", \"3 bars ago\"]"
    );
    assert_eq!(seen[3], "close type kept: float");
}

#[test]
fn a_reference_from_an_earlier_stop_is_stale_once_the_script_has_moved_on() {
    let body = "var arr = array.new_float(0)
array.push(arr, close)
plot(array.size(arr))
";
    let kept: Arc<Mutex<Option<ValueRef>>> = Arc::default();
    let outcomes: Arc<Mutex<Vec<Result<usize, InspectError>>>> = Arc::default();
    let (keep, results) = (Arc::clone(&kept), Arc::clone(&outcomes));
    let probe: Probe = Box::new(move |_, view| {
        let frame = view.stack()[0].index;
        let fresh = view
            .variables(frame)
            .into_iter()
            .flat_map(|group| group.variables)
            .find(|variable| variable.name == "arr")
            .and_then(|variable| variable.expand)
            .unwrap();
        let mut kept = keep.lock().unwrap();
        if let Some(old) = kept.as_ref() {
            results
                .lock()
                .unwrap()
                .push(view.expand(old, 0, 10).map(|page| page.total));
        }
        results
            .lock()
            .unwrap()
            .push(view.expand(&fresh, 0, 10).map(|page| page.total));
        *kept = Some(fresh);
    });
    at_lines(body, 2, &[5], probe);
    assert_eq!(
        *outcomes.lock().unwrap(),
        vec![Ok(1), Err(InspectError::Stale), Ok(2)]
    );
}

#[test]
fn what_changed_since_the_last_inspected_stop_is_marked() {
    let body = "level = 5.0
moving = close
plot(moving + level)
";
    let marks: Arc<Mutex<Vec<Marks>>> = Arc::default();
    let sink = Arc::clone(&marks);
    let probe: Probe = Box::new(move |_, view| {
        let frame = view.stack()[0].index;
        let changed = view
            .variables(frame)
            .into_iter()
            .flat_map(|group| group.variables)
            .filter(|variable| ["level", "moving", "close"].contains(&variable.name.as_str()))
            .map(|variable| (variable.name, variable.changed))
            .collect();
        sink.lock().unwrap().push(changed);
    });
    at_lines(body, 3, &[5], probe);
    let marks = marks.lock().unwrap();
    // `moving` is not in scope on the first line of its own statement, so look from the plot.
    assert!(
        marks[0].iter().all(|(_, changed)| !changed),
        "nothing to compare with yet"
    );
    let second: std::collections::HashMap<_, _> = marks[1].iter().cloned().collect();
    assert!(!second["level"]);
    assert!(second["moving"]);
    assert!(second["close"]);
}

#[test]
fn the_bar_context_describes_the_bar_the_script_is_on() {
    let seen: Arc<Mutex<Vec<BarContext>>> = Arc::default();
    let sink = Arc::clone(&seen);
    let probe: Probe = Box::new(move |_, view| sink.lock().unwrap().push(view.bar()));
    at_lines("plot(close)\n", 3, &[3], probe);
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 3);
    assert_eq!(seen[2].bar_index, 2);
    assert_eq!(seen[2].time_ms, 120_000);
    assert_eq!((seen[2].open, seen[2].close), (102.0, 103.0));
    assert!(seen[2].historical && seen[2].confirmed);
}

// MARK: Watches

fn watches(
    body: &str,
    count: usize,
    line: usize,
    frame_from_top: usize,
    texts: &[&str],
) -> Vec<String> {
    let results: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = Arc::clone(&results);
    let texts: Vec<String> = texts.iter().map(|text| (*text).to_owned()).collect();
    let probe: Probe = Box::new(move |stop, view| {
        if stop.bar_index + 1 != count as i64 {
            return;
        }
        let frame = view.stack()[frame_from_top].index;
        for text in &texts {
            let line = match parse_expression(text) {
                Err(error) => format!("{text} => refused: {error}"),
                Ok(expression) => match view.watch(frame, &expression) {
                    Watched::Value(value) => {
                        format!("{text} => {} ({})", value.value, value.type_name)
                    }
                    Watched::Error(error) => format!("{text} => error: {error}"),
                },
            };
            sink.lock().unwrap().push(line);
        }
    });
    at_lines(body, count, &[line], probe);
    results.lock().unwrap().clone()
}

#[test]
fn watch_expressions_see_the_stopped_scripts_names_with_pine_semantics() {
    let body = "var arr = array.new_float(0)
array.push(arr, close)
signal = close > open
plot(close)
";
    let results = watches(
        body,
        4,
        6,
        0,
        &[
            "close - open",
            "close[1]",
            "close[10]",
            "nz(close[10], -1)",
            "bar_index * 2",
            "signal ? \"up\" : \"down\"",
            "arr.get(0) + array.size(arr)",
            "na(close[10])",
            "str.tostring(close, \"#.0\")",
            "undefined_name + 1",
            "ta.sma(close, 3)",
            "plot(close)",
        ],
    );
    assert_eq!(results[0], "close - open => 1 (float)");
    assert_eq!(results[1], "close[1] => 103 (float)");
    assert_eq!(results[2], "close[10] => na (na)");
    assert_eq!(results[3], "nz(close[10], -1) => -1 (int)");
    assert_eq!(results[4], "bar_index * 2 => 6 (int)");
    assert_eq!(results[5], "signal ? \"up\" : \"down\" => \"up\" (string)");
    assert_eq!(results[6], "arr.get(0) + array.size(arr) => 105 (float)");
    assert_eq!(results[7], "na(close[10]) => true (bool)");
    assert_eq!(
        results[8],
        "str.tostring(close, \"#.0\") => \"104.0\" (string)"
    );
    assert!(
        results[9].starts_with("undefined_name + 1 => error:"),
        "{}",
        results[9]
    );
    assert!(
        results[10].contains("refused") && results[10].contains("keeps state"),
        "{}",
        results[10]
    );
    assert!(
        results[11].contains("refused") && results[11].contains("draws"),
        "{}",
        results[11]
    );
}

#[test]
fn a_watch_in_an_outer_frame_reads_that_frames_names() {
    let body = "x = 5.0
f(x) =>
    x * 2
y = f(10.0)
plot(y)
";
    let inner = watches(body, 1, 5, 0, &["x", "x * 3"]);
    let outer = watches(body, 1, 5, 1, &["x", "x * 3"]);
    assert_eq!(inner, ["x => 10 (float)", "x * 3 => 30 (float)"]);
    assert_eq!(outer, ["x => 5 (float)", "x * 3 => 15 (float)"]);
}

#[test]
fn a_watch_that_returns_a_collection_can_be_expanded() {
    let body = "var arr = array.new_float(0)
array.push(arr, close)
array.push(arr, open)
plot(array.size(arr))
";
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = Arc::clone(&seen);
    let probe: Probe = Box::new(move |_, view| {
        let frame = view.stack()[0].index;
        let expression = parse_expression("arr").unwrap();
        let Watched::Value(value) = view.watch(frame, &expression) else {
            panic!("arr evaluates");
        };
        let page = view.expand(value.expand.as_ref().unwrap(), 0, 10).unwrap();
        sink.lock().unwrap().push(format!(
            "{} {:?}",
            value.type_name,
            page.items
                .iter()
                .map(|v| v.value.clone())
                .collect::<Vec<_>>()
        ));
    });
    at_lines(body, 1, &[6], probe);
    assert_eq!(
        *seen.lock().unwrap(),
        vec!["array<float> [\"101\", \"100\"]"]
    );
}

#[test]
fn a_watch_that_would_allocate_is_refused_and_leaves_no_trace() {
    let body = "var arr = array.new_float(0)
array.push(arr, close)
plot(array.size(arr))
";
    let results = watches(
        body,
        2,
        5,
        0,
        &["str.split(\"a,b\", \",\")", "array.size(arr)"],
    );
    assert!(
        results[0].contains("error") || results[0].contains("refused"),
        "{}",
        results[0]
    );
    assert_eq!(results[1], "array.size(arr) => 2 (int)");
}

// MARK: Debugging changes nothing

fn observed(session: &crate::pine::session::Session) -> String {
    format!("{:?}", session.output())
}

#[test]
fn inspecting_and_evaluating_at_every_statement_changes_no_output() {
    let data = bench_tests::bars(80, bench_tests::MINUTE_MS);
    let scripts = [
        ("simple", bench_tests::SIMPLE.to_owned(), false),
        ("containers", bench_tests::CONTAINERS.to_owned(), false),
        ("functions", bench_tests::NESTED_FUNCTIONS.to_owned(), false),
        ("strategy", bench_tests::STRATEGY.to_owned(), false),
        ("history", bench_tests::LONG_HISTORY.to_owned(), false),
        ("straight", bench_tests::straight_line(30), false),
    ];
    let texts = [
        "close - open",
        "close[1] + close[2]",
        "bar_index % 3 == 0",
        "nz(open[50]) + volume",
        "math.max(high, low)",
        "fast",
        "slow",
        "signal",
        "v3",
        "v29",
        "arr.size()",
        "a",
        "b",
        "x",
        "y",
        "total",
        "ta.sma(close, 3)",
        "undefined_thing",
    ];
    for (name, source, security) in scripts {
        let plain = observed(&run_script(&source, &data, security, |_| {}));
        let probe: Probe = Box::new(move |_, view| {
            for frame in view.stack() {
                let groups = view.variables(frame.index);
                for variable in groups.iter().flat_map(|group| &group.variables) {
                    if let Some(reference) = &variable.expand {
                        let _ = view.expand(reference, 0, 50);
                    }
                }
                for text in texts {
                    if let Ok(expression) = parse_expression(text) {
                        let _ = view.watch(frame.index, &expression);
                    }
                }
            }
        });
        let (runtime, seen) = runtime_with(
            &source,
            Vec::new(),
            true,
            &[Resume::Into; 200_000],
            DebugControl::default(),
            Some(probe),
        );
        let debugged = observed(&run_script(&source, &data, security, |session| {
            session.attach_debugger(runtime);
        }));
        assert_eq!(plain, debugged, "{name}");
        assert!(!seen.lock().unwrap().is_empty(), "{name} never stopped");
    }
}

#[test]
fn evaluating_a_watch_leaves_the_interpreters_state_exactly_as_it_was() {
    let source = with_header(
        "var arr = array.new_float(0)\narray.push(arr, close)\nx = close[1]\nplot(x + array.size(arr))\n",
    );
    let mut session = run_script(&source, &bars(10), false, |_| {});
    let before = format!("{:?}", session.working);
    let context = crate::pine::runtime_types::RuntimeContext::new(
        bars(1).remove(0),
        crate::pine::runtime_types::BarFlags::default(),
    );
    for text in [
        "close[1] + 1",
        "arr.get(0)",
        "array.size(arr) * 2",
        "nz(x[3])",
        "undefined + 1",
    ] {
        let expression = parse_expression(text).unwrap();
        let _ = super::evaluate::evaluate(&mut session, &[], 0, &context, &expression);
    }
    assert_eq!(format!("{:?}", session.working), before);
}
