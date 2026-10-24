//! Argument rules for `log.info()`, `log.warning()` and `log.error()`.
//!
//! Both overloads take a string first, `log.info(message)` and `log.info(formatString, arg0, ...)`.
//! Only what the source settles is checked here: a call without arguments, or one whose first
//! argument is a literal that is not a string. A value computed at run time is formatted as it is.

use super::ast::{Argument, Expression, Literal, Statement};
use super::logs::LogLevel;
use super::traversal::for_each_call_with_args;
use crate::pine::{Diagnostic, DiagnosticCategory, SourceRange};

pub fn validate(statements: &[Statement]) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for_each_call_with_args(statements, &mut |name, arguments, range| {
        if LogLevel::from_function(name).is_some() {
            check(name, arguments, range, &mut diagnostics);
        }
    });
    diagnostics
}

fn check(
    name: &str,
    arguments: &[Argument],
    range: SourceRange,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some(first) = arguments.first() else {
        report(
            diagnostics,
            format!("{name}() needs a message or a format string."),
            range,
        );
        return;
    };
    if let Expression::Literal(literal, at) = &first.value
        && !matches!(literal, Literal::String(_) | Literal::Na)
    {
        report(
            diagnostics,
            format!("{name}() takes a string as its message or format string."),
            *at,
        );
    }
}

fn report(diagnostics: &mut Vec<Diagnostic>, message: String, range: SourceRange) {
    diagnostics.push(Diagnostic::error(
        "PINE3052",
        DiagnosticCategory::Semantic,
        message,
        range,
    ));
}

#[cfg(test)]
mod tests {
    use crate::pine::Limits;
    use crate::pine::compiler::{NoLibraries, compile};

    fn codes(body: &str) -> Vec<String> {
        let source = format!("//@version=6\nindicator(\"T\")\n{body}\n");
        compile(&source, Limits::default(), &NoLibraries)
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code.clone())
            .collect()
    }

    fn rejects(body: &str) -> bool {
        codes(body).contains(&"PINE3052".to_owned())
    }

    #[test]
    fn both_overloads_of_every_level_compile() {
        for level in ["info", "warning", "error"] {
            assert!(!rejects(&format!("log.{level}(\"x\")")));
            assert!(!rejects(&format!("log.{level}(\"x {{0}}\", close)")));
        }
    }

    #[test]
    fn a_call_without_arguments_is_an_error() {
        assert!(rejects("log.info()"));
    }

    #[test]
    fn a_literal_that_is_not_a_string_is_an_error() {
        assert!(rejects("log.warning(1)"));
        assert!(rejects("log.error(true)"));
    }

    #[test]
    fn a_computed_first_argument_is_left_to_the_run() {
        assert!(!rejects("log.info(str.tostring(close))"));
        assert!(!rejects("t = \"x\"\nlog.info(t)"));
    }

    #[test]
    fn a_call_is_found_inside_functions_and_blocks() {
        assert!(rejects("f() =>\n    log.info()\n    1\nf()"));
        assert!(rejects("if close > open\n    log.error()"));
    }
}
