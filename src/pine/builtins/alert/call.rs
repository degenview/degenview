//! Argument rules for `alert()` and `alertcondition()`, ported from `PineAlertCallValidator.swift`.

use super::frequency::AlertFrequency;
use crate::pine::syntax::ast::{Argument, Expression, Literal, Statement};
use crate::pine::syntax::traversal::for_each_call_with_args;
use crate::pine::{Diagnostic, DiagnosticCategory, SourceRange};

/// One `alert()` or `alertcondition()` call in a script's source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AlertCallSite {
    /// How often the call may fire. None when the script computes it, so only a run can say.
    pub frequency: Option<AlertFrequency>,
    /// `alertcondition()` rather than `alert()`.
    pub is_condition: bool,
    pub range: SourceRange,
    /// An `alertcondition()` title, when it is a plain string literal.
    pub title: Option<String>,
    /// The message, when it is a plain string literal.
    pub message: Option<String>,
}

const ALERT_PARAMETERS: [&str; 2] = ["message", "freq"];
const ALERT_CONDITION_PARAMETERS: [&str; 3] = ["condition", "title", "message"];

pub fn validate(statements: &[Statement]) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for_each_call_with_args(statements, &mut |name, arguments, range| match name {
        "alert" => check_alert(arguments, range, &mut diagnostics),
        "alertcondition" => check_alert_condition(arguments, range, &mut diagnostics),
        _ => {}
    });
    diagnostics
}

/// Every alert call in source order.
pub fn call_sites(statements: &[Statement]) -> Vec<AlertCallSite> {
    let mut sites = Vec::new();
    for_each_call_with_args(statements, &mut |name, arguments, range| match name {
        "alert" => {
            let frequency = argument("freq", 1, arguments)
                .map(|argument| resolved_frequency(&argument.value))
                .unwrap_or(Some(AlertFrequency::OncePerBar));
            sites.push(AlertCallSite {
                frequency,
                is_condition: false,
                range,
                title: None,
                message: string_literal(argument("message", 0, arguments)),
            });
        }
        "alertcondition" => sites.push(AlertCallSite {
            frequency: Some(AlertFrequency::All),
            is_condition: true,
            range,
            title: string_literal(argument("title", 1, arguments)),
            message: string_literal(argument("message", 2, arguments)),
        }),
        _ => {}
    });
    sites
}

/// The frequency `expression` names, when it is a constant or a literal of one.
pub fn resolved_frequency(expression: &Expression) -> Option<AlertFrequency> {
    match expression {
        Expression::Identifier(name, _) => AlertFrequency::from_pine_name(name),
        Expression::Literal(Literal::String(name), _) => AlertFrequency::from_pine_name(name),
        _ => None,
    }
}

/// The text of `argument` when it is a plain, non-blank string literal.
fn string_literal(argument: Option<&Argument>) -> Option<String> {
    let Expression::Literal(Literal::String(text), _) = &argument?.value else {
        return None;
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

/// The argument bound to `parameter`: by name, or by `position` among those passed without one.
fn argument<'a>(
    parameter: &str,
    position: usize,
    arguments: &'a [Argument],
) -> Option<&'a Argument> {
    if let Some(named) = arguments
        .iter()
        .find(|argument| argument.name.as_deref() == Some(parameter))
    {
        return Some(named);
    }
    let positional: Vec<&Argument> = arguments
        .iter()
        .filter(|argument| argument.name.is_none())
        .collect();
    positional.get(position).copied()
}

fn check_alert(arguments: &[Argument], range: SourceRange, diagnostics: &mut Vec<Diagnostic>) {
    if arguments.is_empty() || arguments.len() > ALERT_PARAMETERS.len() {
        report(
            diagnostics,
            "PINE3025",
            "alert() takes a message and an optional frequency.",
            range,
        );
    }
    check_names(arguments, &ALERT_PARAMETERS, "alert", range, diagnostics);
    let Some(frequency) = argument("freq", 1, arguments) else {
        return;
    };
    match &frequency.value {
        Expression::Identifier(name, at) if name.starts_with(AlertFrequency::PINE_PREFIX) => {
            if AlertFrequency::from_pine_name(name).is_none() {
                report_frequency(*at, diagnostics);
            }
        }
        Expression::Literal(value, at) => {
            let valid = matches!(value, Literal::String(name) if AlertFrequency::from_pine_name(name).is_some());
            if !valid {
                report_frequency(*at, diagnostics);
            }
        }
        _ => {}
    }
}

fn check_alert_condition(
    arguments: &[Argument],
    range: SourceRange,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if arguments.is_empty() || arguments.len() > ALERT_CONDITION_PARAMETERS.len() {
        report(
            diagnostics,
            "PINE3026",
            "alertcondition() takes a condition, a title and a message.",
            range,
        );
    }
    check_names(
        arguments,
        &ALERT_CONDITION_PARAMETERS,
        "alertcondition",
        range,
        diagnostics,
    );
}

fn check_names(
    arguments: &[Argument],
    allowed: &[&str],
    function: &str,
    range: SourceRange,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for name in arguments
        .iter()
        .filter_map(|argument| argument.name.as_deref())
    {
        if !allowed.contains(&name) {
            report(
                diagnostics,
                "PINE3027",
                format!("{function}() has no parameter '{name}'."),
                range,
            );
        }
    }
}

fn report_frequency(range: SourceRange, diagnostics: &mut Vec<Diagnostic>) {
    report(
        diagnostics,
        "PINE3028",
        "Alert frequency must be alert.freq_all, alert.freq_once_per_bar or alert.freq_once_per_bar_close.",
        range,
    );
}

fn report(
    diagnostics: &mut Vec<Diagnostic>,
    code: &str,
    message: impl Into<String>,
    range: SourceRange,
) {
    diagnostics.push(Diagnostic::error(
        code,
        DiagnosticCategory::Semantic,
        message,
        range,
    ));
}
