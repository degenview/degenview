//! Conditional, hit-count and logpoint breakpoints, and changing them while the script runs.

use std::sync::{Arc, Mutex};

use crate::pine::bench_tests::run_script;
use crate::pine::logs::LogLevel;
use crate::pine::session::Session;

use super::tests::{Probe, Seen, Why, bars, runtime_with, with_header};
use super::*;

/// Line 3 is the first line of the body.
const SCRIPT: &str = "calculate(x) =>
    doubled = x * 2
    result = doubled + 10
    result

value = calculate(close)
plot(value)
";

fn run(
    body: &str,
    count: usize,
    specs: Vec<BreakpointSpec>,
    control: DebugControl,
    probe: Option<Probe>,
) -> (Vec<Seen>, Session) {
    let source = with_header(body);
    let (runtime, seen) = runtime_with(&source, specs, false, &[], control, probe);
    let session = run_script(&source, &bars(count), false, |session| {
        session.attach_debugger(runtime);
    });
    let seen = seen.lock().unwrap().clone();
    (seen, session)
}

fn stops(specs: Vec<BreakpointSpec>, count: usize) -> Vec<(usize, i64)> {
    let (seen, _) = run(SCRIPT, count, specs, DebugControl::default(), None);
    seen.iter().map(|stop| (stop.line, stop.bar)).collect()
}

fn at_call(configure: impl FnOnce(&mut BreakpointSpec)) -> Vec<BreakpointSpec> {
    let mut spec = BreakpointSpec::line(1, 8);
    configure(&mut spec);
    vec![spec]
}

fn messages(session: &mut Session) -> Vec<String> {
    session
        .logs
        .take()
        .events
        .into_iter()
        .map(|event| event.message)
        .collect()
}

#[test]
fn a_condition_decides_which_bars_stop() {
    let only_bar_three = at_call(|spec| spec.condition = Some("bar_index == 3".to_owned()));
    assert_eq!(stops(only_bar_three, 6), vec![(8, 3)]);
    let by_price =
        at_call(|spec| spec.condition = Some("close > 103.5 and close < 105.5".to_owned()));
    // close is 101 + bar index.
    assert_eq!(stops(by_price, 8), vec![(8, 3), (8, 4)]);
}

#[test]
fn a_condition_that_is_false_or_na_never_stops() {
    assert!(
        stops(
            at_call(|spec| spec.condition = Some("close < 0".to_owned())),
            4
        )
        .is_empty()
    );
    assert!(stops(at_call(|spec| spec.condition = Some("na".to_owned())), 4).is_empty());
}

#[test]
fn hit_rules_count_each_time_the_statement_is_reached() {
    let nth = at_call(|spec| spec.hit = HitRule::Equals(3));
    assert_eq!(stops(nth, 6), vec![(8, 2)]);
    let every = at_call(|spec| spec.hit = HitRule::EveryNth(2));
    assert_eq!(stops(every, 6), vec![(8, 1), (8, 3), (8, 5)]);
    let after = at_call(|spec| spec.hit = HitRule::GreaterThan(4));
    assert_eq!(stops(after, 6), vec![(8, 4), (8, 5)]);
}

#[test]
fn hits_are_counted_only_while_the_condition_holds() {
    let both = at_call(|spec| {
        spec.condition = Some("bar_index % 2 == 0".to_owned());
        spec.hit = HitRule::EveryNth(2);
    });
    // The condition holds on bars 0, 2, 4, 6: the 2nd and 4th of those are bars 2 and 6.
    assert_eq!(stops(both, 8), vec![(8, 2), (8, 6)]);
}

#[test]
fn hit_counts_are_published_for_the_panel_and_a_new_session_starts_from_zero() {
    let control = DebugControl::default();
    let specs = at_call(|spec| spec.hit = HitRule::GreaterThan(100));
    run(SCRIPT, 7, specs.clone(), control.clone(), None);
    assert_eq!(control.hit_counts(), vec![(1, 7)]);
    let again = DebugControl::default();
    run(SCRIPT, 3, specs, again.clone(), None);
    assert_eq!(
        again.hit_counts(),
        vec![(1, 3)],
        "a restart counts again from zero"
    );
}

