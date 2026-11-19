//! Where a request sits in the blocks around it, and which statements run before it there.

use super::ExpressionKey;
use crate::pine::syntax::ast::{Expression, Statement};
use crate::pine::syntax::traversal;

/// Where a request sits in the blocks around it.
#[derive(Default)]
pub(super) struct Location<'a> {
    /// The statements written before the request in those blocks, outermost block first.
    pub preceding: Vec<Preceding<'a>>,
}

pub(super) struct Preceding<'a> {
    pub statement: &'a Statement,
    /// Whether it runs once per bar: false inside a loop or a `once` block, which run it for each
    /// iteration or only once.
    pub replayable: bool,
}

/// The place of the expression at `key` in `statement`, a top-level statement. None when `key` is
/// not inside it.
pub(super) fn locate(statement: &Statement, key: ExpressionKey) -> Option<Location<'_>> {
    let mut location = Location::default();
    in_statement(statement, key, true, &mut location).then_some(location)
}

fn in_statement<'a>(
    statement: &'a Statement,
    key: ExpressionKey,
    replayable: bool,
    out: &mut Location<'a>,
) -> bool {
    if traversal::expressions(statement)
        .into_iter()
        .any(|expression| in_expression(expression, key, replayable, out))
    {
        return true;
    }
    let replayable = replayable
        && !matches!(
            statement,
            Statement::ForRange { .. }
                | Statement::ForIn { .. }
                | Statement::WhileLoop { .. }
                | Statement::Once { .. }
        );
    traversal::nested_blocks(statement)
        .into_iter()
        .any(|block| in_block(block, key, replayable, out))
}

fn in_block<'a>(
    block: &'a [Statement],
    key: ExpressionKey,
    replayable: bool,
    out: &mut Location<'a>,
) -> bool {
    let mut before = Vec::new();
    for statement in block {
        if in_statement(statement, key, replayable, out) {
            before.append(&mut out.preceding);
            out.preceding = before;
            return true;
        }
        before.push(Preceding {
            statement,
            replayable,
        });
    }
    false
}

fn in_expression<'a>(
    expression: &'a Expression,
    key: ExpressionKey,
    replayable: bool,
    out: &mut Location<'a>,
) -> bool {
    if super::key(expression) == key {
        return true;
    }
    if let Expression::Statement(statement, _) = expression
        && in_statement(statement, key, replayable, out)
    {
        return true;
    }
    traversal::children(expression)
        .into_iter()
        .any(|child| in_expression(child, key, replayable, out))
}
