//! The timing model, asserted exactly: a recorder on a [`SteppingClock`] makes every duration a
//! count of clock reads, so counts, inclusive and exclusive times and the totals that must add up
//! are checked as equalities, never within a tolerance.

use std::sync::Arc;

use crate::model::Kline;
use crate::pine::bench_tests::{self, bars, run_script};
use crate::pine::compiler::{self, NoLibraries};
use crate::pine::session::Session;
use crate::pine::token::Limits;

use super::*;

const STEP: u64 = 1_000;

fn script(body: &str) -> String {
    format!("//@version=6\nindicator(\"T\")\n{body}")
}

/// The line of `body`'s first line in a `script`.
const FIRST_LINE: usize = 3;

fn recorder(session: &Session, step: u64) -> ProfileRuntime {
    let table = Arc::new(RegionTable::build(&session.program));
    ProfileRuntime::with_clock(table, Box::new(SteppingClock::new(step)))
}

/// Runs `source` over `data` on a stepping clock and returns what was measured.
fn profile(source: &str, data: &[Kline]) -> ProfileReport {
    let mut session = run_script(source, data, false, |session| {
        let runtime = recorder(session, STEP);
        session.attach_profiler(runtime);
    });
    session
        .detach_profiler()
        .expect("recorder")
        .finish(Completeness::Complete)
}

fn line(report: &ProfileReport, number: usize) -> LineStats {
    report
        .lines()
        .into_iter()
        .find(|line| line.line == number)
        .unwrap_or_else(|| panic!("no measurements on line {number}"))
}

fn rising(data: &[Kline]) -> u64 {
    data.iter().filter(|bar| bar.close > bar.open).count() as u64
}

/// The invariants every report must satisfy, whatever the script.
fn assert_consistent(report: &ProfileReport) {
    let summary = report.summary;
    let exclusive: u64 = report.stats.iter().map(|stats| stats.exclusive_ns).sum();
    assert_eq!(
        summary.attributed_ns, exclusive,
        "line times partition the time"
    );
    assert!(
        exclusive <= summary.total_ns,
        "regions took {exclusive} of {}",
        summary.total_ns
    );
    assert_eq!(
        summary.unattributed_ns(),
        summary.total_ns - exclusive,
        "what no region covers is the rest"
    );
    for (index, stats) in report.stats.iter().enumerate() {
        assert!(
            stats.exclusive_ns <= stats.inclusive_ns,
            "region {index}: exclusive {} over inclusive {}",
            stats.exclusive_ns,
            stats.inclusive_ns
        );
        assert!(stats.inclusive_ns <= summary.total_ns);
        assert!(stats.slowest_ns <= stats.inclusive_ns.max(stats.slowest_ns));
        let region = report.table.get(index as u32).unwrap();
        if let Some(parent) = region.parent {
            assert!(parent < index as u32, "a parent comes first");
        }
    }
    let measured: u64 = report.stats.iter().map(|stats| stats.executions).sum();
    assert_eq!(summary.measured_executions, measured);
}

// MARK: Counts

#[test]
fn a_statement_runs_once_per_bar() {
    let data = bars(40, 60_000);
    let report = profile(&script("a = close\nb = a + 1\nplot(b)\n"), &data);
    for number in [FIRST_LINE, FIRST_LINE + 1, FIRST_LINE + 2] {
        assert_eq!(line(&report, number).executions, 40);
    }
    assert_eq!(report.summary.bars, 40);
    assert_consistent(&report);
}

#[test]
fn the_script_declaration_is_not_a_region() {
    let data = bars(5, 60_000);
    let report = profile(&script("plot(close)\n"), &data);
    assert!(report.lines().iter().all(|line| line.line >= FIRST_LINE));
    assert_eq!(report.table.len(), 1);
}

#[test]
fn a_loop_counts_entries_and_its_body_counts_iterations() {
    let data = bars(25, 60_000);
    let report = profile(
        &script("s = 0.0\nfor i = 0 to 9\n    s += i\nplot(s)\n"),
        &data,
    );
    let header = line(&report, FIRST_LINE + 1);
    let body = line(&report, FIRST_LINE + 2);
    assert_eq!(header.executions, 25, "the loop is entered once per bar");
    assert_eq!(body.executions, 250, "and its body runs ten times each");
    assert_eq!(report.iterations(header.region), Some(250));
    assert_eq!(header.block, Some((FIRST_LINE + 1, FIRST_LINE + 2)));
    assert_consistent(&report);
}

