//! `active` on an `input.*` call: whether the user can change the input in the script's settings.
//! TradingView types it `input bool`, so a script sets it from other inputs (`active = useFast`,
//! `active = mode == "Custom"`) and the settings grey the control out as they change.
//!
//! The condition is kept as an expression with the script's own constants folded in, so that it
//! stands alone in the schema; the settings evaluate it against the values being edited.

use std::collections::HashMap;

use super::ast::{Argument, Expression, Literal};
use super::compiler::constant_value;
use super::program::{InputDefinition, InputValue};
use super::runtime_value::RuntimeValue;

/// The condition `argument` sets, with every constant it names replaced by its value and every
/// input it names kept by name. None when it reads anything else (a series, a call) or does not
/// come out as a bool, so it cannot be known before the script runs.
pub fn compile(
    argument: &Argument,
    constants: &HashMap<String, RuntimeValue>,
    declared: &[InputDefinition],
) -> Option<Expression> {
    let condition = bind(&argument.value, constants, declared)?;
    let defaults: HashMap<String, RuntimeValue> = declared
        .iter()
        .map(|input| (input.id.clone(), runtime_value(&input.default_value)))
        .collect();
    evaluate(&condition, &defaults)?;
    Some(condition)
}

/// Whether the condition holds with `values` (an input with no entry holds its default).
pub fn holds(
    condition: &Expression,
    inputs: &[InputDefinition],
    values: &HashMap<String, InputValue>,
) -> bool {
    let environment: HashMap<String, RuntimeValue> = inputs
        .iter()
        .map(|input| {
            let value = values.get(&input.id).unwrap_or(&input.default_value);
            (input.id.clone(), runtime_value(value))
        })
        .collect();
    // A condition that cannot be evaluated was checked when it was compiled; leaving the input
    // editable is the safe side of a mismatch.
    evaluate(condition, &environment).unwrap_or(true)
}

fn evaluate(condition: &Expression, environment: &HashMap<String, RuntimeValue>) -> Option<bool> {
    match constant_value(condition, environment)? {
        RuntimeValue::Bool(value) => Some(value),
        _ => None,
    }
}

fn bind(
    expression: &Expression,
    constants: &HashMap<String, RuntimeValue>,
    declared: &[InputDefinition],
) -> Option<Expression> {
    let again = |inner: &Expression| bind(inner, constants, declared).map(Box::new);
    Some(match expression {
        Expression::Literal(Literal::Na, _) => return None,
        Expression::Literal(..) => expression.clone(),
        Expression::Identifier(name, range) => {
            if declared.iter().any(|input| input.id == *name) {
                expression.clone()
            } else {
                Expression::Literal(literal(constant_value(expression, constants)?)?, *range)
            }
        }
        Expression::Unary { op, operand, range } => Expression::Unary {
            op: *op,
            operand: again(operand)?,
            range: *range,
        },
        Expression::Binary {
            lhs,
            op,
            rhs,
            range,
        } => Expression::Binary {
            lhs: again(lhs)?,
            op: *op,
            rhs: again(rhs)?,
            range: *range,
        },
        Expression::Ternary {
            condition,
            when_true,
            when_false,
            range,
        } => Expression::Ternary {
            condition: again(condition)?,
            when_true: again(when_true)?,
            when_false: again(when_false)?,
            range: *range,
        },
        _ => return None,
    })
}

fn literal(value: RuntimeValue) -> Option<Literal> {
    Some(match value {
        RuntimeValue::Int(value) => Literal::Int(value),
        RuntimeValue::Float(value) => Literal::Float(value),
        RuntimeValue::Bool(value) => Literal::Bool(value),
        RuntimeValue::String(value) => Literal::String(value),
        RuntimeValue::Color(value) => Literal::Color(value),
        _ => return None,
    })
}

/// An input's value as the script reads it; a source is read by its name here.
fn runtime_value(value: &InputValue) -> RuntimeValue {
    match value {
        InputValue::Int(value) => RuntimeValue::Int(*value),
        InputValue::Float(value) => RuntimeValue::Float(*value),
        InputValue::Bool(value) => RuntimeValue::Bool(*value),
        InputValue::String(value) | InputValue::Source(value) => {
            RuntimeValue::String(value.clone())
        }
        InputValue::Color(value) => RuntimeValue::Color(*value),
    }
}
