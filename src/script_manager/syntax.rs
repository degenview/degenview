//! Splits Pine source into roles for the editor, ported from `PineSyntaxClassifier`.
//!
//! Strings and numbers come from the real lexer, `//` comments (which the lexer drops) from a
//! scan that respects string tokens, and builtins are matched against the symbol catalog as whole
//! tokens or whole dotted chains, never substrings. What a *name* is comes from the symbol table
//! (`symbols::analyze`) when the caller has one: a variable, a parameter, a function or a type the
//! script declares, resolved by scope, so a local `close` is the script's own and a `close` outside
//! it is still the builtin. Without a table, names the script declares are found by a lexical
//! pass (variables and functions, but not parameters), which is also what code snippets outside
//! the editor get.
//!
//! This is the only classification of Pine text. It produces roles, not colours: the colour of a
//! role is the theme's (`ui::editor_appearance`), so a colour change classifies nothing again.

use super::symbols::{Kind, SymbolTable};
use crate::model::editor_theme::SyntaxRole;
use crate::pine::compiler::is_named_constant;
use crate::pine::lexer::Lexer;
use crate::pine::symbol_catalog as catalog;
use crate::pine::token::{Token, TokenKind};
use crate::ui::editor_appearance::Appearance;
use eframe::egui::{Color32, FontId, TextFormat, text::LayoutJob};
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::OnceLock;

/// A run of the source with a role, in UTF-8 byte offsets.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Span {
    pub range: Range<usize>,
    pub role: SyntaxRole,
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
const RESERVED_WORDS: [&str; 6] = ["import", "export", "method", "type", "enum", "as"];
/// Words the lexer reads as plain names but that steer a loop: `for i = 0 to 9`, `for x in xs`.
const CONTROL_WORDS: [&str; 2] = ["in", "to"];

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

/// The declared names that are functions (`name(args) =>`), for completion to add the call
/// parentheses.
pub fn declared_functions(source: &str) -> HashSet<String> {
    defined_functions(&Lexer::new(source).lex().tokens)
}

/// The role of a name the symbol table resolves, by what it was declared as.
fn role_of_kind(kind: Kind) -> SyntaxRole {
    match kind {
        Kind::Variable | Kind::LoopVariable => SyntaxRole::UserVariable,
        Kind::Parameter => SyntaxRole::Parameter,
        Kind::Function | Kind::Method => SyntaxRole::UserFunction,
        Kind::Type | Kind::Enum => SyntaxRole::Type,
        Kind::ImportAlias => SyntaxRole::Namespace,
    }
}

/// The role of every name `table` resolves, keyed by the byte offset the name starts at (the
/// table counts characters, spans count bytes).
fn resolved_roles(source: &str, table: &SymbolTable) -> HashMap<usize, SyntaxRole> {
    let mut byte_of_char: Vec<usize> = source.char_indices().map(|(byte, _)| byte).collect();
    byte_of_char.push(source.len());
    let mut roles = HashMap::new();
    for (range, kind) in table.resolved_names() {
        if let Some(&byte) = byte_of_char.get(range.start) {
            roles.insert(byte, role_of_kind(kind));
        }
    }
    roles
}

/// The names `name(args) =>` defines in `tokens`.
fn defined_functions(tokens: &[Token]) -> HashSet<String> {
    let mut functions = HashSet::new();
    for (index, token) in tokens.iter().enumerate() {
        let TokenKind::Identifier(name) = &token.kind else {
            continue;
        };
        let after_dot = index > 0 && tokens[index - 1].kind == TokenKind::Dot;
        let call = tokens.get(index + 1).map(|t| &t.kind) == Some(&TokenKind::LeftParen);
        if !after_dot && call && is_function_definition(tokens, index) {
            functions.insert(name.clone());
        }
    }
    functions
}

