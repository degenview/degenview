//! The roles the editor paints: what a span of source is ([`SyntaxRole`]) and the surfaces
//! around it ([`SurfaceColor`]). Only distinctions the Pine tooling can make reliably are roles:
//! the lexer says what a keyword, string, escape or punctuation mark is, and the symbol table
//! says what a name is declared as.

/// How a theme groups its syntax roles for editing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SyntaxGroup {
    General,
    Variables,
    Functions,
    Language,
}

impl SyntaxGroup {
    pub const ALL: [Self; 4] = [
        Self::General,
        Self::Variables,
        Self::Functions,
        Self::Language,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Variables => "Variables and identifiers",
            Self::Functions => "Functions",
            Self::Language => "Language",
        }
    }
}

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

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|role| role.key() == key)
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Default => "Default text",
            Self::Comment => "Comments",
            Self::String => "Strings",
            Self::Escape => "Escape sequences",
            Self::Number => "Numbers",
            Self::Literal => "true, false and na",
            Self::UserVariable => "User variables",
            Self::Parameter => "Function parameters",
            Self::BuiltinVariable => "Built-in variables",
            Self::BuiltinConstant => "Built-in constants",
            Self::Namespace => "Namespaces",
            Self::UserFunction => "User-defined functions",
            Self::BuiltinFunction => "Built-in functions",
            Self::Keyword => "Keywords",
            Self::ControlKeyword => "Control flow",
            Self::Type => "Types",
            Self::Operator => "Operators",
            Self::Punctuation => "Punctuation",
            Self::Directive => "Directives",
            Self::ColorLiteral => "Color literals",
        }
    }

    pub const fn hint(self) -> &'static str {
        match self {
            Self::Default => "Text that has no color of its own, and the editor's foreground.",
            Self::Comment => "Everything after // on a line.",
            Self::String => "Text between quotes.",
            Self::Escape => "Backslash sequences inside a string, such as \\n.",
            Self::Number => "Integer and decimal literals.",
            Self::Literal => "The literals true, false and na.",
            Self::UserVariable => "Variables the script declares, including loop variables.",
            Self::Parameter => {
                "Parameters of the script's own functions, and name= labels in calls to them."
            }
            Self::BuiltinVariable => {
                "close, volume, bar_index and the other variables Pine provides."
            }
            Self::BuiltinConstant => "color.red, math.pi and the other fixed values.",
            Self::Namespace => "The ta in ta.sma, and the alias of an imported library.",
            Self::UserFunction => {
                "Functions and methods the script defines, where defined and called."
            }
            Self::BuiltinFunction => {
                "ta.sma, plot, input.int and the other functions Pine provides."
            }
            Self::Keyword => "var, import, export, method, type, enum, and, or, not.",
            Self::ControlKeyword => "if, else, for, while, switch, break, continue, in, to, once.",
            Self::Type => "float, series, array and the types the script declares.",
            Self::Operator => "Arithmetic, comparison, assignment and ?: operators.",
            Self::Punctuation => "Brackets, parentheses, commas and dots.",
            Self::Directive => "//@version=6 and the other //@ directives.",
            Self::ColorLiteral => "Colors written as #RRGGBB.",
        }
    }

    pub const fn group(self) -> SyntaxGroup {
        match self {
            Self::Default
            | Self::Comment
            | Self::String
            | Self::Escape
            | Self::Number
            | Self::Literal => SyntaxGroup::General,
            Self::UserVariable
            | Self::Parameter
            | Self::BuiltinVariable
            | Self::BuiltinConstant
            | Self::Namespace => SyntaxGroup::Variables,
            Self::UserFunction | Self::BuiltinFunction => SyntaxGroup::Functions,
            Self::Keyword
            | Self::ControlKeyword
            | Self::Type
            | Self::Operator
            | Self::Punctuation
            | Self::Directive
            | Self::ColorLiteral => SyntaxGroup::Language,
        }
    }

    /// The name a theme file and a saved override use. Never changes once shipped.
    pub const fn key(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Comment => "comment",
            Self::String => "string",
            Self::Escape => "escape",
            Self::Number => "number",
            Self::Literal => "literal",
            Self::UserVariable => "userVariable",
            Self::Parameter => "parameter",
            Self::BuiltinVariable => "builtinVariable",
            Self::BuiltinConstant => "builtinConstant",
            Self::Namespace => "namespace",
            Self::UserFunction => "userFunction",
            Self::BuiltinFunction => "builtinFunction",
            Self::Keyword => "keyword",
            Self::ControlKeyword => "controlKeyword",
            Self::Type => "type",
            Self::Operator => "operator",
            Self::Punctuation => "punctuation",
            Self::Directive => "directive",
            Self::ColorLiteral => "colorLiteral",
        }
    }
}