#[test]
fn a_disabled_breakpoint_does_not_stop() {
    assert!(stops(at_call(|spec| spec.enabled = false), 3).is_empty());
}

#[test]
fn breakpoints_stop_inside_functions_and_loops() {
    let in_function = vec![BreakpointSpec::line(1, 5)];
    let (seen, _) = run(SCRIPT, 2, in_function, DebugControl::default(), None);
    assert_eq!(
        seen.iter()
            .map(|s| (s.line, s.depth, s.bar))
            .collect::<Vec<_>>(),
        vec![(5, 2, 0), (5, 2, 1)]
    );

    let looped = "total = 0.0\nfor i = 0 to 2\n    total += i\nplot(total)\n";
    let (seen, _) = run(
        looped,
        1,
        vec![BreakpointSpec::line(1, 5)],
        DebugControl::default(),
        None,
    );
    assert_eq!(seen.len(), 3);
}

#[test]
fn a_condition_can_read_a_loop_counter() {
    let looped = "total = 0.0\nfor i = 0 to 4\n    total += i\nplot(total)\n";
    let mut spec = BreakpointSpec::line(1, 5);
    spec.condition = Some("i == 3".to_owned());
    let seen_total: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = Arc::clone(&seen_total);
    let probe: Probe = Box::new(move |_, view| {
        let frame = view.stack()[0].index;
        for group in view.variables(frame) {
            for variable in group.variables {
                if variable.name == "total" {
                    sink.lock().unwrap().push(variable.value);
                }
            }
        }
    });
    let (seen, _) = run(looped, 1, vec![spec], DebugControl::default(), Some(probe));
    assert_eq!(seen.len(), 1);
    // 0 + 1 + 2 have been added when i reaches 3.
    assert_eq!(*seen_total.lock().unwrap(), vec!["3"]);
}

#[test]
fn a_condition_that_cannot_be_evaluated_stops_and_says_why() {
    let notices: Arc<Mutex<Vec<Option<String>>>> = Arc::default();
    let sink = Arc::clone(&notices);
    let probe: Probe = Box::new(move |stop, _| sink.lock().unwrap().push(stop.notice.clone()));
    let body = "var arr = array.new_float(1)\nvalue = close\nplot(value)\n";
    let mut spec = BreakpointSpec::line(1, 4);
    spec.condition = Some("array.get(arr, 5) > 1".to_owned());
    let (seen, _) = run(body, 2, vec![spec], DebugControl::default(), Some(probe));
    // Every visit stops, because the breakpoint cannot be trusted to mean "not now".
    assert_eq!(seen.len(), 2);
    let notices = notices.lock().unwrap();
    assert!(
        notices.iter().all(|notice| notice
            .as_deref()
            .is_some_and(|n| n.contains("condition could not be evaluated"))),
        "{notices:?}"
    );
}

#[test]
fn a_logpoint_writes_to_the_pine_logs_and_does_not_stop() {
    let spec = at_call(|spec| spec.log_message = Some("Bar {bar_index}: close={close}".to_owned()));
    let (seen, mut session) = run(SCRIPT, 3, spec, DebugControl::default(), None);
    assert!(seen.is_empty(), "a logpoint never stops the script");
    assert_eq!(
        messages(&mut session),
        vec!["Bar 0: close=101", "Bar 1: close=102", "Bar 2: close=103"]
    );
}

#[test]
fn a_logpoint_event_carries_its_source_range_level_and_bar() {
    let spec = at_call(|spec| spec.log_message = Some("hit".to_owned()));
    let (_, mut session) = run(SCRIPT, 2, spec, DebugControl::default(), None);
    let batch = session.logs.take();
    assert_eq!(batch.events.len(), 2);
    for (index, event) in batch.events.iter().enumerate() {
        assert_eq!(event.level, LogLevel::Info);
        assert_eq!(event.range.start.line, 8);
        assert_eq!(event.bar_index, Some(index as i64));
    }
}

