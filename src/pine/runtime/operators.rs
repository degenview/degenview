//! Pure operator semantics shared by the interpreter and compile-time constant folding.
//! `and`/`or` short-circuit, so the interpreter handles them before it gets here. Ported from
//! `PineOperators.swift`.

use super::value::RuntimeValue;
use crate::pine::syntax::ast::{AssignmentOperator, BinaryOperator};

pub fn apply(op: BinaryOperator, a: &RuntimeValue, b: &RuntimeValue) -> RuntimeValue {
    match op {
        BinaryOperator::Add => {
            if let (RuntimeValue::String(x), RuntimeValue::String(y)) = (a, b) {
                return RuntimeValue::String(format!("{x}{y}"));
            }
            arithmetic(op, a, b)
        }
        BinaryOperator::Subtract
        | BinaryOperator::Multiply
        | BinaryOperator::Divide
        | BinaryOperator::Modulo
        | BinaryOperator::Power => arithmetic(op, a, b),
        BinaryOperator::Equal
        | BinaryOperator::NotEqual
        | BinaryOperator::Less
        | BinaryOperator::LessEqual
        | BinaryOperator::Greater
        | BinaryOperator::GreaterEqual => compare(op, a, b),
        BinaryOperator::And | BinaryOperator::Or => RuntimeValue::Na,
    }
}

pub fn apply_assignment(
    op: AssignmentOperator,
    old: &RuntimeValue,
    rhs: &RuntimeValue,
) -> RuntimeValue {
    match op {
        AssignmentOperator::Reassign => rhs.clone(),
        AssignmentOperator::Compound(binary) => apply(binary, old, rhs),
    }
}

pub fn negate(value: &RuntimeValue) -> RuntimeValue {
    if let RuntimeValue::Int(x) = value {
        return RuntimeValue::Int(0i64.wrapping_sub(*x));
    }
    value
        .number()
        .map(|n| RuntimeValue::Float(-n))
        .unwrap_or(RuntimeValue::Na)
}

fn arithmetic(op: BinaryOperator, a: &RuntimeValue, b: &RuntimeValue) -> RuntimeValue {
    if let (RuntimeValue::Int(x), RuntimeValue::Int(y)) = (a, b) {
        match op {
            BinaryOperator::Add => return RuntimeValue::Int(x.wrapping_add(*y)),
            BinaryOperator::Subtract => return RuntimeValue::Int(x.wrapping_sub(*y)),
            BinaryOperator::Multiply => return RuntimeValue::Int(x.wrapping_mul(*y)),
            BinaryOperator::Modulo => {
                return if *y == 0 || (*x == i64::MIN && *y == -1) {
                    RuntimeValue::Na
                } else {
                    RuntimeValue::Int(x % y)
                };
            }
            _ => {}
        }
    }
    let (Some(x), Some(y)) = (a.number(), b.number()) else {
        return RuntimeValue::Na;
    };
    match op {
        BinaryOperator::Add => RuntimeValue::Float(x + y),
        BinaryOperator::Subtract => RuntimeValue::Float(x - y),
        BinaryOperator::Multiply => RuntimeValue::Float(x * y),
        BinaryOperator::Divide => {
            if y == 0.0 {
                RuntimeValue::Na
            } else {
                RuntimeValue::Float(x / y)
            }
        }
        BinaryOperator::Modulo => {
            if y == 0.0 {
                RuntimeValue::Na
            } else {
                RuntimeValue::Float(x % y)
            }
        }
        BinaryOperator::Power => RuntimeValue::Float(x.powf(y)),
        _ => RuntimeValue::Na,
    }
}

fn compare(op: BinaryOperator, a: &RuntimeValue, b: &RuntimeValue) -> RuntimeValue {
    if *a == RuntimeValue::Na || *b == RuntimeValue::Na {
        return RuntimeValue::Bool(false);
    }
    if let (Some(x), Some(y)) = (a.number(), b.number()) {
        return match op {
            BinaryOperator::Equal => RuntimeValue::Bool(x == y),
            BinaryOperator::NotEqual => RuntimeValue::Bool(x != y),
            BinaryOperator::Less => RuntimeValue::Bool(x < y),
            BinaryOperator::LessEqual => RuntimeValue::Bool(x <= y),
            BinaryOperator::Greater => RuntimeValue::Bool(x > y),
            BinaryOperator::GreaterEqual => RuntimeValue::Bool(x >= y),
            _ => RuntimeValue::Bool(false),
        };
    }
    RuntimeValue::Bool(if op == BinaryOperator::Equal {
        a == b
    } else {
        a != b
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn int_arithmetic_wraps() {
        assert_eq!(
            apply(
                BinaryOperator::Add,
                &RuntimeValue::Int(2),
                &RuntimeValue::Int(3)
            ),
            RuntimeValue::Int(5)
        );
        assert_eq!(
            apply(
                BinaryOperator::Multiply,
                &RuntimeValue::Int(i64::MAX),
                &RuntimeValue::Int(2)
            ),
            RuntimeValue::Int(i64::MAX.wrapping_mul(2))
        );
    }

    #[test]
    fn add_concatenates_strings() {
        assert_eq!(
            apply(
                BinaryOperator::Add,
                &RuntimeValue::String("a".into()),
                &RuntimeValue::String("b".into())
            ),
            RuntimeValue::String("ab".into())
        );
    }

    #[test]
    fn modulo_by_zero_is_na() {
        assert_eq!(
            apply(
                BinaryOperator::Modulo,
                &RuntimeValue::Int(5),
                &RuntimeValue::Int(0)
            ),
            RuntimeValue::Na
        );
        assert_eq!(
            apply(
                BinaryOperator::Modulo,
                &RuntimeValue::Float(5.0),
                &RuntimeValue::Float(0.0)
            ),
            RuntimeValue::Na
        );
    }

    #[test]
    fn comparison_with_na_is_false() {
        assert_eq!(
            apply(
                BinaryOperator::Equal,
                &RuntimeValue::Na,
                &RuntimeValue::Int(1)
            ),
            RuntimeValue::Bool(false)
        );
        assert_eq!(
            apply(
                BinaryOperator::Less,
                &RuntimeValue::Int(1),
                &RuntimeValue::Int(2)
            ),
            RuntimeValue::Bool(true)
        );
    }

    #[test]
    fn negate_flips_numbers() {
        assert_eq!(negate(&RuntimeValue::Int(5)), RuntimeValue::Int(-5));
        assert_eq!(negate(&RuntimeValue::Float(2.5)), RuntimeValue::Float(-2.5));
        assert_eq!(negate(&RuntimeValue::Na), RuntimeValue::Na);
    }
}
