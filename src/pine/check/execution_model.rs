//! Warnings about calls that skip bars, which TradingView raises as `CW10004`.
//!
//! A `ta.*` function keeps its state from the bars it was called on. Called from a ternary arm, the
//! right side of `and`/`or`, an `if`/`switch` branch or a loop body, it is not called on every
//! calculation, so its state (and the history it reads) has holes, which TradingView warns about:
//! `The function "ta.sma" should be called on each calculation for consistency`.
//!
//! Which calls count: a `ta.*` function that keeps state per call site (the families
//! `call_binding::route_of` sends to the `ta` tables), except `ta.tr`, which only reads the previous
//! bar. The body of a user function is not a conditional scope by itself (a function is called from
//! somewhere that decides), but a branch or a loop inside it is. The condition of a ternary, `if` or
//! `while` runs on every calculation of its scope, as does the left side of `and`/`or`.

use crate::pine::runtime::call_binding::route_of;
use crate::pine::syntax::ast::{BinaryOperator, Expression, Route, Statement};
use crate::pine::syntax::traversal::{children, expressions};
use crate::pine::{Diagnostic, DiagnosticCategory, DiagnosticSeverity};

const CODE: &str = "CW10004";

/// The warnings for `statements`, a script's top level.
pub fn validate(statements: &[Statement]) -> Vec<Diagnostic> {
    let mut walker = Walker {
        warnings: Vec::new(),
    };
    walker.block(statements, false);
    walker.warnings
}

struct Walker {
    warnings: Vec<Diagnostic>,
}

impl Walker {
    /// `conditional`: the block runs only when something decided to enter it.
    fn block(&mut self, statements: &[Statement], conditional: bool) {
        for statement in statements {
            self.statement(statement, conditional);
        }
    }

    fn statement(&mut self, statement: &Statement, conditional: bool) {
        match statement {
            Statement::Conditional {
                condition,
                when_true,
                when_false,
                ..
            } => {
                self.expression(condition, conditional);
                self.block(when_true, true);
                self.block(when_false, true);
            }
            Statement::ForRange {
                from,
                to,
                step,
                body,
                ..
            } => {
                for bound in [from, to].into_iter().chain(step) {
                    self.expression(bound, conditional);
                }
                self.block(body, true);
            }
            Statement::ForIn {
                collection, body, ..
            } => {
                self.expression(collection, conditional);
                self.block(body, true);
            }
            Statement::WhileLoop {
                condition, body, ..
            } => {
                self.expression(condition, true);
                self.block(body, true);
            }
            Statement::Once {
                condition, body, ..
            } => {
                if let Some(condition) = condition {
                    self.expression(condition, conditional);
                }
                self.block(body, true);
            }
            Statement::Switch { subject, arms, .. } => {
                if let Some(subject) = subject {
                    self.expression(subject, conditional);
                }
                for (index, arm) in arms.iter().enumerate() {
                    if let Some(condition) = &arm.condition {
                        // The first arm's test always runs; the others only when nothing matched.
                        self.expression(condition, conditional || index > 0);
                    }
                    self.block(&arm.body, true);
                }
            }
            Statement::Function { body, .. } => self.block(body, false),
            other => {
                for expression in expressions(other) {
                    self.expression(expression, conditional);
                }
            }
        }
    }

    fn expression(&mut self, expression: &Expression, conditional: bool) {
        match expression {
            Expression::Ternary {
                condition,
                when_true,
                when_false,
                ..
            } => {
                self.expression(condition, conditional);
                self.expression(when_true, true);
                self.expression(when_false, true);
            }
            Expression::Binary {
                lhs,
                op: BinaryOperator::And | BinaryOperator::Or,
                rhs,
                ..
            } => {
                self.expression(lhs, conditional);
                self.expression(rhs, true);
            }
            Expression::Statement(statement, _) => self.statement(statement, conditional),
            Expression::Call {
                name,
                arguments,
                range,
                ..
            } => {
                if conditional && keeps_state(name) {
                    self.warnings.push(Diagnostic {
                        code: CODE.to_owned(),
                        severity: DiagnosticSeverity::Warning,
                        category: DiagnosticCategory::Semantic,
                        message: format!(
                            "The function \"{name}\" should be called on each calculation for consistency. It is recommended to extract the call from the ternary operator or from the scope"
                        ),
                        range: *range,
                    });
                }
                for argument in arguments {
                    self.expression(&argument.value, conditional);
                }
            }
            other => {
                for child in children(other) {
                    self.expression(child, conditional);
                }
            }
        }
    }
}

/// Whether a call to `name` keeps state from the bars it ran on.
fn keeps_state(name: &str) -> bool {
    name.starts_with("ta.")
        && name != "ta.tr"
        && matches!(
            route_of(name),
            Some(Route::TaTable | Route::TaExtra | Route::TaInline)
        )
}

