//! Scalar builtins: `PineFormat` (str.tostring), `PineMath` (math.*) and `PineStrings` (str.*),
//! ported from `PineFormat.swift`, `PineMath.swift` and `PineStrings.swift`.

use crate::pine::runtime::value::RuntimeValue;
use crate::pine::{Diagnostic, DiagnosticCategory, SourceRange};

// MARK: Format

const DEFAULT_MAX_DIGITS: usize = 8;

pub fn format(value: &RuntimeValue, pattern: Option<&str>, mintick: f64) -> String {
    match value {
        RuntimeValue::String(s) => return s.clone(),
        RuntimeValue::Bool(b) => {
            return if *b {
                "true".to_owned()
            } else {
                "false".to_owned()
            };
        }
        RuntimeValue::Int(i) if pattern.is_none() => return i.to_string(),
        RuntimeValue::Int(_) | RuntimeValue::Float(_) => {}
        _ => return "NaN".to_owned(),
    }
    let Some(x) = value.number() else {
        return "NaN".to_owned();
    };
    if !x.is_finite() {
        return "NaN".to_owned();
    }
    let (min_digits, max_digits, suffix) = digit_style(pattern, mintick);
    let mut text = format!("{x:.max_digits$}");
    if max_digits > min_digits && text.contains('.') {
        let mut trimmable = max_digits - min_digits;
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
    text + suffix
}

fn digit_style(pattern: Option<&str>, mintick: f64) -> (usize, usize, &'static str) {
    match pattern {
        None | Some("format.volume") | Some("format.inherit") => (0, DEFAULT_MAX_DIGITS, ""),
        Some("format.mintick") => {
            let decimals = if mintick > 0.0 && mintick.is_finite() {
                let d = (-mintick.log10()).ceil();
                if d >= 0.0 && d.fract() == 0.0 {
                    d as usize
                } else {
                    2
                }
            } else {
                2
            };
            (decimals, decimals, "")
        }
        Some("format.percent") => (2, 2, "%"),
        Some(pattern) => {
            let Some(dot) = pattern.find('.') else {
                return (0, 0, "");
            };
            let fraction = &pattern[dot + 1..];
            (
                fraction.chars().filter(|c| *c == '0').count(),
                fraction.chars().filter(|c| *c == '0' || *c == '#').count(),
                "",
            )
        }
    }
}

// MARK: Math

pub fn math(name: &str, values: &[RuntimeValue], mintick: f64) -> RuntimeValue {
    let first = values.first().cloned().unwrap_or(RuntimeValue::Na);
    if let Some(function) = unary_math(name) {
        return first
            .number()
            .map(|x| RuntimeValue::Float(function(x)))
            .unwrap_or(RuntimeValue::Na);
    }
    match name {
        "math.max" | "math.min" => extremum(name == "math.max", values),
        "math.abs" => {
            if let RuntimeValue::Int(x) = &first {
                return RuntimeValue::Int(if *x == i64::MIN { i64::MAX } else { x.abs() });
            }
            first
                .number()
                .map(|x| RuntimeValue::Float(x.abs()))
                .unwrap_or(RuntimeValue::Na)
        }
        "math.round" => round(values),
        "math.floor" | "math.ceil" => {
            let Some(x) = first.number().filter(|x| x.is_finite()) else {
                return RuntimeValue::Na;
            };
            RuntimeValue::integral(if name == "math.floor" {
                x.floor()
            } else {
                x.ceil()
            })
        }
        "math.sign" => {
            let Some(x) = first.number() else {
                return RuntimeValue::Na;
            };
            RuntimeValue::Float(if x > 0.0 {
                1.0
            } else if x < 0.0 {
                -1.0
            } else {
                0.0
            })
        }
        "math.avg" => {
            let Some(numbers) = all_numbers(values) else {
                return RuntimeValue::Na;
            };
            RuntimeValue::Float(numbers.iter().sum::<f64>() / numbers.len() as f64)
        }
        "math.round_to_mintick" => {
            let Some(x) = first.number().filter(|x| x.is_finite()) else {
                return RuntimeValue::Na;
            };
            if mintick <= 0.0 {
                return RuntimeValue::Na;
            }
            RuntimeValue::Float((x / mintick).round() * mintick)
        }
        "math.pow" => {
            if values.len() < 2 {
                return RuntimeValue::Na;
            }
            let (Some(x), Some(y)) = (first.number(), values[1].number()) else {
                return RuntimeValue::Na;
            };
            RuntimeValue::Float(x.powf(y))
        }
        _ => RuntimeValue::Na,
    }
}

fn unary_math(name: &str) -> Option<fn(f64) -> f64> {
    match name {
        "math.sqrt" => Some(f64::sqrt),
        "math.log" => Some(f64::ln),
        "math.exp" => Some(f64::exp),
        "math.log10" => Some(f64::log10),
        "math.sin" => Some(f64::sin),
        "math.cos" => Some(f64::cos),
        "math.tan" => Some(f64::tan),
        "math.asin" => Some(f64::asin),
        "math.acos" => Some(f64::acos),
        "math.atan" => Some(f64::atan),
        "math.todegrees" => Some(|x| x * 180.0 / std::f64::consts::PI),
        "math.toradians" => Some(|x| x * std::f64::consts::PI / 180.0),
        _ => None,
    }
}

fn all_numbers(values: &[RuntimeValue]) -> Option<Vec<f64>> {
    let numbers: Vec<f64> = values.iter().filter_map(RuntimeValue::number).collect();
    (!numbers.is_empty() && numbers.len() == values.len()).then_some(numbers)
}

fn extremum(is_max: bool, values: &[RuntimeValue]) -> RuntimeValue {
    let Some(numbers) = all_numbers(values) else {
        return RuntimeValue::Na;
    };
    if values.iter().all(RuntimeValue::is_int) {
        let ints: Vec<i64> = values
            .iter()
            .filter_map(|value| match value {
                RuntimeValue::Int(x) => Some(*x),
                _ => None,
            })
            .collect();
        return ints
            .iter()
            .copied()
            .reduce(if is_max { i64::max } else { i64::min })
            .map(RuntimeValue::Int)
            .unwrap_or(RuntimeValue::Na);
    }
    numbers
        .into_iter()
        .reduce(if is_max { f64::max } else { f64::min })
        .map(RuntimeValue::Float)
        .unwrap_or(RuntimeValue::Na)
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
        let scale = 10f64.powf(precision.round());
        return RuntimeValue::Float((x * scale).round() / scale);
    }
    RuntimeValue::integral(x.round())
}

