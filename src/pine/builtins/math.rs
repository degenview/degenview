//! `math.*`, ported from `Builtins/PineMath.swift`. Pure: the only session input is `mintick`,
//! for `math.round_to_mintick`.

use crate::pine::runtime::value::RuntimeValue;

fn unary(name: &str) -> Option<fn(f64) -> f64> {
    Some(match name {
        "math.sqrt" => f64::sqrt,
        "math.log" => f64::ln,
        "math.exp" => f64::exp,
        "math.log10" => f64::log10,
        "math.sin" => f64::sin,
        "math.cos" => f64::cos,
        "math.tan" => f64::tan,
        "math.asin" => f64::asin,
        "math.acos" => f64::acos,
        "math.atan" => f64::atan,
        "math.todegrees" => |x| x * 180.0 / std::f64::consts::PI,
        "math.toradians" => |x| x * std::f64::consts::PI / 180.0,
        _ => return None,
    })
}

/// Every `math.*` function this module computes (`math.sum` is a windowed `ta` function).
pub const SUPPORTED: [&str; 24] = [
    "math.sqrt",
    "math.log",
    "math.exp",
    "math.log10",
    "math.sin",
    "math.cos",
    "math.tan",
    "math.asin",
    "math.acos",
    "math.atan",
    "math.todegrees",
    "math.toradians",
    "math.max",
    "math.min",
    "math.abs",
    "math.round",
    "math.floor",
    "math.ceil",
    "math.sign",
    "math.avg",
    "math.round_to_mintick",
    "math.pow",
    "math.random",
    "math.sum",
];

pub fn call(name: &str, values: &[RuntimeValue], mintick: f64) -> RuntimeValue {
    let na = RuntimeValue::Na;
    let first = values.first().unwrap_or(&na);
    if let Some(function) = unary(name) {
        return first
            .number()
            .map_or(RuntimeValue::Na, |x| RuntimeValue::Float(function(x)));
    }
    match name {
        "math.max" | "math.min" => extremum(name == "math.max", values),
        "math.abs" => {
            if let RuntimeValue::Int(x) = first {
                return RuntimeValue::Int(if *x == i64::MIN { i64::MAX } else { x.abs() });
            }
            first
                .number()
                .map_or(RuntimeValue::Na, |x| RuntimeValue::Float(x.abs()))
        }
        "math.round" => round(values),
        "math.floor" | "math.ceil" => match first.number() {
            Some(x) if x.is_finite() => RuntimeValue::integral(if name == "math.floor" {
                x.floor()
            } else {
                x.ceil()
            }),
            _ => RuntimeValue::Na,
        },
        "math.sign" => first.number().map_or(RuntimeValue::Na, |x| {
            RuntimeValue::Float(if x > 0.0 {
                1.0
            } else if x < 0.0 {
                -1.0
            } else {
                0.0
            })
        }),
        "math.avg" => all_numbers(values).map_or(RuntimeValue::Na, |numbers| {
            RuntimeValue::Float(numbers.iter().sum::<f64>() / numbers.len() as f64)
        }),
        "math.round_to_mintick" => match first.number() {
            Some(x) if x.is_finite() && mintick > 0.0 => {
                RuntimeValue::Float(round_half_away(x / mintick) * mintick)
            }
            _ => RuntimeValue::Na,
        },
        "math.pow" => match (first.number(), values.get(1).and_then(RuntimeValue::number)) {
            (Some(x), Some(y)) => RuntimeValue::Float(x.powf(y)),
            _ => RuntimeValue::Na,
        },
        _ => RuntimeValue::Na,
    }
}

/// Swift's `.rounded()`: halves round away from zero (Rust's `round` does the same).
fn round_half_away(x: f64) -> f64 {
    x.round()
}

/// Every value as a number, or None when there are none or any is not numeric.
fn all_numbers(values: &[RuntimeValue]) -> Option<Vec<f64>> {
    let numbers: Vec<f64> = values.iter().filter_map(RuntimeValue::number).collect();
    (!numbers.is_empty() && numbers.len() == values.len()).then_some(numbers)
}

fn extremum(is_max: bool, values: &[RuntimeValue]) -> RuntimeValue {
    let Some(numbers) = all_numbers(values) else {
        return RuntimeValue::Na;
    };
    if values.iter().all(RuntimeValue::is_int) {
        let ints = values.iter().filter_map(|value| match value {
            RuntimeValue::Int(x) => Some(*x),
            _ => None,
        });
        let picked = if is_max { ints.max() } else { ints.min() };
        return picked.map_or(RuntimeValue::Na, RuntimeValue::Int);
    }
    let picked = numbers
        .into_iter()
        .reduce(|a, b| if is_max { a.max(b) } else { a.min(b) });
    picked.map_or(RuntimeValue::Na, RuntimeValue::Float)
}

