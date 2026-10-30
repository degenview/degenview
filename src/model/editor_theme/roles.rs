//! The roles the editor paints: what a span of source is ([`SyntaxRole`]) and the surfaces
//! around it ([`SurfaceColor`]). Only distinctions the Pine tooling can make reliably are roles:
//! the lexer says what a keyword, string, escape or punctuation mark is, and the symbol table
//! says what a name is declared as.

/// What a run of Pine source is, from the most specific thing known about it.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SyntaxRole {
    /// Text with no role of its own.
    Default,
    Comment,
    String,
    /// `\n`, `\"` and the other backslash escapes inside a string.
    Escape,
    Number,
    /// `true`, `false` and `na`.
    Literal,
    /// A variable the script declares, including loop variables.
    UserVariable,
    /// A parameter of a function the script declares, and its `name=` labels in a call.
    Parameter,
    /// Builtins whose value changes per bar or symbol: `close`, `bar_index`.
    BuiltinVariable,
    /// Builtins with a fixed value: `color.red`, `math.pi`.
    BuiltinConstant,
    /// `ta` in `ta.sma`, and a library's import alias.
    Namespace,
    /// A function or method the script declares, at its definition and its calls.
    UserFunction,
    BuiltinFunction,
    /// `var`, `varip`, `import`, `export`, `method`, `type`, `enum`, `as`, `and`, `or`, `not`.
    Keyword,
    /// `if`, `else`, `for`, `while`, `switch`, `break`, `continue`, `in`, `to`, `once`.
    ControlKeyword,
    /// Value types, qualifiers and object types, and the types a script declares.
    Type,
    Operator,
    /// Brackets, parentheses, commas and dots.
    Punctuation,
    /// `//@version=6` and the other `//@…` directives.
    Directive,
    /// A colour written in the code: `#FF8800`.
    ColorLiteral,
}

impl SyntaxRole {
    pub const COUNT: usize = 20;

    pub const ALL: [Self; Self::COUNT] = [
        Self::Default,
        Self::Comment,
        Self::String,
        Self::Escape,
        Self::Number,
        Self::Literal,
        Self::UserVariable,
        Self::Parameter,
        Self::BuiltinVariable,
        Self::BuiltinConstant,
        Self::Namespace,
        Self::UserFunction,
        Self::BuiltinFunction,
        Self::Keyword,
        Self::ControlKeyword,
        Self::Type,
        Self::Operator,
        Self::Punctuation,
        Self::Directive,
        Self::ColorLiteral,
    ];

    /// The position in [`Self::ALL`], for tables indexed by role.
    pub const fn index(self) -> usize {
        self as usize
    }
}

/// A surface the editor paints that is not source text. Each is painted by the real editor.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SurfaceColor {
    Background,
    CurrentLine,
    Selection,
    Caret,
    BracketMatch,
    BracketMatchBorder,
    /// The other uses of the name under the caret; the name itself is drawn stronger.
    WordHighlight,
    FindMatch,
    FindMatchActive,
    IndentGuide,
    IndentGuideActive,
    GutterBackground,
    GutterSeparator,
    LineNumber,
    LineNumberActive,
    DiagnosticError,
    DiagnosticWarning,
    PopupBackground,
    PopupSelection,
}

impl SurfaceColor {
    pub const COUNT: usize = 19;

    pub const ALL: [Self; Self::COUNT] = [
        Self::Background,
        Self::CurrentLine,
        Self::Selection,
        Self::Caret,
        Self::BracketMatch,
        Self::BracketMatchBorder,
        Self::WordHighlight,
        Self::FindMatch,
        Self::FindMatchActive,
        Self::IndentGuide,
        Self::IndentGuideActive,
        Self::GutterBackground,
        Self::GutterSeparator,
        Self::LineNumber,
        Self::LineNumberActive,
        Self::DiagnosticError,
        Self::DiagnosticWarning,
        Self::PopupBackground,
        Self::PopupSelection,
    ];

    pub const fn index(self) -> usize {
        self as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_role_sits_at_its_index() {
        for (index, role) in SyntaxRole::ALL.iter().enumerate() {
            assert_eq!(role.index(), index, "{role:?}");
        }
    }

    #[test]
    fn every_surface_sits_at_its_index() {
        for (index, surface) in SurfaceColor::ALL.iter().enumerate() {
            assert_eq!(surface.index(), index, "{surface:?}");
        }
    }
}
