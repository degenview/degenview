//! `log.*()` end to end: compiled scripts run over synthetic bars, and the messages read back as a
//! chart would read them, through [`ExecutionController`] and [`LogHistory`].

use std::collections::HashMap;

use crate::model::Kline;
use crate::pine::compiler::NoLibraries;
use crate::pine::execution::{ExecutionController, ExecutionUpdate};
use crate::pine::logs::{HISTORICAL_LIMIT, LogEvent, LogHistory, LogLevel};
use crate::pine::program::SymbolInfo;
use crate::pine::token::Limits;

const MINUTE: i64 = 60_000;

fn bar(index: i64) -> Kline {
    let price = 100.0 + index as f64;
    Kline {
        open_time: index * MINUTE,
        close_time: (index + 1) * MINUTE,
        open: price,
        high: price + 2.0,
        low: price - 2.0,
        close: price + 1.0,
        volume: 10.0,
    }
}

fn bars(count: i64) -> Vec<Kline> {
    (0..count).map(bar).collect()
}

/// A moment after every bar of `bars(count)` has closed.
fn after(count: i64) -> i64 {
    count * MINUTE + 1
}

fn script(body: &str) -> String {
    format!("//@version=6\nindicator(\"T\")\n{body}\n")
}

fn controller(source: &str) -> ExecutionController {
    ExecutionController::new(
        source,
        HashMap::new(),
        SymbolInfo::default(),
        None,
        Limits::default(),
        &NoLibraries,
        None,
        None,
    )
    .unwrap_or_else(|error| panic!("{error:?}"))
    .with_logs()
}

/// The messages `body` writes over `count` closed bars.
fn history_of(body: &str, count: i64) -> Vec<LogEvent> {
    let mut controller = controller(&script(body));
    let update = controller.update(&bars(count), None, after(count));
    assert!(update.error.is_none(), "{:?}", update.error);
    let mut history = LogHistory::default();
    history.apply(update.logs);
    history.buffer().iter().cloned().collect()
}

fn messages(events: &[LogEvent]) -> Vec<&str> {
    events.iter().map(|event| event.message.as_str()).collect()
}

#[test]
fn only_the_bars_where_the_condition_holds_write_a_message() {
    let events = history_of(
        "if bar_index % 2 == 0\n    log.info(\"even {0}\", bar_index)\n\
         else\n    log.warning(\"odd {0}\", bar_index)",
        4,
    );
    assert_eq!(messages(&events), ["even 0", "odd 1", "even 2", "odd 3"]);
    assert_eq!(events[0].level, LogLevel::Info);
    assert_eq!(events[1].level, LogLevel::Warning);
}

#[test]
fn every_call_writes_its_own_message_in_execution_order() {
    let events = history_of(
        "log.info(\"first\")\nfor i = 0 to 2\n    log.error(\"loop {0}\", i)\nlog.info(\"last\")",
        2,
    );
    assert_eq!(
        messages(&events),
        [
            "first", "loop 0", "loop 1", "loop 2", "last", "first", "loop 0", "loop 1", "loop 2",
            "last"
        ]
    );
    assert!(events.iter().all(|event| !event.realtime));
    assert_eq!(events[0].bar_index, Some(0));
    assert_eq!(events[5].bar_index, Some(1));
    assert!(
        events
            .windows(2)
            .all(|pair| pair[0].sequence < pair[1].sequence)
    );
}

#[test]
fn a_historical_message_carries_its_bars_open_time() {
    let events = history_of("log.info(\"x\")", 3);
    let times: Vec<i64> = events.iter().map(|event| event.time_ms).collect();
    assert_eq!(times, [0, MINUTE, 2 * MINUTE]);
    assert_eq!(
        times,
        events
            .iter()
            .map(|event| event.bar_time_ms)
            .collect::<Vec<_>>()
    );
}

#[test]
fn a_call_in_a_function_or_block_points_at_its_own_line() {
    let events = history_of(
        "f(x) =>\n    log.warning(\"in f {0}\", x)\n    x\nf(1.0)\nif true\n    log.error(\"in if\")",
        1,
    );
    // The version and declaration lines come first; the body starts on line 3.
    assert_eq!(events[0].range.start.line, 4);
    assert_eq!(events[0].range.start.column, 5);
    assert_eq!(events[1].range.start.line, 8);
    assert_eq!(events[1].range.start.column, 5);
}

#[test]
fn the_formatted_overload_follows_str_format() {
    let events = history_of(
        "log.info(\"{0} / {1,number,#.##} / {2} / {3}\", 7, 3.14159, true, na)\n\
         log.info(\"line one\\nline two é✓\")\n\
         log.info(\"{9} stays\")",
        1,
    );
    assert_eq!(events[0].message, "7 / 3.14 / true / NaN");
    assert_eq!(events[1].message, "line one\nline two é✓");
    assert_eq!(events[2].message, "{9} stays");
}

