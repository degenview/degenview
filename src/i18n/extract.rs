//! Reads the `tr!`, `tr_c!` and `tr_n!` calls out of Rust source, for the tests that keep the
//! catalogs and the code in step. A message is found by its text, so the text must be a plain
//! string literal, which the macros already require.

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
}

const MACROS: [&str; 3] = ["tr!(", "tr_c!(", "tr_n!("];

/// Every message `source` writes, or the line and reason of the first call that cannot be read.
pub fn uses(source: &str) -> Result<Vec<Use>, String> {
    let mut found = Vec::new();
    let mut offset = 0;
    for text_line in source.split_inclusive('\n') {
        let trimmed = text_line.trim_start();
        let is_comment = trimmed.starts_with("//");
        if !is_comment {
            for name in MACROS {
                let mut from = 0;
                while let Some(at) = text_line[from..].find(name) {
                    let start = from + at;
                    from = start + name.len();
                    let before = text_line[..start].chars().next_back();
                    if before.is_some_and(|ch| ch.is_alphanumeric() || ch == '_') {
                        continue;
                    }
                    let line = source[..offset].matches('\n').count() + 1;
                    let body = &source[offset + from..];
                    found.push(
                        read_call(name, body)
                            .map(|(kind, key, args)| Use {
                                line,
                                kind,
                                key,
                                args,
                            })
                            .map_err(|reason| format!("line {line}: {name} {reason}"))?,
                    );
                }
            }
        }
        offset += text_line.len();
    }
    Ok(found)
}

type Call = (Kind, String, Vec<String>);

fn read_call(name: &str, body: &str) -> Result<Call, String> {
    let mut cursor = Cursor { text: body, at: 0 };
    match name {
        "tr!(" => {
            let key = cursor.literal()?;
            Ok((Kind::Plain, key, cursor.names()?))
        }
        "tr_c!(" => {
            let context = cursor.literal()?;
            cursor.comma()?;
            let key = cursor.literal()?;
            Ok((Kind::Context(context), key, cursor.names()?))
        }
        _ => {
            cursor.skip_expression()?;
            cursor.comma()?;
            let one = cursor.literal()?;
            cursor.comma()?;
            let other = cursor.literal()?;
            let mut names = cursor.names()?;
            names.insert(0, "count".to_owned());
            Ok((Kind::Counted(one), other, names))
        }
    }
}

struct Cursor<'a> {
    text: &'a str,
    at: usize,
}

