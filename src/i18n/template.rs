//! Messages with named values: `"{count} alerts"`, `"{symbol} crossed {price}"`.
//!
//! A translation moves a value wherever its language wants it, which `format!` cannot do, so a
//! message is filled in at run time by name. `{{` and `}}` write a brace.

use std::fmt::{Display, Write};

/// A value a message takes by name.
pub type Arg<'a> = (&'a str, &'a dyn Display);

/// `template` with every `{name}` replaced by the value of that name. A name with no value stays
/// as it is, so a mistake in a translation shows on screen and in the tests instead of vanishing.
pub fn fill(template: &str, args: &[Arg<'_>]) -> String {
    let mut out = String::with_capacity(template.len() + 8 * args.len());
    let mut rest = template;
    while let Some(at) = rest.find(['{', '}']) {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        if let Some(after) = tail.strip_prefix("{{") {
            out.push('{');
            rest = after;
        } else if let Some(after) = tail.strip_prefix("}}") {
            out.push('}');
            rest = after;
        } else if let Some((name, after)) = tail
            .strip_prefix('{')
            .and_then(|inner| inner.split_once('}'))
        {
            match args.iter().find(|(known, _)| *known == name) {
                // Writing to a `String` cannot fail.
                Some((_, value)) => {
                    let _ = write!(out, "{value}");
                }
                None => {
                    out.push('{');
                    out.push_str(name);
                    out.push('}');
                }
            }
            rest = after;
        } else {
            out.push_str(&tail[..1]);
            rest = &tail[1..];
        }
    }
    out.push_str(rest);
    out
}

/// The names a message takes, in order of appearance and without repeats.
#[cfg(test)]
pub fn placeholders(template: &str) -> Vec<&str> {
    let mut names: Vec<&str> = Vec::new();
    let mut rest = template;
    while let Some(at) = rest.find(['{', '}']) {
        let tail = &rest[at..];
        if let Some(after) = tail.strip_prefix("{{").or_else(|| tail.strip_prefix("}}")) {
            rest = after;
        } else if let Some((name, after)) = tail
            .strip_prefix('{')
            .and_then(|inner| inner.split_once('}'))
        {
            if !names.contains(&name) {
                names.push(name);
            }
            rest = after;
        } else {
            rest = &tail[1..];
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_put_in_by_name_in_any_order() {
        let symbol = "BTC";
        let price = 70_000;
        assert_eq!(
            fill(
                "{symbol} crossed {price}",
                &[("symbol", &symbol), ("price", &price)]
            ),
            "BTC crossed 70000"
        );
        assert_eq!(
            fill(
                "{price} {symbol}",
                &[("symbol", &symbol), ("price", &price)]
            ),
            "70000 BTC"
        );
        assert_eq!(fill("{n} and {n}", &[("n", &2)]), "2 and 2");
    }

    #[test]
    fn a_doubled_brace_writes_a_brace_and_is_not_a_name() {
        assert_eq!(fill("{{n}} is {n}", &[("n", &1)]), "{n} is 1");
        assert_eq!(placeholders("{{n}} is {n}"), ["n"]);
    }

    #[test]
    fn a_name_with_no_value_and_stray_braces_stay_visible() {
        assert_eq!(fill("{missing}", &[]), "{missing}");
        assert_eq!(fill("a } b { c", &[]), "a } b { c");
        assert_eq!(fill("", &[]), "");
        assert_eq!(fill("trailing {", &[]), "trailing {");
    }

    #[test]
    fn text_around_non_ascii_values_is_kept() {
        assert_eq!(
            fill("→ {x} ≥ {y}", &[("x", &"é"), ("y", &"日本")]),
            "→ é ≥ 日本"
        );
    }

    #[test]
    fn the_names_of_a_message_are_listed_once_each() {
        assert_eq!(placeholders("{a} then {b} then {a}"), ["a", "b"]);
        assert!(placeholders("no values").is_empty());
    }
}