#[cfg(test)]
mod tests {
    use crate::pine::compiler::{self, NoLibraries};
    use crate::pine::syntax::token::{DiagnosticSeverity, Limits};

    fn warnings(body: &str) -> Vec<String> {
        let source = format!("//@version=6\nindicator(\"T\")\n{body}");
        compiler::compile(&source, Limits::default(), &NoLibraries)
            .diagnostics
            .into_iter()
            .filter(|d| d.code == "CW10004")
            .inspect(|d| assert_eq!(d.severity, DiagnosticSeverity::Warning))
            .map(|d| d.message)
            .collect()
    }

    fn message(function: &str) -> String {
        format!(
            "The function \"{function}\" should be called on each calculation for consistency. It is recommended to extract the call from the ternary operator or from the scope"
        )
    }

    #[test]
    fn a_stateful_call_in_a_ternary_arm_warns_with_trading_views_text() {
        let found = warnings("plot(close > open ? ta.sma(close, 3) : na)");
        assert_eq!(found, [message("ta.sma")]);
        assert_eq!(
            warnings("plot(close > open ? na : ta.sma(close, 3))"),
            [message("ta.sma")]
        );
    }

    #[test]
    fn the_other_lazy_places_warn_too() {
        for body in [
            "plot(close > open and ta.sma(close, 3) > 1 ? 1 : 0)",
            "plot(close > open or ta.sma(close, 3) > 1 ? 1 : 0)",
            "x = if close > open\n    ta.sma(close, 3)\nelse\n    0.0\nplot(x)",
            "x = switch\n    close > open => ta.sma(close, 3)\n    => 0.0\nplot(x)",
            "if close > open\n    plot(ta.sma(close, 3))",
            "if close > open\n    na\nelse\n    plot(ta.sma(close, 3))",
            "for i = 0 to 2\n    plot(ta.sma(close, 3))",
            "for v in array.from(1.0)\n    plot(ta.sma(close, 3))",
            "while bar_index < 0\n    plot(ta.sma(close, 3))",
            "plot(close > open ? ta.sma(close, 3)[1] : 0)",
            "plot(close > open ? math.abs(ta.sma(close, 3)) : 0)",
            "f(x) =>\n    x > 0 ? ta.sma(close, 3) : 0.0\nplot(f(1))",
            "f(x) =>\n    if x > 0\n        ta.sma(close, 3)\n    else\n        0.0\nplot(f(1))",
        ] {
            assert_eq!(warnings(body), [message("ta.sma")], "{body}");
        }
    }

    #[test]
    fn the_families_of_stateful_functions_all_warn() {
        for call in [
            "ta.ema(close, 3)",
            "ta.change(close)",
            "ta.crossover(close, open) ? 1.0 : 0.0",
            "ta.atr(3)",
            "ta.cum(close)",
            "ta.highest(close, 3)",
            "ta.rsi(close, 3)",
        ] {
            let found = warnings(&format!("plot(close > open ? {call} : 0.0)"));
            assert_eq!(found.len(), 1, "{call}: {found:?}");
        }
    }

    #[test]
    fn every_call_in_the_lazy_place_is_named() {
        let found = warnings("plot(close > open ? ta.sma(ta.ema(close, 2), 3) : 0.0)");
        assert_eq!(found, [message("ta.sma"), message("ta.ema")]);
    }

    #[test]
    fn calls_made_on_every_calculation_do_not_warn() {
        for body in [
            "plot(ta.sma(close, 3))",
            "avg = ta.sma(close, 3)\nplot(close > open ? avg : 0.0)",
            "plot(ta.sma(close, 3) > 1 ? 1 : 0)",
            "plot(ta.sma(close, 3) > 1 and close > open ? 1 : 0)",
            "x = ta.sma(close, 3) > 1 or close > open\nplot(x ? 1 : 0)",
            "if ta.crossover(close, open)\n    plot(1)",
            "x = if ta.crossover(close, open)\n    1.0\nelse\n    0.0\nplot(x)",
            "f(x) =>\n    ta.sma(x, 3)\nplot(f(close))",
            "for i = 0 to ta.highest(close, 3)\n    plot(i)",
            "x = switch\n    ta.crossover(close, open) => 1.0\n    => 0.0\nplot(x)",
            "plot(close > open ? ta.tr : 0.0)",
            "plot(close > open ? ta.tr(true) : 0.0)",
            "plot(close > open ? math.max(close, open) : 0.0)",
        ] {
            assert!(warnings(body).is_empty(), "{body}: {:?}", warnings(body));
        }
    }
}
