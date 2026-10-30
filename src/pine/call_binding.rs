//! Binds each builtin call to the family that evaluates it, when the session is built.
//!
//! `Session::call` used to find a builtin by asking each family in turn, with string tests and table
//! scans, on every execution of every call: about a hundred comparisons before `ta.sma` is reached,
//! about two hundred for `plot` or `strategy.entry`. A call that nothing in the program can capture
//! (no function, method, type or import alias of that name or head) always lands in the same family,
//! so the answer is worked out once here and stored on the call (`Expression::Call::route`).
//!
//! A call this pass leaves alone, or whose family hands it back, takes the old chain: user
//! functions and methods, variable receivers (`levels.push(x)`), the inline `ta.*` functions, input
//! declarations, debugger watch expressions (parsed separately, so never bound). The chain keeps
//! its "unknown function" error as the backstop.
//!
//! `route_of` mirrors the order of that chain, so it must be kept in step with it. The tests run
//! every registered function both ways and compare.

use std::collections::HashSet;

use super::ast::{Expression, Route, Statement};
use super::program::CompiledProgram;

/// Call-site number to the position of the function or method the type checker resolved it to.
type Targets = std::collections::HashMap<usize, usize>;
use super::{
    alert_builtins, array_builtins, drawings, map_builtins, math, matrix_builtins,
    strategy_builtins, strings, ta, ta_extra, time, visuals,
};

/// The names `Session::scalar_call` answers, besides the `time` parts (`time::PART_NAMES`).
pub const SCALAR_NAMES: [&str; 19] = [
    "na",
    "nz",
    "int",
    "float",
    "bool",
    "string",
    "color",
    "color.new",
    "str.format_time",
    "color.r",
    "color.g",
    "color.b",
    "color.t",
    "color.from_gradient",
    "timeframe.in_seconds",
    "timeframe.change",
    "timeframe.from_seconds",
    "color.rgb",
    "time",
];

/// More `scalar_call` names (`time_close`, `timestamp`, `str.tostring`), kept apart so the first
/// list stays the ones a reader would expect.
const MORE_SCALAR_NAMES: [&str; 3] = ["time_close", "timestamp", "str.tostring"];

/// What `Session::call` evaluates inline, between the families, rather than in one of them.
pub(crate) const INLINE: [&str; 15] = [
    "ta.sar",
    "ta.supertrend",
    "fixnan",
    "ta.tr",
    "runtime.error",
    "ta.barssince",
    "ta.vwap",
    "ta.dmi",
    "ta.cum",
    "ta.correlation",
    "ta.atr",
    "ta.bb",
    "ta.pivothigh",
    "ta.pivotlow",
    "ta.linreg",
];

/// The family that evaluates a call to `name`, as `Session::call`'s chain would reach it. `None` for
/// what the chain handles by value or one-off: `request.*`, `log.*`, `ticker.*`, `str.split`, and
/// anything unknown.
pub fn route_of(name: &str) -> Option<Route> {
    if is_input_call(name) {
        return Some(Route::Input);
    }
    if SCALAR_NAMES.contains(&name)
        || MORE_SCALAR_NAMES.contains(&name)
        || time::PART_NAMES.contains(&name)
    {
        return Some(Route::Scalar);
    }
    if array_builtins::handles(name)
        || map_builtins::handles(name)
        || matrix_builtins::handles(name)
    {
        return Some(Route::Collection);
    }
    if name == "str.split" {
        return None;
    }
    if math::SUPPORTED.contains(&name) && name != "math.sum" {
        return Some(Route::Math);
    }
    if strings::SUPPORTED.contains(&name) {
        return Some(Route::Strings);
    }
    if INLINE.contains(&name) {
        return Some(Route::TaInline);
    }
    if ta::is_supported(name) {
        return Some(Route::TaTable);
    }
    if ta_extra::FUNCTIONS.contains(&name) {
        return Some(Route::TaExtra);
    }
    if is_drawing_function(name) {
        return Some(Route::Drawings);
    }
    if visuals::handles(name) {
        return Some(Route::Visuals);
    }
    if name.starts_with("strategy.closedtrades.") || name.starts_with("strategy.opentrades.") {
        return Some(Route::StrategyTrade);
    }
    if strategy_builtins::handles(name) {
        return Some(Route::Strategy);
    }
    if alert_builtins::handles(name) {
        return Some(Route::Alert);
    }
    None
}

/// `indicator()`, `strategy()`, `library()` and the `input` functions.
pub(crate) fn is_input_call(name: &str) -> bool {
    matches!(name, "indicator" | "strategy" | "library" | "input") || name.starts_with("input.")
}

/// `line.new`, `label.set_x1`, … : a member of a drawing namespace, not the `chart.point` constructors.
fn is_drawing_function(name: &str) -> bool {
    drawings::handles(name) && !name.starts_with("chart.point")
}

/// Sets `route` on every call in `program` and in the libraries it imports.
pub fn bind(program: &mut CompiledProgram) {
    let mut captured = HashSet::new();
    collect_captured(program, &mut captured);
    // Only the script's own calls are resolved: a library's call sites are numbered apart and name
    // that library's functions.
    let targets = std::mem::take(&mut program.resolutions);
    bind_block(&mut program.statements, &captured, Some(&targets));
    for import in &mut program.imports {
        if let Some(library) = &mut import.library {
            bind_program(library, &captured);
        }
    }
}

