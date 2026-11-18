//! The statements a `request.security` expression needs to run on another series.
//!
//! `request.security(sym, tf, longTrend)` does not read the chart's `longTrend`: TradingView runs
//! the statements that compute it on the requested series. The session cannot run the whole script
//! there (it would draw and trade), so at build time each request gets the *slice* of top-level
//! statements that declare or reassign a name the expression reads, directly or through other
//! slice statements and the script's functions. The slice runs in the request's own state, in
//! source order, before the expression.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::pine::syntax::ast::{Argument, Expression, Statement};
use crate::pine::syntax::traversal;

/// The request functions whose third argument is the expression.
const REQUESTS: [&str; 2] = ["request.security", "request.security_lower_tf"];
/// Their parameters in order, the first three being `symbol`, `timeframe` and `expression`.
const EXPRESSION_POSITION: usize = 2;

/// Where an expression sits in the source (start and end offsets), which is how the runtime finds
/// the slice of the request it evaluates.
pub(crate) type ExpressionKey = (usize, usize);

pub(crate) type Slices = HashMap<ExpressionKey, Arc<Vec<Statement>>>;

pub(crate) fn key(expression: &Expression) -> ExpressionKey {
    let range = expression.range();
    (range.start.offset, range.end.offset)
}

/// The slice of every `request.security` / `request.security_lower_tf` call outside a function
/// (inside one, a parameter may share a name with a global), for the requests that read a name the
/// script declares at its top level.
pub(crate) fn build(statements: &[Statement]) -> Slices {
    let declared = declared_names(statements);
    if declared.is_empty() {
        return Slices::new();
    }
    let functions: Functions = statements
        .iter()
        .filter_map(|statement| match statement {
            Statement::Function { name, .. } => Some((name.as_str(), statement)),
            _ => None,
        })
        .collect();
    let facts: Vec<Facts> = statements
        .iter()
        .map(|statement| Facts::of(statement, &declared))
        .collect();

    let mut slices = Slices::new();
    for (index, statement) in statements.iter().enumerate() {
        if matches!(statement, Statement::Function { .. }) {
            continue;
        }
        traversal::for_each_call_with_args(
            std::slice::from_ref(statement),
            &mut |name, arguments, _| {
                let Some(expression) = expression_argument(name, arguments) else {
                    return;
                };
                let slice = slice_for(expression, index, statements, &facts, &functions, &declared);
                if !slice.is_empty() {
                    slices.insert(key(expression), Arc::new(slice));
                }
            },
        );
    }
    slices
}

/// The script's top-level functions and methods by name.
type Functions<'a> = HashMap<&'a str, &'a Statement>;

/// What one top-level statement writes (of the names the script declares) and reads.
struct Facts {
    writes: HashSet<String>,
    reads: HashSet<String>,
    /// Whether it calls a `request.*` function. Such a statement is never part of a slice: run
    /// inside another request it would nest requests, so a name it computes keeps the chart's value.
    requests: bool,
}

impl Facts {
    fn of(statement: &Statement, declared: &HashSet<String>) -> Self {
        let mut writes = HashSet::new();
        let mut reads = HashSet::new();
        let mut requests = false;
        if !matches!(statement, Statement::Function { .. }) {
            collect_writes(statement, false, &mut writes);
            writes.retain(|name| declared.contains(name));
            collect_reads(std::slice::from_ref(statement), &mut reads);
            traversal::for_each_call(std::slice::from_ref(statement), &mut |name, _| {
                requests |= name.starts_with("request.");
            });
        }
        Self {
            writes,
            reads,
            requests,
        }
    }
}

fn slice_for(
    expression: &Expression,
    own_statement: usize,
    statements: &[Statement],
    facts: &[Facts],
    functions: &Functions,
    declared: &HashSet<String>,
) -> Vec<Statement> {
    let mut needed = HashSet::new();
    read_names(expression, &mut needed);
    expand_functions(&mut needed, functions);
    needed.retain(|name| declared.contains(name));

    let mut included = vec![false; statements.len()];
    loop {
        let mut changed = false;
        for (index, fact) in facts.iter().enumerate() {
            // The statement holding the request would run the request again from inside itself.
            if included[index]
                || index == own_statement
                || fact.requests
                || fact.writes.is_disjoint(&needed)
            {
                continue;
            }
            included[index] = true;
            changed = true;
            needed.extend(fact.reads.iter().cloned());
            expand_functions(&mut needed, functions);
            needed.retain(|name| declared.contains(name));
        }
        if !changed {
            break;
        }
    }
    statements
        .iter()
        .zip(&included)
        .filter(|(_, included)| **included)
        .map(|(statement, _)| statement.clone())
        .collect()
}

