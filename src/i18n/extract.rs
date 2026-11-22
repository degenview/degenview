//! Reads the `tr!`, `tr_c!` and `tr_n!` calls out of Rust source, for the tests that keep the
//! catalogs and the code in step and for the `.pot` template translators start from.
//!
//! The source is tokenised the way the compiler does (`proc-macro2`), so comments, doc comments
//! and string literals can never be taken for a call, and a call inside another macro (`format!`,
//! `assert_eq!`) or a closure is found like any other.

use proc_macro2::{Delimiter, TokenStream, TokenTree};

/// Which macro a message was written with.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Kind {
    Plain,
    Context(String),
    /// A counted message; the key is its `other` text, this is its `one` text.
    Counted(String),
}

/// One message in the source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Use {
    pub line: usize,
    pub kind: Kind,
    /// The text; for a counted message, its `other` form.
    pub key: String,
    /// The names of the values the call passes (a counted message always passes `count`).
    pub args: Vec<String>,
}

impl Use {
    pub fn context(&self) -> &str {
        match &self.kind {
            Kind::Context(context) => context,
            _ => "",
        }
    }

    /// The text a catalog is keyed by: gettext's `msgid`, which for a counted message is the
    /// singular.
    pub fn msgid(&self) -> &str {
        match &self.kind {
            Kind::Counted(one) => one,
            _ => &self.key,
        }
    }
}

/// Every message `source` writes, or the line and reason of the first call that cannot be read.
pub fn uses(source: &str) -> Result<Vec<Use>, String> {
    let tokens: TokenStream = source
        .parse()
        .map_err(|error| format!("is not Rust source: {error}"))?;
    let mut found = Vec::new();
    collect(tokens, &mut found)?;
    Ok(found)
}

fn collect(tokens: TokenStream, found: &mut Vec<Use>) -> Result<(), String> {
    let trees: Vec<TokenTree> = tokens.into_iter().collect();
    let mut at = 0;
    while at < trees.len() {
        if let TokenTree::Group(group) = &trees[at] {
            collect(group.stream(), found)?;
        } else if let (
            TokenTree::Ident(name),
            Some(TokenTree::Punct(bang)),
            Some(TokenTree::Group(call)),
        ) = (&trees[at], trees.get(at + 1), trees.get(at + 2))
            && bang.as_char() == '!'
            && matches!(name.to_string().as_str(), "tr" | "tr_c" | "tr_n")
            && call.delimiter() != Delimiter::None
        {
            let line = name.span().start().line;
            let used = read_call(&name.to_string(), call.stream())
                .map(|(kind, key, args)| Use {
                    line,
                    kind,
                    key,
                    args,
                })
                .map_err(|reason| format!("line {line}: {name}! {reason}"))?;
            found.push(used);
            // A value can hold another call (`time = format!(…)`, a nested `tr!`).
            collect(call.stream(), found)?;
            at += 3;
            continue;
        }
        at += 1;
    }
    Ok(())
}

type Call = (Kind, String, Vec<String>);

fn read_call(name: &str, stream: TokenStream) -> Result<Call, String> {
    let mut parts = split_commas(stream).into_iter();
    let mut next = |what: &str| parts.next().ok_or_else(|| format!("is missing {what}"));
    match name {
        "tr" => {
            let key = literal(&next("its text")?)?;
            Ok((Kind::Plain, key, names(parts)?))
        }
        "tr_c" => {
            let context = literal(&next("its context")?)?;
            let key = literal(&next("its text")?)?;
            Ok((Kind::Context(context), key, names(parts)?))
        }
        _ => {
            next("its count")?;
            let one = literal(&next("its singular")?)?;
            let other = literal(&next("its plural")?)?;
            let mut args = names(parts)?;
            args.insert(0, "count".to_owned());
            Ok((Kind::Counted(one), other, args))
        }
    }
}

/// The top-level arguments of a call: a `,` inside brackets belongs to the argument.
fn split_commas(stream: TokenStream) -> Vec<Vec<TokenTree>> {
    let mut parts = vec![Vec::new()];
    for tree in stream {
        match &tree {
            TokenTree::Punct(punct) if punct.as_char() == ',' => parts.push(Vec::new()),
            _ => parts.last_mut().expect("one part").push(tree),
        }
    }
    if parts.last().is_some_and(Vec::is_empty) {
        parts.pop();
    }
    parts
}