/// Every name a script or one of its libraries can introduce that a call might then mean: its
/// functions, methods, types, enums and the aliases and exports of its imports. Variables are not
/// here: a builtin always wins over a variable receiver.
fn collect_captured(program: &CompiledProgram, captured: &mut HashSet<String>) {
    for statement in &program.statements {
        match statement {
            Statement::Function { name, .. }
            | Statement::TypeDeclaration { name, .. }
            | Statement::EnumDeclaration { name, .. } => {
                captured.insert(name.clone());
            }
            _ => {}
        }
    }
    captured.extend(program.method_names.iter().cloned());
    captured.extend(program.exported_names.iter().cloned());
    for import in &program.imports {
        captured.insert(import.alias.clone());
        if let Some(library) = &import.library {
            collect_captured(library, captured);
        }
    }
}

fn bind_program(program: &mut CompiledProgram, captured: &HashSet<String>) {
    bind_block(&mut program.statements, captured, None);
    for import in &mut program.imports {
        if let Some(library) = &mut import.library {
            bind_program(library, captured);
        }
    }
}

fn route_for(name: &str, captured: &HashSet<String>) -> Option<Route> {
    let head = name.split('.').next().unwrap_or(name);
    if captured.contains(name) || captured.contains(head) {
        return None;
    }
    route_of(name)
}

fn bind_block(statements: &mut [Statement], captured: &HashSet<String>, targets: Option<&Targets>) {
    for statement in statements {
        bind_statement(statement, captured, targets);
    }
}

fn bind_statement(
    statement: &mut Statement,
    captured: &HashSet<String>,
    targets: Option<&Targets>,
) {
    match statement {
        Statement::Declaration { value, .. }
        | Statement::Assignment { value, .. }
        | Statement::TupleDeclaration { value, .. } => bind_expression(value, captured, targets),
        Statement::FieldAssignment { target, value, .. } => {
            bind_expression(target, captured, targets);
            bind_expression(value, captured, targets);
        }
        Statement::Expression(expression) => bind_expression(expression, captured, targets),
        Statement::Conditional {
            condition,
            when_true,
            when_false,
            ..
        } => {
            bind_expression(condition, captured, targets);
            bind_block(when_true, captured, targets);
            bind_block(when_false, captured, targets);
        }
        Statement::ForRange {
            from,
            to,
            step,
            body,
            ..
        } => {
            bind_expression(from, captured, targets);
            bind_expression(to, captured, targets);
            if let Some(step) = step {
                bind_expression(step, captured, targets);
            }
            bind_block(body, captured, targets);
        }
        Statement::ForIn {
            collection, body, ..
        } => {
            bind_expression(collection, captured, targets);
            bind_block(body, captured, targets);
        }
        Statement::WhileLoop {
            condition, body, ..
        } => {
            bind_expression(condition, captured, targets);
            bind_block(body, captured, targets);
        }
        Statement::Once {
            condition, body, ..
        } => {
            if let Some(condition) = condition {
                bind_expression(condition, captured, targets);
            }
            bind_block(body, captured, targets);
        }
        Statement::Switch { subject, arms, .. } => {
            if let Some(subject) = subject {
                bind_expression(subject, captured, targets);
            }
            for arm in arms {
                if let Some(condition) = &mut arm.condition {
                    bind_expression(condition, captured, targets);
                }
                bind_block(&mut arm.body, captured, targets);
            }
        }
        Statement::LoopControl(..) | Statement::EnumDeclaration { .. } => {}
        Statement::TypeDeclaration { fields, .. } => {
            for field in fields {
                if let Some(default) = &mut field.default_value {
                    bind_expression(default, captured, targets);
                }
            }
        }
        Statement::Function {
            parameters, body, ..
        } => {
            for parameter in parameters {
                if let Some(default) = &mut parameter.default_value {
                    bind_expression(default, captured, targets);
                }
            }
            bind_block(body, captured, targets);
        }
    }
}

fn bind_expression(
    expression: &mut Expression,
    captured: &HashSet<String>,
    targets: Option<&Targets>,
) {
    match expression {
        Expression::Literal(..) | Expression::Identifier(..) => {}
        Expression::Unary { operand, .. } => bind_expression(operand, captured, targets),
        Expression::Binary { lhs, rhs, .. } => {
            bind_expression(lhs, captured, targets);
            bind_expression(rhs, captured, targets);
        }
        Expression::Ternary {
            condition,
            when_true,
            when_false,
            ..
        } => {
            bind_expression(condition, captured, targets);
            bind_expression(when_true, captured, targets);
            bind_expression(when_false, captured, targets);
        }
        Expression::Call {
            name,
            arguments,
            route,
            site,
            target,
            ..
        } => {
            *route = route_for(name, captured);
            *target = targets.and_then(|resolved| resolved.get(site).copied());
            for argument in arguments {
                bind_expression(&mut argument.value, captured, targets);
            }
        }
        Expression::History { base, offset, .. } => {
            bind_expression(base, captured, targets);
            bind_expression(offset, captured, targets);
        }
        Expression::Tuple(values, _) => {
            for value in values {
                bind_expression(value, captured, targets);
            }
        }
        Expression::Member { base, .. } => bind_expression(base, captured, targets),
        Expression::MethodCall {
            receiver,
            arguments,
            site,
            target,
            ..
        } => {
            *target = targets.and_then(|resolved| resolved.get(site).copied());
            bind_expression(receiver, captured, targets);
            for argument in arguments {
                bind_expression(&mut argument.value, captured, targets);
            }
        }
        Expression::Statement(statement, _) => bind_statement(statement, captured, targets),
    }
}
