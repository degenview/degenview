//! `str.*` except `str.tostring` (which is `number_format`), ported from `Builtins/PineStrings.swift`.
//! A "character" is a Unicode scalar here; Swift counted grapheme clusters, which only differs for
//! combining marks and emoji sequences.

use super::number_format;
use super::regex_lite::Regex;
use crate::pine::runtime::value::RuntimeValue;
use crate::pine::syntax::token::{Diagnostic, DiagnosticCategory, SourceRange};

const PREDICATES: [&str; 3] = ["str.contains", "str.startswith", "str.endswith"];

/// Every `str.*` function this module computes.
pub const SUPPORTED: [&str; 17] = [
    "str.replace",
    "str.pos",
    "str.length",
    "str.upper",
    "str.lower",
    "str.trim",
    "str.contains",
    "str.startswith",
    "str.endswith",
    "str.replace_all",
    "str.substring",
    "str.match",
    "str.repeat",
    "str.tonumber",
    "str.format",
    "str.tostring",
    "str.split",
];

pub fn call(
    name: &str,
    values: &[RuntimeValue],
    range: SourceRange,
    mintick: f64,
) -> Result<RuntimeValue, Diagnostic> {
    let text = |index: usize| values.get(index).and_then(RuntimeValue::text_value);
    let Some(subject) = text(0) else {
        // `na` (or a non-string) in, `na` out — except the predicates, which are false.
        return Ok(if PREDICATES.contains(&name) {
            RuntimeValue::Bool(false)
        } else {
            RuntimeValue::Na
        });
    };
    Ok(match name {
        "str.length" => RuntimeValue::Int(subject.chars().count() as i64),
        "str.upper" => RuntimeValue::String(subject.to_uppercase()),
        "str.lower" => RuntimeValue::String(subject.to_lowercase()),
        "str.trim" => RuntimeValue::String(subject.trim().to_owned()),
        "str.contains" => RuntimeValue::Bool(
            text(1).is_some_and(|needle| needle.is_empty() || subject.contains(needle)),
        ),
        "str.startswith" => {
            RuntimeValue::Bool(text(1).is_some_and(|prefix| subject.starts_with(prefix)))
        }
        "str.endswith" => {
            RuntimeValue::Bool(text(1).is_some_and(|suffix| subject.ends_with(suffix)))
        }
        // str.replace(source, target, replacement, occurrence): only the occurrence-th match (from 0).
        "str.replace" => match (text(1), text(2)) {
            (Some(target), Some(replacement)) if !target.is_empty() => {
                let occurrence = values
                    .get(3)
                    .and_then(RuntimeValue::int_value)
                    .map_or(0, |n| n.max(0) as usize);
                match subject.match_indices(target).nth(occurrence) {
                    Some((at, _)) => RuntimeValue::String(format!(
                        "{}{}{}",
                        &subject[..at],
                        replacement,
                        &subject[at + target.len()..]
                    )),
                    None => RuntimeValue::String(subject.to_owned()),
                }
            }
            _ => RuntimeValue::String(subject.to_owned()),
        },
        // str.pos(source, str): the character index of the first match, `na` when there is none.
        "str.pos" => text(1)
            .and_then(|needle| subject.find(needle))
            .map_or(RuntimeValue::Na, |byte| {
                RuntimeValue::Int(subject[..byte].chars().count() as i64)
            }),
        "str.replace_all" => match (text(1), text(2)) {
            (Some(target), Some(replacement)) if !target.is_empty() => {
                RuntimeValue::String(subject.replace(target, replacement))
            }
            _ => RuntimeValue::String(subject.to_owned()),
        },
        "str.substring" => substring(subject, values, range)?,
        "str.match" => RuntimeValue::String(first_match(text(1), subject)),
        "str.repeat" => {
            // str.repeat(source, repeat, separator = ""): `na` for a negative count, "" for zero.
            let Some(count) = values
                .get(1)
                .and_then(RuntimeValue::int_value)
                .filter(|c| *c >= 0)
            else {
                return Ok(RuntimeValue::Na);
            };
            let separator = text(2).unwrap_or("");
            RuntimeValue::String(vec![subject; count as usize].join(separator))
        }
        "str.tonumber" => subject
            .trim()
            .parse::<f64>()
            .ok()
            .map_or(RuntimeValue::Na, RuntimeValue::Float),
        "str.format" => RuntimeValue::String(format_template(subject, &values[1..], mintick)),
        _ => {
            return Err(Diagnostic::error(
                "PINE4007",
                DiagnosticCategory::Runtime,
                format!("Unknown or unsupported function '{name}'."),
                range,
            ));
        }
    })
}

