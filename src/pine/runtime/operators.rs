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
    if let (RuntimeValue::Int(x), RuntimeValue::Int(y)) = (a, b) {
        return RuntimeValue::Bool(relation(op, x, y));
    }
    if let (Some(x), Some(y)) = (a.number(), b.number()) {
        return RuntimeValue::Bool(match op {
            BinaryOperator::Equal => floats_equal(x, y),
            BinaryOperator::NotEqual => !floats_equal(x, y),
            _ => relation(op, &x, &y),
        });
    }
    RuntimeValue::Bool(if op == BinaryOperator::Equal {
        a == b
    } else {
        a != b
    })
}

/// `op` as the exact relation between `x` and `y`; false for an operator that is no comparison.
fn relation<T: PartialOrd>(op: BinaryOperator, x: &T, y: &T) -> bool {
    match op {
        BinaryOperator::Equal => x == y,
        BinaryOperator::NotEqual => x != y,
        BinaryOperator::Less => x < y,
        BinaryOperator::LessEqual => x <= y,
        BinaryOperator::Greater => x > y,
        BinaryOperator::GreaterEqual => x >= y,
        _ => false,
    }
}

/// Whether `==` finds two numbers equal. TradingView rounds both to nine fractional digits first
/// (`0.1 + 0.2 == 0.3` and `1e-10 == 0` are true) and finds nothing equal that is not finite, an
/// infinity included (measured, probes P10 B and A4). `!=` is its negation. The ordering operators
/// are not rounded: `1 + 4e-10 > 1` is true there as well.
fn floats_equal(x: f64, y: f64) -> bool {
    x.is_finite() && y.is_finite() && nine_digits(x) == nine_digits(y)
}

/// `x` rounded to nine fractional digits, as a scaled float: the rounding is done in `f64`, as a
/// cast to an integer would saturate for large magnitudes. A magnitude whose scaled value is out of
/// range is left as it is.
fn nine_digits(x: f64) -> f64 {
    let scaled = x * 1e9;
    if scaled.is_finite() {
        scaled.round()
    } else {
        x
    }
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

    fn compare_floats(op: BinaryOperator, x: f64, y: f64) -> bool {
        apply(op, &RuntimeValue::Float(x), &RuntimeValue::Float(y)) == RuntimeValue::Bool(true)
    }

    #[test]
    fn equality_rounds_floats_to_nine_fractional_digits() {
        use BinaryOperator::{Equal, NotEqual};
        assert!(compare_floats(Equal, 0.1 + 0.2, 0.3));
        assert!(!compare_floats(NotEqual, 0.1 + 0.2, 0.3));
        assert!(compare_floats(Equal, 1e-10, 0.0));
        assert!(compare_floats(Equal, 1.0 + 4e-10, 1.0));
        assert!(!compare_floats(Equal, 1.0 + 6e-10, 1.0));
        assert!(compare_floats(NotEqual, 1.0 + 6e-10, 1.0));
        assert!(compare_floats(Equal, -0.0, 0.0));
        // An integer against a float rounds the float.
        assert_eq!(
            apply(
                Equal,
                &RuntimeValue::Int(5),
                &RuntimeValue::Float(5.0 + 1e-10)
            ),
            RuntimeValue::Bool(true)
        );
    }

    #[test]
    fn ordering_stays_exact() {
        use BinaryOperator::{Greater, GreaterEqual, Less, LessEqual};
        assert!(compare_floats(Greater, 1.0 + 4e-10, 1.0));
        assert!(compare_floats(Greater, 1.0 + 6e-10, 1.0));
        assert!(!compare_floats(GreaterEqual, 1.0, 1.0 + 4e-10));
        assert!(!compare_floats(LessEqual, 1.0 + 4e-10, 1.0));
        assert!(compare_floats(Less, 1.0, 1.0 + 4e-10));
        assert!(compare_floats(GreaterEqual, 0.1 + 0.2, 0.3));
        assert!(!compare_floats(LessEqual, 0.1 + 0.2, 0.3));
    }

    #[test]
    fn equality_of_large_magnitudes_does_not_saturate() {
        use BinaryOperator::Equal;
        // Rounding through an integer cast would make these equal.
        assert!(!compare_floats(Equal, 1e20, 2e20));
        assert!(!compare_floats(Equal, 1e300, 2e300));
        assert!(compare_floats(Equal, 1e300, 1e300));
        assert!(!compare_floats(Equal, 1e10 + 1.0, 1e10));
    }

    #[test]
    fn a_float_that_is_not_finite_is_equal_to_nothing() {
        use BinaryOperator::{Equal, NotEqual};
        for (x, y) in [
            (f64::INFINITY, f64::INFINITY),
            (f64::INFINITY, 1.0),
            (f64::NEG_INFINITY, f64::NEG_INFINITY),
            (f64::NAN, f64::NAN),
            (f64::NAN, 0.0),
        ] {
            assert!(!compare_floats(Equal, x, y), "{x} == {y}");
            assert!(compare_floats(NotEqual, x, y), "{x} != {y}");
        }
    }

    #[test]
    fn integers_compare_exactly() {
        let big = (1_i64 << 53) + 1;
        let int = |op, x, y| apply(op, &RuntimeValue::Int(x), &RuntimeValue::Int(y));
        assert_eq!(
            int(BinaryOperator::Equal, big, big - 1),
            RuntimeValue::Bool(false)
        );
        assert_eq!(
            int(BinaryOperator::NotEqual, big, big - 1),
            RuntimeValue::Bool(true)
        );
        assert_eq!(
            int(BinaryOperator::Greater, big, big - 1),
            RuntimeValue::Bool(true)
        );
        assert_eq!(
            int(BinaryOperator::LessEqual, 3, 3),
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