/// How a theme groups its editor surfaces for editing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SurfaceGroup {
    Editor,
    Gutter,
    Search,
    Diagnostics,
    Popups,
}

impl SurfaceGroup {
    pub const ALL: [Self; 5] = [
        Self::Editor,
        Self::Gutter,
        Self::Search,
        Self::Diagnostics,
        Self::Popups,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Editor => "Editor",
            Self::Gutter => "Gutter",
            Self::Search => "Search and matches",
            Self::Diagnostics => "Diagnostics",
            Self::Popups => "Suggestions",
        }
    }
}

/// A surface the editor paints that is not source text. Each is painted by the real editor.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SurfaceColor {
    Background,
    CurrentLine,
    Selection,
    /// The colour of selected text.
    SelectionText,
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
    pub const COUNT: usize = 20;

    pub const ALL: [Self; Self::COUNT] = [
        Self::Background,
        Self::CurrentLine,
        Self::Selection,
        Self::SelectionText,
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

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|surface| surface.key() == key)
    }

    /// Whether the surface is drawn over something (so its colour may be translucent) rather than
    /// being a solid fill or a line of text.
    pub const fn translucent(self) -> bool {
        !matches!(
            self,
            Self::Background
                | Self::SelectionText
                | Self::Caret
                | Self::GutterBackground
                | Self::LineNumber
                | Self::LineNumberActive
                | Self::DiagnosticError
                | Self::DiagnosticWarning
                | Self::PopupBackground
                | Self::PopupSelection
        )
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Background => "Background",
            Self::CurrentLine => "Current line",
            Self::Selection => "Selection",
            Self::SelectionText => "Selected text",
            Self::Caret => "Caret",
            Self::BracketMatch => "Matching bracket",
            Self::BracketMatchBorder => "Matching bracket border",
            Self::WordHighlight => "Same name",
            Self::FindMatch => "Search matches",
            Self::FindMatchActive => "Current search match",
            Self::IndentGuide => "Indent guides",
            Self::IndentGuideActive => "Active indent guide",
            Self::GutterBackground => "Gutter background",
            Self::GutterSeparator => "Gutter separator",
            Self::LineNumber => "Line numbers",
            Self::LineNumberActive => "Current line number",
            Self::DiagnosticError => "Error underline",
            Self::DiagnosticWarning => "Warning underline",
            Self::PopupBackground => "Suggestion list",
            Self::PopupSelection => "Selected suggestion",
        }
    }

    pub const fn hint(self) -> &'static str {
        match self {
            Self::Background => "The editor's background.",
            Self::CurrentLine => "The band behind the line the caret is on.",
            Self::Selection => "Selected text. Translucent colors let the text show through.",
            Self::SelectionText => "The color of text inside a selection.",
            Self::Caret => "The blinking text cursor, including extra carets.",
            Self::BracketMatch => {
                "The fill behind a bracket and its partner, or behind one with no partner."
            }
            Self::BracketMatchBorder => {
                "An outline around matching brackets. Fully transparent draws none."
            }
            Self::WordHighlight => "Other uses of the name under the caret.",
            Self::FindMatch => "Matches of the search in the find bar.",
            Self::FindMatchActive => "The match the find bar is on.",
            Self::IndentGuide => "The vertical guides at each indent level.",
            Self::IndentGuideActive => "The guide of the block the caret is in.",
            Self::GutterBackground => "The strip with the line numbers.",
            Self::GutterSeparator => {
                "A line between the gutter and the code. Fully transparent draws none."
            }
            Self::LineNumber => "Line numbers in the gutter.",
            Self::LineNumberActive => "The number of the line the caret is on.",
            Self::DiagnosticError => "The underline under code the compiler rejects.",
            Self::DiagnosticWarning => "The underline under code the compiler warns about.",
            Self::PopupBackground => "The background of the suggestion list.",
            Self::PopupSelection => "The highlighted row of the suggestion list.",
        }
    }

    pub const fn group(self) -> SurfaceGroup {
        match self {
            Self::Background
            | Self::CurrentLine
            | Self::Selection
            | Self::SelectionText
            | Self::Caret
            | Self::BracketMatch
            | Self::BracketMatchBorder
            | Self::WordHighlight
            | Self::IndentGuide
            | Self::IndentGuideActive => SurfaceGroup::Editor,
            Self::GutterBackground
            | Self::GutterSeparator
            | Self::LineNumber
            | Self::LineNumberActive => SurfaceGroup::Gutter,
            Self::FindMatch | Self::FindMatchActive => SurfaceGroup::Search,
            Self::DiagnosticError | Self::DiagnosticWarning => SurfaceGroup::Diagnostics,
            Self::PopupBackground | Self::PopupSelection => SurfaceGroup::Popups,
        }
    }

    /// The name a theme file and a saved override use. Never changes once shipped.
    pub const fn key(self) -> &'static str {
        match self {
            Self::Background => "background",
            Self::CurrentLine => "currentLine",
            Self::Selection => "selection",
            Self::SelectionText => "selectionText",
            Self::Caret => "caret",
            Self::BracketMatch => "bracketMatch",
            Self::BracketMatchBorder => "bracketMatchBorder",
            Self::WordHighlight => "wordHighlight",
            Self::FindMatch => "findMatch",
            Self::FindMatchActive => "findMatchActive",
            Self::IndentGuide => "indentGuide",
            Self::IndentGuideActive => "indentGuideActive",
            Self::GutterBackground => "gutterBackground",
            Self::GutterSeparator => "gutterSeparator",
            Self::LineNumber => "lineNumber",
            Self::LineNumberActive => "lineNumberActive",
            Self::DiagnosticError => "diagnosticError",
            Self::DiagnosticWarning => "diagnosticWarning",
            Self::PopupBackground => "popupBackground",
            Self::PopupSelection => "popupSelection",
        }
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
    fn labels_are_unique_and_every_group_has_a_member() {
        let labels: std::collections::HashSet<_> = SyntaxRole::ALL
            .iter()
            .map(|r| r.label())
            .chain(SurfaceColor::ALL.iter().map(|s| s.label()))
            .collect();
        assert_eq!(labels.len(), SyntaxRole::COUNT + SurfaceColor::COUNT);
        for group in SyntaxGroup::ALL {
            assert!(SyntaxRole::ALL.iter().any(|r| r.group() == group));
        }
        for group in SurfaceGroup::ALL {
            assert!(SurfaceColor::ALL.iter().any(|s| s.group() == group));
        }
        assert!(SyntaxRole::ALL.iter().all(|r| r.hint().len() > 10));
        assert!(SurfaceColor::ALL.iter().all(|s| s.hint().len() > 10));
    }

    #[test]
    fn only_overlays_are_translucent() {
        assert!(!SurfaceColor::Background.translucent());
        assert!(!SurfaceColor::Caret.translucent());
        assert!(SurfaceColor::Selection.translucent());
        assert!(SurfaceColor::CurrentLine.translucent());
        assert!(SurfaceColor::FindMatch.translucent());
    }

    #[test]
    fn keys_are_unique_across_roles_and_surfaces() {
        let mut keys: Vec<&str> = SyntaxRole::ALL.iter().map(|r| r.key()).collect();
        keys.extend(SurfaceColor::ALL.iter().map(|s| s.key()));
        let unique: std::collections::HashSet<_> = keys.iter().collect();
        assert_eq!(unique.len(), SyntaxRole::COUNT + SurfaceColor::COUNT);
    }

    #[test]
    fn every_surface_sits_at_its_index() {
        for (index, surface) in SurfaceColor::ALL.iter().enumerate() {
            assert_eq!(surface.index(), index, "{surface:?}");
        }
    }
}
