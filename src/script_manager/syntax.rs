//! Splits Pine source into categorized byte ranges for the editor, ported from
//! `PineSyntaxClassifier` / `PineSyntaxTheme`.
//!
//! Strings and numbers come from the real lexer, `//` comments (which the lexer drops) from a
//! scan that respects string tokens, and names are matched against the symbol catalog as whole
//! tokens or whole dotted chains, never substrings. Names the script declares stay plain.

use crate::pine::compiler::is_named_constant;
use crate::pine::lexer::Lexer;
use crate::pine::symbol_catalog as catalog;
use crate::pine::token::{Token, TokenKind};
use crate::ui::theme::syntax as colors;
use eframe::egui::{Color32, FontId, TextFormat, text::LayoutJob};
use std::collections::HashSet;
use std::ops::Range;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Category {
    Keyword,
    /// Value types, qualifiers and object types: `float`, `series`, `array`.
    Type,
    BuiltinNamespace,
    BuiltinFunction,
    /// Builtins whose value changes per bar or symbol: `close`, `bar_index`.
    BuiltinVariable,
    /// Builtins with a fixed value: `color.red`, `math.pi`, `true`, `na`.
    BuiltinConstant,
    Number,
    String,
    Comment,
    /// `//@version=6` and the other `//@…` directives.
    Annotation,
    Operator,
    ColorLiteral,
    /// A declared name, punctuation or anything Pine does not define: the default text colour.
    Plain,
}

impl Category {
    /// `None` leaves the text in the editor's default colour.
    pub fn color(self) -> Option<Color32> {
        Some(match self {
            Self::Keyword => colors::keyword(),
            Self::Type => colors::r#type(),
            Self::BuiltinNamespace => colors::namespace(),
            Self::BuiltinFunction => colors::function(),
            Self::BuiltinVariable => colors::variable(),
            Self::BuiltinConstant => colors::constant(),
            Self::Number => colors::number(),
            Self::String => colors::string(),
            Self::Comment => colors::comment(),
            Self::Annotation => colors::annotation(),
            Self::Operator => colors::operator(),
            Self::ColorLiteral => colors::color_literal(),
            Self::Plain => return None,
        })
    }
}

/// A categorized run of the source, in UTF-8 byte offsets.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Span {
    pub range: Range<usize>,
    pub category: Category,
}

/// Byte offset of every UTF-16 offset in `source`, plus one past the end.
pub fn utf16_to_byte_offsets(source: &str) -> Vec<usize> {
    let mut offsets = Vec::with_capacity(source.len() + 1);
    for (byte, character) in source.char_indices() {
        for _ in 0..character.len_utf16() {
            offsets.push(byte);
        }
    }
    offsets.push(source.len());
    offsets
}

/// The character index (what egui's cursor counts) at a UTF-16 offset.
pub fn utf16_to_char_index(source: &str, utf16: usize) -> usize {
    let mut units = 0;
    for (index, character) in source.chars().enumerate() {
        if units >= utf16 {
            return index;
        }
        units += character.len_utf16();
    }
    source.chars().count()
}

/// [`utf16_to_char_index`] for an offset into the text as the compiler reads it. The compiler
/// folds each CRLF into one line feed first, so its offsets are one short for every CRLF before
/// them; the text in the editor keeps both characters.
pub fn compiled_utf16_to_char_index(source: &str, utf16: usize) -> usize {
    let mut units = 0;
    let mut chars = source.chars().enumerate().peekable();
    while let Some((index, character)) = chars.next() {
        if units >= utf16 {
            return index;
        }
        if character == '\r' && chars.peek().is_some_and(|(_, next)| *next == '\n') {
            continue;
        }
        units += character.len_utf16();
    }
    source.chars().count()
}

fn namespaces() -> &'static HashSet<String> {
    static SET: OnceLock<HashSet<String>> = OnceLock::new();
    SET.get_or_init(|| {
        let mut set: HashSet<String> = [
            "color",
            "math",
            "ta",
            "str",
            "array",
            "matrix",
            "map",
            "line",
            "label",
            "box",
            "table",
            "linefill",
            "polyline",
            "chart",
            "strategy",
            "syminfo",
            "timeframe",
            "session",
            "barstate",
            "request",
            "input",
            "display",
            "size",
            "shape",
            "location",
            "extend",
            "xloc",
            "yloc",
            "barmerge",
            "scale",
            "dayofweek",
            "currency",
            "format",
            "order",
            "position",
            "plot",
            "hline",
            "text",
            "font",
            "alert",
            "adjustment",
            "backadjustment",
            "settlement_as_close",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
        for name in catalog::VARIABLES
            .iter()
            .chain(catalog::FUNCTIONS)
            .chain(catalog::RESERVED_CONSTANTS)
        {
            if let Some((head, _)) = name.split_once('.') {
                set.insert(head.to_owned());
            }
        }
        set
    })
}