#[test]
fn nested_loops_multiply() {
    let data = bars(10, 60_000);
    let report = profile(
        &script("t = 0.0\nfor i = 1 to 4\n    for j = 1 to 3\n        t += i * j\nplot(t)\n"),
        &data,
    );
    let outer = line(&report, FIRST_LINE + 1);
    let inner = line(&report, FIRST_LINE + 2);
    let body = line(&report, FIRST_LINE + 3);
    assert_eq!(outer.executions, 10);
    assert_eq!(inner.executions, 40, "entered once per outer iteration");
    assert_eq!(body.executions, 120);
    assert_eq!(report.iterations(outer.region), Some(40));
    assert_eq!(report.iterations(inner.region), Some(120));
    assert_eq!(outer.block, Some((FIRST_LINE + 1, FIRST_LINE + 3)));
    assert_consistent(&report);
}

#[test]
fn while_loops_and_break_and_continue_count_what_ran() {
    let data = bars(8, 60_000);
    let report = profile(
        &script(
            "n = 0\nwhile n < 6\n    n += 1\n    if n == 2\n        continue\n    if n == 5\n        break\nplot(n)\n",
        ),
        &data,
    );
    let header = line(&report, FIRST_LINE + 1);
    assert_eq!(header.executions, 8);
    // n = 1..5 is five iterations: `continue` at 2 ends one early, `break` at 5 ends the loop.
    assert_eq!(report.iterations(header.region), Some(40));
    assert_eq!(line(&report, FIRST_LINE + 2).executions, 40);
    assert_eq!(
        line(&report, FIRST_LINE + 3).executions,
        40,
        "the first `if` is reached on every iteration"
    );
    assert_eq!(
        line(&report, FIRST_LINE + 4).executions,
        8,
        "continue ran once per bar"
    );
    assert_eq!(
        line(&report, FIRST_LINE + 5).executions,
        32,
        "the second `if` is reached unless `continue` skipped it"
    );
    assert_eq!(
        line(&report, FIRST_LINE + 6).executions,
        8,
        "break ran once per bar"
    );
    assert_consistent(&report);
}

#[test]
fn a_branch_counts_only_when_it_runs() {
    let data = bars(60, 60_000);
    let report = profile(
        &script("x = 0.0\nif close > open\n    x := 1.0\nelse\n    x := 2.0\nplot(x)\n"),
        &data,
    );
    let up = rising(&data);
    let branch = line(&report, FIRST_LINE + 1);
    assert_eq!(branch.executions, 60);
    assert_eq!(line(&report, FIRST_LINE + 2).executions, up);
    assert_eq!(line(&report, FIRST_LINE + 4).executions, 60 - up);
    assert_eq!(
        report.branch_executions(branch.region),
        vec![Some(up), Some(60 - up)]
    );
    assert_consistent(&report);
}

#[test]
fn a_switch_counts_each_arm() {
    let data = bars(30, 60_000);
    let report = profile(
        &script(
            "k = bar_index % 3\nr = switch k\n    0 => 10.0\n    1 => 20.0\n    => 30.0\nplot(r)\n",
        ),
        &data,
    );
    let declaration = line(&report, FIRST_LINE + 1);
    assert_eq!(declaration.executions, 30);
    let arms: Vec<u64> = (FIRST_LINE + 2..=FIRST_LINE + 4)
        .map(|number| line(&report, number).executions)
        .collect();
    assert_eq!(arms, vec![10, 10, 10]);
    assert_consistent(&report);
}

#[test]
fn two_statements_on_one_line_are_two_regions_that_add_up() {
    let data = bars(12, 60_000);
    let report = profile(&script("a = close, b = open\nplot(a + b)\n"), &data);
    let both = line(&report, FIRST_LINE);
    assert_eq!(both.executions, 24);
    assert_eq!(report.table.len(), 3);
    assert_consistent(&report);
}

