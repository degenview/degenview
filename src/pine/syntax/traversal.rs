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
        Statement::Once { condition, .. } => condition.iter().collect(),
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
        | Statement::Once { body, .. }
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

/// Visits every call whose callee is a name, at any depth: builtins, the script's functions, and
/// `a.b(x)` on a plain variable path, which is a [`Call`](Expression::Call) named `"a.b"`. A method
/// called on anything else (`f().b(x)`) is a [`MethodCall`](Expression::MethodCall) and is not
/// visited.
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
    match expression {
        Expression::Call {
            name,
            arguments,
            range,
            ..
        } => visit(name, arguments, *range),
        Expression::Statement(statement, _) => {
            for_each_call_with_args(std::slice::from_ref(statement.as_ref()), visit);
        }
        _ => {}
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

/// Visits the name of every plain identifier that a history operator reads (`name` in `name[n]`),
/// at any depth. A dotted name is one identifier (`barstate.isfirst[1]`).
pub fn for_each_history_identifier(statements: &[Statement], visit: &mut impl FnMut(&str)) {
    for statement in statements {
        for expression in expressions(statement) {
            for_each_history_identifier_in_expression(expression, visit);
        }
        for block in nested_blocks(statement) {
            for_each_history_identifier(block, visit);
        }
    }
}

pub fn for_each_history_identifier_in_expression(
    expression: &Expression,
    visit: &mut impl FnMut(&str),
) {
    if let Expression::History { base, .. } = expression
        && let Expression::Identifier(name, _) = base.as_ref()
    {
        visit(name);
    }
    if let Expression::Statement(statement, _) = expression {
        for_each_history_identifier(std::slice::from_ref(statement.as_ref()), visit);
    }
    for child in children(expression) {
        for_each_history_identifier_in_expression(child, visit);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::{Lexer, Limits, Parser};
    use std::collections::{BTreeSet, HashSet};

    /// One numbered call in every place a statement or expression can hold one, plus both method
    /// forms.
    const FIXTURE: &str = "\
a = c1(0)
a := c2(0)
[p, q] = c3(0)
type T
    float v = c4(0)
t = T.new()
t.v := c5(0)
if c6(0)
    c7(0)
else
    c8(0)
for i = c9(0) to c10(0) by c11(0)
    c12(0)
for x in c13(0)
    c14(0)
while c15(0)
    c16(0)
switch c17(0)
    c18(0) => c19(0)
    => c20(0)
f(x) =>
    c21(x)
y = c22(0) ? c23(0) : c24(0)
z = -c25(0) + c26(0)[c27(0)]
w = [c28(0), c29(0)]
m = c30(0).name
n = c31(0).method(c32(0))
o = plainobj.method(c33(0))
k = if c34(0)
    c35(0)
else
    c36(0)
";

    fn parse(source: &str) -> Vec<Statement> {
        let lexed = Lexer::new(source).lex();
        assert!(lexed.diagnostics.is_empty(), "{:?}", lexed.diagnostics);
        let mut parser = Parser::new(lexed.tokens, Limits::default(), HashSet::new());
        let (statements, diagnostics) = parser.parse();
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        statements
    }

    #[test]
    fn every_call_in_every_position_is_visited() {
        let statements = parse(FIXTURE);
        let mut plain = BTreeSet::new();
        for_each_call(&statements, &mut |name, _| {
            plain.insert(name.to_owned());
        });
        for number in 1..=36 {
            assert!(
                plain.contains(&format!("c{number}")),
                "c{number} not visited"
            );
        }
        assert!(plain.contains("plainobj.method"), "{plain:?}");
        assert!(plain.contains("T.new"), "{plain:?}");
    }

    #[test]
    fn the_named_call_walkers_leave_out_a_method_on_a_computed_receiver() {
        let statements = parse(FIXTURE);
        let mut names = Vec::new();
        for_each_call(&statements, &mut |name, _| names.push(name.to_owned()));
        assert!(!names.iter().any(|name| name == "method"), "{names:?}");
        assert!(names.iter().any(|name| name == "plainobj.method"));
        let mut with_arguments = 0;
        for_each_call_with_args(&statements, &mut |_, _, _| with_arguments += 1);
        assert_eq!(with_arguments, names.len());
    }

    #[test]
    fn a_method_called_on_a_computed_receiver_is_not_mistaken_for_a_builtin() {
        let statements = parse("x = foo().plot(1)\n");
        let mut names = Vec::new();
        for_each_call(&statements, &mut |name, _| names.push(name.to_owned()));
        assert_eq!(names, vec!["foo".to_owned()]);
    }

    #[test]
    fn assigned_names_are_found_in_every_nested_block() {
        let statements = parse(
            "\
a = 0
b = 0
c = 0
d = 0
e = 0
f = 0
g = 0
if true
    a := 1
for i = 0 to 1
    b += 1
for x in [1]
    c -= 1
while false
    d := 2
switch
    true => e := 3
h() =>
    f := 4
",
        );
        let names = assigned_names(&statements);
        for name in ["a", "b", "c", "d", "e", "f"] {
            assert!(names.contains(name), "{name} missing from {names:?}");
        }
        assert!(!names.contains("g"));
    }
}
