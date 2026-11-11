//! What a user may name a variable, function, parameter or loop counter, ported from
//! `PineIdentifierRules.swift`.
//!
//! Pine identifiers are `[A-Za-z_][A-Za-z0-9_]*`, so a dotted name is never one. A name a builtin
//! already owns cannot be reused; a bare namespace (`math`, `ta`) is not a builtin, so it stays free.

use crate::pine::compiler::symbol_catalog;

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
