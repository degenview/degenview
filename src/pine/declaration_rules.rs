//! Rules about how functions and methods are declared, which need no types: where a declaration may
//! sit, what a method's receiver must look like and which overloads can coexist. TradingView
//! documents the scope and overload rules (User-defined functions page) and that a method's first
//! parameter must declare its type (Methods page); it documents none of the messages, so the texts
//! here are this app's own. Recursion needs to know which declaration a call selects, so the type
//! checker reports it.

use std::collections::{HashMap, HashSet};

use super::ast::{FunctionKind, Parameter, Qualifier, Statement, TypeRef};
use super::traversal::nested_blocks;
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
