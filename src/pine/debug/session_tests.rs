//! The threaded session: commands and events end to end, and its lifecycle. Every wait has a
//! timeout, so a deadlock fails a test instead of hanging the run.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::pine::bench_tests;
use crate::pine::logs::LogHistory;
use crate::pine::program::SymbolInfo;

use super::tests::with_header;
use super::*;

const TIMEOUT: Duration = Duration::from_secs(10);

fn wake() -> Arc<dyn Fn() + Send + Sync> {
    Arc::new(|| {})
}

fn config(body: &str, bar_count: usize) -> DebugConfig {
    let bars = bench_tests::bars(bar_count, bench_tests::MINUTE_MS);
    let now = bars.last().map_or(0, |bar| bar.close_time + 1);
    DebugConfig::new(with_header(body), SymbolInfo::default(), bars, now)
}

fn start(config: DebugConfig) -> DebugSession {
    DebugSession::start(config, wake()).expect("starts")
}

/// The next event matching `pick`, discarding the ones before it.
fn next<T>(session: &mut DebugSession, mut pick: impl FnMut(DebugEvent) -> Option<T>) -> T {
    let deadline = Instant::now() + TIMEOUT;
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        if let Some(event) = session.wait(left.min(Duration::from_millis(200)))
            && let Some(found) = pick(event)
        {
            return found;
        }
    }
    panic!("timed out; the session is {:?}", session.state());
}

fn paused(session: &mut DebugSession) -> Box<PausedSnapshot> {
    next(session, |event| match event {
        DebugEvent::Paused(snapshot) => Some(snapshot),
        DebugEvent::Finished { outcome, .. } => panic!("finished instead of pausing: {outcome:?}"),
        _ => None,
    })
}

fn finished(session: &mut DebugSession) -> Outcome {
    next(session, |event| match event {
        DebugEvent::Finished { outcome, .. } => Some(outcome),
        _ => None,
    })
}