fn substring(
    subject: &str,
    values: &[RuntimeValue],
    range: SourceRange,
) -> Result<RuntimeValue, Diagnostic> {
    let characters: Vec<char> = subject.chars().collect();
    let out_of_range = |which: &str| {
        Diagnostic::error(
            "PINE4016",
            DiagnosticCategory::Runtime,
            format!("str.substring {which} index is out of range."),
            range,
        )
    };
    let begin = values
        .get(1)
        .and_then(RuntimeValue::int_value)
        .filter(|begin| *begin >= 0 && *begin as usize <= characters.len())
        .ok_or_else(|| out_of_range("begin"))? as usize;
    let end = if values.len() > 2 {
        values[2].int_value().unwrap_or(characters.len() as i64)
    } else {
        characters.len() as i64
    };
    if end < begin as i64 || end as usize > characters.len() {
        return Err(out_of_range("end"));
    }
    Ok(RuntimeValue::String(
        characters[begin..end as usize].iter().collect(),
    ))
}

/// `str.format("{0} of {1,number,#.##}", a, b)`: `{n}` inserts the n-th argument,
/// `{n,number,pattern}` formats it with a `str.tostring` pattern.
pub fn format_template(template: &str, values: &[RuntimeValue], mintick: f64) -> String {
    let mut output = String::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        output.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            output.push_str(&rest[open..]);
            return output;
        };
        let parts: Vec<&str> = after[..close].splitn(3, ',').map(str::trim).collect();
        match parts
            .first()
            .and_then(|slot| slot.parse::<usize>().ok())
            .filter(|slot| *slot < values.len())
        {
            Some(slot) => {
                let pattern = (parts.len() > 2 && parts[1] == "number").then(|| parts[2]);
                output.push_str(&number_format::format(&values[slot], pattern, mintick));
            }
            None => output.push_str(&rest[open..open + 1 + close + 1]),
        }
        rest = &after[close + 1..];
    }
    output.push_str(rest);
    output
}

/// `str.match(source, regex)`: the first substring the regular expression matches, or an empty
/// string when nothing matches or the expression is invalid.
fn first_match(pattern: Option<&str>, subject: &str) -> String {
    pattern
        .and_then(Regex::new)
        .and_then(|expression| expression.first_match(subject).map(str::to_owned))
        .unwrap_or_default()
}