const QUALIFIERS: [&str; 4] = ["series", "simple", "const", "input"];
const RESERVED_WORDS: [&str; 8] = [
    "import", "export", "method", "type", "enum", "as", "in", "to",
];

fn token_name(token: &Token) -> Option<String> {
    match &token.kind {
        TokenKind::Identifier(name) => Some(name.clone()),
        TokenKind::TypeKeyword(kind) => Some(format!("{kind:?}").to_lowercase()),
        _ => None,
    }
}

/// Names the script declares: assignment targets, function names, loop variables.
fn declared_names(tokens: &[Token]) -> HashSet<String> {
    let mut declared = HashSet::new();
    let mut depth = 0usize;
    for (index, token) in tokens.iter().enumerate() {
        match &token.kind {
            TokenKind::LeftParen | TokenKind::LeftBracket => depth += 1,
            TokenKind::RightParen | TokenKind::RightBracket => depth = depth.saturating_sub(1),
            TokenKind::Identifier(name) => {
                let previous = index.checked_sub(1).map(|i| &tokens[i].kind);
                let next = tokens.get(index + 1).map(|t| &t.kind);
                if previous == Some(&TokenKind::Dot) {
                    continue;
                }
                let assigned =
                    depth == 0 && matches!(next, Some(TokenKind::Assign | TokenKind::Reassign));
                let after_for = previous == Some(&TokenKind::For);
                // `name(args) =>`: a function definition.
                let defines =
                    next == Some(&TokenKind::LeftParen) && is_function_definition(tokens, index);
                if assigned || after_for || defines {
                    declared.insert(name.clone());
                }
            }
            _ => {}
        }
    }
    declared
}

fn is_function_definition(tokens: &[Token], name_index: usize) -> bool {
    let mut depth = 0usize;
    for (offset, token) in tokens[name_index + 1..].iter().enumerate() {
        match token.kind {
            TokenKind::LeftParen => depth += 1,
            TokenKind::RightParen => {
                depth -= 1;
                if depth == 0 {
                    return tokens
                        .get(name_index + 1 + offset + 1)
                        .is_some_and(|t| t.kind == TokenKind::Arrow);
                }
            }
            TokenKind::Newline | TokenKind::Eof => return false,
            _ => {}
        }
    }
    false
}

/// The names the script declares (assignment targets, functions, loop variables), for completion.
pub fn declared_symbols(source: &str) -> HashSet<String> {
    declared_names(&Lexer::new(source).lex().tokens)
}