impl Cursor<'_> {
    fn rest(&self) -> &str {
        &self.text[self.at..]
    }

    fn skip_space(&mut self) {
        self.at += self.rest().len() - self.rest().trim_start().len();
    }

    fn comma(&mut self) -> Result<(), String> {
        self.skip_space();
        if self.rest().starts_with(',') {
            self.at += 1;
            Ok(())
        } else {
            Err("expects a comma here".to_owned())
        }
    }

    /// A string literal's value, plain or raw.
    fn literal(&mut self) -> Result<String, String> {
        self.skip_space();
        let rest = self.rest();
        if let Some(raw) = rest.strip_prefix('r') {
            let hashes = raw.chars().take_while(|ch| *ch == '#').count();
            let open = &raw[hashes..];
            if let Some(content) = open.strip_prefix('"') {
                let close = format!("\"{}", "#".repeat(hashes));
                let end = content
                    .find(&close)
                    .ok_or("has an unterminated raw string")?;
                let value = content[..end].to_owned();
                self.at += 1 + hashes + 1 + end + close.len();
                return Ok(value);
            }
        }
        let Some(content) = rest.strip_prefix('"') else {
            return Err("takes a string literal".to_owned());
        };
        let mut value = String::new();
        let mut chars = content.char_indices();
        while let Some((index, ch)) = chars.next() {
            match ch {
                '"' => {
                    self.at += 1 + index + 1;
                    return Ok(value);
                }
                '\\' => match chars.next().map(|(_, escaped)| escaped) {
                    Some('n') => value.push('\n'),
                    Some('t') => value.push('\t'),
                    Some('r') => value.push('\r'),
                    Some('0') => value.push('\0'),
                    Some(literal @ ('"' | '\'' | '\\')) => value.push(literal),
                    Some('u') => {
                        let mut digits = String::new();
                        for (_, digit) in chars.by_ref() {
                            if digit == '}' {
                                break;
                            }
                            if digit != '{' {
                                digits.push(digit);
                            }
                        }
                        value.push(
                            u32::from_str_radix(&digits, 16)
                                .ok()
                                .and_then(char::from_u32)
                                .ok_or("has a bad \\u escape")?,
                        );
                    }
                    // A backslash at the end of a line drops the line break and the indent.
                    Some('\n') => {
                        while chars
                            .clone()
                            .next()
                            .is_some_and(|(_, next)| next.is_whitespace())
                        {
                            chars.next();
                        }
                    }
                    _ => return Err("has an escape the tests cannot read".to_owned()),
                },
                _ => value.push(ch),
            }
        }
        Err("has an unterminated string".to_owned())
    }

    /// Skips one argument (the count of a counted message), which may hold calls and strings.
    fn skip_expression(&mut self) -> Result<(), String> {
        let mut depth = 0i32;
        let mut chars = self.rest().char_indices();
        while let Some((index, ch)) = chars.next() {
            match ch {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                ',' if depth == 0 => {
                    self.at += index;
                    return Ok(());
                }
                '"' => {
                    for (_, inner) in chars.by_ref() {
                        if inner == '"' {
                            break;
                        }
                    }
                }
                _ => {}
            }
            if depth < 0 {
                break;
            }
        }
        Err("has no message after its count".to_owned())
    }

    /// The `name = value` pairs after the text, up to the closing parenthesis.
    fn names(&mut self) -> Result<Vec<String>, String> {
        let mut names = Vec::new();
        loop {
            self.skip_space();
            if self.rest().starts_with(')') {
                return Ok(names);
            }
            self.comma()?;
            self.skip_space();
            if self.rest().starts_with(')') {
                return Ok(names);
            }
            let name: String = self
                .rest()
                .chars()
                .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
                .collect();
            if name.is_empty() {
                return Err("passes a value without a name".to_owned());
            }
            self.at += name.len();
            self.skip_space();
            if !self.rest().starts_with('=') || self.rest().starts_with("==") {
                return Err(format!("passes {name} without `{name} = value`"));
            }
            self.at += 1;
            names.push(name);
            self.skip_value()?;
        }
    }

    /// Skips a value up to the comma or parenthesis that ends it.
    fn skip_value(&mut self) -> Result<(), String> {
        let mut depth = 0i32;
        let mut chars = self.rest().char_indices();
        while let Some((index, ch)) = chars.next() {
            match ch {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' if depth > 0 => depth -= 1,
                ')' | ',' if depth == 0 => {
                    self.at += index;
                    return Ok(());
                }
                '"' => {
                    while let Some((_, inner)) = chars.next() {
                        match inner {
                            '\\' => {
                                chars.next();
                            }
                            '"' => break,
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        Err("is not closed".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(source: &str) -> Use {
        let mut found = uses(source).expect("reads");
        assert_eq!(found.len(), 1, "{source}");
        found.remove(0)
    }

    #[test]
    fn a_plain_message_and_the_names_it_is_given() {
        let plain = one(r#"let x = tr!("Save");"#);
        assert_eq!((plain.kind, plain.key.as_str()), (Kind::Plain, "Save"));
        assert!(plain.args.is_empty());

        let named = one(r#"tr!("{a} and {b}", a = f(1, 2), b = "x, y")"#);
        assert_eq!(named.args, ["a", "b"]);
    }

    #[test]
    fn a_message_with_a_context_and_a_counted_one() {
        let context = one(r#"crate::i18n::tr_c!("month abbreviation", "Jan")"#);
        assert_eq!(context.kind, Kind::Context("month abbreviation".into()));
        assert_eq!(context.key, "Jan");
        assert_eq!(context.context(), "month abbreviation");

        let counted = one(r#"tr_n!(items.len(), "{count} item", "{count} items", list = x)"#);
        assert_eq!(counted.kind, Kind::Counted("{count} item".into()));
        assert_eq!(counted.key, "{count} items");
        assert_eq!(counted.args, ["count", "list"]);
    }

    #[test]
    fn a_count_may_be_any_expression() {
        let counted = one(r#"tr_n!(f(a, [1, 2]) as u64, "one", "many")"#);
        assert_eq!(counted.key, "many");
        assert_eq!(counted.args, ["count"]);
    }

    #[test]
    fn escapes_continuations_and_raw_strings_read_as_rust_reads_them() {
        assert_eq!(one(r#"tr!("a\"b\\c\n\u{2192}")"#).key, "a\"b\\c\n→");
        assert_eq!(one("tr!(\"long \\\n        line\")").key, "long line");
        assert_eq!(one(r##"tr!(r#"say "hi""#)"##).key, "say \"hi\"");
    }

    #[test]
    fn comments_and_other_macros_with_a_similar_name_are_not_read() {
        let source = "// tr!(\"in a comment\")\n/// tr!(\"in a doc\")\nlet a = attr!(\"x\");\nlet b = my_tr!(\"y\");\n";
        assert!(uses(source).expect("reads").is_empty());
    }

    #[test]
    fn the_line_of_each_message_is_reported() {
        let found = uses("a\n\ntr!(\"x\")\n").expect("reads");
        assert_eq!(found[0].line, 3);
    }

    #[test]
    fn a_call_that_cannot_be_read_says_where() {
        let error = uses("a\ntr!(some_text)\n").unwrap_err();
        assert!(error.starts_with("line 2:"), "{error}");
        assert!(uses("tr!(\"x\", y)").is_err());
        assert!(uses("tr!(\"unterminated)").is_err());
    }
}