/// The names a script declares at its top level: variables and tuple elements.
fn declared_names(statements: &[Statement]) -> HashSet<String> {
    let mut names = HashSet::new();
    for statement in statements {
        match statement {
            Statement::Declaration { name, .. } => {
                names.insert(name.clone());
            }
            Statement::TupleDeclaration { names: tuple, .. } => names.extend(tuple.iter().cloned()),
            _ => {}
        }
    }
    names
}

/// The `expression` argument of a request call, found the way the call binds its arguments: by
/// name, or as the third argument that has none.
fn expression_argument<'a>(name: &str, arguments: &'a [Argument]) -> Option<&'a Expression> {
    if !REQUESTS.contains(&name) {
        return None;
    }
    let mut position = 0;
    for argument in arguments {
        match argument.name.as_deref() {
            Some("expression") => return Some(&argument.value),
            Some(_) => {}
            None => {
                if position == EXPRESSION_POSITION {
                    return Some(&argument.value);
                }
                position += 1;
            }
        }
    }
    None
}

/// Names the statement declares at its top level, or reassigns anywhere inside it (`x := …`,
/// `x.field := …`); a declaration inside a block belongs to the block.
fn collect_writes(statement: &Statement, nested: bool, writes: &mut HashSet<String>) {
    match statement {
        Statement::Declaration { name, .. } if !nested => {
            writes.insert(name.clone());
        }
        Statement::TupleDeclaration { names, .. } if !nested => {
            writes.extend(names.iter().cloned())
        }
        Statement::Assignment { name, .. } => {
            writes.insert(name.clone());
        }
        Statement::FieldAssignment { target, .. } => {
            if let Some(root) = root_name(target) {
                writes.insert(root);
            }
        }
        _ => {}
    }
    for block in traversal::nested_blocks(statement) {
        for inner in block {
            collect_writes(inner, true, writes);
        }
    }
}

/// The variable at the start of `a.b[1].c`: `a`.
fn root_name(expression: &Expression) -> Option<String> {
    match expression {
        Expression::Identifier(name, _) | Expression::Call { name, .. } => {
            Some(head(name).to_owned())
        }
        Expression::Member { base, .. } | Expression::History { base, .. } => root_name(base),
        Expression::MethodCall { receiver, .. } => root_name(receiver),
        _ => None,
    }
}

fn head(name: &str) -> &str {
    name.split_once('.').map_or(name, |(head, _)| head)
}

fn collect_reads(statements: &[Statement], reads: &mut HashSet<String>) {
    for statement in statements {
        for expression in traversal::expressions(statement) {
            read_names(expression, reads);
        }
        for block in traversal::nested_blocks(statement) {
            collect_reads(block, reads);
        }
    }
}

/// Every name an expression may depend on: variables (a dotted path counts as its head), and the
/// callees, which may be the script's functions or methods.
fn read_names(expression: &Expression, reads: &mut HashSet<String>) {
    match expression {
        Expression::Identifier(name, _) => {
            reads.insert(name.clone());
            reads.insert(head(name).to_owned());
        }
        Expression::Call { name, .. } => {
            reads.insert(name.clone());
            if let Some((receiver, method)) = name.rsplit_once('.') {
                reads.insert(head(receiver).to_owned());
                reads.insert(method.to_owned());
            }
        }
        Expression::MethodCall { name, .. } => {
            reads.insert(name.clone());
        }
        Expression::Statement(statement, _) => {
            collect_reads(std::slice::from_ref(statement.as_ref()), reads);
        }
        _ => {}
    }
    for child in traversal::children(expression) {
        read_names(child, reads);
    }
}

/// Adds what the bodies of the script's functions named in `names` read, however deep the calls go.
/// A function's own parameters and locals are not reads of a global that happens to share the name.
fn expand_functions(names: &mut HashSet<String>, functions: &Functions) {
    let mut pending: Vec<&str> = names
        .iter()
        .filter_map(|name| functions.get_key_value(name.as_str()).map(|(key, _)| *key))
        .collect();
    let mut seen: HashSet<&str> = pending.iter().copied().collect();
    while let Some(function) = pending.pop() {
        let Statement::Function {
            parameters, body, ..
        } = functions[function]
        else {
            continue;
        };
        let mut reads = HashSet::new();
        collect_reads(body, &mut reads);
        let mut locals: HashSet<String> = parameters.iter().map(|p| p.name.clone()).collect();
        collect_locals(body, &mut locals);
        for read in reads.difference(&locals) {
            if let Some((key, _)) = functions.get_key_value(read.as_str())
                && seen.insert(key)
            {
                pending.push(key);
            }
            names.insert(read.clone());
        }
    }
}

