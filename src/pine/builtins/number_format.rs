//! `str.tostring` formatting, ported from `Builtins/PineFormat.swift`. Patterns support `#`/`0`
//! fraction digits (`"#.##"`, `"0.00"`) and `format.mintick` / `format.percent`.

use crate::pine::runtime::value::RuntimeValue;

/// Fraction digits used for `format.mintick` when the mintick is unusable.
const FALLBACK_DECIMALS: usize = 2;
const DEFAULT_MAX_DIGITS: usize = 8;

struct DigitStyle {
    min_digits: usize,
    max_digits: usize,
    suffix: &'static str,
}

pub fn format(value: &RuntimeValue, pattern: Option<&str>, mintick: f64) -> String {
    match value {
        RuntimeValue::String(text) => return text.clone(),
        RuntimeValue::Bool(flag) => return flag.to_string(),
        RuntimeValue::Int(integer) if pattern.is_none() => return integer.to_string(),
        RuntimeValue::Int(_) | RuntimeValue::Float(_) => {}
        _ => return "NaN".to_owned(),
    }
    let Some(x) = value.number().filter(|x| x.is_finite()) else {
        return "NaN".to_owned();
    };
    let style = digit_style(pattern, mintick);
    let mut text = format!("{x:.*}", style.max_digits);
    if style.max_digits > style.min_digits && text.contains('.') {
        let mut trimmable = style.max_digits - style.min_digits;
        while trimmable > 0 && text.ends_with('0') {
            text.pop();
            trimmable -= 1;
        }
        if text.ends_with('.') {
            text.pop();
        }
    }
    if text == "-0" {
        text = "0".to_owned();
    }
    text + style.suffix
}

fn digit_style(pattern: Option<&str>, mintick: f64) -> DigitStyle {
    let fixed = |digits: usize| DigitStyle {
        min_digits: digits,
        max_digits: digits,
        suffix: "",
    };
    match pattern {
        None | Some("format.volume" | "format.inherit") => DigitStyle {
            min_digits: 0,
            max_digits: DEFAULT_MAX_DIGITS,
            suffix: "",
        },
        Some("format.mintick") => {
            if mintick > 0.0 && mintick.is_finite() {
                fixed((-mintick.log10()).ceil().max(0.0) as usize)
            } else {
                fixed(FALLBACK_DECIMALS)
            }
        }
        Some("format.percent") => DigitStyle {
            min_digits: 2,
            max_digits: 2,
            suffix: "%",
        },
        Some(pattern) => {
            let Some(dot) = pattern.find('.') else {
                return fixed(0);
            };
            let fraction = &pattern[dot + 1..];
            DigitStyle {
                min_digits: fraction.chars().filter(|c| *c == '0').count(),
                max_digits: fraction.chars().filter(|c| matches!(c, '0' | '#')).count(),
                suffix: "",
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use RuntimeValue::{Bool, Float, Int, Na};

    fn fmt(value: RuntimeValue, pattern: Option<&str>) -> String {
        format(&value, pattern, 0.01)
    }

    #[test]
    fn plain_values_print_naturally() {
        assert_eq!(fmt(RuntimeValue::String("hi".into()), None), "hi");
        assert_eq!(fmt(Bool(true), None), "true");
        assert_eq!(fmt(Bool(false), None), "false");
        assert_eq!(fmt(Int(42), None), "42");
        assert_eq!(fmt(Float(1.5), None), "1.5");
        assert_eq!(fmt(Float(2.0), None), "2", "trailing zeros are trimmed");
        assert_eq!(
            fmt(Float(0.123_456_789_12), None),
            "0.12345679",
            "at most eight digits"
        );
        assert_eq!(fmt(Na, None), "NaN");
        assert_eq!(fmt(Float(f64::NAN), None), "NaN");
        assert_eq!(fmt(Float(f64::INFINITY), None), "NaN");
        assert_eq!(
            fmt(Float(-0.000_000_001), None),
            "0",
            "negative zero reads as 0"
        );
    }

    #[test]
    fn patterns_set_the_fraction_digits() {
        assert_eq!(fmt(Float(7.45678), Some("#.##")), "7.46");
        assert_eq!(fmt(Float(3.0), Some("#.##")), "3", "# digits are optional");
        assert_eq!(
            fmt(Float(3.0), Some("0.00")),
            "3.00",
            "0 digits are required"
        );
        assert_eq!(fmt(Float(3.5), Some("0.0#")), "3.5");
        assert_eq!(fmt(Float(7.45678), Some("0.0#")), "7.46");
        assert_eq!(fmt(Float(3.7), Some("#")), "4", "no fraction: rounded");
        assert_eq!(
            fmt(Int(5), Some("0.00")),
            "5.00",
            "a pattern formats ints too"
        );
    }

    #[test]
    fn named_formats() {
        assert_eq!(
            format(&Float(1.23456), Some("format.mintick"), 0.01),
            "1.23"
        );
        assert_eq!(
            format(&Float(1.0), Some("format.mintick"), 0.0001),
            "1.0000"
        );
        assert_eq!(
            format(&Float(1.2345), Some("format.mintick"), 0.0),
            "1.23",
            "fallback to two"
        );
        assert_eq!(
            format(&Float(1234.5), Some("format.mintick"), 1.0),
            "1234",
            "an exact tie rounds to even, as printf does"
        );
        assert_eq!(fmt(Float(12.3456), Some("format.percent")), "12.35%");
        assert_eq!(fmt(Float(12.3), Some("format.volume")), "12.3");
        assert_eq!(fmt(Float(12.3), Some("format.inherit")), "12.3");
    }
}
