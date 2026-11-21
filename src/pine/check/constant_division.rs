//! `x / 0`: TradingView refuses a division whose divisor is a literal zero (error CE10224, "Division
//! by zero", probe V7 U1 and a follow-up: `1.0 / 0.0` and `close / 0` do not compile, `close % 0` and a
//! divisor that is only zero at run time do). A zero that comes from a variable the script never
//! assigns (`float z = 0.0` then `close / z`) is refused there too; that is not checked here.

use crate::pine::syntax::ast::Statement;
use crate::pine::syntax::ast::{BinaryOperator, Expression, Literal, UnaryOperator};
use crate::pine::syntax::traversal::{children, expressions, for_each_nested_block};
use crate::pine::{Diagnostic, DiagnosticCategory};

const DIVISION_BY_ZERO: &str = "CE10224";

pub fn validate(statements: &[Statement]) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    visit_block(statements, &mut diagnostics);
    diagnostics.sort_by_key(|diagnostic| diagnostic.range.start.offset);
    diagnostics
}

fn visit_block(statements: &[Statement], diagnostics: &mut Vec<Diagnostic>) {
    for statement in statements {
        for expression in expressions(statement) {
            visit_expression(expression, diagnostics);
        }
        for_each_nested_block(statement, |block| visit_block(block, diagnostics));
    }
}

fn visit_expression(expression: &Expression, diagnostics: &mut Vec<Diagnostic>) {
    if let Expression::Binary {
        op: BinaryOperator::Divide,
        rhs,
        range,
        ..
    } = expression
        && is_zero(rhs)
    {
        diagnostics.push(Diagnostic::error(
            DIVISION_BY_ZERO,
            DiagnosticCategory::Semantic,
            "Division by zero".to_string(),
            *range,
        ));
    }
    if let Expression::Statement(statement, _) = expression {
        visit_block(std::slice::from_ref(statement.as_ref()), diagnostics);
    }
    for child in children(expression) {
        visit_expression(child, diagnostics);
    }
}

/// A literal `0` or `0.0`, signed or in parentheses.
fn is_zero(expression: &Expression) -> bool {
    match expression {
        Expression::Literal(Literal::Int(0), _) => true,
        Expression::Literal(Literal::Float(value), _) => *value == 0.0,
        Expression::Unary {
            op: UnaryOperator::Negate,
            operand,
            ..
        } => is_zero(operand),
        _ => false,
    }
}
