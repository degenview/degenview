//! Many sessions, started, paused, stopped and dropped at awkward moments: none may deadlock, panic
//! or leave a thread behind. Every wait has a timeout.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::pine::bench_tests;
use crate::pine::program::SymbolInfo;

use super::tests::with_header;
use super::*;

const TIMEOUT: Duration = Duration::from_secs(20);

fn wake() -> Arc<dyn Fn() + Send + Sync> {
    Arc::new(|| {})
}

const LOOP: &str = "i = 0\nwhile true\n    i += 1\n";
const WORK: &str = "calculate(x) =>
    doubled = x * 2
    doubled + 10
value = calculate(close)
plot(value)
";

fn config(body: &str, bars: usize) -> DebugConfig {
    let bars = bench_tests::bars(bars, bench_tests::MINUTE_MS);
    let now = bars.last().map_or(0, |bar| bar.close_time + 1);
    let mut config = DebugConfig::new(with_header(body), SymbolInfo::default(), bars, now);
    config.limits.deadline_seconds = 300.0;
    config.limits.instructions_per_bar = usize::MAX / 2;
    config
}

fn wait_for(session: &mut DebugSession, mut pick: impl FnMut(&DebugEvent) -> bool) -> bool {
    let deadline = Instant::now() + TIMEOUT;
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        if let Some(event) = session.wait(left.min(Duration::from_millis(100)))
            && pick(&event)
        {
            return true;
        }
    }
    false
}

