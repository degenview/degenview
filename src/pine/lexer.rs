//! Turns Pine source text into [`Token`]s for the parser.

use super::token::{
    Diagnostic, DiagnosticCategory, Limits, SourcePosition, SourceRange, Token, TokenKind,
    ValueType,
};

#[derive(Clone, Debug, PartialEq)]
pub struct LexResult {
    pub tokens: Vec<Token>,
    pub diagnostics: Vec<Diagnostic>,
}

pub struct Lexer<'source> {
    source: &'source str,
    limits: Limits,
}

impl<'source> Lexer<'source> {
    pub fn new(source: &'source str) -> Self {
        Self::with_limits(source, Limits::default())
    }

    pub const fn with_limits(source: &'source str, limits: Limits) -> Self {
        Self { source, limits }
    }

    pub fn lex(&self) -> LexResult {
        if self.source.chars().count() > self.limits.source_characters {
            return LexResult {
                tokens: vec![Token::new(TokenKind::Eof, SourceRange::ZERO)],
                diagnostics: vec![Diagnostic::error(
                    "PINE8001",
                    DiagnosticCategory::Resource,
                    format!(
                        "Source exceeds the {}-character limit.",
                        self.limits.source_characters
                    ),
                    SourceRange::ZERO,
                )],
            };
        }

        let raw_lines: Vec<&str> = self.source.split('\n').collect();
        let logical_lines = join_multiline_strings(&raw_lines);
        let mut state = State::default();
        let mut last_line = SourceLine::new("", 1, 0);

        for (raw, number) in &logical_lines {
            let line = SourceLine::new(raw, *number, state.offset);
            last_line = line.clone();
            self.lex_line(&line, &mut state);
            state.offset += line.utf16_len() + 1;

            if state.tokens.len() > self.limits.tokens {
                let range = state
                    .tokens
                    .last()
                    .map_or(SourceRange::ZERO, |token| token.range);
                state.diagnostics.push(Diagnostic::error(
                    "PINE8002",
                    DiagnosticCategory::Resource,
                    "Token limit exceeded.",
                    range,
                ));
                break;
            }
        }

        let source_end = self.source.encode_utf16().count();
        let eof_position =
            SourcePosition::new(raw_lines.len(), last_line.character_len() + 1, source_end);
        let eof_range = SourceRange::new(eof_position, eof_position);
        while state.indents.len() > 1 {
            state.indents.pop();
            state.tokens.push(Token::new(TokenKind::Dedent, eof_range));
        }
        state.tokens.push(Token::new(TokenKind::Eof, eof_range));

        LexResult {
            tokens: state.tokens,
            diagnostics: state.diagnostics,
        }
    }

    fn lex_line(&self, line: &SourceLine, state: &mut State) {
        let leading = line
            .text
            .iter()
            .take_while(|character| matches!(character, ' ' | '\t'))
            .count();
        let rest = &line.text[leading..];
        if rest.is_empty() || rest.starts_with(&['/', '/']) {
            return;
        }

        if state.is_at_logical_line_start() {
            if Self::is_wrapped_line(leading, line, state) {
                state.tokens.pop();
            } else {
                self.lex_indentation(leading, line, state);
            }
        }

        let mut index = leading;
        while index < line.text.len() {
            index = self.lex_token(index, line, state);
        }

        state.continued = state.delimiter_depth > 0
            || state
                .tokens
                .last()
                .is_some_and(|token| Self::is_continuation(&token.kind));
        if !state.continued {
            let end = line.text.len();
            state
                .tokens
                .push(Token::new(TokenKind::Newline, line.range(end, end)));
        }
    }

    fn indent_width(line: &SourceLine, leading: usize) -> usize {
        line.text[..leading]
            .iter()
            .map(|character| if *character == '\t' { 4 } else { 1 })
            .sum()
    }

    fn is_wrapped_line(leading: usize, line: &SourceLine, state: &State) -> bool {
        Self::indent_width(line, leading) % 4 != 0
            && state
                .tokens
                .last()
                .is_some_and(|token| token.kind == TokenKind::Newline)
    }