/// Classifies the source. Spans are sorted, non-overlapping and cover only what is not plain.
pub fn classify(source: &str) -> Vec<Span> {
    let result = Lexer::new(source).lex();
    let offsets = utf16_to_byte_offsets(source);
    let byte = |utf16: usize| offsets.get(utf16).copied().unwrap_or(source.len());
    let tokens = &result.tokens;
    let declared = declared_names(tokens);
    let mut spans: Vec<Span> = Vec::new();
    let mut strings: Vec<Range<usize>> = Vec::new();
    let range_of = |token: &Token| byte(token.range.start.offset)..byte(token.range.end.offset);

    let mut index = 0;
    let mut paren_depth = 0usize;
    while index < tokens.len() {
        let token = &tokens[index];
        let range = range_of(token);
        let next_kind = tokens.get(index + 1).map(|t| &t.kind);
        let mut push = |range: Range<usize>, category: Category| {
            if range.start < range.end && category != Category::Plain {
                spans.push(Span { range, category });
            }
        };
        match &token.kind {
            TokenKind::Identifier(_) | TokenKind::TypeKeyword(_) => {
                // After a `.` this is a member of a value, not a chain head.
                if index > 0 && tokens[index - 1].kind == TokenKind::Dot {
                    index += 1;
                    continue;
                }
                let mut chain = vec![index];
                while let Some(&last) = chain.last()
                    && tokens.get(last + 1).map(|t| &t.kind) == Some(&TokenKind::Dot)
                    && tokens.get(last + 2).and_then(token_name).is_some()
                {
                    chain.push(last + 2);
                }
                let end = chain[chain.len() - 1] + 1;
                let names: Vec<String> = chain
                    .iter()
                    .map(|&i| token_name(&tokens[i]).unwrap_or_default())
                    .collect();
                let is_call = tokens.get(end).map(|t| &t.kind) == Some(&TokenKind::LeftParen);
                let categories = categories_of(
                    &names,
                    &tokens[chain[0]].kind,
                    is_call,
                    &declared,
                    paren_depth > 0 && next_kind == Some(&TokenKind::Assign),
                    next_kind,
                );
                for (position, &token_index) in chain.iter().enumerate() {
                    push(range_of(&tokens[token_index]), categories[position]);
                }
                index = end;
                continue;
            }
            TokenKind::Number { .. } => push(range, Category::Number),
            TokenKind::Color(_) => push(range, Category::ColorLiteral),
            TokenKind::Bool(_) => push(range, Category::BuiltinConstant),
            TokenKind::Na => push(
                range,
                if next_kind == Some(&TokenKind::LeftParen) {
                    Category::BuiltinFunction
                } else {
                    Category::BuiltinConstant
                },
            ),
            TokenKind::String(_) => {
                strings.push(range.clone());
                push(range, Category::String);
            }
            TokenKind::And
            | TokenKind::Or
            | TokenKind::Not
            | TokenKind::If
            | TokenKind::Else
            | TokenKind::Var
            | TokenKind::Varip
            | TokenKind::For
            | TokenKind::Break
            | TokenKind::Continue
            | TokenKind::While
            | TokenKind::Switch => push(range, Category::Keyword),
            TokenKind::LeftParen => paren_depth += 1,
            TokenKind::RightParen => paren_depth = paren_depth.saturating_sub(1),
            TokenKind::Plus
            | TokenKind::Minus
            | TokenKind::Star
            | TokenKind::Slash
            | TokenKind::Percent
            | TokenKind::Power
            | TokenKind::Assign
            | TokenKind::Reassign
            | TokenKind::PlusAssign
            | TokenKind::MinusAssign
            | TokenKind::StarAssign
            | TokenKind::SlashAssign
            | TokenKind::Equal
            | TokenKind::NotEqual
            | TokenKind::Less
            | TokenKind::LessEqual
            | TokenKind::Greater
            | TokenKind::GreaterEqual
            | TokenKind::Question
            | TokenKind::Colon
            | TokenKind::Arrow => push(range, Category::Operator),
            _ => {}
        }
        index += 1;
    }

    // `//` comments and `//@` annotations: the lexer drops them, so find each outside any string.
    let mut line_start = 0;
    for line in source.split('\n') {
        let mut search = 0;
        while let Some(found) = line[search..].find("//") {
            let at = line_start + search + found;
            if strings.iter().any(|string| string.contains(&at)) {
                search += found + 2;
                continue;
            }
            let end = line_start + line.len();
            let annotation = annotation_end(&source[at..end]).map(|len| at + len);
            match annotation {
                Some(stop) => {
                    spans.push(Span {
                        range: at..stop,
                        category: Category::Annotation,
                    });
                    if stop < end {
                        spans.push(Span {
                            range: stop..end,
                            category: Category::Comment,
                        });
                    }
                }
                None => spans.push(Span {
                    range: at..end,
                    category: Category::Comment,
                }),
            }
            break;
        }
        line_start += line.len() + 1;
    }
    spans.sort_by_key(|span| (span.range.start, std::cmp::Reverse(span.range.end)));
    // A comment that begins inside an already-coloured run (a stray token boundary) wins over
    // anything it overlaps; drop spans that start inside an earlier one.
    let mut kept: Vec<Span> = Vec::with_capacity(spans.len());
    for span in spans {
        if let Some(last) = kept.last()
            && span.range.start < last.range.end
        {
            continue;
        }
        kept.push(span);
    }
    kept
}

/// `//@version=6` → its length: `//@` + letters/underscores + an optional `=digits`.
fn annotation_end(comment: &str) -> Option<usize> {
    let rest = comment.strip_prefix("//@")?;
    let name = rest
        .bytes()
        .take_while(|b| b.is_ascii_alphabetic() || *b == b'_')
        .count();
    if name == 0 {
        return None;
    }
    let mut end = 3 + name;
    let tail = &comment[end..];
    if let Some(digits) = tail.strip_prefix('=') {
        let n = digits.bytes().take_while(u8::is_ascii_digit).count();
        if n > 0 {
            end += 1 + n;
        }
    }
    Some(end)
}

