//! Token and source-location types shared by the lexer, parser and diagnostics.

use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ValueType {
    Int,
    Float,
    Bool,
    String,
    Color,
    Line,
    Label,
    Box,
    Table,
    Array,
    Object,
    Map,
    Matrix,
    Time,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct SourcePosition {
    pub line: usize,
    pub column: usize,
    /// UTF-16 code-unit offset, matching AppKit's `NSRange` convention.
    pub offset: usize,
}

impl SourcePosition {
    pub const fn new(line: usize, column: usize, offset: usize) -> Self {
        Self {
            line,
            column,
            offset,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SourceRange {
    pub start: SourcePosition,
    pub end: SourcePosition,
}

impl SourceRange {
    pub const ZERO: Self = Self {
        start: SourcePosition::new(1, 1, 0),
        end: SourcePosition::new(1, 1, 0),
    };

    pub const fn new(start: SourcePosition, end: SourcePosition) -> Self {
        Self { start, end }
    }
}

impl Default for SourceRange {
    fn default() -> Self {
        Self::ZERO
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DiagnosticSeverity {
    Error,
    Warning,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DiagnosticCategory {
    Lexical,
    Syntax,
    Semantic,
    Unsupported,
    Resource,
    Runtime,
    Cancellation,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: DiagnosticSeverity,
    pub category: DiagnosticCategory,
    pub message: String,
    pub range: SourceRange,
}

impl Diagnostic {
    pub fn error(
        code: impl Into<String>,
        category: DiagnosticCategory,
        message: impl Into<String>,
        range: SourceRange,
    ) -> Self {
        Self {
            code: code.into(),
            severity: DiagnosticSeverity::Error,
            category,
            message: message.into(),
            range,
        }
    }

    pub fn id(&self) -> String {
        format!("{}:{}:{}", self.code, self.range.start.offset, self.message)
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} at {}:{}: {}",
            self.code, self.range.start.line, self.range.start.column, self.message
        )
    }
}

impl std::error::Error for Diagnostic {}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limits {
    /// A runaway-input guard rather than a Pine language rule.
    pub source_characters: usize,
    pub tokens: usize,
    pub ast_nodes: usize,
    pub instructions_per_bar: usize,
    pub call_depth: usize,
    pub deadline_seconds: f64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            source_characters: 500_000,
            tokens: 50_000,
            ast_nodes: 50_000,
            instructions_per_bar: 20_000_000,
            call_depth: 64,
            deadline_seconds: 10.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum TokenKind {
    Identifier(String),
    Number { value: f64, is_integer: bool },
    String(String),
    Bool(bool),
    Color(u32),
    Na,
    Newline,
    Indent,
    Dedent,
    Eof,
    LeftParen,
    RightParen,
    LeftBracket,
    RightBracket,
    Comma,
    Dot,
    Question,
    Colon,
    Assign,
    Reassign,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Power,
    PlusAssign,
    MinusAssign,
    StarAssign,
    SlashAssign,
    PercentAssign,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    And,
    Or,
    Not,
    If,
    Else,
    Var,
    Varip,
    For,
    Break,
    Continue,
    While,
    Once,
    Switch,
    TypeKeyword(ValueType),
    Arrow,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub range: SourceRange,
}

impl Token {
    pub fn new(kind: TokenKind, range: SourceRange) -> Self {
        Self { kind, range }
    }
}