fn round(values: &[RuntimeValue]) -> RuntimeValue {
    let Some(x) = values
        .first()
        .and_then(RuntimeValue::number)
        .filter(|x| x.is_finite())
    else {
        return RuntimeValue::Na;
    };
    if let Some(precision) = values.get(1).and_then(RuntimeValue::number) {
        let scale = 10_f64.powf(precision.round());
        return RuntimeValue::Float((x * scale).round() / scale);
    }
    RuntimeValue::integral(x.round())
}

#[cfg(test)]
mod tests {
    use super::*;
    use RuntimeValue::{Float, Int, Na};

    fn call(name: &str, values: &[RuntimeValue]) -> RuntimeValue {
        super::call(name, values, 0.01)
    }

    #[test]
    fn unary_functions_pass_na_through() {
        assert_eq!(call("math.sqrt", &[Float(9.0)]), Float(3.0));
        assert_eq!(
            call("math.sqrt", &[Int(16)]),
            Float(4.0),
            "ints become floats"
        );
        assert_eq!(call("math.sqrt", &[Na]), Na);
        assert_eq!(call("math.sqrt", &[]), Na);
        assert_eq!(call("math.log10", &[Float(1000.0)]), Float(3.0));
        assert_eq!(call("math.exp", &[Float(0.0)]), Float(1.0));
        assert_eq!(call("math.log", &[Float(1.0)]), Float(0.0));
        assert_eq!(
            call("math.todegrees", &[Float(std::f64::consts::PI)]),
            Float(180.0)
        );
        assert_eq!(
            call("math.toradians", &[Float(180.0)]),
            Float(std::f64::consts::PI)
        );
        assert_eq!(call("math.sin", &[Float(0.0)]), Float(0.0));
        assert!(call("math.sqrt", &[Float(-1.0)]).number().unwrap().is_nan());
    }

    #[test]
    fn max_and_min_keep_ints_when_every_argument_is_an_int() {
        assert_eq!(call("math.max", &[Int(3), Int(9), Int(5)]), Int(9));
        assert_eq!(call("math.min", &[Int(3), Int(9)]), Int(3));
        assert_eq!(call("math.max", &[Int(3), Float(3.5)]), Float(3.5));
        assert_eq!(call("math.min", &[Float(2.5), Int(1)]), Float(1.0));
        assert_eq!(call("math.max", &[Int(3), Na]), Na, "any na makes na");
        assert_eq!(call("math.max", &[]), Na);
    }

    #[test]
    fn abs_sign_floor_and_ceil() {
        assert_eq!(call("math.abs", &[Int(-4)]), Int(4));
        assert_eq!(
            call("math.abs", &[Int(i64::MIN)]),
            Int(i64::MAX),
            "no overflow"
        );
        assert_eq!(call("math.abs", &[Float(-2.5)]), Float(2.5));
        assert_eq!(call("math.sign", &[Float(-3.0)]), Float(-1.0));
        assert_eq!(call("math.sign", &[Float(0.0)]), Float(0.0));
        assert_eq!(call("math.sign", &[Int(8)]), Float(1.0));
        assert_eq!(call("math.floor", &[Float(2.7)]), Int(2));
        assert_eq!(call("math.floor", &[Float(-2.1)]), Int(-3));
        assert_eq!(call("math.ceil", &[Float(2.1)]), Int(3));
        assert_eq!(call("math.ceil", &[Float(f64::INFINITY)]), Na);
        assert_eq!(call("math.floor", &[Na]), Na);
    }

    #[test]
    fn round_with_and_without_precision() {
        assert_eq!(
            call("math.round", &[Float(2.5)]),
            Int(3),
            "halves round away from zero"
        );
        assert_eq!(call("math.round", &[Float(-2.5)]), Int(-3));
        assert_eq!(call("math.round", &[Float(2.4)]), Int(2));
        assert_eq!(call("math.round", &[Float(7.45678), Int(2)]), Float(7.46));
        assert_eq!(call("math.round", &[Float(1234.5), Int(-2)]), Float(1200.0));
        assert_eq!(call("math.round", &[Float(f64::NAN)]), Na);
    }

    #[test]
    fn average_power_and_mintick_rounding() {
        assert_eq!(call("math.avg", &[Int(1), Int(2), Float(6.0)]), Float(3.0));
        assert_eq!(call("math.avg", &[Int(1), Na]), Na);
        assert_eq!(call("math.pow", &[Int(2), Int(10)]), Float(1024.0));
        assert_eq!(call("math.pow", &[Int(2)]), Na);
        assert_eq!(
            super::call("math.round_to_mintick", &[Float(1.2345)], 0.01),
            Float(1.23)
        );
        assert_eq!(
            super::call("math.round_to_mintick", &[Float(1.2345)], 0.0),
            Na,
            "unusable mintick"
        );
        assert_eq!(
            super::call("math.round_to_mintick", &[Float(10.0)], 0.25),
            Float(10.0)
        );
        assert_eq!(call("math.nope", &[Int(1)]), Na);
        for name in SUPPORTED
            .iter()
            .filter(|n| **n != "math.random" && **n != "math.sum")
        {
            assert_ne!(
                super::call(name, &[Float(0.5), Float(2.0)], 0.01),
                Na,
                "{name} computes"
            );
        }
    }
}