fn categories_of(
    names: &[String],
    head_kind: &TokenKind,
    is_call: bool,
    declared: &HashSet<String>,
    named_argument: bool,
    next: Option<&TokenKind>,
) -> Vec<Category> {
    let plain = vec![Category::Plain; names.len()];
    let head = names[0].as_str();
    if names.len() == 1 {
        // `title = "x"` inside a call is a named argument, whatever the name is.
        if named_argument || declared.contains(head) {
            return plain;
        }
        let type_start = matches!(
            next,
            Some(TokenKind::Identifier(_) | TokenKind::TypeKeyword(_))
        );
        let category = if matches!(head_kind, TokenKind::TypeKeyword(_)) {
            if is_call {
                Category::BuiltinFunction
            } else {
                Category::Type
            }
        } else if RESERVED_WORDS.contains(&head) && !is_call {
            Category::Keyword
        } else if QUALIFIERS.contains(&head) && !is_call && type_start {
            Category::Type
        } else if catalog::OBJECT_TYPES.contains(&head) {
            if is_call {
                Category::BuiltinFunction
            } else {
                Category::Type
            }
        } else if is_call {
            if catalog::FUNCTIONS.contains(&head) {
                Category::BuiltinFunction
            } else {
                Category::Plain
            }
        } else if catalog::is_variable(head) {
            Category::BuiltinVariable
        } else {
            Category::Plain
        };
        return vec![category];
    }
    if declared.contains(head) || !namespaces().contains(head) {
        return plain;
    }
    let mut result = vec![Category::BuiltinNamespace; names.len()];
    let full = names.join(".");
    let last = names.len() - 1;
    result[last] = if is_named_constant(&full) || catalog::is_reserved_constant(&full) {
        Category::BuiltinConstant
    } else if is_call {
        Category::BuiltinFunction
    } else if catalog::is_variable(&full) {
        Category::BuiltinVariable
    } else {
        Category::Plain
    };
    result
}

