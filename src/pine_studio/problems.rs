//! The one list of what is wrong with the script in the editor. A script can fail in two ways: the
//! compiler rejects it, or it compiles and then fails when it runs (an unknown function, an index
//! out of range). The Problems tab, its count, the chip under the editor and the underline in the
//! text all show the same list, so a failure is never in one place and missing from another.
//!
//! Pure: no egui. Where the failures come from and whether they are still about the text on screen
//! is decided by the caller (`PreviewState::runtime_error_for`, `Profiler::failure`).

use crate::pine::token::{Diagnostic, DiagnosticSeverity};

/// The runtime failures to show beside `compile`: what the chart's run reported, then what the
/// profile's run reported, unless it is the same failure.
///
/// Nothing is added while the compile list has an error: the chart and the profile then ran an
/// earlier text, and their failure says nothing about this one.
pub fn runtime_problems(
    compile: &[Diagnostic],
    chart: Option<&Diagnostic>,
    profile: Option<&Diagnostic>,
) -> Vec<Diagnostic> {
    if compile
        .iter()
        .any(|d| d.severity == DiagnosticSeverity::Error)
    {
        return Vec::new();
    }
    let mut out: Vec<Diagnostic> = Vec::new();
    for failure in chart.into_iter().chain(profile) {
        if !out.iter().any(|known| known.id() == failure.id()) {
            out.push(failure.clone());
        }
    }
    out
}

/// The compile diagnostics, then the runtime failures.
pub fn combined(compile: &[Diagnostic], runtime: &[Diagnostic]) -> Vec<Diagnostic> {
    compile.iter().chain(runtime).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::token::{DiagnosticCategory, SourceRange};

    fn diagnostic(severity: DiagnosticSeverity, message: &str) -> Diagnostic {
        Diagnostic {
            code: "PINE0001".to_owned(),
            severity,
            category: DiagnosticCategory::Runtime,
            message: message.to_owned(),
            range: SourceRange::ZERO,
        }
    }

    fn error(message: &str) -> Diagnostic {
        diagnostic(DiagnosticSeverity::Error, message)
    }

    fn warning(message: &str) -> Diagnostic {
        diagnostic(DiagnosticSeverity::Warning, message)
    }

    #[test]
    fn a_clean_compile_with_no_failures_has_no_runtime_problems() {
        assert!(runtime_problems(&[], None, None).is_empty());
        assert!(runtime_problems(&[warning("w")], None, None).is_empty());
    }

    #[test]
    fn the_chart_and_the_profile_failures_are_listed_in_that_order() {
        let (chart, profile) = (error("chart"), error("profile"));
        let list = runtime_problems(&[], Some(&chart), Some(&profile));
        assert_eq!(list, vec![chart, profile]);
    }

    #[test]
    fn a_failure_both_runs_reported_is_listed_once() {
        let failure = error("Unknown or unsupported function 'array.resize'.");
        let list = runtime_problems(&[], Some(&failure), Some(&failure.clone()));
        assert_eq!(list, vec![failure]);
    }

    #[test]
    fn either_run_alone_is_enough() {
        let failure = error("boom");
        assert_eq!(runtime_problems(&[], Some(&failure), None).len(), 1);
        assert_eq!(runtime_problems(&[], None, Some(&failure)).len(), 1);
    }

    #[test]
    fn a_compile_error_hides_runtime_failures_of_older_text() {
        let failure = error("boom");
        let compile = [error("syntax")];
        assert!(runtime_problems(&compile, Some(&failure), Some(&failure)).is_empty());
        // A compile warning does not: the text compiles and ran.
        let list = runtime_problems(&[warning("w")], Some(&failure), None);
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn the_combined_list_is_compile_first_then_runtime() {
        let compile = [warning("w")];
        let runtime = [error("boom")];
        let list = combined(&compile, &runtime);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].message, "w");
        assert_eq!(list[1].message, "boom");
        assert!(combined(&[], &[]).is_empty());
    }
}
