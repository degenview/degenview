//! What a user may name a variable, function, parameter or loop counter, ported from
//! `PineIdentifierRules.swift`.
//!
//! Pine identifiers are `[A-Za-z_][A-Za-z0-9_]*`, so a dotted name is never one. A name a builtin
//! already owns cannot be reused; a bare namespace (`math`, `ta`) is not a builtin, so it stays free.
//!
//! The thirteen words Pine reserves ([`RESERVED_WORDS`]) cannot name anything the script declares: a
//! variable, a function, a parameter, a loop counter, a tuple element. Only a *declared name* is
//! refused. A named argument (`label.new(text = "x")`), the `text.*` namespace, `ta.range`, a type's
//! field and a method's name (a method is picked by its receiver, like a builtin it may share a name
//! with; whether TradingView refuses `method text(...)` is not measured) stay free.

use crate::pine::compiler::symbol_catalog;

/// The words TradingView reserves ("A number of words are reserved and cannot be used for variable
/// or function names", the manual's *To Pine Script v5* migration guide). `in` is a loop keyword the
/// parser never takes for a name, so it is here for the editor's rename check.
pub const RESERVED_WORDS: [&str; 13] = [
    "catch", "class", "do", "ellipse", "in", "is", "polygon", "range", "return", "struct", "text",
    "throw", "try",
];

/// A naming violation: the diagnostic code and its message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Violation {
    pub code: &'static str,
    pub message: String,
}

/// The violation for a user-declared `name`, or None when it is free. `_` is always free.
pub fn violation(name: &str) -> Option<Violation> {
    if name == "_" {
        return None;
    }
    if name.contains('.') {
        return Some(Violation {
            code: "CE10090",
            message: format!(
                "User variable identifiers should not contain \".\" character: \"{name}\"."
            ),
        });
    }
    if RESERVED_WORDS.contains(&name) {
        return Some(Violation {
            code: "CE10150",
            message: format!("\"{name}\" cannot be used as a variable or function name."),
        });
    }
    if symbol_catalog::is_variable(name) {
        return Some(Violation {
            code: "CE10190",
            message: format!("Cannot shadow the built-in variable \"{name}\"."),
        });
    }
    if symbol_catalog::is_function(name) {
        return Some(Violation {
            code: "CE10190",
            message: format!("Cannot shadow the built-in function \"{name}\"."),
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn underscore_and_plain_names_are_free() {
        assert_eq!(violation("_"), None);
        assert_eq!(violation("myVar"), None);
        assert_eq!(violation("math"), None, "a bare namespace stays free");
        assert_eq!(violation("color"), None, "color is a namespace first");
    }

    #[test]
    fn dotted_names_are_ce10090() {
        let v = violation("a.b").unwrap();
        assert_eq!(v.code, "CE10090");
    }

    #[test]
    fn every_reserved_word_is_ce10150() {
        for word in RESERVED_WORDS {
            let v = violation(word).unwrap_or_else(|| panic!("{word} is free"));
            assert_eq!(v.code, "CE10150", "{word}");
            assert_eq!(
                v.message,
                format!("\"{word}\" cannot be used as a variable or function name.")
            );
        }
    }

    #[test]
    fn names_that_only_contain_a_reserved_word_are_free() {
        for name in [
            "ranges",
            "text_size",
            "my_range",
            "Struct",
            "isDone",
            "doIt",
        ] {
            assert_eq!(violation(name), None, "{name}");
        }
    }

    #[test]
    fn builtins_cannot_be_shadowed() {
        assert_eq!(violation("close").unwrap().code, "CE10190");
        assert_eq!(violation("plot").unwrap().code, "CE10190");
        assert_eq!(violation("bar_index").unwrap().code, "CE10190");
        assert_eq!(
            violation("ta").map(|v| v.code),
            None,
            "a bare namespace stays free"
        );
    }
}