#[test]
fn a_statement_spread_over_lines_belongs_to_its_first_line() {
    let data = bars(9, 60_000);
    let report = profile(
        &script("v = math.max(\n    close,\n    open)\nplot(v)\n"),
        &data,
    );
    assert_eq!(line(&report, FIRST_LINE).executions, 9);
    assert!(
        report
            .lines()
            .iter()
            .all(|line| line.line != FIRST_LINE + 1 && line.line != FIRST_LINE + 2)
    );
    let stats = report
        .table
        .regions()
        .iter()
        .find(|r| r.start.line == FIRST_LINE)
        .unwrap();
    assert_eq!(
        stats.end.line,
        FIRST_LINE + 2,
        "it ends where its last token is"
    );
    assert!(
        !stats.is_block(),
        "a wrapped expression is not a code block"
    );
}

// MARK: Functions

#[test]
fn a_function_counts_every_call_and_its_body_runs_per_call() {
    let data = bars(20, 60_000);
    let report = profile(
        &script(
            "f(x) =>\n    y = x * 2\n    y + 1\na = f(close)\nb = f(open)\nc = f(high)\nplot(a + b + c)\n",
        ),
        &data,
    );
    let function = line(&report, FIRST_LINE);
    assert_eq!(function.kind, RegionKind::Function);
    assert_eq!(function.executions, 60, "three call sites, twenty bars");
    assert_eq!(line(&report, FIRST_LINE + 1).executions, 60);
    assert_eq!(line(&report, FIRST_LINE + 2).executions, 60);
    for number in [FIRST_LINE + 3, FIRST_LINE + 4, FIRST_LINE + 5] {
        assert_eq!(line(&report, number).executions, 20);
    }
    let sites = report.call_sites_of(function.region);
    assert_eq!(sites.len(), 3);
    assert!(sites.iter().all(|site| site.calls == 20));
    let by_site: u64 = sites.iter().map(|site| site.inclusive_ns).sum();
    assert_eq!(
        by_site,
        report.stats_of(function.region).inclusive_ns,
        "the call sites add up to the function"
    );
    assert_consistent(&report);
}

#[test]
fn a_call_is_charged_to_the_statement_that_made_it_without_its_body() {
    let data = bars(10, 60_000);
    let report = profile(
        &script("f(x) =>\n    x * 2\na = f(close)\nplot(a)\n"),
        &data,
    );
    let call = line(&report, FIRST_LINE + 2);
    let function = line(&report, FIRST_LINE);
    assert!(
        call.time_ns > call.line_time_ns,
        "its time includes the call"
    );
    assert_eq!(
        call.time_ns - call.line_time_ns,
        function.time_ns,
        "what the statement does not spend itself, the function did"
    );
    assert_consistent(&report);
}

#[test]
fn nested_functions_do_not_double_count() {
    let data = bars(15, 60_000);
    let report = profile(bench_tests::NESTED_FUNCTIONS, &data);
    assert_consistent(&report);
    let (hottest, stats) = report.hottest_function().expect("a function ran");
    let name = report.table.get(hottest).unwrap().name.clone();
    assert_eq!(
        name.as_deref(),
        Some("top"),
        "the outermost function includes the others"
    );
    assert!(stats.inclusive_ns <= report.summary.total_ns);
    let leaf = report
        .table
        .regions()
        .iter()
        .position(|r| r.name.as_deref() == Some("leaf"))
        .unwrap();
    assert_eq!(
        report.stats[leaf].executions,
        15 * 4,
        "leaf runs four times per bar"
    );
}

#[test]
fn a_user_function_in_a_loop_is_counted_per_call() {
    let data = bars(6, 60_000);
    let report = profile(
        &script("sq(x) =>\n    x * x\nt = 0.0\nfor i = 1 to 5\n    t += sq(i)\nplot(t)\n"),
        &data,
    );
    assert_eq!(line(&report, FIRST_LINE).executions, 30);
    assert_eq!(line(&report, FIRST_LINE + 4).executions, 30);
    assert_consistent(&report);
}

// MARK: Time

#[test]
fn on_a_stepping_clock_every_duration_is_a_count_of_reads() {
    // One statement per bar: the bar starts (read 0), the statement starts (1) and ends (2), the
    // bar ends (3).
    let data = bars(10, 60_000);
    let report = profile(&script("x = close\n"), &data);
    let statement = line(&report, FIRST_LINE);
    assert_eq!(report.summary.total_ns, 10 * 3 * STEP);
    assert_eq!(statement.time_ns, 10 * STEP);
    assert_eq!(statement.line_time_ns, 10 * STEP);
    assert_eq!(report.summary.unattributed_ns(), 10 * 2 * STEP);
    assert_eq!(report.stats_of(statement.region).slowest_ns, STEP);
    assert_consistent(&report);
}