    fn lex_indentation(&self, leading: usize, line: &SourceLine, state: &mut State) {
        let width = Self::indent_width(line, leading);
        if !width.is_multiple_of(4) {
            state.diagnostics.push(Diagnostic::error(
                "PINE1002",
                DiagnosticCategory::Lexical,
                "Indentation must use multiples of four spaces.",
                line.range(0, leading.max(1)),
            ));
        }

        if width > *state.indents.last().expect("indent stack is never empty") {
            state.indents.push(width);
            state
                .tokens
                .push(Token::new(TokenKind::Indent, line.range(0, leading)));
        }
        while width < *state.indents.last().expect("indent stack is never empty") {
            state.indents.pop();
            state
                .tokens
                .push(Token::new(TokenKind::Dedent, line.range(0, leading)));
        }
    }

    fn lex_token(&self, index: usize, line: &SourceLine, state: &mut State) -> usize {
        let character = line.text[index];
        if matches!(character, ' ' | '\t') {
            return index + 1;
        }
        if character == '/' && line.character(index + 1) == Some('/') {
            return line.text.len();
        }
        if character.is_alphabetic() || character == '_' {
            return self.lex_word(index, line, state);
        }
        if Self::is_digit(Some(character))
            || (character == '.' && Self::is_digit(line.character(index + 1)))
        {
            return self.lex_number(index, line, state);
        }
        match character {
            '"' | '\'' => self.lex_string(index, character, line, state),
            '#' => self.lex_color(index, line, state),
            _ => self.lex_operator(index, line, state),
        }
    }

    fn is_digit(character: Option<char>) -> bool {
        matches!(character, Some(value) if value.is_ascii_digit())
    }

    fn lex_word(&self, start: usize, line: &SourceLine, state: &mut State) -> usize {
        let mut end = start + 1;
        while end < line.text.len()
            && (line.text[end].is_alphabetic()
                || line.text[end].is_numeric()
                || line.text[end] == '_')
        {
            end += 1;
        }
        let word: String = line.text[start..end].iter().collect();
        let kind = Self::keyword(&word).unwrap_or(TokenKind::Identifier(word));
        state.tokens.push(Token::new(kind, line.range(start, end)));
        end
    }

    fn lex_number(&self, start: usize, line: &SourceLine, state: &mut State) -> usize {
        let mut end = start + 1;
        let mut has_fraction = line.text[start] == '.';
        while end < line.text.len()
            && (Self::is_digit(Some(line.text[end])) || (!has_fraction && line.text[end] == '.'))
        {
            if line.text[end] == '.' {
                has_fraction = true;
            }
            end += 1;
        }
        if matches!(line.character(end), Some('e' | 'E')) {
            end += 1;
            if matches!(line.character(end), Some('+' | '-')) {
                end += 1;
            }
            while Self::is_digit(line.character(end)) {
                end += 1;
            }
            has_fraction = true;
        }

        let literal: String = line.text[start..end].iter().collect();
        let value = literal.parse::<f64>().unwrap_or(0.0);
        state.tokens.push(Token::new(
            TokenKind::Number {
                value,
                is_integer: !has_fraction,
            },
            line.range(start, end),
        ));
        end
    }

    fn lex_string(&self, start: usize, quote: char, line: &SourceLine, state: &mut State) -> usize {
        let mut end = start + 1;
        let mut value = String::new();
        let mut closed = false;
        while end < line.text.len() {
            let character = line.text[end];
            if character == quote {
                end += 1;
                closed = true;
                break;
            }
            if character == '\\' {
                if let Some(escaped) = line.character(end + 1) {
                    end += 1;
                    value.push(if escaped == 'n' { '\n' } else { escaped });
                } else {
                    value.push(character);
                }
            } else {
                value.push(character);
            }
            end += 1;
        }

        let range = line.range(start, end);
        if !closed {
            state.diagnostics.push(Diagnostic::error(
                "PINE1003",
                DiagnosticCategory::Lexical,
                "Unterminated string literal.",
                range,
            ));
        }
        state
            .tokens
            .push(Token::new(TokenKind::String(value), range));
        end
    }