#[test]
fn a_call_writes_nothing_unless_logs_were_asked_for() {
    let mut controller = ExecutionController::new(
        &script("log.info(\"quiet\")"),
        HashMap::new(),
        SymbolInfo::default(),
        None,
        Limits::default(),
        &NoLibraries,
        None,
        None,
    )
    .unwrap();
    let update = controller.update(&bars(3), None, after(3));
    assert!(update.error.is_none());
    assert!(update.logs.events.is_empty());
}

#[test]
fn a_log_call_never_stops_the_script_or_changes_its_value() {
    let mut controller = controller(&script(
        "log.error(\"bad\")\nlog.warning(\"hmm\")\nplot(close)",
    ));
    let update = controller.update(&bars(3), None, after(3));
    assert!(update.error.is_none());
    assert_eq!(update.logs.events.len(), 6);
    assert_eq!(update.output.plots.len(), 1);
}

/// The history after applying each update in turn.
fn apply(history: &mut LogHistory, update: ExecutionUpdate) {
    assert!(update.error.is_none(), "{:?}", update.error);
    history.apply(update.logs);
}

const TICKING: &str = "var int ticks = 0\nticks += 1\nif barstate.isconfirmed\n    log.info(\"confirmed {0}\", ticks)\nelse\n    log.warning(\"tick {0}\", ticks)";

#[test]
fn realtime_messages_survive_the_rollback_that_resets_the_bars_variables() {
    let mut controller = controller(&script(TICKING));
    let mut history = LogHistory::default();
    let closed = bars(3);
    // Three closed bars, then a fourth that is still open.
    let mut with_live = closed.clone();
    let mut live = bar(3);
    live.close_time = 10 * MINUTE;
    with_live.push(live.clone());
    let now = after(3) + 1_000;
    apply(&mut history, controller.update(&with_live, None, now));
    assert_eq!(history.len(), 4);

    // The open bar ticks again: its variables roll back to the committed state, its messages stay.
    live.close = 150.0;
    *with_live.last_mut().unwrap() = live.clone();
    apply(
        &mut history,
        controller.update(&with_live, None, now + 1_000),
    );
    live.close = 151.0;
    *with_live.last_mut().unwrap() = live.clone();
    apply(
        &mut history,
        controller.update(&with_live, None, now + 2_000),
    );

    let events: Vec<LogEvent> = history.buffer().iter().cloned().collect();
    assert_eq!(
        messages(&events),
        [
            "confirmed 1",
            "confirmed 2",
            "confirmed 3",
            "tick 4",
            "tick 4",
            "tick 4"
        ]
    );
    let realtime: Vec<&LogEvent> = events.iter().filter(|event| event.realtime).collect();
    assert_eq!(realtime.len(), 3);
    assert!(realtime.iter().all(|event| !event.confirmed));
    // A realtime message is stamped with the clock at the tick, not the bar's open time.
    assert_eq!(
        realtime
            .iter()
            .map(|event| event.time_ms)
            .collect::<Vec<_>>(),
        [now, now + 1_000, now + 2_000]
    );
    assert!(realtime.iter().all(|event| event.bar_time_ms == 3 * MINUTE));
}

#[test]
fn closing_the_open_bar_writes_a_confirmed_realtime_message() {
    let mut controller = controller(&script(TICKING));
    let mut history = LogHistory::default();
    let mut all = bars(2);
    let mut live = bar(2);
    live.close_time = 5 * MINUTE;
    all.push(live.clone());
    apply(&mut history, controller.update(&all, None, after(2) + 1));
    live.close_time = 3 * MINUTE;
    *all.last_mut().unwrap() = live;
    apply(&mut history, controller.update(&all, None, 3 * MINUTE + 5));
    let last = history.buffer().iter().last().unwrap();
    assert_eq!(last.message, "confirmed 3");
    assert!(last.realtime && last.confirmed);
}

#[test]
fn a_rebuild_starts_the_history_over() {
    let mut controller = controller(&script("log.info(\"bar {0}\", bar_index)"));
    let mut history = LogHistory::default();
    apply(&mut history, controller.update(&bars(3), None, after(3)));
    assert_eq!(history.len(), 3);
    // Different history behind the same last bar: the run restarts.
    let mut other = bars(3);
    other[0].close += 5.0;
    let update = controller.update(&other, None, after(3));
    assert!(update.rebuilt && update.logs.reset);
    apply(&mut history, update);
    assert_eq!(history.len(), 3);
    assert!(!history.gap);
}

#[test]
fn an_unchanged_snapshot_writes_nothing_new() {
    let mut controller = controller(&script("log.info(\"x\")"));
    let mut history = LogHistory::default();
    apply(&mut history, controller.update(&bars(2), None, after(2)));
    let update = controller.update(&bars(2), None, after(2));
    assert!(!update.rebuilt && update.logs.events.is_empty() && !update.logs.reset);
    apply(&mut history, update);
    assert_eq!(history.len(), 2);
}

#[test]
fn a_bar_that_arrives_closed_after_the_history_is_appended() {
    let mut controller = controller(&script("log.info(\"bar {0}\", bar_index)"));
    let mut history = LogHistory::default();
    apply(&mut history, controller.update(&bars(2), None, after(2)));
    apply(&mut history, controller.update(&bars(3), None, after(3)));
    let events: Vec<LogEvent> = history.buffer().iter().cloned().collect();
    assert_eq!(messages(&events), ["bar 0", "bar 1", "bar 2"]);
    assert!(!history.gap);
}