fn ended(alive: &std::sync::Weak<()>) -> bool {
    let deadline = Instant::now() + TIMEOUT;
    while Instant::now() < deadline {
        if alive.strong_count() == 0 {
            return true;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    false
}

#[test]
fn starting_and_dropping_a_session_at_every_phase_leaves_no_thread() {
    let mut threads = Vec::new();
    for round in 0..40 {
        let mut config = config(WORK, 200);
        config.stop_on_entry = round % 2 == 0;
        config.breakpoints = vec![BreakpointSpec::line(1, 5)];
        let mut session = DebugSession::start(config, wake()).expect("starts");
        threads.push(session.liveness());
        match round % 4 {
            // Dropped before it has said anything.
            0 => {}
            // Dropped once it is running or stopped.
            1 => {
                assert!(wait_for(&mut session, |e| matches!(e, DebugEvent::Started)));
            }
            2 => {
                assert!(wait_for(&mut session, |e| matches!(
                    e,
                    DebugEvent::Paused(_)
                )));
            }
            _ => {
                assert!(wait_for(&mut session, |e| matches!(
                    e,
                    DebugEvent::Paused(_)
                )));
                session.command(DebugCommand::Continue).ok();
            }
        }
    }
    for alive in &threads {
        assert!(ended(alive), "a thread outlived its session");
    }
}

#[test]
fn pause_and_stop_racing_a_running_script_always_end_cleanly() {
    for round in 0..60 {
        let mut session = DebugSession::start(config(LOOP, 1), wake()).expect("starts");
        let alive = session.liveness();
        assert!(wait_for(&mut session, |e| matches!(e, DebugEvent::Started)));
        match round % 3 {
            // Pause, then stop at once, before the pause is answered.
            0 => {
                session.command(DebugCommand::Pause).unwrap();
                session.command(DebugCommand::Stop).unwrap();
            }
            // Pause, wait for the stop, then stop.
            1 => {
                session.command(DebugCommand::Pause).unwrap();
                assert!(wait_for(&mut session, |e| matches!(
                    e,
                    DebugEvent::Paused(_)
                )));
                session.command(DebugCommand::Stop).unwrap();
            }
            // Stop, then pause, which is refused.
            _ => {
                session.command(DebugCommand::Stop).unwrap();
                assert_eq!(session.command(DebugCommand::Pause), Err(Rejected::NotNow));
            }
        }
        assert!(
            wait_for(&mut session, |e| matches!(e, DebugEvent::Finished { .. })),
            "round {round} never finished"
        );
        drop(session);
        assert!(ended(&alive));
    }
}

#[test]
fn a_restart_while_stopped_replaces_the_session_without_leaking_the_first() {
    let mut alive = Vec::new();
    let mut session: Option<DebugSession> = None;
    for _ in 0..25 {
        let mut config = config(WORK, 100);
        config.breakpoints = vec![BreakpointSpec::line(1, 5)];
        let mut next = DebugSession::start(config, wake()).expect("starts");
        alive.push(next.liveness());
        assert!(wait_for(&mut next, |e| matches!(e, DebugEvent::Paused(_))));
        // The old session is dropped while stopped, as Restart does.
        session = Some(next);
    }
    drop(session);
    for token in &alive {
        assert!(ended(token));
    }
}

#[test]
fn many_sessions_at_once_each_answer_for_themselves() {
    let mut sessions: Vec<DebugSession> = (0..8)
        .map(|index| {
            let mut config = config(WORK, 60);
            config.breakpoints = vec![BreakpointSpec::line(1, 5)];
            config.stop_on_entry = index % 2 == 1;
            DebugSession::start(config, wake()).expect("starts")
        })
        .collect();
    for session in &mut sessions {
        assert!(wait_for(session, |e| matches!(e, DebugEvent::Paused(_))));
    }
    // Finish them in the opposite order they started.
    for session in sessions.iter_mut().rev() {
        session.command(DebugCommand::Stop).unwrap();
        assert!(wait_for(session, |e| matches!(
            e,
            DebugEvent::Finished { .. }
        )));
    }
}

#[test]
fn a_script_with_alerts_and_orders_runs_under_the_debugger_without_leaking_either() {
    let body = "strategy(\"S\", overlay = true)
if bar_index % 5 == 0
    strategy.entry(\"L\", strategy.long)
if bar_index % 5 == 3
    strategy.close(\"L\")
alert(\"tick\", alert.freq_once_per_bar)
plot(close)
";
    let bars = bench_tests::bars(60, bench_tests::MINUTE_MS);
    let now = bars.last().map_or(0, |bar| bar.close_time + 1);
    let source = format!("//@version=6\n{body}");
    let mut config = DebugConfig::new(source, SymbolInfo::default(), bars, now);
    config.stop_on_entry = true;
    let mut session = DebugSession::start(config, wake()).expect("starts");
    // Step through a few bars of it; the only things that come back are stops and logs.
    for _ in 0..30 {
        assert!(wait_for(&mut session, |e| matches!(
            e,
            DebugEvent::Paused(_)
        )));
        session.command(DebugCommand::StepInto).unwrap();
    }
    session.command(DebugCommand::Stop).ok();
    assert!(wait_for(&mut session, |e| matches!(
        e,
        DebugEvent::Finished { .. }
    )));
}

/// How long a stop takes to arrive after the request, and a resume to reach the next stop. Run by
/// hand: `cargo test --release --lib pine::debug::stress_tests::latency -- --ignored --nocapture`.
#[test]
#[ignore = "prints measurements"]
fn latency_of_pausing_and_resuming() {
    let mut pauses = Vec::new();
    let mut resumes = Vec::new();
    for _ in 0..50 {
        let mut session = DebugSession::start(config(LOOP, 1), wake()).expect("starts");
        assert!(wait_for(&mut session, |e| matches!(e, DebugEvent::Started)));
        std::thread::sleep(Duration::from_millis(20));
        let asked = Instant::now();
        session.command(DebugCommand::Pause).unwrap();
        assert!(wait_for(&mut session, |e| matches!(
            e,
            DebugEvent::Paused(_)
        )));
        pauses.push(asked.elapsed());
        let asked = Instant::now();
        session.command(DebugCommand::StepInto).unwrap();
        assert!(wait_for(&mut session, |e| matches!(
            e,
            DebugEvent::Paused(_)
        )));
        resumes.push(asked.elapsed());
        session.command(DebugCommand::Stop).unwrap();
    }
    for (name, mut times) in [("pause", pauses), ("step", resumes)] {
        times.sort();
        println!(
            "{name} latency: median {:?}, p90 {:?}, max {:?}",
            times[times.len() / 2],
            times[times.len() * 9 / 10],
            times[times.len() - 1]
        );
    }
}