    fn lex_color(&self, start: usize, line: &SourceLine, state: &mut State) -> usize {
        const MAX_COLOR_DIGITS: usize = 8;

        let mut end = start + 1;
        while end < line.text.len()
            && line.text[end].is_ascii_hexdigit()
            && end - start <= MAX_COLOR_DIGITS
        {
            end += 1;
        }
        let digit_count = end - start - 1;
        let raw = u32::from_str_radix(&line.text[start + 1..end].iter().collect::<String>(), 16);
        let Ok(raw) = raw else {
            state.diagnostics.push(Diagnostic::error(
                "PINE1004",
                DiagnosticCategory::Lexical,
                "A hex color requires exactly six or eight digits.",
                line.range(start, end),
            ));
            return end;
        };
        if !matches!(digit_count, 6 | 8) {
            state.diagnostics.push(Diagnostic::error(
                "PINE1004",
                DiagnosticCategory::Lexical,
                "A hex color requires exactly six or eight digits.",
                line.range(start, end),
            ));
            return end;
        }

        let rgba = if digit_count == 6 {
            (raw << 8) | 0xff
        } else {
            raw
        };
        state
            .tokens
            .push(Token::new(TokenKind::Color(rgba), line.range(start, end)));
        end
    }

    fn lex_operator(&self, start: usize, line: &SourceLine, state: &mut State) -> usize {
        let character = line.text[start];
        if let Some(next) = line.character(start + 1)
            && let Some(kind) = Self::two_character_operator(character, next)
        {
            state
                .tokens
                .push(Token::new(kind, line.range(start, start + 2)));
            return start + 2;
        }
        if let Some(kind) = Self::one_character_operator(character) {
            state
                .tokens
                .push(Token::new(kind, line.range(start, start + 1)));
            if matches!(character, '(' | '[') {
                state.delimiter_depth += 1;
            }
            if matches!(character, ')' | ']') {
                state.delimiter_depth = state.delimiter_depth.saturating_sub(1);
            }
        } else {
            state.diagnostics.push(Diagnostic::error(
                "PINE1001",
                DiagnosticCategory::Lexical,
                format!("Unexpected character '{character}'."),
                line.range(start, start + 1),
            ));
        }
        start + 1
    }

    fn keyword(word: &str) -> Option<TokenKind> {
        Some(match word {
            "true" => TokenKind::Bool(true),
            "false" => TokenKind::Bool(false),
            "na" => TokenKind::Na,
            "and" => TokenKind::And,
            "or" => TokenKind::Or,
            "not" => TokenKind::Not,
            "if" => TokenKind::If,
            "else" => TokenKind::Else,
            "var" => TokenKind::Var,
            "varip" => TokenKind::Varip,
            "for" => TokenKind::For,
            "break" => TokenKind::Break,
            "continue" => TokenKind::Continue,
            "while" => TokenKind::While,
            "once" => TokenKind::Once,
            "switch" => TokenKind::Switch,
            "int" => TokenKind::TypeKeyword(ValueType::Int),
            "float" => TokenKind::TypeKeyword(ValueType::Float),
            "bool" => TokenKind::TypeKeyword(ValueType::Bool),
            "string" => TokenKind::TypeKeyword(ValueType::String),
            "color" => TokenKind::TypeKeyword(ValueType::Color),
            _ => return None,
        })
    }

