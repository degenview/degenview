//! What the compiler checks of a message written with `tr!`, `tr_c!` or `tr_n!`.
//!
//! The macros call [`message`] in a `const` block, so a message that is empty, padded with spaces,
//! names a value it is not given, or is given a value it never uses does not compile; the error
//! points at the call. (A translation is checked by the catalog tests instead, since it is read
//! at run time.)

/// Checks `text`, written with `{name}` for each value. `allowed` are the names that may appear in
/// it and `required` the ones that must (a subset of `allowed`). `{{` and `}}` write a brace.
pub const fn message(text: &str, allowed: &[&str], required: &[&str]) {
    let bytes = text.as_bytes();
    assert!(!bytes.is_empty(), "a message is empty");
    assert!(
        !bytes[0].is_ascii_whitespace() && !bytes[bytes.len() - 1].is_ascii_whitespace(),
        "a message starts or ends with a space; a translation cannot keep it"
    );

    let mut seen = [false; 16];
    assert!(
        required.len() <= seen.len(),
        "a message takes too many values"
    );
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'{' if at + 1 < bytes.len() && bytes[at + 1] == b'{' => at += 2,
            b'}' if at + 1 < bytes.len() && bytes[at + 1] == b'}' => at += 2,
            b'{' => {
                let start = at + 1;
                let mut end = start;
                while end < bytes.len() && bytes[end] != b'}' {
                    end += 1;
                }
                assert!(end < bytes.len(), "a `{{` in a message is never closed");
                let allowed_at = position(bytes, start, end, allowed);
                assert!(
                    allowed_at.is_some(),
                    "a {{name}} in a message is given no value"
                );
                if let Some(index) = position(bytes, start, end, required) {
                    seen[index] = true;
                }
                at = end + 1;
            }
            b'}' => panic!("a `}}` in a message has no `{{` before it"),
            _ => at += 1,
        }
    }
    let mut index = 0;
    while index < required.len() {
        assert!(seen[index], "a value is passed that the message never uses");
        index += 1;
    }
}

/// Checks a message with no values. It is returned as written, without `fill`, so a brace in it
/// would show twice (`{{`) or not be a value (`{x}`): it has none.
pub const fn plain(text: &str) {
    message(text, &[], &[]);
    let bytes = text.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        assert!(
            bytes[at] != b'{' && bytes[at] != b'}',
            "a message with no values has a brace in it; pass its values by name"
        );
        at += 1;
    }
}

/// The index in `names` of the one equal to `bytes[start..end]`.
const fn position(bytes: &[u8], start: usize, end: usize, names: &[&str]) -> Option<usize> {
    let mut index = 0;
    while index < names.len() {
        let name = names[index].as_bytes();
        if name.len() == end - start {
            let mut at = 0;
            while at < name.len() && name[at] == bytes[start + at] {
                at += 1;
            }
            if at == name.len() {
                return Some(index);
            }
        }
        index += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::message;

    // The same checks, which a test can run and a `const` block turns into a compile error.
    fn fails(text: &str, allowed: &[&str], required: &[&str]) -> bool {
        std::panic::catch_unwind(|| message(text, allowed, required)).is_err()
    }

    #[test]
    fn a_message_whose_values_match_passes() {
        message("Save", &[], &[]);
        message("{a} and {b}", &["a", "b"], &["a", "b"]);
        message("{a} {a}", &["a"], &["a"]);
        message("{{a}} is {a}", &["a"], &["a"]);
        // A counted message may leave its count out of a form.
        message("One alert", &["count"], &[]);
        message("{count} alerts", &["count"], &[]);
    }

    #[test]
    fn a_value_with_no_name_or_no_place_is_refused() {
        assert!(fails("{a} {b}", &["a"], &["a"]));
        assert!(fails("{a}", &["a", "b"], &["a", "b"]));
        assert!(fails("{count} of {n}", &["count"], &[]));
    }

    #[test]
    fn a_message_without_values_has_no_braces() {
        super::plain("Save");
        assert!(std::panic::catch_unwind(|| super::plain("a {{b}}")).is_err());
        assert!(std::panic::catch_unwind(|| super::plain("a {b}")).is_err());
    }

    #[test]
    fn padding_emptiness_and_stray_braces_are_refused() {
        assert!(fails("", &[], &[]));
        assert!(fails(" Save", &[], &[]));
        assert!(fails("Save ", &[], &[]));
        assert!(fails("{a", &["a"], &["a"]));
        assert!(fails("a } b", &[], &[]));
    }
}
