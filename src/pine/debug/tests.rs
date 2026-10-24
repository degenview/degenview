//! Stepping, breakpoint and equivalence tests. A scripted [`Debugger`] answers every stop from a list
//! and records what it saw, so a test states exactly where the script stops and how deep it is.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::model::Kline;
use crate::pine::bench_tests::{self, run_script};
use crate::pine::compiler::{self, NoLibraries};
use crate::pine::session::Session;
use crate::pine::token::Limits;

use super::*;

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Why {
    Breakpoint,
    Step,
    Pause,
    Bar,
    Error(String),
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Seen {
    pub why: Why,
    pub line: usize,
    pub depth: usize,
    pub bar: i64,
    /// Innermost first.
    pub stack: Vec<String>,
}

pub(super) type Log = Arc<Mutex<Vec<Seen>>>;

/// Looks at each stop through the view before the scripted answer is given.
pub(super) type Probe = Box<dyn FnMut(&Stop, &mut StopView<'_>) + Send>;

struct Scripted {
    answers: VecDeque<Resume>,
    seen: Log,
    probe: Option<Probe>,
}

impl Debugger for Scripted {
    fn stopped(&mut self, stop: &Stop, view: &mut StopView<'_>) -> Resume {
        let why = match &stop.reason {
            StopReason::Breakpoint(_) => Why::Breakpoint,
            StopReason::Step => Why::Step,
            StopReason::Pause => Why::Pause,
            StopReason::Bar => Why::Bar,
            StopReason::Exception(error) => Why::Error(error.code.clone()),
        };
        if let Some(probe) = self.probe.as_mut() {
            probe(stop, view);
        }
        self.seen.lock().unwrap().push(Seen {
            why,
            line: stop.location.map_or(0, |location| location.line),
            depth: stop.depth,
            bar: stop.bar_index,
            stack: view.stack().into_iter().map(|frame| frame.label).collect(),
        });
        self.answers.pop_front().unwrap_or(Resume::Continue)
    }
}

pub(super) fn bars(count: usize) -> Vec<Kline> {
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

pub(super) fn with_header(body: &str) -> String {
    format!("//@version=6\nindicator(\"T\")\n{body}")
}

fn runtime(
    source: &str,
    breakpoint_lines: &[usize],
    stop_on_entry: bool,
    answers: &[Resume],
    control: DebugControl,
) -> (DebugRuntime, Log) {
    let program = compiler::compile(source, Limits::default(), &NoLibraries);
    assert!(program.is_valid(), "{:?}", program.diagnostics);
    let map = SourceMap::build(&program.statements);
    let specs = breakpoint_lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            match map.resolve_line(*line) {
                LineResolution::Executable { .. } => {}
                other => panic!("line {line} is not executable: {other:?}"),
            }
            BreakpointSpec::line(index as u64 + 1, *line)
        })
        .collect();
    runtime_with(source, specs, stop_on_entry, answers, control, None)
}

pub(super) fn runtime_with(
    source: &str,
    specs: Vec<BreakpointSpec>,
    stop_on_entry: bool,
    answers: &[Resume],
    control: DebugControl,
    probe: Option<Probe>,
) -> (DebugRuntime, Log) {
    let program = compiler::compile(source, Limits::default(), &NoLibraries);
    assert!(program.is_valid(), "{:?}", program.diagnostics);
    let map = SourceMap::build(&program.statements);
    let seen = Log::default();
    let debugger = Scripted {
        answers: answers.iter().copied().collect(),
        seen: Arc::clone(&seen),
        probe,
    };
    let runtime = DebugRuntime::new(
        Arc::new(map),
        Box::new(debugger),
        control,
        DebugOptions {
            stop_on_entry,
            breakpoints: specs,
        },
    );
    (runtime, seen)
}

/// Runs `body` over `count` bars under the scripted debugger and returns every stop, plus whether
/// the run completed.
fn debug(
    body: &str,
    count: usize,
    breakpoint_lines: &[usize],
    stop_on_entry: bool,
    answers: &[Resume],
) -> Vec<Seen> {
    let source = with_header(body);
    let (runtime, seen) = runtime(
        &source,
        breakpoint_lines,
        stop_on_entry,
        answers,
        DebugControl::default(),
    );
    run_script(&source, &bars(count), false, |session| {
        session.attach_debugger(runtime);
    });
    seen.lock().unwrap().clone()
}

fn lines(seen: &[Seen]) -> Vec<usize> {
    seen.iter().map(|stop| stop.line).collect()
}

fn depths(seen: &[Seen]) -> Vec<usize> {
    seen.iter().map(|stop| stop.depth).collect()
}

/// Line 3 is the first line of the body.
const CALCULATE: &str = "calculate(x) =>
    doubled = x * 2
    result = doubled + 10
    result

value = calculate(close)
plot(value)
";

#[test]
fn a_breakpoint_stops_before_its_statement_with_the_stack() {
    let seen = debug(CALCULATE, 1, &[8], false, &[]);
    assert_eq!(
        seen,
        vec![Seen {
            why: Why::Breakpoint,
            line: 8,
            depth: 1,
            bar: 0,
            stack: vec!["Main Script".into()],
        }]
    );
}

#[test]
fn step_into_enters_the_function_and_names_its_caller() {
    let seen = debug(CALCULATE, 1, &[8], false, &[Resume::Into]);
    assert_eq!(lines(&seen), vec![8, 4]);
    assert_eq!(seen[1].why, Why::Step);
    assert_eq!(seen[1].depth, 2);
    assert_eq!(seen[1].stack, vec!["calculate()", "Main Script"]);
}

#[test]
fn step_over_runs_the_call_and_stops_at_the_next_statement_of_the_caller() {
    let seen = debug(CALCULATE, 1, &[8], false, &[Resume::Over]);
    assert_eq!(lines(&seen), vec![8, 9]);
    assert_eq!(depths(&seen), vec![1, 1]);
}

#[test]
fn step_over_inside_a_function_stays_in_it() {
    let seen = debug(CALCULATE, 1, &[4], false, &[Resume::Over, Resume::Over]);
    assert_eq!(lines(&seen), vec![4, 5, 6]);
    assert_eq!(depths(&seen), vec![2, 2, 2]);
}

#[test]
fn step_out_returns_to_the_caller_at_its_next_statement() {
    let seen = debug(CALCULATE, 1, &[5], false, &[Resume::Out]);
    assert_eq!(lines(&seen), vec![5, 9]);
    assert_eq!(depths(&seen), vec![2, 1]);
}

#[test]
fn step_out_of_the_script_runs_on_like_continue() {
    let seen = debug(CALCULATE, 3, &[8], false, &[Resume::Out]);
    // The script frame has no caller: stepping out of it only stops at the next breakpoint.
    assert_eq!(lines(&seen), vec![8, 8, 8]);
    assert_eq!(
        seen.iter().map(|s| s.bar).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
}

#[test]
fn step_bar_stops_at_the_first_statement_of_the_next_bar_from_any_depth() {
    // From inside `calculate` on bar 0, Step Bar lands on bar 1's first statement ...
    let seen = debug(CALCULATE, 3, &[5], false, &[Resume::Bar]);
    assert_eq!(lines(&seen)[..2], [5, 8]);
    assert_eq!(depths(&seen)[..2], [2, 1]);
    assert_eq!(seen.iter().map(|s| s.bar).collect::<Vec<_>>()[..2], [0, 1]);
    // ... and a breakpoint reached first on that bar still wins over the step.
    assert_eq!(lines(&seen), vec![5, 8, 5, 5]);
}

#[test]
fn step_bar_chains_one_bar_at_a_time() {
    let seen = debug(
        CALCULATE,
        4,
        &[],
        true,
        &[Resume::Bar, Resume::Bar, Resume::Bar],
    );
    assert_eq!(lines(&seen), vec![8, 8, 8, 8]);
    assert_eq!(
        seen.iter().map(|s| s.bar).collect::<Vec<_>>(),
        vec![0, 1, 2, 3]
    );
}

#[test]
fn continue_runs_to_the_next_breakpoint_on_every_bar() {
    let seen = debug(CALCULATE, 3, &[5], false, &[]);
    assert_eq!(lines(&seen), vec![5, 5, 5]);
    assert!(seen.iter().all(|stop| stop.why == Why::Breakpoint));
    assert_eq!(
        seen.iter().map(|s| s.bar).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
}

#[test]
fn stopping_on_entry_steps_through_every_statement_in_order() {
    let answers = [Resume::Into; 8];
    let seen = debug(CALCULATE, 1, &[], true, &answers);
    assert_eq!(lines(&seen), vec![8, 4, 5, 6, 9]);
    assert_eq!(depths(&seen), vec![1, 2, 2, 2, 1]);
}

#[test]
fn no_statement_is_stopped_at_twice_when_stepping_into() {
    let answers = [Resume::Into; 40];
    let seen = debug(CALCULATE, 2, &[], true, &answers);
    // Two bars of five statements each; every stop is a distinct (bar, line).
    let mut places: Vec<(i64, usize)> = seen.iter().map(|s| (s.bar, s.line)).collect();
    let total = places.len();
    places.dedup();
    assert_eq!(places.len(), total);
    assert_eq!(total, 10);
}

#[test]
fn loops_stop_once_per_iteration_in_the_body_and_once_for_the_header() {
    let body = "total = 0.0
for i = 0 to 2
    total += i
plot(total)
";
    let seen = debug(body, 1, &[], true, &[Resume::Into; 12]);
    assert_eq!(lines(&seen), vec![3, 4, 5, 5, 5, 6]);
    let over = debug(body, 1, &[4], false, &[Resume::Over, Resume::Over]);
    assert_eq!(lines(&over), vec![4, 5, 5]);
}

#[test]
fn only_the_taken_branch_is_stopped_in() {
    let body = "x = 0.0
if close > open
    x := 1.0
else
    x := 2.0
plot(x)
";
    // close = open + 1 on every bar, so the true branch runs.
    let seen = debug(body, 1, &[], true, &[Resume::Into; 8]);
    assert_eq!(lines(&seen), vec![3, 4, 5, 8]);
}

#[test]
fn a_statement_spread_over_lines_stops_once_at_its_start() {
    let body = "x = math.max(
     close,
     open)
plot(x)
";
    let seen = debug(body, 1, &[], true, &[Resume::Into; 4]);
    assert_eq!(lines(&seen), vec![3, 6]);
}

#[test]
fn a_call_inside_an_expression_is_entered_by_step_into() {
    let body = "f(x) =>
    x + 1
plot(f(close) + f(open))
";
    let seen = debug(body, 1, &[], true, &[Resume::Into; 6]);
    assert_eq!(lines(&seen), vec![5, 4, 4]);
    assert_eq!(depths(&seen), vec![1, 2, 2]);
}

#[test]
fn a_function_returning_na_is_stepped_like_any_other() {
    let body = "f(x) =>
    na
v = f(close)
plot(nz(v))
";
    let seen = debug(body, 1, &[], true, &[Resume::Into; 6]);
    assert_eq!(lines(&seen), vec![5, 4, 6]);
}

#[test]
fn stopping_ends_the_script_and_no_later_statement_runs() {
    let source = with_header(CALCULATE);
    let (runtime, seen) = runtime(
        &source,
        &[8],
        false,
        &[Resume::Stop],
        DebugControl::default(),
    );
    let program = compiler::compile(&source, Limits::default(), &NoLibraries);
    let mut session = Session::new(
        program,
        Default::default(),
        Limits::default(),
        None,
        Default::default(),
        None,
        None,
    );
    session.attach_debugger(runtime);
    let outcome = session.evaluate(&bars(5));
    assert!(outcome.is_err(), "a stopped script does not complete");
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn a_pause_request_stops_at_the_next_statement() {
    let source = with_header(CALCULATE);
    let control = DebugControl::default();
    let (runtime, seen) = runtime(&source, &[], false, &[], control.clone());
    control.request_pause();
    run_script(&source, &bars(2), false, |session| {
        session.attach_debugger(runtime)
    });
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1, "the request is spent by one stop");
    assert_eq!(seen[0].why, Why::Pause);
    assert_eq!(seen[0].line, 8);
}

#[test]
fn a_runtime_error_stops_where_it_happened_and_cannot_be_continued_past() {
    let body = "arr = array.new_float(1)
x = array.get(arr, 5)
plot(x)
";
    let source = with_header(body);
    let (runtime, seen) = runtime(
        &source,
        &[],
        false,
        &[Resume::Continue],
        DebugControl::default(),
    );
    let program = compiler::compile(&source, Limits::default(), &NoLibraries);
    let mut session = Session::new(
        program,
        Default::default(),
        Limits::default(),
        None,
        Default::default(),
        None,
        None,
    );
    session.attach_debugger(runtime);
    let outcome = session.evaluate(&bars(3));
    assert!(outcome.is_err());
    let seen = seen.lock().unwrap().clone();
    assert_eq!(
        seen.len(),
        1,
        "one report per failure, however far it unwinds"
    );
    assert!(matches!(seen[0].why, Why::Error(_)));
    assert_eq!(seen[0].line, 4);
    assert_eq!(seen[0].bar, 0);
}

#[test]
fn an_error_inside_a_function_reports_the_whole_stack_once() {
    let body = "f(a) =>
    array.get(a, 5)
arr = array.new_float(1)
plot(f(arr))
";
    let source = with_header(body);
    let (runtime, seen) = runtime(&source, &[], false, &[], DebugControl::default());
    let program = compiler::compile(&source, Limits::default(), &NoLibraries);
    let mut session = Session::new(
        program,
        Default::default(),
        Limits::default(),
        None,
        Default::default(),
        None,
        None,
    );
    session.attach_debugger(runtime);
    assert!(session.evaluate(&bars(1)).is_err());
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].line, 4);
    assert_eq!(seen[0].stack, vec!["f()", "Main Script"]);
}