    fn two_character_operator(first: char, second: char) -> Option<TokenKind> {
        Some(match (first, second) {
            (':', '=') => TokenKind::Reassign,
            ('+', '=') => TokenKind::PlusAssign,
            ('-', '=') => TokenKind::MinusAssign,
            ('*', '=') => TokenKind::StarAssign,
            ('/', '=') => TokenKind::SlashAssign,
            ('%', '=') => TokenKind::PercentAssign,
            ('=', '=') => TokenKind::Equal,
            ('!', '=') => TokenKind::NotEqual,
            ('<', '=') => TokenKind::LessEqual,
            ('>', '=') => TokenKind::GreaterEqual,
            ('*', '*') => TokenKind::Power,
            ('=', '>') => TokenKind::Arrow,
            _ => return None,
        })
    }

    fn one_character_operator(character: char) -> Option<TokenKind> {
        Some(match character {
            '(' => TokenKind::LeftParen,
            ')' => TokenKind::RightParen,
            '[' => TokenKind::LeftBracket,
            ']' => TokenKind::RightBracket,
            ',' => TokenKind::Comma,
            '.' => TokenKind::Dot,
            '?' => TokenKind::Question,
            ':' => TokenKind::Colon,
            '=' => TokenKind::Assign,
            '+' => TokenKind::Plus,
            '-' => TokenKind::Minus,
            '*' => TokenKind::Star,
            '/' => TokenKind::Slash,
            '%' => TokenKind::Percent,
            '<' => TokenKind::Less,
            '>' => TokenKind::Greater,
            _ => return None,
        })
    }

    const fn is_continuation(kind: &TokenKind) -> bool {
        matches!(
            kind,
            TokenKind::Assign
                | TokenKind::Reassign
                | TokenKind::Plus
                | TokenKind::Minus
                | TokenKind::Star
                | TokenKind::Slash
                | TokenKind::Percent
                | TokenKind::Power
                | TokenKind::And
                | TokenKind::Or
                | TokenKind::Comma
                | TokenKind::Question
                | TokenKind::Colon
        )
    }
}

struct State {
    tokens: Vec<Token>,
    diagnostics: Vec<Diagnostic>,
    offset: usize,
    indents: Vec<usize>,
    delimiter_depth: usize,
    continued: bool,
}

impl State {
    fn is_at_logical_line_start(&self) -> bool {
        self.delimiter_depth == 0 && !self.continued
    }
}

#[derive(Clone)]
struct SourceLine {
    text: Vec<char>,
    number: usize,
    start_offset: usize,
    utf16_prefix: Vec<usize>,
}

/// A string literal may run over several lines (TradingView accepts a long tooltip written that
/// way): the lines are joined with the newline kept in the text, and each keeps the number of the
/// line it starts on. A string still open after a few lines is left alone, so a stray quote
/// reports "Unterminated string literal" where it is instead of swallowing the file.
fn join_multiline_strings<'a>(lines: &[&'a str]) -> Vec<(std::borrow::Cow<'a, str>, usize)> {
    const MAX_EXTRA_LINES: usize = 8;
    let mut joined = Vec::with_capacity(lines.len());
    let mut index = 0;
    while index < lines.len() {
        let number = index + 1;
        let mut open = open_quote_after(lines[index], None);
        let mut extra = 0;
        let mut text = std::borrow::Cow::Borrowed(lines[index]);
        while open.is_some() && extra < MAX_EXTRA_LINES && index + extra + 1 < lines.len() {
            extra += 1;
            let next = lines[index + extra];
            text.to_mut().push('\n');
            text.to_mut().push_str(next);
            open = open_quote_after(next, open);
        }
        if open.is_some() {
            // Never closed within reach: report it on the line it started on.
            joined.push((std::borrow::Cow::Borrowed(lines[index]), number));
            index += 1;
        } else {
            joined.push((text, number));
            index += extra + 1;
        }
    }
    joined
}

/// The quote still open at the end of `line` when it starts with `open` already open.
fn open_quote_after(line: &str, mut open: Option<char>) -> Option<char> {
    let mut characters = line.chars().peekable();
    while let Some(character) = characters.next() {
        match open {
            Some(quote) => {
                if character == '\\' {
                    characters.next();
                } else if character == quote {
                    open = None;
                }
            }
            None => {
                if character == '"' || character == '\'' {
                    open = Some(character);
                } else if character == '/' && characters.peek() == Some(&'/') {
                    break;
                }
            }
        }
    }
    open
}