/// Classifies the source. Spans are sorted, non-overlapping and cover only what has a role other
/// than [`SyntaxRole::Default`]. `table` is the script's symbol table, if the caller has one.
pub fn classify(source: &str, table: Option<&SymbolTable>) -> Vec<Span> {
    let result = Lexer::new(source).lex();
    let offsets = utf16_to_byte_offsets(source);
    let byte = |utf16: usize| offsets.get(utf16).copied().unwrap_or(source.len());
    let tokens = &result.tokens;
    let declared = declared_names(tokens);
    let functions = defined_functions(tokens);
    let resolved = table.map(|table| resolved_roles(source, table));
    let mut spans: Vec<Span> = Vec::new();
    let mut strings: Vec<Range<usize>> = Vec::new();
    let range_of = |token: &Token| byte(token.range.start.offset)..byte(token.range.end.offset);

    let mut index = 0;
    let mut paren_depth = 0usize;
    while index < tokens.len() {
        let token = &tokens[index];
        let range = range_of(token);
        let next_kind = tokens.get(index + 1).map(|t| &t.kind);
        let mut push = |range: Range<usize>, role: SyntaxRole| {
            if range.start < range.end && role != SyntaxRole::Default {
                spans.push(Span { range, role });
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
                let named_argument = paren_depth > 0 && next_kind == Some(&TokenKind::Assign);
                // What the script says this name is. With a symbol table its answer is final: a
                // name it does not place is not the script's (a builtin read above a shadowing
                // declaration, or a name the table could not place in half-typed code). Without
                // one, a name the script declares anywhere is its own.
                let user = match &resolved {
                    Some(roles) => roles.get(&range.start).copied(),
                    None => (!named_argument && declared.contains(names[0].as_str())).then(|| {
                        if functions.contains(names[0].as_str()) {
                            SyntaxRole::UserFunction
                        } else {
                            SyntaxRole::UserVariable
                        }
                    }),
                };
                let roles = roles_of(
                    &names,
                    &tokens[chain[0]].kind,
                    is_call,
                    user,
                    named_argument,
                    next_kind,
                );
                for (position, &token_index) in chain.iter().enumerate() {
                    push(range_of(&tokens[token_index]), roles[position]);
                    if position + 1 < chain.len() {
                        push(range_of(&tokens[token_index + 1]), SyntaxRole::Punctuation);
                    }
                }
                index = end;
                continue;
            }
            TokenKind::Number { .. } => push(range, SyntaxRole::Number),
            TokenKind::Color(_) => push(range, SyntaxRole::ColorLiteral),
            TokenKind::Bool(_) => push(range, SyntaxRole::Literal),
            TokenKind::Na => push(
                range,
                if next_kind == Some(&TokenKind::LeftParen) {
                    SyntaxRole::BuiltinFunction
                } else {
                    SyntaxRole::Literal
                },
            ),
            TokenKind::String(_) => {
                strings.push(range.clone());
                for (part, escape) in string_parts(source, &range) {
                    push(
                        part,
                        if escape {
                            SyntaxRole::Escape
                        } else {
                            SyntaxRole::String
                        },
                    );
                }
            }
            TokenKind::And | TokenKind::Or | TokenKind::Not | TokenKind::Var | TokenKind::Varip => {
                push(range, SyntaxRole::Keyword)
            }
            TokenKind::If
            | TokenKind::Else
            | TokenKind::For
            | TokenKind::Break
            | TokenKind::Continue
            | TokenKind::While
            | TokenKind::Once
            | TokenKind::Switch => push(range, SyntaxRole::ControlKeyword),
            TokenKind::LeftParen => {
                paren_depth += 1;
                push(range, SyntaxRole::Punctuation);
            }
            TokenKind::RightParen => {
                paren_depth = paren_depth.saturating_sub(1);
                push(range, SyntaxRole::Punctuation);
            }
            TokenKind::LeftBracket
            | TokenKind::RightBracket
            | TokenKind::Comma
            | TokenKind::Dot => push(range, SyntaxRole::Punctuation),
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
            | TokenKind::PercentAssign
            | TokenKind::Equal
            | TokenKind::NotEqual
            | TokenKind::Less
            | TokenKind::LessEqual
            | TokenKind::Greater
            | TokenKind::GreaterEqual
            | TokenKind::Question
            | TokenKind::Colon
            | TokenKind::Arrow => push(range, SyntaxRole::Operator),
            _ => {}
        }
        index += 1;
    }

    // `//` comments and `//@` directives: the lexer drops them, so find each outside any string.
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
            let directive = directive_end(&source[at..end]).map(|len| at + len);
            match directive {
                Some(stop) => {
                    spans.push(Span {
                        range: at..stop,
                        role: SyntaxRole::Directive,
                    });
                    if stop < end {
                        spans.push(Span {
                            range: stop..end,
                            role: SyntaxRole::Comment,
                        });
                    }
                }
                None => spans.push(Span {
                    range: at..end,
                    role: SyntaxRole::Comment,
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

/// A string literal cut into its plain text and its backslash escapes (`true` for an escape).
/// A backslash and the character after it are one escape, whatever the character is.
fn string_parts(source: &str, range: &Range<usize>) -> Vec<(Range<usize>, bool)> {
    let text = &source[range.clone()];
    let mut parts = Vec::new();
    let mut plain_from = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((at, character)) = chars.next() {
        if character != '\\' {
            continue;
        }
        let Some(&(_, next)) = chars.peek() else {
            break;
        };
        let end = at + 1 + next.len_utf8();
        if plain_from < at {
            parts.push((range.start + plain_from..range.start + at, false));
        }
        parts.push((range.start + at..range.start + end, true));
        plain_from = end;
        chars.next();
    }
    if plain_from < text.len() {
        parts.push((range.start + plain_from..range.end, false));
    }
    parts
}

/// `//@version=6` → its length: `//@` + letters/underscores + an optional `=digits`.
fn directive_end(comment: &str) -> Option<usize> {
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

/// The role of each name in a dotted chain. `user` is what the script declares the head as, if it
/// does: then the chain is the script's own (`obj.field`, `lib.fn()`) and nothing in it is looked
/// up in the catalog.
fn roles_of(
    names: &[String],
    head_kind: &TokenKind,
    is_call: bool,
    user: Option<SyntaxRole>,
    named_argument: bool,
    next: Option<&TokenKind>,
) -> Vec<SyntaxRole> {
    let mut roles = vec![SyntaxRole::Default; names.len()];
    let head = names[0].as_str();
    if let Some(role) = user {
        roles[0] = role;
        // `alias.function(...)` calls a function the library defines.
        if role == SyntaxRole::Namespace && is_call && names.len() > 1 {
            roles[names.len() - 1] = SyntaxRole::UserFunction;
        }
        return roles;
    }
    if names.len() == 1 {
        // `title = "x"` inside a call is a named argument, whatever the name is.
        if named_argument {
            return roles;
        }
        let type_start = matches!(
            next,
            Some(TokenKind::Identifier(_) | TokenKind::TypeKeyword(_))
        );
        roles[0] = if matches!(head_kind, TokenKind::TypeKeyword(_)) {
            if is_call {
                SyntaxRole::BuiltinFunction
            } else {
                SyntaxRole::Type
            }
        } else if CONTROL_WORDS.contains(&head) && !is_call {
            SyntaxRole::ControlKeyword
        } else if RESERVED_WORDS.contains(&head) && !is_call {
            SyntaxRole::Keyword
        } else if QUALIFIERS.contains(&head) && !is_call && type_start {
            SyntaxRole::Type
        } else if catalog::OBJECT_TYPES.contains(&head) {
            if is_call {
                SyntaxRole::BuiltinFunction
            } else {
                SyntaxRole::Type
            }
        } else if is_call {
            if catalog::FUNCTIONS.contains(&head) {
                SyntaxRole::BuiltinFunction
            } else {
                SyntaxRole::Default
            }
        } else if catalog::is_variable(head) {
            SyntaxRole::BuiltinVariable
        } else {
            SyntaxRole::Default
        };
        return roles;
    }
    if !namespaces().contains(head) {
        return roles;
    }
    let full = names.join(".");
    let last = names.len() - 1;
    roles.fill(SyntaxRole::Namespace);
    roles[last] = if is_named_constant(&full) || catalog::is_reserved_constant(&full) {
        SyntaxRole::BuiltinConstant
    } else if is_call {
        SyntaxRole::BuiltinFunction
    } else if catalog::is_variable(&full) {
        SyntaxRole::BuiltinVariable
    } else {
        SyntaxRole::Default
    };
    roles
}

/// The source as a coloured layout. Runs of equal colour are one section however many spans make
/// them up (a theme gives user names and punctuation the default colour, so most of a script is
/// one run), which keeps egui's per-section shaping to what the eye can tell apart.
pub fn layout_job(
    source: &str,
    spans: &[Span],
    font: FontId,
    appearance: &Appearance,
    default: Color32,
) -> LayoutJob {
    let mut job = LayoutJob::default();
    let format = |color: Color32| TextFormat::simple(font.clone(), color);
    let mut run: Option<(Range<usize>, Color32)> = None;
    let mut add = |job: &mut LayoutJob, range: Range<usize>, color: Color32| {
        if let Some((open, open_color)) = &mut run
            && *open_color == color
            && open.end == range.start
        {
            open.end = range.end;
            return;
        }
        if let Some((open, open_color)) = run.replace((range, color)) {
            job.append(&source[open], 0.0, format(open_color));
        }
    };
    let mut cursor = 0;
    for span in spans {
        if span.range.start < cursor || span.range.end > source.len() {
            continue;
        }
        if span.range.start > cursor {
            add(&mut job, cursor..span.range.start, default);
        }
        add(&mut job, span.range.clone(), appearance.role(span.role));
        cursor = span.range.end;
    }
    if cursor < source.len() {
        add(&mut job, cursor..source.len(), default);
    }
    if let Some((open, open_color)) = run {
        job.append(&source[open], 0.0, format(open_color));
    }
    job
}

#[cfg(test)]
mod tests;
