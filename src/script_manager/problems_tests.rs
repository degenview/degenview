//! A script that compiles and then fails when it runs, through the whole Script Manager window: the
//! failure has to be listed, counted, shown in the chip and underlined in the text, the same as a
//! compile error, and has to go when the text changes.

use std::time::{Duration, Instant};

use super::debug_tests::Rig;
use super::editor::status_chip_with_runtime;
use super::*;
use crate::pine::token::DiagnosticSeverity;

/// `array.resize` is not Pine v6; the compiler lets the call through and the run rejects it, on the
/// last bar only: the shape of a failure that no compile error announces.
const FAILS_ON_THE_LAST_BAR: &str = "//@version=6
indicator(\"Fails\")
var values = array.new_float()
if barstate.islast
    array.resize(values, 5)
plot(close)
";

const FINE: &str = "//@version=6
indicator(\"Fine\")
plot(close)
";

fn until(rig: &mut Rig, what: &str, done: impl Fn(&Rig) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        rig.frame();
        if done(rig) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn editor(rig: &Rig) -> &editor::EditorState {
    &rig.manager.open.as_ref().expect("a script is open").editor
}

fn source(rig: &Rig) -> String {
    editor(rig).source.clone()
}

fn runtime(rig: &Rig) -> Vec<crate::pine::token::Diagnostic> {
    rig.manager
        .runtime_problems(&source(rig), &editor(rig).diagnostics)
}

/// A rig whose chart has run the failing script and reported it.
fn failing(tag: &str) -> Rig {
    let mut rig = Rig::new(tag);
    rig.set_source(FAILS_ON_THE_LAST_BAR);
    until(&mut rig, "the chart to report the failure", |rig| {
        rig.manager.preview.runtime_error().is_some()
    });
    rig.frame();
    rig
}

#[test]
fn the_failing_script_compiles_clean() {
    let rig = failing("problems-compiles");
    assert!(
        editor(&rig).diagnostics.is_empty(),
        "{:?}",
        editor(&rig).diagnostics
    );
    assert_eq!(editor(&rig).status, CompileStatus::Valid);
}

#[test]
fn a_failure_is_in_the_list_the_chip_and_the_editor() {
    let rig = failing("problems-everywhere");
    let failures = runtime(&rig);
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(
        failures[0].message.contains("array.resize"),
        "{}",
        failures[0].message
    );
    assert_eq!(failures[0].severity, DiagnosticSeverity::Error);

    // The Problems tab and its count read this list.
    let list = problems::combined(&editor(&rig).diagnostics, &failures);
    assert_eq!(list.len(), 1);

    // The editor was handed the failure this frame, and the chip counts it.
    assert_eq!(editor(&rig).runtime_problems().len(), 1);
    let (_, _, text) = status_chip_with_runtime(
        editor(&rig).status,
        &editor(&rig).diagnostics,
        editor(&rig).runtime_problems(),
    );
    assert_eq!(text, "1 error");
}

#[test]
fn the_failure_points_at_the_whole_failing_name() {
    let rig = failing("problems-position");
    let failure = &runtime(&rig)[0];
    let text = source(&rig);
    let from = syntax::compiled_utf16_to_char_index(&text, failure.range.start.offset);
    let to = syntax::compiled_utf16_to_char_index(&text, failure.range.end.offset);
    let marked: String = text.chars().skip(from).take(to - from).collect();
    // Not only `array`: the name that is not a function is `array.resize`.
    assert_eq!(marked, "array.resize");
}

#[test]
fn editing_the_script_takes_the_failure_away() {
    let mut rig = failing("problems-edit");
    rig.set_source(FINE);
    until(&mut rig, "the chart to run the new text", |rig| {
        runtime(rig).is_empty() && rig.manager.preview.runtime_error().is_none()
    });
    rig.frame();
    assert!(editor(&rig).runtime_problems().is_empty());
    let (_, _, text) = status_chip_with_runtime(
        editor(&rig).status,
        &editor(&rig).diagnostics,
        editor(&rig).runtime_problems(),
    );
    assert_eq!(text, "No problems");
}

#[test]
fn text_the_chart_has_not_run_does_not_inherit_the_old_failure() {
    let mut rig = failing("problems-stale");
    // Edited, and not yet applied to the chart: the old failure's offsets mean nothing here.
    let edited = format!("// a comment that moves everything down\n{FAILS_ON_THE_LAST_BAR}");
    rig.set_source(&edited);
    assert!(runtime(&rig).is_empty());
}

#[test]
fn a_compile_error_hides_the_failure_of_older_text() {
    let mut rig = failing("problems-compile-error");
    rig.set_source("//@version=6\nindicator(\"Broken\")\nplot(close\n");
    rig.frame();
    assert!(!editor(&rig).diagnostics.is_empty());
    assert!(runtime(&rig).is_empty());
    let list = problems::combined(&editor(&rig).diagnostics, &runtime(&rig));
    assert!(list.iter().all(|d| d.severity == DiagnosticSeverity::Error));
}

#[test]
fn a_failure_the_chart_kept_reporting_is_not_forgotten() {
    // The controller used to report a failure once and then read as a success: here the same
    // bars arrive again and again, and the failure has to stay.
    let mut rig = failing("problems-sticky");
    for _ in 0..30 {
        rig.frame();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(rig.manager.preview.runtime_error().is_some());
    assert_eq!(runtime(&rig).len(), 1);
}

#[test]
fn a_profile_that_failed_is_listed_once_beside_the_charts_failure() {
    let mut rig = failing("problems-profile");
    rig.manager.cmds.push(Cmd::ProfilerToggle);
    until(&mut rig, "the profile to finish", |rig| {
        rig.manager
            .profiler
            .failure(rig.manager.profiler_now.as_ref())
            .is_some()
    });
    let failures = runtime(&rig);
    assert_eq!(
        failures.len(),
        1,
        "the chart and the profile saw one failure: {failures:?}"
    );
    assert!(failures[0].message.contains("array.resize"));
}

#[test]
fn a_clean_script_has_no_problems_anywhere() {
    let mut rig = Rig::new("problems-clean");
    rig.set_source(FINE);
    for _ in 0..40 {
        rig.frame();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(runtime(&rig).is_empty());
    assert!(editor(&rig).runtime_problems().is_empty());
}