/// `str.split(source, separator)` as the pieces; an empty separator splits into characters.
pub fn split(subject: &str, separator: &str) -> Vec<String> {
    if separator.is_empty() {
        subject.chars().map(String::from).collect()
    } else {
        subject.split(separator).map(str::to_owned).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use RuntimeValue::{Bool, Float, Int, Na};

    fn s(text: &str) -> RuntimeValue {
        RuntimeValue::String(text.to_owned())
    }

    fn run(name: &str, values: &[RuntimeValue]) -> RuntimeValue {
        call(name, values, SourceRange::ZERO, 0.01).expect("no diagnostic")
    }

    #[test]
    fn a_missing_subject_is_na_but_the_predicates_are_false() {
        assert_eq!(run("str.upper", &[Na]), Na);
        assert_eq!(
            run("str.length", &[Int(3)]),
            Na,
            "a non-string is not a subject"
        );
        assert_eq!(run("str.contains", &[Na, s("a")]), Bool(false));
        assert_eq!(run("str.startswith", &[]), Bool(false));
        assert_eq!(run("str.endswith", &[Na, Na]), Bool(false));
    }

    #[test]
    fn case_length_and_trimming() {
        assert_eq!(run("str.length", &[s("héllo")]), Int(5));
        assert_eq!(run("str.upper", &[s("aBc")]), s("ABC"));
        assert_eq!(run("str.lower", &[s("aBc")]), s("abc"));
        assert_eq!(run("str.trim", &[s("  hi \n")]), s("hi"));
    }

    #[test]
    fn replace_changes_one_occurrence_and_pos_finds_the_first() {
        assert_eq!(
            run("str.replace", &[s("a-b-c-d"), s("-"), s("+")]),
            s("a+b-c-d")
        );
        assert_eq!(
            run("str.replace", &[s("a-b-c-d"), s("-"), s("+"), Int(2)]),
            s("a-b-c+d")
        );
        assert_eq!(
            run("str.replace", &[s("a-b"), s("-"), s("+"), Int(5)]),
            s("a-b"),
            "a missing occurrence leaves the text alone"
        );
        assert_eq!(
            run("str.replace", &[s("BTCUSDT"), s("BTC"), s("")]),
            s("USDT")
        );
        assert_eq!(run("str.pos", &[s("héllo"), s("llo")]), Int(2));
        assert_eq!(run("str.pos", &[s("hello"), s("z")]), Na);
    }

    #[test]
    fn searching() {
        assert_eq!(run("str.contains", &[s("hello"), s("ell")]), Bool(true));
        assert_eq!(
            run("str.contains", &[s("hello"), s("")]),
            Bool(true),
            "empty is always contained"
        );
        assert_eq!(run("str.contains", &[s("hello"), Na]), Bool(false));
        assert_eq!(run("str.startswith", &[s("hello"), s("he")]), Bool(true));
        assert_eq!(run("str.startswith", &[s("hello"), s("lo")]), Bool(false));
        assert_eq!(run("str.endswith", &[s("hello"), s("lo")]), Bool(true));
    }

    #[test]
    fn replacing_repeating_and_parsing() {
        assert_eq!(
            run("str.replace_all", &[s("a-b-c"), s("-"), s("+")]),
            s("a+b+c")
        );
        assert_eq!(
            run("str.replace_all", &[s("abc"), s(""), s("x")]),
            s("abc"),
            "empty target: unchanged"
        );
        assert_eq!(
            run("str.replace_all", &[s("abc"), s("b")]),
            s("abc"),
            "no replacement: unchanged"
        );
        assert_eq!(run("str.repeat", &[s("ab"), Int(3)]), s("ababab"));
        assert_eq!(run("str.repeat", &[s("ab"), Int(3), s("-")]), s("ab-ab-ab"));
        assert_eq!(run("str.repeat", &[s("ab"), Int(0)]), s(""));
        assert_eq!(run("str.repeat", &[s("ab"), Int(-1)]), Na);
        assert_eq!(run("str.tonumber", &[s(" 12.5 ")]), Float(12.5));
        assert_eq!(run("str.tonumber", &[s("abc")]), Na);
        assert_eq!(run("str.tonumber", &[s("")]), Na);
    }

    #[test]
    fn substring_checks_its_bounds() {
        assert_eq!(run("str.substring", &[s("hello"), Int(1), Int(3)]), s("el"));
        assert_eq!(run("str.substring", &[s("hello"), Int(2)]), s("llo"));
        assert_eq!(
            run("str.substring", &[s("日本語"), Int(1), Int(2)]),
            s("本")
        );
        assert_eq!(run("str.substring", &[s("hello"), Int(5)]), s(""));
        let error = call(
            "str.substring",
            &[s("hello"), Int(6)],
            SourceRange::ZERO,
            0.01,
        )
        .unwrap_err();
        assert_eq!(error.code, "PINE4016");
        assert!(error.message.contains("begin"));
        let error = call(
            "str.substring",
            &[s("hello"), Int(3), Int(2)],
            SourceRange::ZERO,
            0.01,
        )
        .unwrap_err();
        assert!(error.message.contains("end"));
        assert!(
            call(
                "str.substring",
                &[s("hello"), Int(0), Int(9)],
                SourceRange::ZERO,
                0.01
            )
            .is_err()
        );
        assert!(
            call("str.substring", &[s("hello")], SourceRange::ZERO, 0.01).is_err(),
            "begin is required"
        );
    }

    #[test]
    fn unknown_functions_are_a_diagnostic() {
        let error = call("str.nope", &[s("x")], SourceRange::ZERO, 0.01).unwrap_err();
        assert_eq!(error.code, "PINE4007");
        assert!(error.message.contains("str.nope"));
    }

    #[test]
    fn format_fills_numbered_slots_with_optional_patterns() {
        let values = [Float(7.45678), Int(7), s("x")];
        assert_eq!(
            format_template("{0} and {1}", &values, 0.01),
            "7.45678 and 7"
        );
        assert_eq!(format_template("{0,number,#.##}", &values, 0.01), "7.46");
        assert_eq!(format_template("{1,number,0.00}", &values, 0.01), "7.00");
        assert_eq!(format_template("{2}{2}", &values, 0.01), "xx");
        assert_eq!(
            format_template("{9} stays", &values, 0.01),
            "{9} stays",
            "unknown slot is kept"
        );
        assert_eq!(format_template("{x} stays", &values, 0.01), "{x} stays");
        assert_eq!(
            format_template("open { never closed", &values, 0.01),
            "open { never closed"
        );
        assert_eq!(format_template("no slots", &values, 0.01), "no slots");
        assert_eq!(run("str.format", &[s("{0}-{1}"), Int(1), Int(2)]), s("1-2"));
        assert_eq!(
            format_template("{0,number,format.percent}", &[Float(5.0)], 0.01),
            "5.00%"
        );
    }

    #[test]
    fn matching_returns_the_first_hit_or_nothing() {
        assert_eq!(run("str.match", &[s("order 4521"), s(r"\d+")]), s("4521"));
        assert_eq!(run("str.match", &[s("abc"), s(r"\d+")]), s(""));
        assert_eq!(
            run("str.match", &[s("abc"), s("(")]),
            s(""),
            "an invalid expression matches nothing"
        );
        assert_eq!(run("str.match", &[s("abc"), Na]), s(""));
    }

    #[test]
    fn split_handles_empty_separators() {
        assert_eq!(split("a,b,,c", ","), ["a", "b", "", "c"]);
        assert_eq!(split("abc", ""), ["a", "b", "c"]);
        assert_eq!(split("", ","), [""]);
    }
}
