//! The profiler through the whole Script Manager window: turning it on, the run on the preview
//! chart's bars, what the editor and the drawer show, staleness, failure, cancellation, and the
//! run's end with the script, the profiler and the window.

use std::time::{Duration, Instant};

use super::debug_tests::Rig;
use super::prefs::DrawerTab;
use super::profile_marks::ProfileColumn;
use super::profiler::Status;
use super::*;

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
        message.starts_with("The script failed after") && message.contains("out of bounds"),
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
        .set_candles_for_test(super::debug_tests::bars(45));
    until(&mut rig, "the run that was asked for", |s| {
        *s == Status::Ready
    });
    assert_eq!(
        rig.manager
            .profiler
            .profile()
            .unwrap()
            .key
            .bars
            .map(|b| b.0),
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
            .is_none_or(|p| p.key.script_id == other),
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
