//! `varip` on a realtime bar: what survives the ticks that roll back, per call site, in an object's
//! field and behind a reference. Driven through `ExecutionController` like a chart does: a history,
//! then a live bar updated twice and closed.

use std::collections::HashMap;

use super::bench_tests::{bars, limits, symbol};
use super::compiler::NoLibraries;
use super::execution::ExecutionController;
use crate::model::Kline;

const HISTORY: usize = 3;

/// The last value of every plot after the history and after each of the live bar's three
/// executions: its first tick, a changed tick and its close.
fn live_bar(source: &str) -> Vec<Vec<f64>> {
    let source = format!("//@version=6\nindicator(\"T\")\n{source}");
    let mut candles: Vec<Kline> = bars(HISTORY + 1, 60_000);
    let live = candles.pop().expect("a live bar");
    let mut controller = ExecutionController::new(
        &source,
        HashMap::new(),
        symbol(),
        None,
        limits(),
        &NoLibraries,
        None,
        None,
    )
    .expect("valid script");
    let last = |update: &super::execution::ExecutionUpdate| -> Vec<f64> {
        assert!(update.error.is_none(), "{:?}", update.error);
        update
            .output
            .plots
            .iter()
            .map(|plot| plot.values.last().copied().flatten().expect("a value"))
            .collect()
    };
    let mut seen = vec![last(&controller.update(
        &candles,
        None,
        candles[HISTORY - 1].close_time,
    ))];
    let mut snapshot = candles.clone();
    snapshot.push(live.clone());
    seen.push(last(&controller.update(
        &snapshot,
        None,
        live.open_time + 10_000,
    )));
    let mut changed = live.clone();
    changed.close *= 1.001;
    *snapshot.last_mut().expect("live bar") = changed;
    seen.push(last(&controller.update(
        &snapshot,
        None,
        live.open_time + 20_000,
    )));
    seen.push(last(&controller.update(&snapshot, None, live.close_time)));
    seen
}

#[test]
fn a_varip_local_in_a_function_counts_per_call_site_through_the_ticks() {
    let seen = live_bar(
        "count() =>\n    varip int n = 0\n    n += 1\n    n\n\
         plot(count())\n\
         plot(count() * 10)",
    );
    // Three bars, then the live bar's three executions: each site keeps its own count, and the
    // count is not rolled back between ticks.
    assert_eq!(
        seen,
        [
            vec![3.0, 30.0],
            vec![4.0, 40.0],
            vec![5.0, 50.0],
            vec![6.0, 60.0]
        ]
    );
}

#[test]
fn a_global_varip_and_a_function_local_of_the_same_name_stay_apart() {
    let seen = live_bar(
        "varip int n = 0\n\
         n += 1\n\
         f() =>\n    int n = 100\n    n += 1\n    n\n\
         plot(n)\n\
         plot(f())",
    );
    assert_eq!(
        seen,
        [
            vec![3.0, 101.0],
            vec![4.0, 101.0],
            vec![5.0, 101.0],
            vec![6.0, 101.0]
        ]
    );
}

#[test]
fn a_varip_field_keeps_its_value_through_the_ticks_and_the_other_fields_roll_back() {
    let seen = live_bar(
        "type Counter\n    varip int hits = 0\n    int per_bar = 0\n\
         var Counter c = Counter.new()\n\
         c.hits += 1\n\
         c.per_bar += 1\n\
         plot(c.hits)\n\
         plot(c.per_bar)",
    );
    assert_eq!(
        seen,
        [
            vec![3.0, 3.0],
            // `hits` goes on counting; `per_bar` restarts from the committed 3 on every tick.
            vec![4.0, 4.0],
            vec![5.0, 4.0],
            vec![6.0, 4.0]
        ]
    );
}

#[test]
fn a_varip_array_keeps_what_was_pushed_on_earlier_ticks() {
    let seen = live_bar(
        "varip array<int> a = array.new<int>()\n\
         array.push(a, bar_index)\n\
         plot(array.size(a))",
    );
    assert_eq!(seen, [vec![3.0], vec![4.0], vec![5.0], vec![6.0]]);
}

#[test]
fn an_array_that_is_not_varip_rolls_back_on_every_tick() {
    let seen = live_bar(
        "var array<int> a = array.new<int>()\n\
         array.push(a, bar_index)\n\
         plot(array.size(a))",
    );
    assert_eq!(seen, [vec![3.0], vec![4.0], vec![4.0], vec![4.0]]);
}

#[test]
fn an_array_made_on_a_tick_that_rolls_back_does_not_alias_a_later_one() {
    // Each tick makes a plain array too; the varip one must keep its own contents and not pick up
    // the plain array's handle.
    let seen = live_bar(
        "varip array<int> kept = array.new<int>()\n\
         array<int> scratch = array.new<int>()\n\
         array.push(scratch, 7)\n\
         array.push(scratch, 8)\n\
         array.push(kept, 1)\n\
         plot(array.size(kept))\n\
         plot(array.size(scratch))",
    );
    assert_eq!(
        seen,
        [
            vec![3.0, 2.0],
            vec![4.0, 2.0],
            vec![5.0, 2.0],
            vec![6.0, 2.0]
        ]
    );
}