// MARK: Strings

const PREDICATES: [&str; 3] = ["str.contains", "str.startswith", "str.endswith"];

pub fn string(
    name: &str,
    values: &[RuntimeValue],
    range: SourceRange,
    mintick: f64,
) -> Result<RuntimeValue, Diagnostic> {
    fn text(values: &[RuntimeValue], index: usize) -> Option<String> {
        values
            .get(index)
            .and_then(|v| v.text_value().map(str::to_owned))
    }
    let Some(subject) = text(values, 0) else {
        return Ok(if PREDICATES.contains(&name) {
            RuntimeValue::Bool(false)
        } else {
            RuntimeValue::Na
        });
    };
    match name {
        "str.length" => Ok(RuntimeValue::Int(subject.chars().count() as i64)),
        "str.upper" => Ok(RuntimeValue::String(subject.to_uppercase())),
        "str.lower" => Ok(RuntimeValue::String(subject.to_lowercase())),
        "str.trim" => Ok(RuntimeValue::String(subject.trim().to_owned())),
        "str.contains" => Ok(RuntimeValue::Bool(
            text(values, 1).is_some_and(|t| t.is_empty() || subject.contains(&t)),
        )),
        "str.startswith" => Ok(RuntimeValue::Bool(
            text(values, 1).is_some_and(|t| subject.starts_with(&t)),
        )),
        "str.endswith" => Ok(RuntimeValue::Bool(
            text(values, 1).is_some_and(|t| subject.ends_with(&t)),
        )),
        "str.replace_all" => {
            let Some(target) = text(values, 1).filter(|t| !t.is_empty()) else {
                return Ok(RuntimeValue::String(subject));
            };
            let replacement = text(values, 2).unwrap_or_default();
            Ok(RuntimeValue::String(subject.replace(&target, &replacement)))
        }
        "str.substring" => substring(&subject, values, range),
        "str.match" => Ok(RuntimeValue::String(String::new())),
        "str.repeat" => {
            let count = values.get(1).and_then(RuntimeValue::int_value);
            let Some(count) = count.filter(|c| *c >= 0) else {
                return Ok(RuntimeValue::Na);
            };
            let separator = text(values, 2).unwrap_or_default();
            let repeated = std::iter::repeat_n(subject.as_str(), count as usize)
                .collect::<Vec<_>>()
                .join(&separator);
            Ok(RuntimeValue::String(repeated))
        }
        "str.tonumber" => Ok(subject
            .trim()
            .parse::<f64>()
            .map(RuntimeValue::Float)
            .unwrap_or(RuntimeValue::Na)),
        "str.format" => Ok(RuntimeValue::String(format_template(
            &subject,
            &values[1..],
            mintick,
        ))),
        _ => Err(Diagnostic::error(
            "PINE4007",
            DiagnosticCategory::Runtime,
            format!("Unknown or unsupported function '{name}'."),
            range,
        )),
    }
}

fn substring(
    subject: &str,
    values: &[RuntimeValue],
    range: SourceRange,
) -> Result<RuntimeValue, Diagnostic> {
    let characters: Vec<char> = subject.chars().collect();
    let begin = values.get(1).and_then(RuntimeValue::int_value);
    let Some(begin) = begin.filter(|b| *b >= 0 && (*b as usize) <= characters.len()) else {
        return Err(Diagnostic::error(
            "PINE4016",
            DiagnosticCategory::Runtime,
            "str.substring begin index is out of range.",
            range,
        ));
    };
    let begin = begin as usize;
    let end = values
        .get(2)
        .and_then(RuntimeValue::int_value)
        .map(|e| e as usize)
        .unwrap_or(characters.len());
    if end < begin || end > characters.len() {
        return Err(Diagnostic::error(
            "PINE4016",
            DiagnosticCategory::Runtime,
            "str.substring end index is out of range.",
            range,
        ));
    }
    Ok(RuntimeValue::String(
        characters[begin..end].iter().collect(),
    ))
}

/// `str.format("{0} of {1,number,#.##}", a, b)`: `{n}` inserts the n-th argument.
fn format_template(template: &str, values: &[RuntimeValue], mintick: f64) -> String {
    let mut output = String::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let (before, after) = rest.split_at(open);
        output.push_str(before);
        let Some(close) = after.find('}') else {
            output.push_str(after);
            return output;
        };
        let inner = &after[1..close];
        let parts: Vec<&str> = inner.splitn(3, ',').map(str::trim).collect();
        if let Some(slot) = parts.first().and_then(|p| p.parse::<usize>().ok())
            && let Some(value) = values.get(slot)
        {
            let pattern = (parts.len() > 2 && parts[1] == "number").then_some(parts[2]);
            output.push_str(&format(value, pattern, mintick));
            rest = &after[close + 1..];
            continue;
        }
        output.push_str(&after[..=close]);
        rest = &after[close + 1..];
    }
    output.push_str(rest);
    output
}
