//! AST traversal helpers, ported from `PineSyntaxTraversal.swift`.

use super::ast::{Expression, Statement};
use crate::pine::SourceRange;

/// Direct subexpressions. A `Statement`'s body is statements, reached through `nested_blocks`.
pub fn children(expression: &Expression) -> Vec<&Expression> {
    match expression {
        Expression::Literal(_, _) | Expression::Identifier(_, _) | Expression::Statement(_, _) => {
            Vec::new()
        }
        Expression::Unary { operand, .. } => vec![operand.as_ref()],
        Expression::Binary { lhs, rhs, .. } => vec![lhs.as_ref(), rhs.as_ref()],
        Expression::Ternary {
            condition,
            when_true,
            when_false,
            ..
        } => vec![condition.as_ref(), when_true.as_ref(), when_false.as_ref()],
        Expression::Call { arguments, .. } => arguments.iter().map(|a| &a.value).collect(),
        Expression::History { base, offset, .. } => vec![base.as_ref(), offset.as_ref()],
        Expression::Tuple(values, _) => values.iter().collect(),
        Expression::Member { base, .. } => vec![base.as_ref()],
        Expression::MethodCall {
            receiver,
            arguments,
            ..
        } => {
            let mut out = vec![receiver.as_ref()];
            out.extend(arguments.iter().map(|a| &a.value));
            out
        }
    }
}

/// Expressions this statement owns directly (not those of nested statements).
pub fn expressions(statement: &Statement) -> Vec<&Expression> {
    match statement {
        Statement::Declaration { value, .. }
        | Statement::Assignment { value, .. }
        | Statement::TupleDeclaration { value, .. } => vec![value],
        Statement::FieldAssignment { target, value, .. } => vec![target, value],
        Statement::Expression(expression) => vec![expression],
        Statement::Conditional { condition, .. } | Statement::WhileLoop { condition, .. } => {
            vec![condition]
        }
        Statement::ForRange { from, to, step, .. } => {
            let mut out = vec![from, to];
            if let Some(step) = step {
                out.push(step);
            }
            out
        }
        Statement::ForIn { collection, .. } => vec![collection],
        Statement::Switch { subject, arms, .. } => {
            let mut out = Vec::new();
            if let Some(subject) = subject {
                out.push(subject);
            }
            out.extend(arms.iter().filter_map(|arm| arm.condition.as_ref()));
            out
        }
        Statement::LoopControl(_, _) | Statement::Function { .. } => Vec::new(),
        Statement::TypeDeclaration { fields, .. } => fields
            .iter()
            .filter_map(|f| f.default_value.as_ref())
            .collect(),
        Statement::EnumDeclaration { .. } => Vec::new(),
    }
}

/// Directly nested statement lists: branches, loop and function bodies, switch arms.
pub fn nested_blocks(statement: &Statement) -> Vec<&[Statement]> {
    let mut blocks = Vec::new();
    for_each_nested_block(statement, |block| blocks.push(block));
    blocks
}

/// `nested_blocks` without building the list, for code that runs once per executed statement.
pub fn for_each_nested_block<'a>(statement: &'a Statement, mut visit: impl FnMut(&'a [Statement])) {
    match statement {
        Statement::Conditional {
            when_true,
            when_false,
            ..
        } => {
            visit(when_true);
            visit(when_false);
        }
        Statement::ForRange { body, .. }
        | Statement::ForIn { body, .. }
        | Statement::WhileLoop { body, .. }
        | Statement::Function { body, .. } => visit(body),
        Statement::Switch { arms, .. } => {
            for arm in arms {
                visit(&arm.body);
            }
        }
        _ => {}
    }
}

/// Names that are the target of `:=` or a compound assignment anywhere in `statements`.
pub fn assigned_names(statements: &[Statement]) -> std::collections::HashSet<String> {
    let mut names = std::collections::HashSet::new();
    for statement in statements {
        if let Statement::Assignment { name, .. } = statement {
            names.insert(name.clone());
        }
        for block in nested_blocks(statement) {
            names.extend(assigned_names(block));
        }
    }
    names
}

/// Visits every call expression in `statements`, at any depth.
pub fn for_each_call(statements: &[Statement], visit: &mut impl FnMut(&str, SourceRange)) {
    for_each_call_with_args(statements, &mut |name, _, range| visit(name, range));
}

/// Like `for_each_call`, with each call's arguments.
pub fn for_each_call_with_args(
    statements: &[Statement],
    visit: &mut impl FnMut(&str, &[super::ast::Argument], SourceRange),
) {
    for statement in statements {
        for expression in expressions(statement) {
            for_each_call_in_expression(expression, visit);
        }
        for block in nested_blocks(statement) {
            for_each_call_with_args(block, visit);
        }
    }
}

fn for_each_call_in_expression(
    expression: &Expression,
    visit: &mut impl FnMut(&str, &[super::ast::Argument], SourceRange),
) {
    if let Expression::Call {
        name,
        arguments,
        range,
        ..
    } = expression
    {
        visit(name, arguments, *range);
    }
    if let Expression::Statement(statement, _) = expression {
        for_each_call_with_args(std::slice::from_ref(statement.as_ref()), visit);
    }
    for child in children(expression) {
        for_each_call_in_expression(child, visit);
    }
}

/// Visits every plain or dotted name read in `statements`, at any depth.
pub fn for_each_identifier(statements: &[Statement], visit: &mut impl FnMut(&str, SourceRange)) {
    for statement in statements {
        for expression in expressions(statement) {
            for_each_identifier_in_expression(expression, visit);
        }
        for block in nested_blocks(statement) {
            for_each_identifier(block, visit);
        }
    }
}

fn for_each_identifier_in_expression(
    expression: &Expression,
    visit: &mut impl FnMut(&str, SourceRange),
) {
    if let Expression::Identifier(name, range) = expression {
        visit(name, *range);
    }
    if let Expression::Statement(statement, _) = expression {
        for_each_identifier(std::slice::from_ref(statement.as_ref()), visit);
    }
    for child in children(expression) {
        for_each_identifier_in_expression(child, visit);
    }
}

/// Visits every string literal in `statements`, at any depth.
pub fn for_each_string_literal(
    statements: &[Statement],
    visit: &mut impl FnMut(&str, SourceRange),
) {
    for statement in statements {
        for expression in expressions(statement) {
            for_each_string_literal_in_expression(expression, visit);
        }
        for block in nested_blocks(statement) {
            for_each_string_literal(block, visit);
        }
    }
}

fn for_each_string_literal_in_expression(
    expression: &Expression,
    visit: &mut impl FnMut(&str, SourceRange),
) {
    if let Expression::Literal(super::ast::Literal::String(text), range) = expression {
        visit(text, *range);
    }
    if let Expression::Statement(statement, _) = expression {
        for_each_string_literal(std::slice::from_ref(statement.as_ref()), visit);
    }
    for child in children(expression) {
        for_each_string_literal_in_expression(child, visit);
    }
}