/// The value of an argument that is a plain or raw string literal and nothing else.
fn literal(tokens: &[TokenTree]) -> Result<String, String> {
    let [TokenTree::Literal(literal)] = tokens else {
        return Err("takes a string literal here".to_owned());
    };
    syn::parse_str::<syn::LitStr>(&literal.to_string())
        .map(|text| text.value())
        .map_err(|_| "takes a string literal here".to_owned())
}

/// The names of `name = value` arguments.
fn names(parts: impl Iterator<Item = Vec<TokenTree>>) -> Result<Vec<String>, String> {
    parts
        .map(|part| match part.as_slice() {
            [TokenTree::Ident(name), TokenTree::Punct(equals), ..] if equals.as_char() == '=' => {
                Ok(name.to_string())
            }
            _ => Err("passes a value that is not `name = value`".to_owned()),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(source: &str) -> Use {
        let mut all = uses(source).expect("reads");
        assert_eq!(all.len(), 1, "{all:?}");
        all.remove(0)
    }

    #[test]
    fn a_plain_message_and_its_values_are_read() {
        let used = one(r#"fn f() { let _ = tr!("Hi {name}", name = who()); }"#);
        assert_eq!(used.kind, Kind::Plain);
        assert_eq!(used.key, "Hi {name}");
        assert_eq!(used.args, ["name"]);
        assert_eq!(used.line, 1);
    }

    #[test]
    fn a_context_and_a_counted_message_are_read() {
        let used = one(r#"fn f() { tr_c!("month", "{n} left", n = 3) }"#);
        assert_eq!(used.kind, Kind::Context("month".into()));
        assert_eq!(used.context(), "month");
        assert_eq!(used.args, ["n"]);

        let used =
            one(r#"fn f() { tr_n!(items.len(), "{count} alert", "{count} alerts", total = 9) }"#);
        assert_eq!(used.kind, Kind::Counted("{count} alert".into()));
        assert_eq!(used.key, "{count} alerts");
        assert_eq!(used.msgid(), "{count} alert");
        assert_eq!(used.args, ["count", "total"]);
    }

    #[test]
    fn a_path_a_nested_call_and_a_raw_string_are_found() {
        let used = one(r##"fn f() { crate::i18n::tr!(r#"Say "hi""#) }"##);
        assert_eq!(used.key, r#"Say "hi""#);
        let all = uses(
            r#"fn f() { tr!("{month} {time}", month = m, time = format!("{}", tr!("Now"))) }"#,
        )
        .expect("reads");
        let keys: Vec<&str> = all.iter().map(|used| used.key.as_str()).collect();
        assert_eq!(keys, ["{month} {time}", "Now"]);
    }

    #[test]
    fn escapes_and_line_continuations_are_decoded() {
        let used = one("fn f() { tr!(\"a\\n\\u{e9}\\\"b\\\n    c\") }");
        assert_eq!(used.key, "a\né\"bc");
    }

    #[test]
    fn a_comma_inside_brackets_belongs_to_its_value() {
        let used = one(r#"fn f() { tr!("{a} {b}", a = f(1, 2), b = [3, 4].len()) }"#);
        assert_eq!(used.args, ["a", "b"]);
    }

    #[test]
    fn comments_strings_definitions_and_other_macros_are_not_calls() {
        let source = r#"
            // tr!("In a comment")
            /* tr!("In a block comment") */
            /// tr!("In a doc comment")
            macro_rules! tr { ($key:literal) => {}; }
            pub(crate) use {tr, tr_c, tr_n};
            fn f() { let _ = "tr!(\"in a string\")"; print!("tr"); tr!("Real"); }
        "#;
        let all = uses(source).expect("reads");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].key, "Real");
    }

    #[test]
    fn a_call_that_cannot_be_read_names_its_line() {
        let error = uses("fn f() {\n    tr!(TEXT)\n}").expect_err("not a literal");
        assert!(error.starts_with("line 2: tr!"), "{error}");
        assert!(uses(r#"fn f() { tr!(concat!("a", "b")) }"#).is_err());
        assert!(uses(r#"fn f() { tr!("{a}", 5) }"#).is_err());
        assert!(uses(r#"fn f() { tr_c!("context") }"#).is_err());
    }
}