#[test]
fn a_logpoint_honours_its_condition_and_hit_rule() {
    let spec = at_call(|spec| {
        spec.log_message = Some("b{bar_index}".to_owned());
        spec.condition = Some("bar_index >= 2".to_owned());
        spec.hit = HitRule::EveryNth(2);
    });
    let (_, mut session) = run(SCRIPT, 8, spec, DebugControl::default(), None);
    // Condition holds on bars 2..8; every second of those hits is bar 3, 5, 7.
    assert_eq!(messages(&mut session), vec!["b3", "b5", "b7"]);
}

#[test]
fn a_logpoint_formats_values_and_survives_a_failing_expression() {
    let spec = at_call(|spec| {
        spec.log_message = Some(
            "{close > open} {str.tostring(close, \"#.0\")} {math.pow(close, \"x\")}".to_owned(),
        );
    });
    let (_, mut session) = run(SCRIPT, 1, spec, DebugControl::default(), None);
    let messages = messages(&mut session);
    assert_eq!(messages.len(), 1);
    assert!(messages[0].starts_with("true 101.0 "), "{}", messages[0]);
}

#[test]
fn a_logpoint_inside_a_function_logs_every_call() {
    let source = "f(x) =>\n    x + 1\nplot(f(close) + f(open))\n";
    let (seen, mut session) = run(
        source,
        1,
        vec![{
            let mut spec = BreakpointSpec::line(1, 4);
            spec.log_message = Some("x={x}".to_owned());
            spec
        }],
        DebugControl::default(),
        None,
    );
    assert!(seen.is_empty());
    assert_eq!(messages(&mut session), vec!["x=101", "x=100"]);
}

#[test]
fn the_logs_stay_bounded_however_often_a_logpoint_fires() {
    let spec = at_call(|spec| spec.log_message = Some("tick".to_owned()));
    let (_, mut session) = run(SCRIPT, 12_000, spec, DebugControl::default(), None);
    let batch = session.logs.take();
    assert_eq!(
        batch.events.len(),
        10_000,
        "the sink keeps its historical limit"
    );
}

#[test]
fn breakpoints_can_be_replaced_while_the_script_runs() {
    let control = DebugControl::default();
    let swap = control.clone();
    let probe: Probe = Box::new(move |stop, _| {
        if stop.location.map(|location| location.line) == Some(8) {
            // Move the breakpoint into the function body.
            swap.set_breakpoints(vec![BreakpointSpec::line(1, 5)]);
        }
    });
    let (seen, _) = run(
        SCRIPT,
        3,
        vec![BreakpointSpec::line(1, 8)],
        control,
        Some(probe),
    );
    let places: Vec<(usize, i64)> = seen.iter().map(|s| (s.line, s.bar)).collect();
    // Stopped at 8 on bar 0, then the new breakpoint at 5 takes over: line 8 is no longer a stop.
    assert_eq!(places, vec![(8, 0), (5, 0), (5, 1), (5, 2)]);
    assert!(seen.iter().all(|stop| stop.why == Why::Breakpoint));
}

#[test]
fn hit_counts_survive_a_live_update_of_the_same_breakpoint() {
    let control = DebugControl::default();
    let update = control.clone();
    let probe: Probe = Box::new(move |stop, _| {
        if stop.bar_index == 1 {
            let mut spec = BreakpointSpec::line(1, 8);
            spec.hit = HitRule::Equals(4);
            update.set_breakpoints(vec![spec]);
        }
    });
    let mut spec = BreakpointSpec::line(1, 8);
    spec.hit = HitRule::EveryNth(2);
    let (seen, _) = run(SCRIPT, 6, vec![spec], control, Some(probe));
    // Hits 2 (bar 1) stopped under the old rule; after the edit the count continues from 2 and the
    // new rule fires on hit 4 (bar 3).
    assert_eq!(seen.iter().map(|s| s.bar).collect::<Vec<_>>(), vec![1, 3]);
}