/// The source as a coloured layout.
pub fn layout_job(source: &str, spans: &[Span], font: FontId, default: Color32) -> LayoutJob {
    let mut job = LayoutJob::default();
    let mut cursor = 0;
    let format = |color: Color32| TextFormat::simple(font.clone(), color);
    for span in spans {
        if span.range.start < cursor || span.range.end > source.len() {
            continue;
        }
        if span.range.start > cursor {
            job.append(&source[cursor..span.range.start], 0.0, format(default));
        }
        job.append(
            &source[span.range.clone()],
            0.0,
            format(span.category.color().unwrap_or(default)),
        );
        cursor = span.range.end;
    }
    if cursor < source.len() {
        job.append(&source[cursor..], 0.0, format(default));
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(source: &str) -> Vec<(String, Category)> {
        classify(source)
            .iter()
            .map(|s| (source[s.range.clone()].to_owned(), s.category))
            .collect()
    }

    fn has(found: &[(String, Category)], text: &str, category: Category) -> bool {
        found.iter().any(|(t, c)| t == text && *c == category)
    }

    #[test]
    fn compiler_offsets_skip_the_carriage_return_of_a_crlf() {
        // The compiler sees "a\nb\nc"; the editor holds "a\r\nb\r\nc".
        let crlf = "a\r\nb\r\nc";
        assert_eq!(compiled_utf16_to_char_index(crlf, 0), 0);
        assert_eq!(compiled_utf16_to_char_index(crlf, 2), 3);
        assert_eq!(compiled_utf16_to_char_index(crlf, 4), 6);
        // Without CRLF it is the plain conversion, astral characters counting two units.
        let plain = "a😀b\nc";
        for offset in 0..=6 {
            assert_eq!(
                compiled_utf16_to_char_index(plain, offset),
                utf16_to_char_index(plain, offset)
            );
        }
        // A lone CR was folded into a line feed by the compiler, one unit as it is here.
        assert_eq!(compiled_utf16_to_char_index("a\rb", 2), 2);
    }

    #[test]
    fn a_diagnostic_in_a_crlf_script_is_found_where_it_is() {
        let source =
            "//@version=6\r\nindicator(\"T\")\r\n// a comment\r\n\r\n\r\nlog.hi(\"hiii\")\r\n";
        let program = crate::pine::compiler::compile(
            source,
            crate::pine::token::Limits::default(),
            &crate::pine::compiler::NoLibraries,
        );
        let diagnostic = program
            .diagnostics
            .iter()
            .find(|d| d.code == "PINE3051")
            .expect("log.hi is not a function");
        let index = compiled_utf16_to_char_index(source, diagnostic.range.start.offset);
        let rest: String = source.chars().skip(index).collect();
        assert!(rest.starts_with("log.hi"), "{rest:?}");
        // The plain conversion lands in the line above, which is what clicking the problem did.
        let wrong = utf16_to_char_index(source, diagnostic.range.start.offset);
        assert!(
            !source
                .chars()
                .skip(wrong)
                .collect::<String>()
                .starts_with("log.hi")
        );
    }

    #[test]
    fn builtins_are_matched_as_whole_chains() {
        let source = "//@version=6\nindicator(\"x\", overlay=true)\nlen = 14\nx = ta.sma(close, len)\nplot(x, color=color.red)\nif barstate.isfirst\n    na\n";
        let f = found(source);
        assert!(has(&f, "//@version=6", Category::Annotation), "{f:?}");
        assert!(has(&f, "indicator", Category::BuiltinFunction));
        assert!(has(&f, "\"x\"", Category::String));
        assert!(has(&f, "true", Category::BuiltinConstant));
        assert!(has(&f, "14", Category::Number));
        assert!(has(&f, "ta", Category::BuiltinNamespace));
        assert!(has(&f, "sma", Category::BuiltinFunction));
        assert!(has(&f, "close", Category::BuiltinVariable));
        assert!(has(&f, "red", Category::BuiltinConstant));
        assert!(has(&f, "if", Category::Keyword));
        assert!(has(&f, "isfirst", Category::BuiltinVariable));
        assert!(has(&f, "na", Category::BuiltinConstant));
        // Declared names, named arguments and unknown names stay plain (no span).
        assert!(
            !f.iter()
                .any(|(t, _)| t == "len" || t == "x" || t == "overlay")
        );
    }

    #[test]
    fn user_declarations_shadow_builtins() {
        let f = found("close = 1\nplot(close)\nmy(a) =>\n    a\nmy(2)\n");
        assert!(!f.iter().any(|(t, _)| t == "close"), "{f:?}");
        assert!(!f.iter().any(|(t, _)| t == "my"), "{f:?}");
    }

    #[test]
    fn types_colour_literals_and_operators() {
        let f = found("var float x = na\ny = #FF8800 + 1\n");
        assert!(has(&f, "var", Category::Keyword), "{f:?}");
        assert!(has(&f, "float", Category::Type));
        assert!(has(&f, "#FF8800", Category::ColorLiteral));
        assert!(has(&f, "+", Category::Operator));
        assert!(has(&f, "=", Category::Operator));
    }

    #[test]
    fn comments_annotations_and_strings_do_not_mix() {
        let source = "a = 1 // trailing\nb = \"http://x\" // real\n//@version=6 // note\n";
        let f = found(source);
        let comments: Vec<&str> = f
            .iter()
            .filter(|(_, c)| *c == Category::Comment)
            .map(|(t, _)| t.as_str())
            .collect();
        assert_eq!(comments, ["// trailing", "// real", " // note"]);
        assert!(has(&f, "\"http://x\"", Category::String));
        assert!(has(&f, "//@version=6", Category::Annotation));
    }

    #[test]
    fn multibyte_text_keeps_offsets_aligned() {
        let source = "// 日本語 ✓\nlabel = \"héllo\"\nplot(1)\n";
        let f = found(source);
        assert!(has(&f, "// 日本語 ✓", Category::Comment), "{f:?}");
        assert!(has(&f, "\"héllo\"", Category::String));
        assert!(has(&f, "plot", Category::BuiltinFunction));
    }

    #[test]
    fn utf16_offsets_map_to_bytes_and_character_indexes() {
        let source = "a😀b";
        assert_eq!(utf16_to_byte_offsets(source), [0, 1, 1, 5, 6]);
        assert_eq!(utf16_to_char_index(source, 3), 2);
        assert_eq!(utf16_to_char_index(source, 99), 3);
    }

    #[test]
    fn the_layout_covers_the_whole_source_in_order() {
        let source = "plot(1) // note";
        let job = layout_job(
            source,
            &classify(source),
            FontId::monospace(13.0),
            Color32::WHITE,
        );
        assert_eq!(job.text, source);
        assert_eq!(
            layout_job("", &[], FontId::monospace(13.0), Color32::WHITE).text,
            ""
        );
    }
}