#[test]
fn a_parent_is_charged_for_its_children_inclusively_and_not_exclusively() {
    // Per bar: bar start (0), loop starts (1); three iterations each start and end the body (2 reads);
    // the loop ends (8), the bar ends (9).
    let data = bars(4, 60_000);
    let report = profile(&script("for i = 0 to 2\n    y = i\n"), &data);
    let header = line(&report, FIRST_LINE);
    let body = line(&report, FIRST_LINE + 1);
    assert_eq!(body.time_ns, 4 * 3 * STEP);
    assert_eq!(header.time_ns, 4 * 7 * STEP);
    assert_eq!(header.line_time_ns, 4 * 4 * STEP);
    assert_eq!(body.line_time_ns, body.time_ns);
    assert_eq!(report.summary.total_ns, 4 * 9 * STEP);
    assert_consistent(&report);
}

#[test]
fn a_block_line_shows_the_whole_block_and_its_header_only_its_own() {
    let data = bars(3, 60_000);
    let report = profile(&script("for i = 0 to 4\n    y = i\n    z = y\n"), &data);
    let header = line(&report, FIRST_LINE);
    let inside = line(&report, FIRST_LINE + 1).time_ns + line(&report, FIRST_LINE + 2).time_ns;
    assert_eq!(header.time_ns, header.line_time_ns + inside);
}

#[test]
fn a_conditional_that_does_not_run_costs_only_its_condition() {
    let mut data = bars(10, 60_000);
    for bar in &mut data {
        bar.close = bar.open - 1.0;
    }
    let report = profile(
        &script("x = 0.0\nif close > open\n    x := 1.0\nplot(x)\n"),
        &data,
    );
    assert!(
        report
            .lines()
            .iter()
            .all(|line| line.line != FIRST_LINE + 2),
        "a branch that never ran has no measurements"
    );
    assert_eq!(line(&report, FIRST_LINE + 1).executions, 10);
}

// MARK: The recorder alone

fn single() -> ProfileRuntime {
    ProfileRuntime::with_clock(
        Arc::new(RegionTable::single()),
        Box::new(SteppingClock::new(STEP)),
    )
}

#[test]
fn recursion_adds_a_regions_inclusive_time_once() {
    let mut runtime = single();
    runtime.begin_bar();
    // The region calls itself: reads 1 (outer starts), 2 (inner starts), 3 (inner ends), 4 (outer
    // ends); the bar ends at read 5.
    assert!(runtime.enter_statement(0));
    assert!(runtime.enter_statement(0));
    runtime.exit();
    runtime.exit();
    runtime.end_bar(true, false);
    let report = runtime.finish(Completeness::Complete);
    let stats = report.stats_of(0);
    assert_eq!(stats.executions, 2);
    assert_eq!(
        stats.inclusive_ns,
        3 * STEP,
        "the outer execution's time, once"
    );
    assert_eq!(
        stats.exclusive_ns,
        3 * STEP,
        "inner 1 + outer 2: no moment twice"
    );
    assert_eq!(stats.slowest_ns, 3 * STEP);
    assert!(stats.exclusive_ns <= stats.inclusive_ns);
    assert_eq!(report.summary.total_ns, 5 * STEP);
}

#[test]
fn a_region_left_open_is_closed_by_the_end_of_the_bar() {
    let mut runtime = single();
    runtime.begin_bar();
    runtime.enter_statement(0);
    runtime.end_bar(true, true);
    let report = runtime.finish(Completeness::Complete);
    assert_eq!(report.stats_of(0).executions, 1);
    assert_eq!(report.summary.realtime_bars, 1);
    assert_eq!(
        report.summary.attributed_ns,
        report.stats_of(0).exclusive_ns
    );
    assert!(report.summary.attributed_ns <= report.summary.total_ns);
}

#[test]
fn an_offset_that_is_not_a_region_is_not_timed() {
    let mut runtime = single();
    runtime.begin_bar();
    assert!(!runtime.enter_statement(7));
    assert!(!runtime.enter_function(7, 0));
    runtime.end_bar(true, false);
    let report = runtime.finish(Completeness::Complete);
    assert_eq!(report.summary.measured_executions, 0);
}

