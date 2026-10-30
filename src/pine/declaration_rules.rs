//! Rules about how functions and methods are declared, which need no types: where a declaration may
//! sit, what a method's receiver must look like, which overloads can coexist, and that nothing calls
//! itself. TradingView documents the scope, overload and recursion rules (User-defined functions
//! page) and that a method's first parameter must declare its type (Methods page); it documents none
//! of the messages, so the texts here are this app's own.
//!
//! Recursion is judged by name, so it only covers a plain call (`down(n - 1)`) to a name declared
//! once: `x.size()` inside a user `size` method may be the builtin, and an overloaded name may call
//! an earlier overload. Those need the receiver's type and are resolved by the type checker.

use std::collections::{HashMap, HashSet};

use super::ast::{Expression, FunctionKind, Parameter, Qualifier, Statement, TypeRef};
use super::traversal::{children, expressions, nested_blocks};
use crate::pine::{Diagnostic, DiagnosticCategory, SourceRange};

/// What tells two declarations of one name apart: the type and qualifier of every required parameter.
/// Optional parameters and parameter names do not count.
type Required = Vec<(Option<TypeRef>, Qualifier)>;

struct Declaration<'a> {
    name: &'a str,
    kind: FunctionKind,
    parameters: &'a [Parameter],
    body: &'a [Statement],
    range: SourceRange,
}

/// Checks every function and method in `statements`. `exported` is the set of names the script
/// exports (only a library may).
pub fn validate(statements: &[Statement], exported: &HashSet<String>) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for statement in statements {
        misplaced(statement, &mut diagnostics);
    }
    let declarations: Vec<Declaration> = statements
        .iter()
        .filter_map(|statement| match statement {
            Statement::Function {
                name,
                kind,
                parameters,
                body,
                range,
            } => Some(Declaration {
                name,
                kind: *kind,
                parameters,
                body,
                range: *range,
            }),
            _ => None,
        })
        .collect();
    for declaration in &declarations {
        if declaration.kind == FunctionKind::Method {
            receiver(declaration, &mut diagnostics);
        }
        if exported.contains(declaration.name) {
            exported_parameters(declaration, &mut diagnostics);
        }
    }
    overloads(&declarations, &mut diagnostics);
    recursion(&declarations, &mut diagnostics);
    diagnostics
}

fn error(code: &str, message: String, range: SourceRange) -> Diagnostic {
    Diagnostic::error(code, DiagnosticCategory::Semantic, message, range)
}

fn noun(kind: FunctionKind) -> &'static str {
    match kind {
        FunctionKind::Function => "function",
        FunctionKind::Method => "method",
    }
}

/// A function or method declared inside another statement: only the global scope may declare them.
fn misplaced(statement: &Statement, diagnostics: &mut Vec<Diagnostic>) {
    for block in nested_blocks(statement) {
        for inner in block {
            if let Statement::Function {
                name, kind, range, ..
            } = inner
            {
                diagnostics.push(error(
                    "PINE3061",
                    format!(
                        "The {} '{name}' must be declared in the global scope.",
                        noun(*kind)
                    ),
                    *range,
                ));
            }
            misplaced(inner, diagnostics);
        }
    }
}

/// A method's first parameter is the receiver, and the type it declares is what the method attaches to.
fn receiver(declaration: &Declaration, diagnostics: &mut Vec<Diagnostic>) {
    let name = declaration.name;
    match declaration.parameters.first() {
        None => diagnostics.push(error(
            "PINE3060",
            format!("The method '{name}' needs a receiver: declare its first parameter with a type."),
            declaration.range,
        )),
        Some(first) if first.type_ref.is_none() => diagnostics.push(error(
            "PINE3060",
            format!(
                "The receiver of the method '{name}' (its first parameter, '{}') must declare its type: it is the type the method attaches to.",
                first.name
            ),
            declaration.range,
        )),
        Some(_) => {}
    }
}

