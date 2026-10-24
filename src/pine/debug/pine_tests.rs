//! Behaviour specific to Pine: stopping at a bar, `request.security()` boundaries, and the
//! deadline and limits while a person is looking.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::pine::bench_tests::{self, run_script};
use crate::pine::compiler::{self, NoLibraries};
use crate::pine::session::{RunError, Session};
use crate::pine::token::Limits;

use super::tests::{Probe, Seen, Why, bars, runtime_with, with_header};
use super::*;

const SCRIPT: &str = "calculate(x) =>
    doubled = x * 2
    doubled + 10

value = calculate(close)
plot(value)
";

fn run_with(
    control: DebugControl,
    count: usize,
    specs: Vec<BreakpointSpec>,
    probe: Option<Probe>,
) -> Vec<Seen> {
    let source = with_header(SCRIPT);
    let (runtime, seen) = runtime_with(&source, specs, false, &[], control, probe);
    run_script(&source, &bars(count), false, |session| {
        session.attach_debugger(runtime);
    });
    seen.lock().unwrap().clone()
}

fn places(seen: &[Seen]) -> Vec<(usize, i64)> {
    seen.iter().map(|stop| (stop.line, stop.bar)).collect()
}

// MARK: Break on bar

#[test]
fn a_bar_index_stops_at_the_first_statement_of_that_bar_once() {
    let control = DebugControl::default();
    control.set_bar_break(Some(BarBreak::Index(4)));
    let seen = run_with(control, 8, Vec::new(), None);
    assert_eq!(places(&seen), vec![(7, 4)]);
    assert_eq!(seen[0].why, Why::Bar);
    assert_eq!(seen[0].depth, 1);
}

#[test]
fn a_time_stops_at_the_first_bar_that_opens_at_or_after_it() {
    let control = DebugControl::default();
    // Bars open every 60 s from 0: 250 s falls inside bar 4, so bar 5 is the first at or after it.
    control.set_bar_break(Some(BarBreak::Time(250_000)));
    assert_eq!(
        places(&run_with(control, 8, Vec::new(), None)),
        vec![(7, 5)]
    );

    let exact = DebugControl::default();
    exact.set_bar_break(Some(BarBreak::Time(180_000)));
    assert_eq!(places(&run_with(exact, 8, Vec::new(), None)), vec![(7, 3)]);
}

#[test]
fn a_bar_break_can_be_set_and_cancelled_while_the_script_runs() {
    let control = DebugControl::default();
    let retarget = control.clone();
    let probe: Probe = Box::new(move |stop, _| match stop.bar_index {
        // From the first stop, ask for bar 5 and then change our mind.
        1 => retarget.set_bar_break(Some(BarBreak::Index(5))),
        5 => retarget.set_bar_break(Some(BarBreak::Index(7))),
        _ => {}
    });
    control.set_bar_break(Some(BarBreak::Index(1)));
    let seen = run_with(control, 9, Vec::new(), Some(probe));
    assert_eq!(
        seen.iter().map(|s| s.bar).collect::<Vec<_>>(),
        vec![1, 5, 7]
    );

    let cancelled = DebugControl::default();
    cancelled.set_bar_break(Some(BarBreak::Index(2)));
    cancelled.set_bar_break(None);
    assert!(run_with(cancelled, 5, Vec::new(), None).is_empty());
}

#[test]
fn a_bar_break_on_a_statement_with_a_breakpoint_stops_once_for_the_breakpoint() {
    let control = DebugControl::default();
    control.set_bar_break(Some(BarBreak::Index(2)));
    let seen = run_with(control, 4, vec![BreakpointSpec::line(1, 7)], None);
    // Bar 2's first statement is line 7, which also holds a breakpoint: one stop, and the more
    // specific reason.
    assert_eq!(places(&seen), vec![(7, 0), (7, 1), (7, 2), (7, 3)]);
    assert_eq!(seen[2].why, Why::Breakpoint);
}

// MARK: request.security

const REQUEST: &str = "base = 2.0
scaled(x) =>
    y = x * base
    y + 1