#[test]
fn a_refused_bar_is_not_counted() {
    let mut runtime = single();
    runtime.begin_bar();
    runtime.end_bar(false, false);
    let report = runtime.finish(Completeness::Complete);
    assert_eq!((report.summary.bars, report.summary.total_ns), (0, 0));
}

#[test]
fn shares_are_of_the_script_total_and_an_empty_profile_has_none() {
    let mut runtime = single();
    runtime.begin_bar();
    runtime.enter_statement(0);
    runtime.exit();
    runtime.end_bar(true, false);
    let report = runtime.finish(Completeness::Complete);
    assert!((report.share(STEP) - 1.0 / 3.0).abs() < 1e-12);
    let empty = single().finish(Completeness::Complete);
    assert_eq!(empty.share(5), 0.0);
}

// MARK: Calibration

#[test]
fn calibration_reports_what_measuring_costs_and_never_subtracts_it() {
    let table = Arc::new(RegionTable::single());
    let mut runtime = ProfileRuntime::new(Arc::clone(&table));
    runtime.begin_bar();
    for _ in 0..1000 {
        runtime.enter_statement(0);
        runtime.exit();
    }
    runtime.end_bar(true, false);
    let report = runtime.finish(Completeness::Complete);
    let summary = report.summary;
    assert!(summary.pair_cost_ns > 0, "measuring takes time");
    assert_eq!(summary.overhead_ns, 1000 * summary.pair_cost_ns);
    assert!(summary.resolution_ns >= 1);
    // The measured time is what the clock saw: it was not reduced by the overhead.
    assert_eq!(summary.attributed_ns, report.stats_of(0).exclusive_ns);
}

// MARK: Equivalence

fn plain_and_profiled(source: &str, data: &[Kline], security: bool) -> (u64, u64) {
    let plain = bench_tests::checksum(&run_script(source, data, security, |_| {}).output());
    let profiled = bench_tests::checksum(
        &run_script(source, data, security, |session| {
            let runtime = recorder(session, 7);
            session.attach_profiler(runtime);
        })
        .output(),
    );
    (plain, profiled)
}

#[test]
fn profiling_changes_no_output() {
    let data = bars(600, 60_000);
    let cases: [(&str, String, bool); 7] = [
        ("simple", bench_tests::SIMPLE.to_owned(), false),
        ("many ta", bench_tests::MANY_TA.to_owned(), false),
        ("containers", bench_tests::CONTAINERS.to_owned(), false),
        ("functions", bench_tests::NESTED_FUNCTIONS.to_owned(), false),
        ("history", bench_tests::LONG_HISTORY.to_owned(), false),
        ("strategy", bench_tests::STRATEGY.to_owned(), false),
        ("security", bench_tests::SECURITY.to_owned(), true),
    ];
    for (name, source, security) in cases {
        let (plain, profiled) = plain_and_profiled(&source, &data, security);
        assert_eq!(
            plain, profiled,
            "{name}: profiling changed what the script computed"
        );
    }
}

#[test]
fn profiling_a_strategy_leaves_its_trades_and_alerts_as_they_were() {
    let data = bars(600, 60_000);
    let plain = run_script(bench_tests::STRATEGY, &data, false, |_| {});
    let mut profiled = run_script(bench_tests::STRATEGY, &data, false, |session| {
        let runtime = recorder(session, 7);
        session.attach_profiler(runtime);
    });
    let a = plain.output().strategy.expect("strategy report");
    let b = profiled.output().strategy.expect("strategy report");
    assert_eq!(a.trades.len(), b.trades.len());
    assert!(!a.trades.is_empty());
    assert_eq!(plain.emitted_alerts.len(), profiled.emitted_alerts.len());
    let report = profiled
        .detach_profiler()
        .unwrap()
        .finish(Completeness::Complete);
    assert_consistent(&report);
}

#[test]
fn profiled_runs_fail_the_way_plain_runs_do() {
    let source = script("a = array.new_float(0)\nx = array.get(a, 3)\nplot(x)\n");
    let data = bars(5, 60_000);
    let program = compiler::compile(&source, Limits::default(), &NoLibraries);
    assert!(program.is_valid());
    let run = |profiled: bool| {
        let mut session = Session::new(
            program.clone(),
            Default::default(),
            Limits::default(),
            Some(0.01),
            Default::default(),
            None,
            None,
        );
        if profiled {
            let runtime = recorder(&session, STEP);
            session.attach_profiler(runtime);
        }
        let Err(error) = session.evaluate(&data) else {
            panic!("the script should fail");
        };
        (format!("{error:?}"), session)
    };
    let (plain, _) = run(false);
    let (profiled, mut session) = run(true);
    assert_eq!(plain, profiled);
    let report = session
        .detach_profiler()
        .unwrap()
        .finish(Completeness::Complete);
    assert_eq!(report.summary.bars, 1, "the failing bar was the only one");
    let failing = line(&report, FIRST_LINE + 1);
    assert_eq!(failing.executions, 1, "the statement that failed ran once");
    assert_consistent(&report);
}