/// An exported function or method declares the type of every parameter.
fn exported_parameters(declaration: &Declaration, diagnostics: &mut Vec<Diagnostic>) {
    for (index, parameter) in declaration.parameters.iter().enumerate() {
        // A method's untyped receiver is already reported as such.
        let receiver = declaration.kind == FunctionKind::Method && index == 0;
        if parameter.type_ref.is_none() && !receiver {
            diagnostics.push(error(
                "PINE3062",
                format!(
                    "The parameter '{}' of the exported {} '{}' must declare its type.",
                    parameter.name,
                    noun(declaration.kind),
                    declaration.name
                ),
                declaration.range,
            ));
        }
    }
}

fn required(parameters: &[Parameter]) -> Required {
    parameters
        .iter()
        .filter(|parameter| parameter.default_value.is_none())
        .map(|parameter| {
            (
                parameter.type_ref.clone(),
                parameter.qualifier.unwrap_or(Qualifier::Series),
            )
        })
        .collect()
}

/// Two declarations of one name conflict when their required parameters have the same types and
/// qualifiers, which includes the same count; names and optional parameters do not tell them apart.
fn overloads(declarations: &[Declaration], diagnostics: &mut Vec<Diagnostic>) {
    let mut seen: HashMap<&str, Vec<Required>> = HashMap::new();
    for declaration in declarations {
        let signature = required(declaration.parameters);
        let known = seen.entry(declaration.name).or_default();
        if known.contains(&signature) {
            diagnostics.push(error(
                "PINE3024",
                format!(
                    "The {} '{}' is already declared with the same required parameters: overloads must differ in how many required parameters they take or in a parameter's type or qualifier.",
                    noun(declaration.kind),
                    declaration.name
                ),
                declaration.range,
            ));
        }
        known.push(signature);
    }
}

/// Plain calls made by `body`, in source order, with where each sits.
fn plain_calls(body: &[Statement]) -> Vec<(String, SourceRange)> {
    fn walk(expression: &Expression, calls: &mut Vec<(String, SourceRange)>) {
        match expression {
            Expression::Call { name, range, .. } if !name.contains('.') => {
                calls.push((name.clone(), *range));
            }
            Expression::Statement(statement, _) => statement_calls(statement, calls),
            _ => {}
        }
        for child in children(expression) {
            walk(child, calls);
        }
    }
    fn statement_calls(statement: &Statement, calls: &mut Vec<(String, SourceRange)>) {
        for expression in expressions(statement) {
            walk(expression, calls);
        }
        for block in nested_blocks(statement) {
            for inner in block {
                statement_calls(inner, calls);
            }
        }
    }
    let mut calls = Vec::new();
    for statement in body {
        statement_calls(statement, &mut calls);
    }
    calls
}

/// A function or method that reaches itself through plain calls to names declared once.
fn recursion(declarations: &[Declaration], diagnostics: &mut Vec<Diagnostic>) {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for declaration in declarations {
        *counts.entry(declaration.name).or_default() += 1;
    }
    let once: HashMap<&str, &Declaration> = declarations
        .iter()
        .filter(|declaration| counts[declaration.name] == 1)
        .map(|declaration| (declaration.name, declaration))
        .collect();
    let edges: HashMap<&str, Vec<(String, SourceRange)>> = once
        .values()
        .map(|declaration| {
            let calls = plain_calls(declaration.body)
                .into_iter()
                .filter(|(callee, _)| once.contains_key(callee.as_str()))
                .collect();
            (declaration.name, calls)
        })
        .collect();
    for declaration in declarations
        .iter()
        .filter(|declaration| once.contains_key(declaration.name))
    {
        // The first call out of the function that leads back to it.
        let leads_back = edges[declaration.name].iter().find(|(callee, _)| {
            let mut visited: HashSet<&str> = HashSet::new();
            let mut pending = vec![callee.as_str()];
            while let Some(current) = pending.pop() {
                if current == declaration.name {
                    return true;
                }
                if visited.insert(current) {
                    pending.extend(edges[current].iter().map(|(next, _)| next.as_str()));
                }
            }
            false
        });
        if let Some((callee, range)) = leads_back {
            let how = if callee == declaration.name {
                "itself".to_owned()
            } else {
                format!("itself through '{callee}'")
            };
            diagnostics.push(error(
                "PINE3063",
                format!(
                    "The {} '{}' calls {how}: Pine functions and methods cannot be recursive, so use a loop.",
                    noun(declaration.kind),
                    declaration.name
                ),
                *range,
            ));
        }
    }
}