fn gone(alive: &std::sync::Weak<()>) -> bool {
    let deadline = Instant::now() + TIMEOUT;
    while Instant::now() < deadline {
        if alive.strong_count() == 0 {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

/// Line 3 is the first line of the body.
const SCRIPT: &str = "calculate(x) =>
    doubled = x * 2
    doubled + 10

value = calculate(close)
plot(value)
";

fn stopping_at(line: usize, bars: usize) -> DebugConfig {
    let mut config = config(SCRIPT, bars);
    config.breakpoints = vec![BreakpointSpec::line(1, line)];
    config
}

#[test]
fn start_runs_to_a_breakpoint_and_describes_where_it_stopped() {
    let mut session = start(stopping_at(5, 4));
    let stop = paused(&mut session);
    assert_eq!(session.state(), DebugState::Paused);
    assert_eq!(stop.stop.reason, StopReason::Breakpoint(1));
    assert_eq!(stop.stop.location.map(|l| l.line), Some(5));
    assert_eq!(
        stop.stack
            .iter()
            .map(|f| f.label.as_str())
            .collect::<Vec<_>>(),
        ["calculate()", "Main Script"]
    );
    assert_eq!(stop.bar.bar_index, 0);
    assert!(stop.bar.historical && stop.bar.confirmed);
    assert!(stop.variables.iter().any(|g| g.group == Group::Local));
    assert_eq!(stop.hits, vec![(1, 1)]);
    assert!(session.is_paused());
}

#[test]
fn continue_runs_on_to_the_next_stop_and_then_to_the_end() {
    let mut session = start(stopping_at(5, 3));
    for bar in 0..3 {
        let stop = paused(&mut session);
        assert_eq!(stop.stop.bar_index, bar);
        assert!(session.command(DebugCommand::Continue).is_ok());
        assert_eq!(session.state(), DebugState::Running);
    }
    assert_eq!(finished(&mut session), Outcome::Completed);
    assert_eq!(session.state(), DebugState::Completed);
    assert!(!session.is_paused());
}

#[test]
fn commands_are_accepted_only_where_the_state_allows_them() {
    let mut session = start(stopping_at(5, 3));
    let _ = paused(&mut session);
    assert!(session.allows(DebugCommand::StepOver));
    assert!(!session.allows(DebugCommand::Pause));
    assert!(session.command(DebugCommand::StepOver).is_ok());
    // The script is running again: a second step before it stops is refused, not queued.
    assert_eq!(
        session.command(DebugCommand::StepOver),
        Err(Rejected::NotNow)
    );
    assert_eq!(
        session.command(DebugCommand::Continue),
        Err(Rejected::NotNow)
    );
    assert_eq!(
        session.command(DebugCommand::Restart),
        Err(Rejected::NotNow)
    );
    let _ = paused(&mut session);
    assert!(session.command(DebugCommand::Stop).is_ok());
    assert_eq!(finished(&mut session), Outcome::Stopped);
}

#[test]
fn step_commands_walk_the_script_through_the_thread() {
    let mut session = start(stopping_at(7, 1));
    let mut seen = vec![paused(&mut session).stop.location.unwrap().line];
    for command in [
        DebugCommand::StepInto,
        DebugCommand::StepOver,
        DebugCommand::StepOver,
    ] {
        session.command(command).unwrap();
        seen.push(paused(&mut session).stop.location.unwrap().line);
    }
    // 7 is the call; Into enters at 4; Over moves to 5; Over from the last body line goes on to the
    // caller's next statement.
    assert_eq!(seen, [7, 4, 5, 8]);
    session.command(DebugCommand::Stop).unwrap();
    assert_eq!(finished(&mut session), Outcome::Stopped);
}

#[test]
fn step_bar_advances_one_bar_at_a_time() {
    let mut config = config(SCRIPT, 5);
    config.stop_on_entry = true;
    let mut session = start(config);
    let mut bars = vec![paused(&mut session).stop.bar_index];
    for _ in 0..3 {
        session.command(DebugCommand::StepBar).unwrap();
        bars.push(paused(&mut session).stop.bar_index);
    }
    assert_eq!(bars, [0, 1, 2, 3]);
}

#[test]
fn a_running_script_pauses_when_asked_and_can_then_be_stopped() {
    let mut config = config(
        bench_tests::SIMPLE.trim_start_matches("//@version=6\nindicator(\"Simple\")\n"),
        60_000,
    );
    config.stop_on_entry = false;
    let mut session = start(config);
    next(&mut session, |event| {
        matches!(event, DebugEvent::Started).then_some(())
    });
    session.command(DebugCommand::Pause).unwrap();
    assert_eq!(session.state(), DebugState::Pausing);
    let stop = paused(&mut session);
    assert_eq!(stop.stop.reason, StopReason::Pause);
    assert_eq!(session.state(), DebugState::Paused);
    session.command(DebugCommand::Stop).unwrap();
    assert_eq!(finished(&mut session), Outcome::Stopped);
}

#[test]
fn stop_ends_a_script_that_is_running_flat_out() {
    let mut config = config("i = 0\nwhile true\n    i += 1\n", 1);
    config.limits.deadline_seconds = 120.0;
    config.limits.instructions_per_bar = usize::MAX / 2;
    let mut session = start(config);
    next(&mut session, |event| {
        matches!(event, DebugEvent::Started).then_some(())
    });
    std::thread::sleep(Duration::from_millis(100));
    session.command(DebugCommand::Stop).unwrap();
    assert_eq!(finished(&mut session), Outcome::Stopped);
    assert_eq!(session.state(), DebugState::Completed);
}

#[test]
fn a_runaway_loop_can_be_paused_inside_the_loop() {
    let mut config = config("i = 0\nwhile true\n    i += 1\n", 1);
    config.limits.deadline_seconds = 120.0;
    config.limits.instructions_per_bar = usize::MAX / 2;
    let mut session = start(config);
    next(&mut session, |event| {
        matches!(event, DebugEvent::Started).then_some(())
    });
    session.command(DebugCommand::Pause).unwrap();
    let stop = paused(&mut session);
    assert!(matches!(stop.stop.location.map(|l| l.line), Some(3..=5)));
    session.command(DebugCommand::Stop).unwrap();
    assert_eq!(finished(&mut session), Outcome::Stopped);
}

#[test]
fn dropping_a_stopped_session_ends_its_thread() {
    let mut session = start(stopping_at(5, 3));
    let _ = paused(&mut session);
    let alive = session.liveness();
    assert!(session.thread_alive());
    drop(session);
    assert!(gone(&alive), "the thread outlived its session");
}

#[test]
fn dropping_a_running_session_ends_its_thread() {
    let mut config = config("i = 0\nwhile true\n    i += 1\n", 1);
    config.limits.deadline_seconds = 120.0;
    config.limits.instructions_per_bar = usize::MAX / 2;
    let mut session = start(config);
    next(&mut session, |event| {
        matches!(event, DebugEvent::Started).then_some(())
    });
    let alive = session.liveness();
    drop(session);
    assert!(gone(&alive));
}

#[test]
fn a_restart_is_a_new_session_and_leaves_none_behind() {
    let mut first = start(stopping_at(5, 3));
    let _ = paused(&mut first);
    let alive = first.liveness();
    drop(first);
    let mut second = start(stopping_at(5, 3));
    let stop = paused(&mut second);
    assert_eq!(
        stop.stop.bar_index, 0,
        "the new session starts from the beginning"
    );
    assert_eq!(stop.hits, vec![(1, 1)], "and counts hits from zero");
    assert!(gone(&alive));
}

#[test]
fn two_sessions_are_independent() {
    let mut a = start(stopping_at(5, 3));
    let mut b = start(stopping_at(4, 3));
    let first_a = paused(&mut a);
    let first_b = paused(&mut b);
    assert_eq!(first_a.stop.location.map(|l| l.line), Some(5));
    assert_eq!(first_b.stop.location.map(|l| l.line), Some(4));
    // Letting one go on does not move the other.
    a.command(DebugCommand::Continue).unwrap();
    let second_a = paused(&mut a);
    assert_eq!(second_a.stop.bar_index, 1);
    assert!(b.poll().is_empty());
    assert_eq!(b.state(), DebugState::Paused);
    b.command(DebugCommand::Stop).unwrap();
    a.command(DebugCommand::Stop).unwrap();
    assert_eq!(finished(&mut a), Outcome::Stopped);
    assert_eq!(finished(&mut b), Outcome::Stopped);
}

#[test]
fn rapid_commands_cannot_skip_a_stop() {
    let mut config = config(SCRIPT, 3);
    config.stop_on_entry = true;
    let mut session = start(config);
    let mut stops = 0;
    loop {
        match session.wait(TIMEOUT).expect("an event") {
            DebugEvent::Paused(_) => {
                stops += 1;
                let accepted = (0..200)
                    .filter(|_| session.command(DebugCommand::StepInto).is_ok())
                    .count();
                assert_eq!(accepted, 1, "one command per stop");
            }
            DebugEvent::Finished { outcome, .. } => {
                assert_eq!(outcome, Outcome::Completed);
                break;
            }
            _ => {}
        }
    }
    // Four statements run per bar (the call, two in the function, the plot): none was skipped.
    assert_eq!(stops, 3 * 4);
}

#[test]
fn the_thread_answers_inspection_requests_while_stopped() {
    let body = "var arr = array.new_float(0)\narray.push(arr, close)\narray.push(arr, open)\nplot(array.size(arr))\n";
    let mut config = config(body, 2);
    config.breakpoints = vec![BreakpointSpec::line(1, 6)];
    let mut session = start(config);
    let stop = paused(&mut session);
    let arr = stop
        .variables
        .iter()
        .flat_map(|g| &g.variables)
        .find(|v| v.name == "arr")
        .unwrap();
    let token = session
        .request_expand(arr.expand.clone().unwrap(), 0, 10)
        .unwrap();
    let page = next(&mut session, |event| match event {
        DebugEvent::Expanded { token: t, result } if t == token => Some(result),
        _ => None,
    })
    .unwrap();
    assert_eq!(page.total, 2);

    let token = session
        .request_evaluate(stop.frame, "bar_index + 1".to_owned())
        .unwrap();
    let answer = next(&mut session, |event| match event {
        DebugEvent::Evaluated { token: t, result } if t == token => Some(result),
        _ => None,
    });
    assert!(
        matches!(answer, Watched::Value(ref v) if v.value == "1"),
        "{answer:?}"
    );

    let token = session
        .request_evaluate(stop.frame, "ta.sma(close, 2)".to_owned())
        .unwrap();
    let refused = next(&mut session, |event| match event {
        DebugEvent::Evaluated { token: t, result } if t == token => Some(result),
        _ => None,
    });
    assert!(
        matches!(refused, Watched::Error(ref m) if m.contains("keeps state")),
        "{refused:?}"
    );

    let token = session.request_variables(stop.frame).unwrap();
    let groups = next(&mut session, |event| match event {
        DebugEvent::Variables {
            token: t, groups, ..
        } if t == token => Some(groups),
        _ => None,
    });
    assert!(!groups.is_empty());
    session.command(DebugCommand::Stop).unwrap();
}

#[test]
fn requests_are_refused_when_the_script_is_not_stopped() {
    let mut session = start(stopping_at(5, 1));
    let stop = paused(&mut session);
    session.command(DebugCommand::Continue).unwrap();
    assert!(session.request_variables(stop.frame).is_none());
    assert!(
        session
            .request_evaluate(stop.frame, "close".to_owned())
            .is_none()
    );
    assert_eq!(finished(&mut session), Outcome::Completed);
}

#[test]
fn watches_are_evaluated_at_every_stop_and_can_change_while_stopped() {
    let mut config = config(SCRIPT, 3);
    config.breakpoints = vec![BreakpointSpec::line(1, 5)];
    config.watches = vec![
        "bar_index + 1".to_owned(),
        "bar_index * 10".to_owned(),
        "nope(".to_owned(),
    ];
    let mut session = start(config);
    let first = paused(&mut session);
    assert_eq!(first.watches.len(), 3);
    assert!(matches!(&first.watches[0].result, Watched::Value(v) if v.value == "1"));
    assert!(matches!(&first.watches[1].result, Watched::Value(v) if v.value == "0"));
    assert!(
        matches!(&first.watches[2].result, Watched::Error(_)),
        "a bad watch reports itself"
    );
    session.set_watches(vec!["bar_index + 100".to_owned()]);
    session.command(DebugCommand::Continue).unwrap();
    let second = paused(&mut session);
    assert_eq!(second.watches.len(), 1);
    assert!(matches!(&second.watches[0].result, Watched::Value(v) if v.value == "101"));
    session.command(DebugCommand::Stop).unwrap();
}

#[test]
fn logpoints_and_script_logs_reach_the_reader_in_order_without_gaps() {
    let body = "log.info(\"script says {0}\", bar_index)\nvalue = close\nplot(value)\n";
    let mut config = config(body, 4);
    let mut spec = BreakpointSpec::line(1, 4);
    spec.log_message = Some("logpoint {bar_index}".to_owned());
    config.breakpoints = vec![spec, BreakpointSpec::line(2, 5)];
    let mut session = start(config);
    let mut history = LogHistory::default();
    let mut stops = 0;
    loop {
        let event = session.wait(TIMEOUT).expect("an event");
        match event {
            DebugEvent::Logs(batch) => history.apply(batch),
            DebugEvent::Paused(_) => {
                stops += 1;
                session.command(DebugCommand::Continue).unwrap();
            }
            DebugEvent::Finished { outcome, .. } => {
                assert_eq!(outcome, Outcome::Completed);
                break;
            }
            _ => {}
        }
    }
    assert_eq!(stops, 4);
    assert_eq!(history.dropped(), 0);
    let messages: Vec<String> = history
        .buffer()
        .iter()
        .map(|event| event.message.clone())
        .collect();
    let expected: Vec<String> = (0..4)
        .flat_map(|bar| [format!("script says {bar}"), format!("logpoint {bar}")])
        .collect();
    assert_eq!(messages, expected);
}

#[test]
fn a_runtime_error_pauses_for_inspection_and_cannot_be_continued() {
    let body = "arr = array.new_float(1)\nx = array.get(arr, 5)\nplot(x)\n";
    let mut session = start(config(body, 3));
    let stop = paused(&mut session);
    assert!(matches!(stop.stop.reason, StopReason::Exception(_)));
    assert_eq!(stop.stop.location.map(|l| l.line), Some(4));
    assert!(session.paused_on_error());
    assert!(!session.allows(DebugCommand::Continue));
    assert!(!session.allows(DebugCommand::StepOver));
    assert_eq!(
        session.command(DebugCommand::Continue),
        Err(Rejected::NotNow)
    );
    // The state can still be inspected.
    let token = session
        .request_evaluate(stop.frame, "bar_index".to_owned())
        .unwrap();
    let answer = next(&mut session, |event| match event {
        DebugEvent::Evaluated { token: t, result } if t == token => Some(result),
        _ => None,
    });
    assert!(matches!(answer, Watched::Value(_)));
    session.command(DebugCommand::Stop).unwrap();
    assert!(
        matches!(finished(&mut session), Outcome::Failed(error) if error.category == crate::pine::DiagnosticCategory::Runtime)
    );
}

#[test]
fn a_stop_on_the_open_last_bar_runs_it_as_a_realtime_tick() {
    let mut config = config(SCRIPT, 4);
    // Now falls inside the last bar: it has not closed.
    config.now_ms = config.bars[3].open_time + 10_000;
    config.breakpoints = vec![BreakpointSpec::line(1, 5)];
    let mut session = start(config);
    let mut flags = Vec::new();
    loop {
        let stop = paused(&mut session);
        flags.push((stop.bar.bar_index, stop.bar.historical, stop.bar.confirmed));
        if stop.bar.bar_index == 3 {
            break;
        }
        session.command(DebugCommand::Continue).unwrap();
    }
    assert_eq!(
        flags,
        vec![
            (0, true, true),
            (1, true, true),
            (2, true, true),
            (3, false, false)
        ]
    );
    session.command(DebugCommand::Stop).unwrap();
}

#[test]
fn starting_refuses_a_script_with_errors_or_no_bars_with_the_reason() {
    let mut empty = config(SCRIPT, 1);
    empty.bars.clear();
    assert!(matches!(
        DebugSession::start(empty, wake()),
        Err(StartError::NoBars)
    ));
    let broken = DebugConfig::new(
        "//@version=6\nindicator(\"T\")\nx = (\n",
        SymbolInfo::default(),
        bench_tests::bars(2, bench_tests::MINUTE_MS),
        0,
    );
    match DebugSession::start(broken, wake()) {
        Err(StartError::Compile(error)) => assert!(!error.message.is_empty()),
        other => panic!("expected a compile error, got {:?}", other.map(|_| ())),
    }
}

#[test]
fn a_session_knows_which_source_it_was_started_from() {
    let config = config(SCRIPT, 2);
    let source = config.source.clone();
    let session = start(config);
    assert!(session.is_for_source(&source));
    assert!(!session.is_for_source(&format!("{source}// edited\n")));
}

#[test]
fn breakpoints_and_bar_breaks_can_be_changed_through_the_session() {
    let mut config = config(SCRIPT, 8);
    config.stop_on_entry = true;
    let mut session = start(config);
    let _ = paused(&mut session);
    session.set_breakpoints(vec![BreakpointSpec::line(1, 5)]);
    session.set_bar_break(Some(BarBreak::Index(4)));
    session.command(DebugCommand::Continue).unwrap();
    let mut seen = Vec::new();
    for _ in 0..3 {
        let stop = paused(&mut session);
        seen.push((stop.stop.reason.clone(), stop.stop.bar_index));
        session.command(DebugCommand::Continue).unwrap();
    }
    assert_eq!(seen[0], (StopReason::Breakpoint(1), 0));
    assert_eq!(seen[1], (StopReason::Breakpoint(1), 1));
    assert_eq!(seen[2], (StopReason::Breakpoint(1), 2));
    session.command(DebugCommand::Stop).ok();
}

#[test]
fn a_shutdown_while_many_sessions_run_leaves_no_thread() {
    let mut sessions: Vec<DebugSession> = (0..6)
        .map(|index| {
            let mut config = config(SCRIPT, 50);
            config.stop_on_entry = index % 2 == 0;
            config.breakpoints = vec![BreakpointSpec::line(1, 5)];
            start(config)
        })
        .collect();
    let alive: Vec<_> = sessions.iter().map(|s| s.liveness()).collect();
    for session in &mut sessions {
        let _ = paused(session);
    }
    drop(sessions);
    for token in &alive {
        assert!(gone(token));
    }
}