impl SourceLine {
    fn new(text: &str, number: usize, start_offset: usize) -> Self {
        let text: Vec<char> = text.chars().collect();
        let mut utf16_prefix = Vec::with_capacity(text.len() + 1);
        utf16_prefix.push(0);
        for character in &text {
            utf16_prefix
                .push(utf16_prefix.last().copied().unwrap_or_default() + character.len_utf16());
        }
        Self {
            text,
            number,
            start_offset,
            utf16_prefix,
        }
    }

    fn character(&self, index: usize) -> Option<char> {
        self.text.get(index).copied()
    }

    fn character_len(&self) -> usize {
        self.text.len()
    }

    fn utf16_len(&self) -> usize {
        self.utf16_prefix[self.text.len()]
    }

    fn range(&self, from: usize, to: usize) -> SourceRange {
        SourceRange::new(
            SourcePosition::new(
                self.number,
                from + 1,
                self.start_offset + self.utf16_prefix[from],
            ),
            SourcePosition::new(
                self.number,
                to + 1,
                self.start_offset + self.utf16_prefix[to],
            ),
        )
    }
}

impl Default for State {
    fn default() -> Self {
        Self {
            tokens: Vec::new(),
            diagnostics: Vec::new(),
            offset: 0,
            indents: vec![0],
            delimiter_depth: 0,
            continued: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lex(source: &str) -> LexResult {
        Lexer::new(source).lex()
    }

    fn kinds(source: &str) -> Vec<TokenKind> {
        lex(source)
            .tokens
            .into_iter()
            .map(|token| token.kind)
            .collect()
    }

    #[test]
    fn lexes_literals_keywords_and_operators() {
        let result = lex(
            "var int count = 12\nfloat ratio = .5e+2\nready := true and not false\n#12aBcD #01020304\nf => f ** 2",
        );
        assert!(result.diagnostics.is_empty());
        assert!(result.tokens.iter().any(|token| {
            token.kind
                == TokenKind::Number {
                    value: 12.0,
                    is_integer: true,
                }
        }));
        assert!(result.tokens.iter().any(|token| {
            token.kind
                == TokenKind::Number {
                    value: 50.0,
                    is_integer: false,
                }
        }));
        assert!(
            result
                .tokens
                .iter()
                .any(|token| token.kind == TokenKind::Color(0x12ab_cdff))
        );
        assert!(
            result
                .tokens
                .iter()
                .any(|token| token.kind == TokenKind::Color(0x0102_0304))
        );
        assert!(
            result
                .tokens
                .iter()
                .any(|token| token.kind == TokenKind::Arrow)
        );
        assert!(
            result
                .tokens
                .iter()
                .any(|token| token.kind == TokenKind::Power)
        );
    }

    #[test]
    fn emits_indentation_and_closes_it_at_eof() {
        assert_eq!(
            kinds("if true\n    x = 1\n    if x\n        y = 2\nz = 3"),
            vec![
                TokenKind::If,
                TokenKind::Bool(true),
                TokenKind::Newline,
                TokenKind::Indent,
                TokenKind::Identifier("x".into()),
                TokenKind::Assign,
                TokenKind::Number {
                    value: 1.0,
                    is_integer: true,
                },
                TokenKind::Newline,
                TokenKind::If,
                TokenKind::Identifier("x".into()),
                TokenKind::Newline,
                TokenKind::Indent,
                TokenKind::Identifier("y".into()),
                TokenKind::Assign,
                TokenKind::Number {
                    value: 2.0,
                    is_integer: true,
                },
                TokenKind::Newline,
                TokenKind::Dedent,
                TokenKind::Dedent,
                TokenKind::Identifier("z".into()),
                TokenKind::Assign,
                TokenKind::Number {
                    value: 3.0,
                    is_integer: true,
                },
                TokenKind::Newline,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn ignores_blank_and_comment_only_lines() {
        let result = lex("x = 1\n\n    // ignored\ny = 2 // trailing");
        assert!(result.diagnostics.is_empty());
        assert_eq!(
            result
                .tokens
                .iter()
                .filter(|token| token.kind == TokenKind::Newline)
                .count(),
            2
        );
        assert!(
            !result
                .tokens
                .iter()
                .any(|token| token.kind == TokenKind::Slash)
        );
    }

    #[test]
    fn joins_operator_bracket_and_wrapped_continuations() {
        for source in ["x =\n    1", "x = (\n    1 +\n    2\n)", "x = 1\n  + 2"] {
            let result = lex(source);
            assert!(result.diagnostics.is_empty(), "{source:?}");
            assert_eq!(
                result
                    .tokens
                    .iter()
                    .filter(|token| token.kind == TokenKind::Newline)
                    .count(),
                1,
                "{source:?}"
            );
            assert!(
                !result
                    .tokens
                    .iter()
                    .any(|token| matches!(token.kind, TokenKind::Indent | TokenKind::Dedent))
            );
        }
    }

    #[test]
    fn reports_invalid_indentation_on_first_logical_line() {
        let result = lex("  x = 1");
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(result.diagnostics[0].code, "PINE1002");
        assert_eq!(
            result.diagnostics[0].range,
            SourceLine::new("  x = 1", 1, 0).range(0, 2)
        );
        assert!(
            result
                .tokens
                .iter()
                .any(|token| token.kind == TokenKind::Indent)
        );
    }

    #[test]
    fn lexes_both_string_quotes_and_escape_rules() {
        let result = lex("a = \"line\\nnext\"\nb = 'it\\'s'\nc = \"open");
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(result.diagnostics[0].code, "PINE1003");
        let strings: Vec<String> = result
            .tokens
            .iter()
            .filter_map(|token| match &token.kind {
                TokenKind::String(value) => Some(value.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(strings, ["line\nnext", "it's", "open"]);
    }

    #[test]
    fn reports_bad_colors_and_unexpected_characters() {
        let result = lex("#12345 @");
        assert_eq!(
            result
                .diagnostics
                .iter()
                .map(|diagnostic| diagnostic.code.as_str())
                .collect::<Vec<_>>(),
            ["PINE1004", "PINE1001"]
        );
        assert!(
            !result
                .tokens
                .iter()
                .any(|token| matches!(token.kind, TokenKind::Color(_)))
        );
    }

    #[test]
    fn ranges_use_character_columns_and_utf16_offsets() {
        let result = lex("@😀x\ny");
        let x = result
            .tokens
            .iter()
            .find(|token| token.kind == TokenKind::Identifier("x".into()))
            .unwrap();
        let y = result
            .tokens
            .iter()
            .find(|token| token.kind == TokenKind::Identifier("y".into()))
            .unwrap();
        assert_eq!(x.range.start, SourcePosition::new(1, 3, 3));
        assert_eq!(y.range.start, SourcePosition::new(2, 1, 5));
        assert_eq!(result.tokens.last().unwrap().range.start.offset, 6);
    }

    #[test]
    fn enforces_source_and_token_limits_with_terminating_eof() {
        let source_limited = Lexer::with_limits(
            "abc",
            Limits {
                source_characters: 2,
                ..Limits::default()
            },
        )
        .lex();
        assert_eq!(source_limited.tokens.len(), 1);
        assert_eq!(source_limited.tokens[0].kind, TokenKind::Eof);
        assert_eq!(source_limited.diagnostics[0].code, "PINE8001");

        let token_limited = Lexer::with_limits(
            "a b c",
            Limits {
                tokens: 2,
                ..Limits::default()
            },
        )
        .lex();
        assert_eq!(token_limited.diagnostics[0].code, "PINE8002");
        assert_eq!(token_limited.tokens.last().unwrap().kind, TokenKind::Eof);
    }
}