/// The names the statements declare, at any depth: variables, tuple elements and loop variables.
fn collect_locals(statements: &[Statement], locals: &mut HashSet<String>) {
    for statement in statements {
        match statement {
            Statement::Declaration { name, .. } => {
                locals.insert(name.clone());
            }
            Statement::TupleDeclaration { names, .. } => locals.extend(names.iter().cloned()),
            Statement::ForRange { variable, .. } => {
                locals.insert(variable.clone());
            }
            Statement::ForIn { index, value, .. } => {
                locals.extend(index.iter().cloned());
                locals.insert(value.clone());
            }
            _ => {}
        }
        for block in traversal::nested_blocks(statement) {
            collect_locals(block, locals);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::compiler::{self, NoLibraries};
    use crate::pine::syntax::token::Limits;

    /// How many statements each request's slice has, in source order of the requests.
    fn slice_sizes(source: &str) -> Vec<usize> {
        let program = compiler::compile(source, Limits::default(), &NoLibraries);
        let slices = build(&program.statements);
        let mut sizes: Vec<(ExpressionKey, usize)> = slices
            .iter()
            .map(|(key, slice)| (*key, slice.len()))
            .collect();
        sizes.sort();
        sizes.into_iter().map(|(_, size)| size).collect()
    }

    #[test]
    fn a_variable_pulls_in_the_statements_that_compute_it() {
        let sizes = slice_sizes(
            "//@version=6\nindicator(\"T\")\nlength = 3\nbase = ta.sma(close, length)\nunused = open\ntrend = base - 1\nplot(request.security(syminfo.tickerid, \"D\", trend))",
        );
        // `length`, `base`, `trend`; not `unused`.
        assert_eq!(sizes, [3]);
    }

    #[test]
    fn a_reassignment_and_a_called_function_are_followed() {
        let sizes = slice_sizes(
            "//@version=6\nindicator(\"T\")\nfloor_ = 1.0\nlevel = 0.0\nif close > 2\n    level := close\nscaled(x) =>\n    x * floor_\nplot(request.security(syminfo.tickerid, \"D\", scaled(level)))",
        );
        // `floor_` (read by `scaled`), `level`, the `if` that reassigns it.
        assert_eq!(sizes, [3]);
    }

    #[test]
    fn builtins_and_the_requests_own_statement_need_no_slice() {
        assert!(
            slice_sizes(
                "//@version=6\nindicator(\"T\")\nx = request.security(syminfo.tickerid, \"D\", close)\nplot(x)"
            )
            .is_empty()
        );
        assert!(
            slice_sizes(
                "//@version=6\nindicator(\"T\")\nx = close\ny = request.security(syminfo.tickerid, \"D\", y[1])\nplot(y)"
            )
            .is_empty(),
            "a statement is not its own dependency"
        );
    }

    #[test]
    fn a_function_local_is_not_a_read_of_the_global_with_its_name() {
        let sizes = slice_sizes(
            "//@version=6\nindicator(\"T\")\nup = close > open\nf() =>\n    up = close < open\n    up ? 1 : 0\nplot(request.security(syminfo.tickerid, \"D\", f()))",
        );
        assert!(sizes.is_empty(), "{sizes:?}");
    }

    #[test]
    fn a_statement_that_makes_a_request_is_left_out() {
        let sizes = slice_sizes(
            "//@version=6\nindicator(\"T\")\ndaily = request.security(syminfo.tickerid, \"D\", close)\nscaled = daily * 2\nplot(request.security(syminfo.tickerid, \"W\", scaled))",
        );
        // `scaled` is computed on the weekly series from the chart's `daily`.
        assert_eq!(sizes, [1]);
    }

    #[test]
    fn the_expression_is_found_by_name_too() {
        let sizes = slice_sizes(
            "//@version=6\nindicator(\"T\")\nx = close * 2\nplot(request.security(syminfo.tickerid, \"D\", expression = x))",
        );
        assert_eq!(sizes, [1]);
    }
}