// MARK: Semantic equivalence

fn scripts() -> Vec<(&'static str, String, bool)> {
    vec![
        ("simple", bench_tests::SIMPLE.to_owned(), false),
        ("many_ta", bench_tests::MANY_TA.to_owned(), false),
        ("history", bench_tests::LONG_HISTORY.to_owned(), false),
        ("containers", bench_tests::CONTAINERS.to_owned(), false),
        ("functions", bench_tests::NESTED_FUNCTIONS.to_owned(), false),
        ("security", bench_tests::SECURITY.to_owned(), true),
        ("strategy", bench_tests::STRATEGY.to_owned(), false),
        ("straight", bench_tests::straight_line(40), false),
    ]
}

/// Everything a run produced that a person or a chart can observe.
fn observed(session: &Session) -> String {
    format!("{:?}", session.output())
}

#[test]
fn attaching_a_debugger_that_never_stops_changes_nothing() {
    let data = bench_tests::bars(300, bench_tests::MINUTE_MS);
    for (name, source, security) in scripts() {
        let plain = observed(&run_script(&source, &data, security, |_| {}));
        let (runtime, seen) = runtime(&source, &[], false, &[], DebugControl::default());
        let debugged = observed(&run_script(&source, &data, security, |session| {
            session.attach_debugger(runtime);
        }));
        assert_eq!(plain, debugged, "{name}");
        assert!(seen.lock().unwrap().is_empty(), "{name} stopped");
    }
}

#[test]
fn stepping_through_every_statement_changes_nothing() {
    let data = bench_tests::bars(60, bench_tests::MINUTE_MS);
    for (name, source, security) in scripts() {
        let plain = observed(&run_script(&source, &data, security, |_| {}));
        let (runtime, seen) = runtime(
            &source,
            &[],
            true,
            &[Resume::Into; 100_000],
            DebugControl::default(),
        );
        let debugged = observed(&run_script(&source, &data, security, |session| {
            session.attach_debugger(runtime);
        }));
        assert_eq!(plain, debugged, "{name}");
        assert!(!seen.lock().unwrap().is_empty(), "{name} never stopped");
    }
}
