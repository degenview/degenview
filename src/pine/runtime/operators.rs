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
        .raw_number()
        .map_or(RuntimeValue::Na, |n| RuntimeValue::ieee(-n))
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
    // The IEEE value, not the finite one: a zero-divisor quotient is an infinity that the next
    // operation goes on with (`100 - 100 / (1 + up / down)` with `down` = 0 is 100 on TradingView,
    // probes P09 S5 and P10 A). What has no value (0 / 0, an infinity minus itself) is NaN, so `na`.
    let (Some(x), Some(y)) = (a.raw_number(), b.raw_number()) else {
        return RuntimeValue::Na;
    };
    match op {
        BinaryOperator::Add => RuntimeValue::ieee(x + y),
        BinaryOperator::Subtract => RuntimeValue::ieee(x - y),
        BinaryOperator::Multiply => RuntimeValue::ieee(x * y),
        BinaryOperator::Divide => RuntimeValue::ieee(x / y),
        BinaryOperator::Modulo if y == 0.0 => RuntimeValue::Na,
        BinaryOperator::Modulo => RuntimeValue::ieee(x % y),
        BinaryOperator::Power => RuntimeValue::ieee(x.powf(y)),
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
    if let (Some(x), Some(y)) = (a.raw_number(), b.raw_number()) {
        return RuntimeValue::Bool(float_relation(op, x, y));
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

/// How close two floats must be for the comparisons to call them equal. TradingView-measured
/// (probes P13-P15, BTCUSDT 4h, 2026-11-21): equal when the distance is at most 1e-10, whatever the
/// magnitude (`1e10` and its next float are not equal, `1` and `1 + 1.01e-10` are not), which
/// is not the manual's rounding to nine fractional digits (`1 + 4e-10` and `1 + 1e-10` differ).
const EQUAL_WITHIN: f64 = 1e-10;

/// Whether two floats are equal for `==`. False when either is not finite: the distance of two
/// infinities is not a number, and an infinity is far from everything else.
fn floats_equal(x: f64, y: f64) -> bool {
    (x - y).abs() <= EQUAL_WITHIN
}

/// `op` between two floats, every one of them through `floats_equal`: `<` and `>` are false for
/// numbers that are equal (`1e-10 < 2e-10` and `0.1 + 0.2 > 0.3` are false), `<=` and `>=` are true
/// for them (false for two infinities of one sign, which are not equal). `!=` is false for a number
/// that is not finite, an infinity included (`inf != 1`), as TradingView gives; the ordering
/// operators keep the infinities ordered (`-inf < inf`).
fn float_relation(op: BinaryOperator, x: f64, y: f64) -> bool {
    match op {
        BinaryOperator::Equal => floats_equal(x, y),
        BinaryOperator::NotEqual => x.is_finite() && y.is_finite() && !floats_equal(x, y),
        BinaryOperator::Less => x < y && !floats_equal(x, y),
        BinaryOperator::Greater => x > y && !floats_equal(x, y),
        // Strictly ordered, or equal by `floats_equal`: two infinities of one sign are neither
        // (`inf >= inf` is false on TradingView, probe V7 U4), which `x >= y` alone would not say.
        BinaryOperator::LessEqual => x < y || floats_equal(x, y),
        BinaryOperator::GreaterEqual => x > y || floats_equal(x, y),
        _ => false,
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
    fn floats_within_a_ten_billionth_are_equal() {
        use BinaryOperator::{Equal, NotEqual};
        // TradingView, probes P13-P15: equal up to and including 1e-10, not beyond.
        for (x, y, equal) in [
            (0.1 + 0.2, 0.3, true),
            (0.0, 0.5e-10, true),
            (0.0, 0.99e-10, true),
            (0.0, 1e-10, true),
            (-1e-10, 0.0, true),
            (1e-10, 2e-10, true),
            (1.0, 1.0 + 0.9e-10, true),
            (1e4, 1e4 + 0.9e-10, true),
            (-0.0, 0.0, true),
            (0.0, 1.01e-10, false),
            (0.0, 2e-10, false),
            (-0.9e-10, 0.9e-10, false),
            (1.0, 1.0 + 1.1e-10, false),
            (1.0000000004, 1.0000000001, false),
            (1.0000000006, 1.0000000001, false),
            (4e-10, 6e-10, false),
            (1e4, 1e4 + 1.1e-10, false),
            (1e10, 1e10 + 1e-6, false),
            (1e300, 1e300 * (1.0 + 1e-15), false),
        ] {
            assert_eq!(compare_floats(Equal, x, y), equal, "{x} == {y}");
            assert_eq!(compare_floats(NotEqual, x, y), !equal, "{x} != {y}");
        }
        // An integer against a float is compared the same way.
        assert_eq!(
            apply(
                Equal,
                &RuntimeValue::Int(5),
                &RuntimeValue::Float(5.0 + 0.9e-10)
            ),
            RuntimeValue::Bool(true)
        );
    }

    #[test]
    fn ordering_does_not_separate_equal_floats() {
        use BinaryOperator::{Greater, GreaterEqual, Less, LessEqual};
        for (x, y) in [
            (0.1 + 0.2, 0.3),
            (1e-10, 2e-10),
            (0.0, 1e-10),
            (1.0, 1.0 + 0.9e-10),
        ] {
            for (op, expected) in [
                (Less, false),
                (Greater, false),
                (LessEqual, true),
                (GreaterEqual, true),
            ] {
                assert_eq!(compare_floats(op, x, y), expected, "{op:?} {x} {y}");
                assert_eq!(compare_floats(op, y, x), expected, "{op:?} {y} {x}");
            }
        }
        // Beyond the distance they are ordered as numbers are.
        assert!(compare_floats(Greater, 1.0 + 4e-10, 1.0));
        assert!(compare_floats(Greater, 1.0000000001, 1.0));
        assert!(compare_floats(Less, 4e-10, 6e-10));
        assert!(!compare_floats(GreaterEqual, 1.0, 1.0 + 4e-10));
        assert!(!compare_floats(LessEqual, 1.0 + 4e-10, 1.0));
        assert!(compare_floats(LessEqual, 1e-10, 2e-10));
    }

    #[test]
    fn a_float_that_is_not_finite_is_equal_to_and_different_from_nothing() {
        use BinaryOperator::{Equal, Greater, Less, NotEqual};
        // TradingView, probes P13 and P14: neither `==` nor `!=` holds, the ordering still does.
        for (x, y) in [
            (f64::INFINITY, f64::INFINITY),
            (f64::INFINITY, 1.0),
            (f64::NEG_INFINITY, f64::NEG_INFINITY),
            (f64::NAN, f64::NAN),
            (f64::NAN, 0.0),
        ] {
            assert!(!compare_floats(Equal, x, y), "{x} == {y}");
            assert!(!compare_floats(NotEqual, x, y), "{x} != {y}");
        }
        assert!(compare_floats(Greater, f64::INFINITY, 1.0));
        assert!(compare_floats(Less, f64::NEG_INFINITY, f64::INFINITY));
        assert!(!compare_floats(Less, f64::NAN, 1.0));
        assert!(!compare_floats(Greater, f64::NAN, f64::NAN));
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

    fn arithmetic_of(op: BinaryOperator, x: f64, y: f64) -> RuntimeValue {
        apply(op, &RuntimeValue::Float(x), &RuntimeValue::Float(y))
    }

    #[test]
    fn dividing_by_zero_gives_an_infinity_and_zero_over_zero_gives_na() {
        use BinaryOperator::Divide;
        let infinity = f64::INFINITY;
        assert_eq!(
            arithmetic_of(Divide, 1.0, 0.0),
            RuntimeValue::Float(infinity)
        );
        assert_eq!(
            arithmetic_of(Divide, -1.0, 0.0),
            RuntimeValue::Float(-infinity)
        );
        assert_eq!(
            arithmetic_of(Divide, 1.0, -0.0),
            RuntimeValue::Float(-infinity)
        );
        assert_eq!(arithmetic_of(Divide, 0.0, 0.0), RuntimeValue::Na);
        // Two ints divide as floats too.
        assert_eq!(
            apply(Divide, &RuntimeValue::Int(1), &RuntimeValue::Int(0)),
            RuntimeValue::Float(infinity)
        );
        assert_eq!(
            apply(Divide, &RuntimeValue::Int(0), &RuntimeValue::Int(0)),
            RuntimeValue::Na
        );
    }

    #[test]
    fn arithmetic_carries_an_infinity_on() {
        use BinaryOperator::{Add, Divide, Multiply, Subtract};
        let infinity = RuntimeValue::Float(f64::INFINITY);
        // `1 / (close / 0)` is 0 and `100 - 100 / (1 + x / 0)` is 100 (TradingView, P09 S5, P10 A2).
        assert_eq!(
            apply(Divide, &RuntimeValue::Float(1.0), &infinity),
            RuntimeValue::Float(0.0)
        );
        let denominator = apply(Add, &RuntimeValue::Float(1.0), &infinity);
        assert_eq!(denominator, infinity);
        let ratio = apply(Divide, &RuntimeValue::Float(100.0), &denominator);
        assert_eq!(
            apply(Subtract, &RuntimeValue::Float(100.0), &ratio),
            RuntimeValue::Float(100.0)
        );
        assert_eq!(
            apply(Multiply, &RuntimeValue::Float(2.0), &infinity),
            infinity
        );
        assert_eq!(
            apply(Multiply, &RuntimeValue::Int(-2), &infinity),
            RuntimeValue::Float(f64::NEG_INFINITY)
        );
    }

    #[test]
    fn arithmetic_that_has_no_value_is_na() {
        use BinaryOperator::{Add, Modulo, Multiply, Power, Subtract};
        let infinity = f64::INFINITY;
        // `x - x` for an infinite x is NaN, which is na (P10 A5).
        assert_eq!(
            arithmetic_of(Subtract, infinity, infinity),
            RuntimeValue::Na
        );
        assert_eq!(arithmetic_of(Add, infinity, -infinity), RuntimeValue::Na);
        assert_eq!(arithmetic_of(Multiply, 0.0, infinity), RuntimeValue::Na);
        assert_eq!(arithmetic_of(Power, -8.0, 0.5), RuntimeValue::Na);
        assert_eq!(arithmetic_of(Modulo, infinity, 3.0), RuntimeValue::Na);
        assert_eq!(arithmetic_of(Modulo, 3.0, 0.0), RuntimeValue::Na);
        // na stays na, and a float NaN that reaches an operator behaves as na.
        assert_eq!(
            apply(Add, &RuntimeValue::Na, &RuntimeValue::Float(1.0)),
            RuntimeValue::Na
        );
        assert_eq!(arithmetic_of(Add, f64::NAN, 1.0), RuntimeValue::Na);
        // Overflow is an infinity, not na.
        assert_eq!(
            arithmetic_of(Multiply, 1e308, 10.0),
            RuntimeValue::Float(infinity)
        );
    }

    #[test]
    fn ordering_sees_the_infinity() {
        use BinaryOperator::{Greater, GreaterEqual, Less, LessEqual};
        let infinity = f64::INFINITY;
        // `close / 0 > 1e300` is true on TradingView (P10 A4).
        assert!(compare_floats(Greater, infinity, 1e300));
        assert!(compare_floats(GreaterEqual, infinity, 1e300));
        assert!(compare_floats(Less, -infinity, -1e300));
        assert!(compare_floats(LessEqual, 1.0, infinity));
        assert!(!compare_floats(Less, infinity, 1e300));
        assert!(compare_floats(Greater, infinity, f64::MAX));
        assert!(!compare_floats(Greater, f64::NAN, 1.0));
        assert!(!compare_floats(Less, f64::NAN, 1.0));
    }

    #[test]
    fn two_equal_infinities_are_neither_ordered_nor_equal() {
        use BinaryOperator::{Equal, Greater, GreaterEqual, Less, LessEqual, NotEqual};
        // TradingView, probe V7 U4 / U4b: every cell false except `-inf < inf` and `-inf <= inf`.
        let (high, low) = (f64::INFINITY, f64::NEG_INFINITY);
        for op in [Greater, GreaterEqual, Less, LessEqual, Equal, NotEqual] {
            assert!(!compare_floats(op, high, high), "{op:?} +inf, +inf");
            assert!(!compare_floats(op, low, low), "{op:?} -inf, -inf");
        }
        assert!(compare_floats(Less, low, high));
        assert!(compare_floats(LessEqual, low, high));
        assert!(!compare_floats(Equal, high, low));
        assert!(!compare_floats(GreaterEqual, low, high));
    }

    #[test]
    fn negating_an_infinity_flips_it() {
        assert_eq!(
            negate(&RuntimeValue::Float(f64::INFINITY)),
            RuntimeValue::Float(f64::NEG_INFINITY)
        );
        assert_eq!(negate(&RuntimeValue::Float(f64::NAN)), RuntimeValue::Na);
    }

    #[test]
    fn negate_flips_numbers() {
        assert_eq!(negate(&RuntimeValue::Int(5)), RuntimeValue::Int(-5));
        assert_eq!(negate(&RuntimeValue::Float(2.5)), RuntimeValue::Float(-2.5));
        assert_eq!(negate(&RuntimeValue::Na), RuntimeValue::Na);
    }
}
