//! The profiler through the whole Pine Studio window: turning it on, the run on the preview
//! chart's bars, what the editor and the drawer show, staleness, failure, cancellation, and the
//! run's end with the script, the profiler and the window.

use std::time::{Duration, Instant};

use super::Status;
use super::marks::ProfileColumn;
use crate::pine_studio::debug::studio_tests::Rig;
use crate::pine_studio::library::prefs::DrawerTab;
use crate::pine_studio::*;

/// Line 3 declares `sq` (its body is line 4), 5 declares `total`, 6 is the loop, 7 its body, which
/// calls `sq`, and 8 plots.
const SOURCE: &str = "//@version=6
indicator(\"Profile demo\")
sq(x) =>
    x * x
total = 0.0
for i = 1 to 20
    total += sq(i)
plot(total)
";

/// Compiles clean and fails on the last bar: an index past the end of an empty array.
const FAILS: &str = "//@version=6
indicator(\"Fails\")
var values = array.new_float()
x = 0.0
if barstate.islast
    x := array.get(values, 5)
plot(close + x)
";

fn status(rig: &Rig) -> Status {
    rig.manager
        .profiler
        .status(rig.manager.profiler_now.as_ref())
}

fn until(rig: &mut Rig, what: &str, done: impl Fn(&Status) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        rig.frame();
        if done(&status(rig)) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {what}; the profiler is {:?}",
            status(rig)
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn profile_on(tag: &str) -> Rig {
    let mut rig = Rig::new(tag);
    rig.set_source(SOURCE);
    rig.manager.cmds.push(Cmd::ProfilerToggle);
    until(&mut rig, "a profile", |s| *s == Status::Ready);
    rig
}

/// The Executions row of a line's tooltip.
fn executions(rig: &Rig, line: usize) -> String {
    let marks = &rig.manager.profiler.profile().unwrap().marks;
    marks
        .mark(line)
        .unwrap()
        .tooltip
        .rows
        .iter()
        .find(|(label, _)| *label == "Executions")
        .map(|(_, value)| value.clone())
        .unwrap()
}

fn column(rig: &Rig) -> Option<ProfileColumn> {
    rig.manager
        .profiler
        .column(rig.manager.profiler_now.as_ref())
}

#[test]
fn the_profiler_is_off_until_asked_for_and_then_costs_nothing_to_leave_off() {
    let mut rig = Rig::new("profiler-off");
    rig.set_source(SOURCE);
    for _ in 0..5 {
        rig.frame();
    }
    assert_eq!(status(&rig), Status::Off);
    assert!(
        rig.manager.profiler_now.is_none(),
        "nothing is hashed while off"
    );
    assert!(column(&rig).is_none(), "no column, so no room taken");
    assert!(!rig.manager.profiler.is_running());
    assert!(rig.manager.profiler.profile().is_none());
}

#[test]
fn turning_it_on_profiles_the_open_script_and_marks_its_lines() {
    let mut rig = Rig::new("profiler-on");
    rig.set_source(SOURCE);
    rig.manager.cmds.push(Cmd::ProfilerToggle);
    rig.frame();
    assert!(rig.manager.profiler.is_on());
    assert_eq!(rig.manager.prefs.drawer_tab, DrawerTab::Profile);
    assert!(rig.manager.prefs.drawer_open, "the person asked to see it");
    until(&mut rig, "a profile", |s| *s == Status::Ready);
    let profile = rig.manager.profiler.profile().unwrap();
    assert!(profile.report.is_complete());
    assert_eq!(profile.report.summary.bars, 40);
    assert!(profile.label.contains('\u{b7}'), "{}", profile.label);
    // The loop (line 6), its body (7), the function (3) and its body (4) all ran.
    for line in [3, 4, 5, 6, 7, 8] {
        assert!(profile.marks.mark(line).is_some(), "line {line}");
    }
    assert_eq!(executions(&rig, 7), "800");
    match column(&rig) {
        Some(ProfileColumn::Marks(marks)) => assert!(marks.mark(6).is_some()),
        other => panic!("expected marks, got {other:?}"),
    }
    // The editor was handed the column and drew it.
    rig.frame();
}

#[test]
fn a_script_that_fails_when_it_runs_has_no_profile() {
    let mut rig = Rig::new("profiler-fails");
    rig.set_source(FAILS);
    rig.manager.cmds.push(Cmd::ProfilerToggle);
    until(&mut rig, "the failure", |s| matches!(s, Status::Failed(_)));
    let Status::Failed(message) = status(&rig) else {
        unreachable!()
    };
    assert!(
        message.starts_with("The script failed on bar") && message.contains("out of bounds"),
        "{message}"
    );
    // No numbers of any kind: not in the drawer, not beside the code.
    assert!(rig.manager.profiler.profile().is_none());
    assert!(!matches!(column(&rig), Some(ProfileColumn::Marks(_))));
    // The failure is still the Problems list's to show.
    let failure = rig
        .manager
        .profiler
        .failure(rig.manager.profiler_now.as_ref())
        .expect("the diagnostic");
    assert!(failure.message.contains("out of bounds"));
}

#[test]
fn a_failed_run_drops_the_profile_of_the_text_before_it() {
    let mut rig = profile_on("profiler-fails-after-ok");
    assert!(rig.manager.profiler.profile().is_some());
    rig.set_source(FAILS);
    rig.manager.cmds.push(Cmd::ProfilerRun);
    until(&mut rig, "the failure", |s| matches!(s, Status::Failed(_)));
    assert!(
        rig.manager.profiler.profile().is_none(),
        "the earlier script's numbers are not this script's"
    );
    // Fixing the script profiles it again.
    rig.set_source(SOURCE);
    rig.manager.cmds.push(Cmd::ProfilerRun);
    until(&mut rig, "a profile", |s| *s == Status::Ready);
    assert!(
        rig.manager
            .profiler
            .failure(rig.manager.profiler_now.as_ref())
            .is_none()
    );
}

#[test]
fn editing_the_script_makes_the_profile_stale_and_hides_its_marks() {
    let mut rig = profile_on("profiler-stale-edit");
    rig.set_source(&SOURCE.replace("20", "30"));
    rig.frame();
    assert_eq!(status(&rig), Status::Stale);
    assert!(
        matches!(column(&rig), Some(ProfileColumn::Empty(why)) if why.contains("earlier")),
        "stale marks are never drawn on text they do not describe"
    );
    assert!(
        rig.manager.profiler.profile().is_some(),
        "but it is kept, and labelled"
    );
    // Not run again by itself.
    for _ in 0..10 {
        rig.frame();
    }
    assert_eq!(status(&rig), Status::Stale);
    // Run again: current, and about the new text.
    rig.manager.cmds.push(Cmd::ProfilerRun);
    until(&mut rig, "a new profile", |s| *s == Status::Ready);
    assert_eq!(executions(&rig, 7), "1,200");
}

#[test]
fn a_run_asked_for_while_the_preview_has_no_bars_starts_when_they_arrive() {
    let mut rig = profile_on("profiler-run-waits");
    rig.manager.preview.set_candles_for_test(Vec::new());
    rig.frame();
    rig.manager.cmds.push(Cmd::ProfilerRun);
    rig.frame();
    assert_eq!(
        status(&rig),
        Status::Stale,
        "nothing to run on yet, so the old profile stays"
    );
    rig.manager
        .preview
        .set_candles_for_test(crate::pine_studio::debug::studio_tests::bars(45));
    until(&mut rig, "the run that was asked for", |s| {
        *s == Status::Ready
    });
    assert_eq!(
        rig.manager
            .profiler
            .profile()
            .map(|p| p.report.summary.bars),
        Some(45),
        "the new profile is of the bars that arrived"
    );
}

#[test]
fn a_script_with_errors_is_not_run_and_is_run_when_fixed() {
    let mut rig = Rig::new("profiler-errors");
    rig.set_source("//@version=6\nindicator(\"Bad\")\nx = nope(\nplot(x)\n");
    rig.manager.cmds.push(Cmd::ProfilerToggle);
    until(&mut rig, "a refusal", |s| matches!(s, Status::Failed(_)));
    let Status::Failed(message) = status(&rig) else {
        unreachable!()
    };
    assert!(message.contains("errors"), "{message}");
    assert!(!rig.manager.profiler.is_running());
    // Fixed: the attempt that could not run is made again without being asked.
    rig.set_source(SOURCE);
    until(&mut rig, "a profile of the fixed script", |s| {
        *s == Status::Ready
    });
}

#[test]
fn a_library_has_nothing_to_run() {
    let mut rig = Rig::new("profiler-library");
    rig.manager.open.as_mut().unwrap().script_type = ScriptType::Library;
    rig.manager.cmds.push(Cmd::ProfilerToggle);
    until(&mut rig, "a refusal", |s| matches!(s, Status::Failed(_)));
}

#[test]
fn turning_it_off_forgets_the_profile_and_ends_a_run() {
    let mut rig = profile_on("profiler-off-again");
    rig.manager.cmds.push(Cmd::ProfilerToggle);
    rig.frame();
    assert_eq!(status(&rig), Status::Off);
    assert!(rig.manager.profiler.profile().is_none());
    assert!(column(&rig).is_none());
    // A run in flight ends with the profiler.
    rig.set_source(
        "//@version=6\nindicator(\"Slow\")\nt = 0.0\nfor i = 0 to 3000000\n    t += i\nplot(t)\n",
    );
    rig.manager.cmds.push(Cmd::ProfilerToggle);
    until(&mut rig, "a run", |s| s.is_running());
    let alive = rig.manager.profiler.liveness().expect("a run");
    rig.manager.cmds.push(Cmd::ProfilerToggle);
    rig.frame();
    let deadline = Instant::now() + Duration::from_secs(30);
    while alive.strong_count() > 0 {
        assert!(Instant::now() < deadline, "the thread never ended");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn cancelling_keeps_what_was_measured_and_says_it_is_partial() {
    let mut rig = Rig::new("profiler-cancel");
    rig.set_source(
        "//@version=6\nindicator(\"Slow\")\nt = 0.0\nfor i = 0 to 400000\n    t += i\nplot(t)\n",
    );
    rig.manager.cmds.push(Cmd::ProfilerToggle);
    until(
        &mut rig,
        "progress",
        |s| matches!(s, Status::Running { done, .. } if *done > 0),
    );
    rig.manager.cmds.push(Cmd::ProfilerCancel);
    until(&mut rig, "a partial profile", |s| *s == Status::Partial);
    let profile = rig.manager.profiler.profile().unwrap();
    assert!(!profile.report.is_complete());
    assert!(profile.report.summary.bars < 40);
    assert!(
        profile
            .marks
            .caveat
            .as_deref()
            .unwrap()
            .contains("cancelled")
    );
    // Partial results are drawn, labelled by the caveat in the drawer.
    assert!(matches!(column(&rig), Some(ProfileColumn::Marks(_))));
}

#[test]
fn switching_scripts_drops_the_profile_of_the_first() {
    let mut rig = profile_on("profiler-switch");
    let other = rig
        .manager
        .scripts
        .iter()
        .map(|s| s.id.clone())
        .find(|id| Some(id.as_str()) != rig.manager.open.as_ref().map(|o| o.id.as_str()));
    let Some(other) = other else {
        return;
    };
    rig.manager.open_script(&other);
    rig.frame();
    assert!(
        rig.manager
            .profiler
            .profile()
            .is_none_or(|p| p.key.is_script(&other)),
        "a profile is never shown for another script"
    );
}

#[test]
fn two_windows_profile_independently() {
    let mut a = profile_on("profiler-a");
    let mut b = Rig::new("profiler-b");
    b.set_source("//@version=6\nindicator(\"Other\")\nx = close\nplot(x)\n");
    assert_eq!(status(&b), Status::Off, "a's profiler is a's");
    b.manager.cmds.push(Cmd::ProfilerToggle);
    until(&mut b, "b's profile", |s| *s == Status::Ready);
    assert_eq!(status(&a), Status::Ready);
    let a_bars = a
        .manager
        .profiler
        .profile()
        .unwrap()
        .report
        .summary
        .measured_executions;
    let b_bars = b
        .manager
        .profiler
        .profile()
        .unwrap()
        .report
        .summary
        .measured_executions;
    assert_ne!(a_bars, b_bars);
    a.frame();
}

// MARK: What a profile is of

/// A rig with a ready profile of `SOURCE` over `bars`.
fn ready_on(tag: &str, bars: Vec<crate::model::Kline>) -> Rig {
    let mut rig = Rig::new(tag);
    rig.set_source(SOURCE);
    rig.manager.preview.set_candles_for_test(bars);
    rig.manager.cmds.push(Cmd::ProfilerToggle);
    until(&mut rig, "a profile", |s| *s == Status::Ready);
    rig
}

fn stale_after(rig: &mut Rig, what: &str) {
    rig.frame();
    assert_eq!(status(rig), Status::Stale, "{what} makes a profile stale");
}

#[test]
fn a_changed_price_on_an_unchanged_timestamp_makes_the_profile_stale() {
    let mut data = crate::pine_studio::debug::studio_tests::bars(45);
    let mut rig = ready_on("profiler-ohlcv", data.clone());
    data[10].close += 1.0;
    rig.manager.preview.set_candles_for_test(data);
    stale_after(&mut rig, "a corrected close");
}

#[test]
fn a_changed_close_time_or_volume_makes_the_profile_stale() {
    let mut data = crate::pine_studio::debug::studio_tests::bars(45);
    let mut rig = ready_on("profiler-close-time", data.clone());
    data[44].close_time += 1;
    rig.manager.preview.set_candles_for_test(data.clone());
    stale_after(&mut rig, "a later close time");
    rig.manager.cmds.push(Cmd::ProfilerRun);
    until(&mut rig, "a new profile", |s| *s == Status::Ready);
    data[44].volume += 5.0;
    rig.manager.preview.set_candles_for_test(data);
    stale_after(&mut rig, "a changed volume");
}

#[test]
fn a_forming_candle_that_closes_makes_the_profile_stale() {
    let rig = Rig::new("profiler-closes");
    let preview = &rig.manager.preview;
    let last_close = crate::pine_studio::debug::studio_tests::bars(40)
        .last()
        .unwrap()
        .close_time;
    let forming = preview.run_identity("", last_close - 1, false);
    let closed = preview.run_identity("", last_close, false);
    assert_ne!(forming, closed);
    assert_eq!(forming, preview.run_identity("", last_close - 2, false));
}

#[test]
fn the_theme_the_chart_is_drawn_on_is_part_of_what_a_profile_is_of() {
    let rig = Rig::new("profiler-theme");
    let preview = &rig.manager.preview;
    assert_ne!(
        preview.run_identity("", 0, true),
        preview.run_identity("", 0, false)
    );
}

#[test]
fn the_same_ticker_on_another_source_is_another_profile() {
    use crate::model::DataSource;
    let mut rig = ready_on(
        "profiler-source",
        crate::pine_studio::debug::studio_tests::bars(45),
    );
    // The preview opens on a Binance market.
    let other = DataSource::CoinGecko;
    rig.manager.preview.select_source_for_test(other);
    stale_after(&mut rig, "another data source");
}

#[test]
fn changing_an_input_makes_the_profile_stale() {
    use crate::pine::compiler::program::InputValue;
    let mut rig = ready_on(
        "profiler-input",
        crate::pine_studio::debug::studio_tests::bars(45),
    );
    rig.manager
        .preview
        .set_input_for_test("Length", InputValue::Int(7));
    stale_after(&mut rig, "an input");
}

#[test]
fn an_unchanged_ready_profile_stays_current_and_its_key_is_plain_data() {
    fn copy<T: Copy>() {}
    copy::<super::ProfileKey>();
    let mut rig = ready_on(
        "profiler-steady",
        crate::pine_studio::debug::studio_tests::bars(45),
    );
    let key = rig.manager.profiler_now;
    for _ in 0..20 {
        rig.frame();
    }
    assert_eq!(status(&rig), Status::Ready);
    assert_eq!(rig.manager.profiler_now, key);
}

// MARK: The visible range

/// Counts the bars whose open time lies between the two visible-bar-time builtins: line 5 runs
/// once for each.
const READS_RANGE: &str = "//@version=6
indicator(\"Visible\")
x = 0.0
if time >= chart.left_visible_bar_time and time <= chart.right_visible_bar_time
    x := close
plot(x)
";

#[test]
fn a_script_that_reads_the_visible_range_is_profiled_over_the_range_on_screen() {
    let mut rig = Rig::new("profiler-range");
    rig.set_source(READS_RANGE);
    rig.manager
        .preview
        .set_candles_for_test(crate::pine_studio::debug::studio_tests::bars(40));
    rig.manager.preview.set_visible_candles_for_test(12);
    let feed = rig
        .manager
        .preview
        .debug_feed(crate::pine_studio::model::now_ms(), false)
        .expect("a feed");
    let (left, right) = feed.visible_range.expect("a range");
    let inside = feed
        .bars
        .iter()
        .filter(|bar| bar.open_time >= left && bar.open_time <= right)
        .count();
    assert!(inside < 40, "the range is not the whole series");
    rig.manager.cmds.push(Cmd::ProfilerToggle);
    until(&mut rig, "a profile", |s| *s == Status::Ready);
    assert_eq!(executions(&rig, 5), inside.to_string());
}

#[test]
fn panning_makes_only_a_script_that_reads_the_range_stale() {
    let rig = Rig::new("profiler-pan");
    let mut preview_rig = rig;
    preview_rig
        .manager
        .preview
        .set_candles_for_test(crate::pine_studio::debug::studio_tests::bars(40));
    let before_reader = preview_rig
        .manager
        .preview
        .run_identity(READS_RANGE, 0, false);
    let before_plain = preview_rig.manager.preview.run_identity(SOURCE, 0, false);
    preview_rig.manager.preview.set_visible_candles_for_test(8);
    assert_ne!(
        before_reader,
        preview_rig
            .manager
            .preview
            .run_identity(READS_RANGE, 0, false)
    );
    assert_eq!(
        before_plain,
        preview_rig.manager.preview.run_identity(SOURCE, 0, false)
    );
}

// MARK: Failures and the data they happened on

const INPUT_FAILS: &str = "//@version=6
indicator(\"Input fails\")
n = input.int(5, \"Length\")
var values = array.new_float()
x = 0.0
if barstate.islast
    x := array.get(values, n)
plot(close + x)
";

#[test]
fn a_failure_is_not_a_problem_once_an_input_or_the_data_has_changed() {
    use crate::pine::compiler::program::InputValue;
    let mut rig = Rig::new("profiler-failure-fresh");
    rig.set_source(INPUT_FAILS);
    rig.manager.cmds.push(Cmd::ProfilerToggle);
    until(&mut rig, "the failure", |s| matches!(s, Status::Failed(_)));
    let listed = |rig: &Rig| {
        rig.manager
            .profiler
            .failure(rig.manager.profiler_now.as_ref())
            .is_some()
    };
    assert!(listed(&rig), "the failure of this run is a problem");
    rig.manager
        .preview
        .set_input_for_test("Length", InputValue::Int(0));
    rig.frame();
    assert!(!listed(&rig), "the input is not the one that failed");
    assert!(!matches!(status(&rig), Status::Failed(_)));
}

#[test]
fn a_new_candle_neither_cures_nor_repeats_a_refusal_but_a_new_text_does() {
    let mut rig = Rig::new("profiler-refusal");
    rig.set_source("//@version=6\nindicator(\"Bad\")\nx = nope(\nplot(x)\n");
    rig.manager.cmds.push(Cmd::ProfilerToggle);
    until(&mut rig, "a refusal", |s| matches!(s, Status::Failed(_)));
    rig.manager.preview.set_candles_for_test({
        let mut data = crate::pine_studio::debug::studio_tests::bars(40);
        data[39].close += 3.0;
        data
    });
    rig.frame();
    assert!(
        matches!(status(&rig), Status::Failed(_)),
        "still the same refusal, about the same text"
    );
    assert!(
        !rig.manager.profiler.wants_start(),
        "and not asked again for a candle"
    );
    rig.set_source(SOURCE);
    until(&mut rig, "a profile of the fixed script", |s| {
        *s == Status::Ready
    });
}