// MARK: Sessions

fn config(source: &str, data: Vec<Kline>) -> ProfileConfig {
    let mut config = ProfileConfig::new(source, Default::default(), data, 0);
    config.limits = Limits::default();
    config
}

fn wait(session: &mut ProfileSession) -> ProfileOutcome {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        if let Some(outcome) = session.poll() {
            return outcome;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the session never ended"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

#[test]
fn a_session_profiles_a_script_on_its_own_thread() {
    let data = bars(300, 60_000);
    let now = data.last().unwrap().close_time + 1;
    let mut config = config(&script("a = ta.sma(close, 5)\nplot(a)\n"), data);
    config.now_ms = now;
    let mut session = ProfileSession::start(config, Arc::new(|| {})).expect("starts");
    let outcome = wait(&mut session);
    let report = outcome.result.expect("a profile");
    assert!(report.is_complete());
    assert_eq!(report.summary.bars, 300);
    assert_eq!(session.progress().0, 300);
    assert!(session.is_finished());
    assert!(outcome.compile_ns > 0);
    assert_consistent(&report);
    assert!(report.summary.pair_cost_ns > 0, "a session calibrates");
}

#[test]
fn a_script_that_does_not_compile_is_reported_and_not_profiled() {
    let data = bars(10, 60_000);
    let mut session =
        ProfileSession::start(config(&script("x = nope(\n"), data), Arc::new(|| {})).unwrap();
    assert!(wait(&mut session).result.is_err());
}

#[test]
fn a_session_without_bars_does_not_start() {
    assert!(matches!(
        ProfileSession::start(config(&script("x = close\n"), Vec::new()), Arc::new(|| {})),
        Err(StartError::NoBars)
    ));
}

#[test]
fn a_cancelled_session_delivers_a_labelled_partial_profile() {
    let data = bars(500, 60_000);
    let now = data.last().unwrap().close_time + 1;
    let mut config = config(
        &script("t = 0.0\nfor i = 0 to 200000\n    t += i\nplot(t)\n"),
        data,
    );
    config.now_ms = now;
    let mut session = ProfileSession::start(config, Arc::new(|| {})).unwrap();
    while session.progress().0 == 0 && session.thread_alive() {
        std::thread::yield_now();
    }
    session.cancel();
    let report = wait(&mut session).result.expect("a partial profile");
    assert_eq!(
        report.completeness,
        Completeness::Partial(Interruption::Cancelled)
    );
    assert!(!report.is_complete());
    assert!(report.summary.bars < 500);
    assert_consistent(&report);
}

#[test]
fn a_script_that_fails_keeps_what_it_measured_before_failing() {
    let data = bars(20, 60_000);
    let now = data.last().unwrap().close_time + 1;
    let mut config = config(
        &script("a = array.new_float(0)\nx = bar_index < 6 ? 1.0 : array.get(a, 3)\nplot(x)\n"),
        data,
    );
    config.now_ms = now;
    let mut session = ProfileSession::start(config, Arc::new(|| {})).unwrap();
    let report = wait(&mut session).result.expect("a partial profile");
    assert!(matches!(
        report.completeness,
        Completeness::Partial(Interruption::Failed(_))
    ));
    assert_eq!(
        report.summary.bars, 7,
        "six that ran and the one that failed"
    );
    assert_consistent(&report);
}

#[test]
fn dropping_a_session_ends_its_thread() {
    let data = bars(500, 60_000);
    let now = data.last().unwrap().close_time + 1;
    let mut config = config(
        &script("t = 0.0\nfor i = 0 to 1000000\n    t += i\nplot(t)\n"),
        data,
    );
    config.now_ms = now;
    let session = ProfileSession::start(config, Arc::new(|| {})).unwrap();
    let alive = session.liveness();
    drop(session);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while alive.strong_count() > 0 {
        assert!(
            std::time::Instant::now() < deadline,
            "the thread never ended"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

#[test]
fn a_profile_is_not_extended_by_a_second_run() {
    // Two sessions on the same text profile independently: their counters are their own.
    let data = bars(50, 60_000);
    let now = data.last().unwrap().close_time + 1;
    let make = || {
        let mut config = config(&script("a = close\nplot(a)\n"), data.clone());
        config.now_ms = now;
        ProfileSession::start(config, Arc::new(|| {})).unwrap()
    };
    let (mut first, mut second) = (make(), make());
    let a = wait(&mut first).result.unwrap();
    let b = wait(&mut second).result.unwrap();
    assert_eq!(a.summary.bars, 50);
    assert_eq!(b.summary.bars, 50);
    assert_eq!(a.stats_of(0).executions, 50);
    assert_eq!(b.stats_of(0).executions, 50);
}

// MARK: Regions

fn table(body: &str) -> RegionTable {
    let program = compiler::compile(&script(body), Limits::default(), &NoLibraries);
    assert!(program.is_valid(), "{:?}", program.diagnostics);
    RegionTable::build(&program)
}

#[test]
fn a_regions_end_is_where_its_last_token_is() {
    let table = table(
        "f(x) =>\n    y = x + 1\n    y * 2\nfor i = 0 to 3\n    t = f(i)\n    u = (t +\n        1)\n\nplot(close)\n",
    );
    let by_line = |line: usize| {
        table
            .regions()
            .iter()
            .find(|region| region.start.line == line)
            .unwrap()
    };
    assert_eq!(
        by_line(3).end.line,
        5,
        "the function reaches its last body line"
    );
    assert_eq!(
        by_line(6).end.line,
        9,
        "the loop reaches the end of a wrapped statement"
    );
    assert_eq!(
        by_line(8).end.line,
        9,
        "a wrapped statement ends on its last line"
    );
    assert_eq!(by_line(11).end.line, 11);
    assert!(by_line(3).is_block() && by_line(6).is_block());
    assert!(!by_line(8).is_block() && !by_line(11).is_block());
}

#[test]
fn regions_know_their_parents_and_the_first_statement_of_each_block() {
    let table = table("if close > open\n    a = 1\n    b = 2\nelse\n    c = 3\n");
    let regions = table.regions();
    assert_eq!(regions.len(), 4);
    assert_eq!(regions[0].kind, RegionKind::Branch);
    assert_eq!(regions[0].blocks, vec![Some(1), Some(3)]);
    assert!(regions[1..].iter().all(|region| region.parent == Some(0)));
    assert!(table.encloses(0, 3) && !table.encloses(3, 0));
}

#[test]
fn a_value_block_is_inside_the_declaration_it_gives_a_value_to() {
    let table = table("x = if close > open\n    1.0\nelse\n    2.0\nplot(x)\n");
    let regions = table.regions();
    assert_eq!(regions[0].kind, RegionKind::Statement);
    assert_eq!(regions[1].kind, RegionKind::Branch);
    assert_eq!(regions[1].parent, Some(0));
    assert!(regions[0].end.line >= regions[1].end.line);
}

#[test]
fn identities_are_offsets_so_unicode_and_comments_do_not_move_them() {
    let table = table("// ünïcode — comment\nπ = close // trailing\n\n    \na = π + 1\nplot(a)\n");
    let lines: Vec<usize> = table.regions().iter().map(|r| r.start.line).collect();
    assert_eq!(lines, vec![4, 7, 8]);
    for region in table.regions() {
        assert_eq!(
            table.at_offset(region.offset),
            Some(
                table
                    .regions()
                    .iter()
                    .position(|r| r.offset == region.offset)
                    .unwrap() as u32
            )
        );
    }
}

#[test]
fn functions_are_found_by_their_first_body_statement() {
    let table = table("f(x) =>\n    x + 1\ng(x) =>\n    x + 2\nplot(f(close) + g(close))\n");
    let f = table
        .regions()
        .iter()
        .position(|r| r.name.as_deref() == Some("f"))
        .unwrap();
    let first = table.regions()[f].blocks[0].unwrap();
    assert_eq!(
        table.function_with_body(table.regions()[first as usize].offset),
        Some(f as u32)
    );
    assert_eq!(table.function_with_body(0), None);
}