htf = request.security(syminfo.tickerid, \"5\", scaled(close))
plot(htf)
";

#[test]
fn a_request_appears_as_a_boundary_frame_between_the_expression_and_the_chart() {
    let source = with_header(REQUEST);
    let captured: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = Arc::clone(&captured);
    let probe: Probe = Box::new(move |_, view| {
        let stack = view.stack();
        let mut out = sink.lock().unwrap();
        out.push(format!(
            "stack: {:?}",
            stack
                .iter()
                .map(|frame| frame.label.clone())
                .collect::<Vec<_>>()
        ));
        for frame in &stack {
            let names: Vec<String> = view
                .variables(frame.index)
                .into_iter()
                .filter(|group| group.group != Group::BuiltIn)
                .flat_map(|group| group.variables)
                .map(|variable| format!("{}={}", variable.name, variable.value))
                .collect();
            out.push(format!("{}: {names:?}", frame.label));
        }
        let bar = view.bar();
        let request = bar.request.expect("inside a request");
        out.push(format!(
            "request {} {}s chart bar {}",
            request.symbol, request.interval_seconds, request.chart_bar_index
        ));
        let top = stack[0].index;
        let outer = stack.last().unwrap().index;
        let expression = parse_expression("x * 3").unwrap();
        out.push(format!("inside: {:?}", view.watch(top, &expression)));
        let expression = parse_expression("base").unwrap();
        out.push(format!("outside: {:?}", view.watch(outer, &expression)));
    });
    let (runtime, seen) = runtime_with(
        &source,
        vec![BreakpointSpec {
            hit: HitRule::Equals(1),
            ..BreakpointSpec::line(1, 6)
        }],
        false,
        &[],
        DebugControl::default(),
        Some(probe),
    );
    run_script(
        &source,
        &bench_tests::bars(30, bench_tests::MINUTE_MS),
        true,
        |session| {
            session.attach_debugger(runtime);
        },
    );
    assert_eq!(seen.lock().unwrap().len(), 1);
    let captured = captured.lock().unwrap();
    assert!(
        captured[0].contains("scaled()")
            && captured[0].contains("request.security()")
            && captured[0].contains("5m")
            && captured[0].contains("Main Script"),
        "{}",
        captured[0]
    );
    assert!(
        captured[1].starts_with("scaled(): ")
            && captured[1].contains("x=")
            && captured[1].contains("y="),
        "{}",
        captured[1]
    );
    assert!(
        captured[2].ends_with("[]"),
        "the boundary holds no variables: {}",
        captured[2]
    );
    assert!(
        captured[3].contains("base=2"),
        "the chart's variables are read back: {}",
        captured[3]
    );
    assert!(captured[4].contains("300s"), "{}", captured[4]);
    assert!(captured[5].starts_with("inside: Value"), "{}", captured[5]);
    assert!(
        captured[6].contains("outside the request.security()"),
        "{}",
        captured[6]
    );
}

#[test]
fn stepping_out_of_a_function_in_a_request_returns_into_the_request_and_on() {
    let source = with_header(REQUEST);
    let (runtime, seen) = runtime_with(
        &source,
        vec![BreakpointSpec {
            hit: HitRule::Equals(1),
            ..BreakpointSpec::line(1, 6)
        }],
        false,
        &[Resume::Out, Resume::Out],
        DebugControl::default(),
        None,
    );
    run_script(
        &source,
        &bench_tests::bars(30, bench_tests::MINUTE_MS),
        true,
        |session| {
            session.attach_debugger(runtime);
        },
    );
    let seen = seen.lock().unwrap();
    // Out of `scaled` (depth 3: scaled, request, script) lands on the next statement that runs
    // shallower than that: the chart's `plot` statement.
    assert_eq!(seen[0].depth, 3);
    assert_eq!((seen[1].line, seen[1].depth), (8, 1));
}

#[test]
fn debugging_a_script_with_requests_changes_none_of_its_outputs() {
    let data = bench_tests::bars(120, bench_tests::MINUTE_MS);
    let source = with_header(REQUEST);
    let plain = format!("{:?}", run_script(&source, &data, true, |_| {}).output());
    let probe: Probe = Box::new(|_, view| {
        for frame in view.stack() {
            let _ = view.variables(frame.index);
            if let Ok(expression) = parse_expression("base * 2") {
                let _ = view.watch(frame.index, &expression);
            }
        }
    });
    let (runtime, seen) = runtime_with(
        &source,
        Vec::new(),
        true,
        &[Resume::Into; 100_000],
        DebugControl::default(),
        Some(probe),
    );
    let debugged = format!(
        "{:?}",
        run_script(&source, &data, true, |session| session
            .attach_debugger(runtime))
        .output()
    );
    assert_eq!(plain, debugged);
    assert!(!seen.lock().unwrap().is_empty());
}

// MARK: Deadline and limits

fn session_with(limits: Limits, body: &str) -> Session {
    let source = with_header(body);
    let program = compiler::compile(&source, limits, &NoLibraries);
    assert!(program.is_valid(), "{:?}", program.diagnostics);
    Session::new(
        program,
        Default::default(),
        limits,
        None,
        Default::default(),
        None,
        None,
    )
}

fn runtime_for(
    body: &str,
    answers: &[Resume],
    stop_on_entry: bool,
    probe: Option<Probe>,
) -> DebugRuntime {
    let source = with_header(body);
    runtime_with(
        &source,
        Vec::new(),
        stop_on_entry,
        answers,
        DebugControl::default(),
        probe,
    )
    .0
}

#[test]
fn time_spent_stopped_does_not_count_against_the_deadline() {
    let limits = Limits {
        deadline_seconds: 0.4,
        ..Limits::default()
    };
    let body = "plot(close)\n";
    let probe: Probe = Box::new(|_, _| std::thread::sleep(Duration::from_millis(900)));
    let mut session = session_with(limits, body);
    session.attach_debugger(runtime_for(body, &[Resume::Continue], true, Some(probe)));
    assert!(
        session.evaluate(&bars(3)).is_ok(),
        "a long pause is not a slow script"
    );
}

#[test]
fn a_runaway_script_still_hits_the_deadline_under_the_debugger() {
    let limits = Limits {
        deadline_seconds: 0.3,
        ..Limits::default()
    };
    let body = "i = 0\nwhile true\n    i += 1\n";
    let mut session = session_with(limits, body);
    session.attach_debugger(runtime_for(body, &[], false, None));
    match session.evaluate(&bars(1)) {
        Err(RunError::Script(error)) => assert_eq!(error.code, "PINE8007", "{error:?}"),
        other => panic!("expected the deadline, got {:?}", other.map(|_| ())),
    }
}

#[test]
fn the_instruction_limit_applies_while_stepping() {
    let limits = Limits {
        instructions_per_bar: 5_000,
        ..Limits::default()
    };
    let body = "i = 0\nwhile true\n    i += 1\n";
    let mut session = session_with(limits, body);
    session.attach_debugger(runtime_for(body, &[], false, None));
    match session.evaluate(&bars(1)) {
        Err(RunError::Script(error)) => assert_eq!(error.code, "PINE8004", "{error:?}"),
        other => panic!(
            "expected the instruction limit, got {:?}",
            other.map(|_| ())
        ),
    }
}

#[test]
fn a_resource_error_stops_with_a_location_and_cannot_be_continued() {
    let limits = Limits {
        instructions_per_bar: 3_000,
        ..Limits::default()
    };
    let body = "i = 0\nwhile true\n    i += 1\n";
    let seen: Arc<Mutex<Vec<Seen>>> = Arc::default();
    let source = with_header(body);
    let (runtime, log) = runtime_with(
        &source,
        Vec::new(),
        false,
        &[Resume::Continue],
        DebugControl::default(),
        None,
    );
    let mut session = session_with(limits, body);
    session.attach_debugger(runtime);
    let _ = (&seen, session.evaluate(&bars(1)));
    let log = log.lock().unwrap();
    assert_eq!(log.len(), 1);
    assert!(
        matches!(log[0].why, Why::Error(ref code) if code == "PINE8004"),
        "{:?}",
        log[0]
    );
    assert!(
        log[0].line >= 3,
        "the failing statement is in the loop: {:?}",
        log[0]
    );
}
