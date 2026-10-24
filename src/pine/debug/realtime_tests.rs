//! Realtime execution under a debugger: ticks re-run the script from the committed state, a close
//! commits, `var` and `varip` behave as they do without one, and stepping through a tick is possible.

use std::sync::{Arc, Mutex};

use crate::pine::bench_tests;
use crate::pine::compiler::NoLibraries;
use crate::pine::execution::ExecutionController;
use crate::pine::token::Limits;

use super::tests::{Probe, runtime_with, with_header};
use super::*;

const SCRIPT: &str = "var a = 0
varip b = 0
a += 1
b += 1
c = ta.sma(close, 3)
plot(a + b)
plot(c)
";

fn controller(source: &str) -> ExecutionController {
    ExecutionController::new(
        source,
        Default::default(),
        Default::default(),
        None,
        Limits::default(),
        &NoLibraries,
        None,
        None,
    )
    .expect("valid script")
}

/// A rebuild over history, then for each new bar: a tick, a changed tick and the close. Returns the
/// output after every update.
fn drive(mut controller: ExecutionController) -> Vec<String> {
    const MINUTE: i64 = bench_tests::MINUTE_MS;
    let all = bench_tests::bars(60, MINUTE);
    let (history, future) = all.split_at(40);
    let mut candles = history.to_vec();
    let mut outputs = vec![format!(
        "{:?}",
        controller
            .update(&candles, None, candles[39].close_time)
            .output
    )];
    for next in future {
        let open = next.open_time;
        let mut bar = next.clone();
        let mut snapshot = candles.clone();
        snapshot.push(bar.clone());
        for (now, price_factor) in [
            (open + 10_000, 1.0),
            (open + 20_000, 1.001),
            (open + MINUTE, 1.001),
        ] {
            bar.close = next.close * price_factor;
            *snapshot.last_mut().expect("live bar") = bar.clone();
            let update = controller.update(&snapshot, None, now);
            outputs.push(format!("{:?}", update.output));
        }
        candles.push(bar);
    }
    outputs
}

#[test]
fn realtime_ticks_roll_back_and_commit_exactly_as_without_a_debugger() {
    let source = with_header(SCRIPT);
    let plain = drive(controller(&source));

    let (runtime, seen) = runtime_with(
        &source,
        Vec::new(),
        false,
        &[],
        DebugControl::default(),
        None,
    );
    let idle = drive(controller(&source).with_debugger(runtime));
    assert_eq!(plain, idle);
    assert!(seen.lock().unwrap().is_empty());

    // Stopping at every statement of every execution, ticks and re-executions included, and
    // looking at everything at each, changes nothing either.
    let probe: Probe = Box::new(|_, view| {
        for frame in view.stack() {
            let _ = view.variables(frame.index);
            let expression = parse_expression("a + b").unwrap();
            let _ = view.watch(frame.index, &expression);
        }
    });
    let (runtime, seen) = runtime_with(
        &source,
        Vec::new(),
        true,
        &[Resume::Into; 1_000_000],
        DebugControl::default(),
        Some(probe),
    );
    let stepped = drive(controller(&source).with_debugger(runtime));
    assert_eq!(plain, stepped);
    assert!(seen.lock().unwrap().len() > 100);
}

#[test]
fn a_tick_that_is_stepped_through_is_marked_realtime_and_unconfirmed() {
    let source = with_header(SCRIPT);
    let flags: Arc<Mutex<Vec<(bool, bool)>>> = Arc::default();
    let sink = Arc::clone(&flags);
    let probe: Probe = Box::new(move |_, view| {
        let bar = view.bar();
        sink.lock().unwrap().push((bar.historical, bar.confirmed));
    });
    let mut spec = BreakpointSpec::line(1, 5);
    spec.condition = Some("bar_index >= 40".to_owned());
    let (runtime, _) = runtime_with(
        &source,
        vec![spec],
        false,
        &[],
        DebugControl::default(),
        Some(probe),
    );
    drive(controller(&source).with_debugger(runtime));
    let flags = flags.lock().unwrap();
    // Per new bar the first tick and the changed tick are live and open; the close confirms.
    assert!(
        flags.contains(&(false, false)),
        "ticks are unconfirmed: {flags:?}"
    );
    assert!(
        flags.contains(&(false, true)),
        "the close confirms: {flags:?}"
    );
}

#[test]
fn varip_survives_a_tick_the_debugger_stopped_in_and_var_rolls_back() {
    let source = with_header(SCRIPT);
    let seen: Arc<Mutex<Vec<(i64, String, String)>>> = Arc::default();
    let sink = Arc::clone(&seen);
    let probe: Probe = Box::new(move |stop, view| {
        let frame = view.stack()[0].index;
        let read = |view: &mut StopView<'_>, text: &str| match view
            .watch(frame, &parse_expression(text).unwrap())
        {
            Watched::Value(value) => value.value,
            Watched::Error(error) => error,
        };
        let (a, b) = (read(view, "a"), read(view, "b"));
        sink.lock().unwrap().push((stop.bar_index, a, b));
    });
    // Line 8 is `plot(a + b)`: both counters were incremented by then.
    let mut spec = BreakpointSpec::line(1, 8);
    spec.condition = Some("bar_index >= 40".to_owned());
    let (runtime, _) = runtime_with(
        &source,
        vec![spec],
        false,
        &[],
        DebugControl::default(),
        Some(probe),
    );
    drive(controller(&source).with_debugger(runtime));
    let seen = seen.lock().unwrap();
    // Bar 40 runs three times (tick, changed tick, close); `a` is rolled back to the committed
    // value each time, so it reads the same, while `varip` keeps counting across the ticks.
    let bar_40: Vec<&(i64, String, String)> = seen.iter().filter(|s| s.0 == 40).collect();
    assert_eq!(bar_40.len(), 3, "{seen:?}");
    assert!(
        bar_40.iter().all(|s| s.1 == bar_40[0].1),
        "a is rolled back: {bar_40:?}"
    );
    let counts: Vec<i64> = bar_40.iter().map(|s| s.2.parse().unwrap()).collect();
    assert!(
        counts[0] < counts[1] && counts[1] < counts[2],
        "b keeps counting: {counts:?}"
    );
}