#[test]
fn a_dropped_update_is_noticed_instead_of_leaving_a_silent_hole() {
    let mut controller = controller(&script("log.info(\"bar {0}\", bar_index)"));
    let mut history = LogHistory::default();
    apply(&mut history, controller.update(&bars(2), None, after(2)));
    let _lost = controller.update(&bars(3), None, after(3));
    apply(&mut history, controller.update(&bars(4), None, after(4)));
    assert!(history.gap);
    assert_eq!(history.len(), 1);
}

#[test]
fn a_noisy_history_keeps_the_last_ten_thousand_messages() {
    let mut controller = controller(&script(
        "for i = 0 to 99\n    log.info(\"{0}:{1}\", bar_index, i)",
    ));
    let update = controller.update(&bars(120), None, after(120));
    assert!(update.error.is_none());
    let mut history = LogHistory::default();
    history.apply(update.logs);
    assert_eq!(history.len(), HISTORICAL_LIMIT);
    let last = history.buffer().iter().last().unwrap();
    assert_eq!(last.message, "119:99");
    assert!(history.dropped() >= 2_000);
}

#[test]
fn an_oversized_message_is_cut() {
    let events = history_of("log.info(str.repeat(\"ab\", 5000))", 1);
    assert_eq!(
        events[0].message.chars().count(),
        crate::pine::logs::MESSAGE_LIMIT
    );
    assert!(events[0].message.ends_with('…'));
}

fn strategy_script(options: &str) -> String {
    format!(
        "//@version=6\nstrategy(\"S\"{options})\nlog.info(\"ran {{0}}\", barstate.isconfirmed)\n"
    )
}

fn live_strategy_messages(options: &str) -> Vec<String> {
    let mut controller = controller(&strategy_script(options));
    let mut history = LogHistory::default();
    let mut all = bars(2);
    let mut live = bar(2);
    live.close_time = 10 * MINUTE;
    all.push(live.clone());
    apply(&mut history, controller.update(&all, None, after(2) + 1));
    live.close = 200.0;
    *all.last_mut().unwrap() = live;
    apply(&mut history, controller.update(&all, None, after(2) + 2));
    history
        .buffer()
        .iter()
        .filter(|event| event.realtime)
        .map(|event| event.message.clone())
        .collect()
}

#[test]
fn a_strategy_writes_realtime_messages_only_when_it_runs_on_ticks() {
    assert!(live_strategy_messages("").is_empty());
    assert_eq!(
        live_strategy_messages(", calc_on_every_tick = true"),
        ["ran false", "ran false"]
    );
}

struct Libraries(HashMap<String, String>);

impl crate::pine::compiler::LibraryResolver for Libraries {
    fn source_for_library(&self, path: &str) -> Option<String> {
        self.0.get(path).cloned()
    }
}

#[test]
fn an_exported_function_logs_for_the_script_that_calls_it() {
    let libraries = Libraries(HashMap::from([(
        "user/Dbg/1".to_owned(),
        "//@version=6\nlibrary(\"Dbg\")\nexport trace(float x) =>\n    log.info(\"lib {0}\", x)\n    x\n"
            .to_owned(),
    )]));
    let source = script("import user/Dbg/1 as dbg\ny = dbg.trace(close)\nlog.info(\"script\")");
    let mut controller = ExecutionController::new(
        &source,
        HashMap::new(),
        SymbolInfo::default(),
        None,
        Limits::default(),
        &libraries,
        None,
        None,
    )
    .unwrap_or_else(|error| panic!("{error:?}"))
    .with_logs();
    let update = controller.update(&bars(1), None, after(1));
    assert!(update.error.is_none(), "{:?}", update.error);
    let events = update.logs.events;
    assert_eq!(messages(&events), ["lib 101", "script"]);
    // The library's call is in the library's own source (line 4), told apart by its path.
    assert_eq!(events[0].library.as_deref(), Some("user/Dbg/1"));
    assert_eq!(events[0].range.start.line, 4);
    assert_eq!(events[1].library, None);
}

#[test]
fn requested_data_writes_each_bar_once_and_says_where_it_came_from() {
    let events = history_of(
        "f() =>\n    log.info(\"req {0}\", close)\n    close\n\
         x = request.security(syminfo.tickerid, \"5\", f())\nplot(x)",
        20,
    );
    assert!(!events.is_empty());
    assert!(events.iter().all(|event| event.request.is_some()));
    assert!(events.iter().all(|event| event.bar_index.is_none()));
    assert!(
        events
            .iter()
            .all(|event| event.request.as_ref().unwrap().interval_seconds == 300.0)
    );
    // Each requested bar writes its message once, not once for every chart bar it spans.
    let mut times: Vec<i64> = events.iter().map(|event| event.bar_time_ms).collect();
    let total = times.len();
    times.sort_unstable();
    times.dedup();
    assert_eq!(times.len(), total, "a requested bar logged twice");
    assert!(times.iter().all(|time| time % (5 * MINUTE) == 0));
}
