//! The builtin variables a request records histories of.
//!
//! `ta.obv[1]` reads the builtin's value on the previous bar, which the session keeps by reading it
//! at the end of every confirmed bar (`Session::read_history_builtins`). The chart does that for
//! every name the script indexes anywhere; a request's series does not need all of them, only those
//! its own expression indexes: in the expression, in the statements of its slice, and in the bodies
//! of the script's functions and methods those reach. Reading a name the request never indexes would
//! compute it and store a trail nothing reads.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::members::expression_argument;
use super::{ExpressionKey, Slices, key};
use crate::pine::syntax::ast::{Expression, Statement};
use crate::pine::syntax::traversal;

/// For each request (by the position of its expression), the builtin variables it records.
pub(crate) type HistoryNames = HashMap<ExpressionKey, Arc<[String]>>;

/// The names of `script_wide` that each request outside the functions indexes. A request that is
/// not in the result (one inside a function) records `script_wide`.
pub(crate) fn history_names(
    statements: &[Statement],
    slices: &Slices,
    script_wide: &[String],
) -> HistoryNames {
    let mut names = HistoryNames::new();
    if script_wide.is_empty() {
        return names;
    }
    let functions = Functions::new(statements);
    for statement in statements {
        if matches!(statement, Statement::Function { .. }) {
            continue;
        }
        traversal::for_each_call_with_args(
            std::slice::from_ref(statement),
            &mut |name, arguments, _| {
                let Some(expression) = expression_argument(name, arguments) else {
                    return;
                };
                let key = key(expression);
                let slice = slices.get(&key).map_or(&[][..], |slice| slice.as_slice());
                let indexed = functions.indexed(expression, slice);
                let kept: Vec<String> = script_wide
                    .iter()
                    .filter(|name| indexed.contains(name.as_str()))
                    .cloned()
                    .collect();
                names.insert(key, kept.into());
            },
        );
    }
    names
}

/// The script's functions and methods, by name.
struct Functions<'a> {
    bodies: Vec<&'a [Statement]>,
    by_name: HashMap<&'a str, Vec<usize>>,
}

impl<'a> Functions<'a> {
    fn new(statements: &'a [Statement]) -> Self {
        let mut bodies = Vec::new();
        let mut by_name: HashMap<&str, Vec<usize>> = HashMap::new();
        for statement in statements {
            if let Statement::Function { name, body, .. } = statement {
                by_name.entry(name).or_default().push(bodies.len());
                bodies.push(body.as_slice());
            }
        }
        Self { bodies, by_name }
    }

    /// Every name read with a history operator in `expression`, in `slice`, and in the functions
    /// they call, those called from those too. A call is matched by name (all overloads, and a
    /// method called on a variable by its own name), which can only add names.
    fn indexed(&self, expression: &Expression, slice: &[Statement]) -> HashSet<String> {
        let mut indexed = HashSet::new();
        let mut callees = Vec::new();
        let mut note_history = |name: &str| {
            indexed.insert(name.to_owned());
        };
        traversal::for_each_history_identifier_in_expression(expression, &mut note_history);
        traversal::for_each_history_identifier(slice, &mut note_history);
        let mut note_callee = |name: &str| callees.push(name.to_owned());
        traversal::for_each_callee_in_expression(expression, &mut note_callee);
        traversal::for_each_callee(slice, &mut note_callee);

        let mut visited = HashSet::new();
        while let Some(callee) = callees.pop() {
            let method = callee.rsplit('.').next().unwrap_or(&callee);
            for name in [callee.as_str(), method] {
                for &index in self.by_name.get(name).into_iter().flatten() {
                    if !visited.insert(index) {
                        continue;
                    }
                    let body = self.bodies[index];
                    traversal::for_each_history_identifier(body, &mut |name| {
                        indexed.insert(name.to_owned());
                    });
                    traversal::for_each_callee(body, &mut |name| callees.push(name.to_owned()));
                }
            }
        }
        indexed
    }
}
