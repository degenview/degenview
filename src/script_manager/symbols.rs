//! Which name means what, for go-to-definition and rename.
//!
//! This reads the lexer's tokens rather than the AST: the AST has no span for parameters, loop
//! variables, tuple names or import aliases, and drops every statement it cannot parse, so it
//! goes blind exactly while the user is typing. The lexer's logical lines (continuations folded)
//! are the statements; scopes come from each statement's raw leading whitespace, not the lexer's
//! indent tokens, which an open parenthesis erases.
//!
//! Every range is a character index into the text as given (what egui's cursor counts). A script
//! it cannot make sense of simply resolves less: an unresolved name is left alone, never guessed.
//!
//! Resolution rules:
//! - A declaration is visible from the end of its statement, block included, so `x = x + 1` reads
//!   the outer `x`. An inner declaration shadows the outer one until its block ends.
//! - Values and callables are separate namespaces (`kama = kama(src, len)`); `name(` looks up
//!   functions and methods, any other use looks up everything else.
//! - Members (`a.b`) resolve only through `a`. The one exception is `obj.name(` where `name` is a
//!   user `method`, matched by name alone because there are no types here. Fields, enum members
//!   and library symbols (`lib.fn`) are not resolved.
//! - `name =` labels inside a call to a user function are references to that parameter.
//! - Functions with one name in one scope are overloads: they rename together.

use crate::pine::identifier_rules;
use crate::pine::lexer::Lexer;
use crate::pine::symbol_catalog;
use crate::pine::token::{Token, TokenKind};
use std::ops::Range;

pub type SymbolId = usize;
type ScopeId = usize;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    Variable,
    Parameter,
    LoopVariable,
    Function,
    Method,
    Type,
    Enum,
    ImportAlias,
}

impl Kind {
    fn callable(self) -> bool {
        matches!(self, Self::Function | Self::Method)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Variable => "variable",
            Self::Parameter => "parameter",
            Self::LoopVariable => "loop variable",
            Self::Function => "function",
            Self::Method => "method",
            Self::Type => "type",
            Self::Enum => "enum",
            Self::ImportAlias => "import alias",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Symbol {
    pub name: String,
    pub kind: Kind,
    /// The declaring name.
    pub declaration: Range<usize>,
    scope: ScopeId,
    visible_from: usize,
    /// A function's own scope, which holds its parameters.
    body: Option<ScopeId>,
}

#[derive(Clone, Debug)]
struct Reference {
    range: Range<usize>,
    symbol: SymbolId,
}

struct Scope {
    parent: Option<ScopeId>,
    symbols: Vec<SymbolId>,
}

#[derive(Default)]
pub struct SymbolTable {
    symbols: Vec<Symbol>,
    scopes: Vec<Scope>,
    /// Sorted by position.
    references: Vec<Reference>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RenameError {
    /// The text is not a name Pine allows for this symbol; says why.
    Invalid(String),
    /// The new name would change what some name in the script means.
    Conflict,
}

impl RenameError {
    pub fn message(&self) -> String {
        match self {
            Self::Invalid(why) => why.clone(),
            Self::Conflict => "That name would clash with another name in scope.".to_owned(),
        }
    }
}

impl SymbolTable {
    /// The symbol, and the range of the name, whose declaration or reference covers the character
    /// at `index` (the caret counts as on a name when it touches either end).
    pub fn name_at(&self, index: usize) -> Option<(SymbolId, Range<usize>)> {
        let touches = |range: &Range<usize>| range.start <= index && index <= range.end;
        let position = self.references.partition_point(|r| r.range.end < index);
        if let Some(reference) = self.references[position..]
            .iter()
            .take_while(|r| r.range.start <= index)
            .find(|r| touches(&r.range))
        {
            return Some((reference.symbol, reference.range.clone()));
        }
        self.symbols
            .iter()
            .position(|symbol| touches(&symbol.declaration))
            .map(|id| (id, self.symbols[id].declaration.clone()))
    }

    /// Every name the table places, as the character range it covers and what its symbol was
    /// declared as: each declaration, then each use. The highlighter reads it to tell a
    /// parameter from a variable from a function.
    pub fn resolved_names(&self) -> impl Iterator<Item = (Range<usize>, Kind)> + '_ {
        let declarations = self
            .symbols
            .iter()
            .map(|symbol| (symbol.declaration.clone(), symbol.kind));
        let uses = self
            .references
            .iter()
            .map(|reference| (reference.range.clone(), self.symbols[reference.symbol].kind));
        declarations.chain(uses)
    }

    pub fn symbol_at(&self, index: usize) -> Option<SymbolId> {
        self.name_at(index).map(|(id, _)| id)
    }

    pub fn symbol(&self, id: SymbolId) -> &Symbol {
        &self.symbols[id]
    }

    /// Where the name at `index` is declared, when it is a use of a declared name. `None` on the
    /// declaration itself and on anything unresolved. For overloads: the nearest one above.
    pub fn definition_of(&self, index: usize) -> Option<Range<usize>> {
        let id = self.symbol_at(index)?;
        let symbol = &self.symbols[id];
        let on_declaration = symbol.declaration.start <= index && index <= symbol.declaration.end;
        let on_reference = self
            .references
            .iter()
            .any(|r| r.symbol == id && r.range.start <= index && index <= r.range.end);
        if on_declaration && !on_reference {
            return None;
        }
        Some(symbol.declaration.clone())
    }

    /// The calls of the function or method declared at `index`, which must be on its declaration
    /// name: every call of any of its overloads (a method called through `value.name(` included),
    /// sorted. `None` anywhere else, so a click elsewhere is not about calls.
    pub fn calls_of(&self, index: usize) -> Option<Vec<Range<usize>>> {
        let id = self.symbols.iter().position(|symbol| {
            symbol.kind.callable()
                && symbol.declaration.start <= index
                && index <= symbol.declaration.end
        })?;
        let group = self.group(id);
        let mut calls: Vec<Range<usize>> = self
            .references
            .iter()
            .filter(|reference| group.contains(&reference.symbol))
            .map(|reference| reference.range.clone())
            .collect();
        calls.sort_by_key(|range| range.start);
        calls.dedup();
        Some(calls)
    }

    /// The symbols that rename together with `id`: overloads of one function.
    fn group(&self, id: SymbolId) -> Vec<SymbolId> {
        let symbol = &self.symbols[id];
        if !symbol.kind.callable() {
            return vec![id];
        }
        self.symbols
            .iter()
            .enumerate()
            .filter(|(_, other)| {
                other.kind.callable() && other.name == symbol.name && other.scope == symbol.scope
            })
            .map(|(other, _)| other)
            .collect()
    }

    /// Every range that names `id` (declarations and uses), sorted.
    pub fn occurrences(&self, id: SymbolId) -> Vec<Range<usize>> {
        let group = self.group(id);
        let mut ranges: Vec<Range<usize>> = group
            .iter()
            .map(|member| self.symbols[*member].declaration.clone())
            .chain(
                self.references
                    .iter()
                    .filter(|r| group.contains(&r.symbol))
                    .map(|r| r.range.clone()),
            )
            .collect();
        ranges.sort_by_key(|range| range.start);
        ranges.dedup();
        ranges
    }

    /// The replacements, sorted by position, that rename `id` to `new_name`.
    ///
    /// Renaming is proven rather than reasoned about: the renamed text is analysed again and
    /// every name must still mean what it meant. A name that would capture, or be captured by,
    /// another declaration changes a resolution and is refused.
    pub fn rename(
        &self,
        source: &str,
        id: SymbolId,
        new_name: &str,
    ) -> Result<Vec<(Range<usize>, String)>, RenameError> {
        check_name(new_name)?;
        let edits: Vec<(Range<usize>, String)> = self
            .occurrences(id)
            .into_iter()
            .map(|range| (range, new_name.to_owned()))
            .collect();
        let mut renamed: Vec<char> = source.chars().collect();
        for (range, text) in edits.iter().rev() {
            renamed.splice(range.clone(), text.chars());
        }
        let renamed: String = renamed.into_iter().collect();
        let again = analyze(&renamed);
        let group = self.group(id);
        let same = again.symbols.len() == self.symbols.len()
            && again.references.len() == self.references.len()
            && self
                .references
                .iter()
                .zip(&again.references)
                .all(|(before, after)| before.symbol == after.symbol)
            && self.symbols.iter().enumerate().all(|(index, before)| {
                let after = &again.symbols[index];
                let expected = if group.contains(&index) {
                    new_name
                } else {
                    &before.name
                };
                after.name == expected && after.kind == before.kind
            });
        if same {
            Ok(edits)
        } else {
            Err(RenameError::Conflict)
        }
    }
}

const RESERVED: [&str; 11] = [
    "import", "export", "method", "type", "enum", "as", "in", "to", "by", "var", "varip",
];

/// Whether `name` could be a user symbol's name: one identifier, not a keyword or builtin.
pub fn check_name(name: &str) -> Result<(), RenameError> {
    let invalid = |why: &str| Err(RenameError::Invalid(why.to_owned()));
    if name.is_empty() {
        return invalid("Enter a name.");
    }
    let lexed = Lexer::new(name).lex();
    let single = matches!(
        lexed.tokens.first().map(|t| &t.kind),
        Some(TokenKind::Identifier(word)) if word == name
    ) && lexed
        .tokens
        .iter()
        .filter(|t| !matches!(t.kind, TokenKind::Newline | TokenKind::Eof))
        .count()
        == 1;
    if !single || !lexed.diagnostics.is_empty() {
        return invalid(
            "A name is letters, digits and underscores, and does not start with a digit.",
        );
    }
    if RESERVED.contains(&name) {
        return invalid("That word is reserved.");
    }
    if let Some(violation) = identifier_rules::violation(name) {
        return invalid(&violation.message);
    }
    if symbol_catalog::is_reserved_constant(name) {
        return invalid("That name is a Pine constant.");
    }
    Ok(())
}

/// A lexed token with its range in characters.
#[derive(Clone)]
struct Tok {
    kind: TokenKind,
    range: Range<usize>,
}

impl Tok {
    fn name(&self) -> Option<&str> {
        match &self.kind {
            TokenKind::Identifier(name) => Some(name),
            _ => None,
        }
    }

    fn is_word(&self, word: &str) -> bool {
        self.name() == Some(word)
    }
}

struct Statement {
    tokens: Vec<Tok>,
    /// Leading whitespace of the statement's line, a tab being four columns.
    indent: usize,
    /// Where the statement and its indented block end.
    end: usize,
}

/// What a block of statements holds, which changes how its lines read.
#[derive(Clone, Copy, PartialEq)]
enum Body {
    Code,
    /// `switch`: a case line is `condition => value`, not a function definition.
    Switch,
    /// `type`: a line is a field.
    Fields,
    /// `enum`: a line is a member.
    Members,
}

struct Frame {
    indent: usize,
    scope: ScopeId,
    body: Body,
}

/// What the previous statement leaves for the block that may follow it.
struct Pending {
    /// A scope the header made (function parameters, loop variables); the block reuses it.
    scope: Option<ScopeId>,
    body: Body,
}

pub fn analyze(source: &str) -> SymbolTable {
    let chars: Vec<char> = source.chars().collect();
    let lexed = Lexer::new(source).lex();
    let statements = statements(&chars, &lexed.tokens);
    let mut builder = Builder {
        table: SymbolTable::default(),
        statements: &statements,
    };
    builder.table.scopes.push(Scope {
        parent: None,
        symbols: Vec::new(),
    });
    builder.run();
    builder
        .table
        .references
        .sort_by_key(|r| (r.range.start, r.range.end));
    builder.table
}

fn statements(chars: &[char], tokens: &[Token]) -> Vec<Statement> {
    // UTF-16 offset to character index.
    let mut by_unit: Vec<usize> = Vec::with_capacity(chars.len() + 1);
    for (index, character) in chars.iter().enumerate() {
        for _ in 0..character.len_utf16() {
            by_unit.push(index);
        }
    }
    by_unit.push(chars.len());
    let at = |unit: usize| by_unit.get(unit).copied().unwrap_or(chars.len());

    let mut statements: Vec<Statement> = Vec::new();
    let mut current: Vec<Tok> = Vec::new();
    let mut flush = |current: &mut Vec<Tok>| {
        if current.is_empty() {
            return;
        }
        let first = current[0].range.start;
        let mut line_start = first;
        while line_start > 0 && !matches!(chars[line_start - 1], '\n' | '\r') {
            line_start -= 1;
        }
        let indent = chars[line_start..first]
            .iter()
            .map(|c| if *c == '\t' { 4 } else { 1 })
            .sum();
        statements.push(Statement {
            tokens: std::mem::take(current),
            indent,
            end: chars.len(),
        });
    };
    for token in tokens {
        match token.kind {
            TokenKind::Indent | TokenKind::Dedent => {}
            TokenKind::Newline | TokenKind::Eof => flush(&mut current),
            _ => current.push(Tok {
                kind: token.kind.clone(),
                range: at(token.range.start.offset)..at(token.range.end.offset),
            }),
        }
    }
    flush(&mut current);

    // A statement ends where the next statement at the same or a shallower indent begins.
    let mut open: Vec<usize> = Vec::new();
    for index in 0..statements.len() {
        let (indent, start) = (
            statements[index].indent,
            statements[index].tokens[0].range.start,
        );
        while let Some(&last) = open.last() {
            if statements[last].indent >= indent {
                statements[last].end = start;
                open.pop();
            } else {
                break;
            }
        }
        open.push(index);
    }
    statements
}

struct Builder<'a> {
    table: SymbolTable,
    statements: &'a [Statement],
}

impl Builder<'_> {
    fn run(&mut self) {
        let mut stack = vec![Frame {
            indent: 0,
            scope: 0,
            body: Body::Code,
        }];
        let mut pending: Option<Pending> = None;
        let mut previous_indent: Option<usize> = None;
        for (index, statement) in self.statements.iter().enumerate() {
            let indent = statement.indent;
            if let Some(before) = previous_indent
                && indent > before
            {
                let opener = pending.take().unwrap_or(Pending {
                    scope: None,
                    body: Body::Code,
                });
                let scope = opener.scope.unwrap_or_else(|| {
                    let parent = stack.last().map(|f| f.scope);
                    self.new_scope(parent)
                });
                stack.push(Frame {
                    indent,
                    scope,
                    body: opener.body,
                });
            } else {
                while stack.len() > 1 && indent < stack.last().map_or(0, |f| f.indent) {
                    stack.pop();
                }
            }
            previous_indent = Some(indent);
            let (scope, body) = stack
                .last()
                .map_or((0, Body::Code), |frame| (frame.scope, frame.body));
            pending = self.statement(index, scope, body);
        }
    }

    fn new_scope(&mut self, parent: Option<ScopeId>) -> ScopeId {
        self.table.scopes.push(Scope {
            parent,
            symbols: Vec::new(),
        });
        self.table.scopes.len() - 1
    }

    fn declare(
        &mut self,
        scope: ScopeId,
        name: &str,
        kind: Kind,
        declaration: Range<usize>,
        visible_from: usize,
    ) -> SymbolId {
        let id = self.table.symbols.len();
        self.table.symbols.push(Symbol {
            name: name.to_owned(),
            kind,
            declaration,
            scope,
            visible_from,
            body: None,
        });
        self.table.scopes[scope].symbols.push(id);
        id
    }

    /// The latest visible declaration of `name` at `position`, innermost scope first.
    fn lookup(
        &self,
        scope: ScopeId,
        name: &str,
        callable: bool,
        position: usize,
        methods_only: bool,
    ) -> Option<SymbolId> {
        let mut current = Some(scope);
        while let Some(scope_id) = current {
            let found = self.table.scopes[scope_id]
                .symbols
                .iter()
                .copied()
                .filter(|id| {
                    let symbol = &self.table.symbols[*id];
                    symbol.name == name
                        && symbol.kind.callable() == callable
                        && (!methods_only || symbol.kind == Kind::Method)
                        && symbol.visible_from <= position
                })
                .max_by_key(|id| self.table.symbols[*id].visible_from);
            if found.is_some() {
                return found;
            }
            current = self.table.scopes[scope_id].parent;
        }
        None
    }

    fn reference(&mut self, range: Range<usize>, symbol: SymbolId) {
        self.table.references.push(Reference { range, symbol });
    }

    /// Resolves every name in `tokens` as a use, in `scope`.
    fn uses(&mut self, tokens: &[Tok], scope: ScopeId) {
        // For each open bracket: the user function a call to it names, if any.
        let mut calls: Vec<Option<SymbolId>> = Vec::new();
        for (index, token) in tokens.iter().enumerate() {
            match token.kind {
                TokenKind::LeftParen => {
                    let callee = index
                        .checked_sub(1)
                        .filter(|before| *before == 0 || tokens[*before - 1].kind != TokenKind::Dot)
                        .and_then(|before| {
                            let name = tokens[before].name()?;
                            self.lookup(scope, name, true, tokens[before].range.start, false)
                        });
                    calls.push(callee);
                }
                TokenKind::LeftBracket => calls.push(None),
                TokenKind::RightParen | TokenKind::RightBracket => {
                    calls.pop();
                }
                _ => {}
            }
            let Some(name) = token.name() else { continue };
            let previous = index.checked_sub(1).map(|i| &tokens[i].kind);
            let next = tokens.get(index + 1).map(|t| &t.kind);
            let position = token.range.start;
            if previous == Some(&TokenKind::Dot) {
                if next == Some(&TokenKind::LeftParen)
                    && let Some(method) = self.lookup(scope, name, true, position, true)
                {
                    self.reference(token.range.clone(), method);
                }
                continue;
            }
            let after_open = matches!(previous, Some(TokenKind::LeftParen | TokenKind::Comma));
            if after_open
                && next == Some(&TokenKind::Assign)
                && let Some(Some(function)) = calls.last()
            {
                // `f(name = value)`: the label names one of `f`'s parameters.
                let parameter = self.table.symbols[*function].body.and_then(|body| {
                    self.table.scopes[body].symbols.iter().copied().find(|id| {
                        let symbol = &self.table.symbols[*id];
                        symbol.kind == Kind::Parameter && symbol.name == name
                    })
                });
                if let Some(parameter) = parameter {
                    self.reference(token.range.clone(), parameter);
                }
                continue;
            }
            if after_open && next == Some(&TokenKind::Assign) && !calls.is_empty() {
                // A label on a builtin: never ours.
                continue;
            }
            let callable = next == Some(&TokenKind::LeftParen);
            if let Some(symbol) = self.lookup(scope, name, callable, position, false) {
                self.reference(token.range.clone(), symbol);
            }
        }
    }

    /// Reads one statement. Returns what a block after it should inherit.
    fn statement(&mut self, index: usize, scope: ScopeId, body: Body) -> Option<Pending> {
        let statements = self.statements;
        let statement = &statements[index];
        let tokens = &statement.tokens;
        let end = statement.end;
        match body {
            Body::Fields => {
                self.field(tokens, scope);
                return None;
            }
            Body::Members => {
                self.member(tokens, scope);
                return None;
            }
            Body::Code | Body::Switch => {}
        }
        // `export` only publishes what follows.
        let skip = usize::from(tokens[0].is_word("export") && tokens.len() > 1);
        let tokens = &tokens[skip..];
        let first = &tokens[0];

        if first.is_word("import") {
            self.import(tokens);
            return None;
        }
        if (first.is_word("type") || first.is_word("enum"))
            && let Some(name) = tokens.get(1)
            && let Some(text) = name.name()
            && tokens.len() == 2
        {
            let (kind, body) = if first.is_word("type") {
                (Kind::Type, Body::Fields)
            } else {
                (Kind::Enum, Body::Members)
            };
            self.declare(scope, text, kind, name.range.clone(), name.range.start);
            return Some(Pending { scope: None, body });
        }
        if body == Body::Code
            && let Some(opener) = self.function(tokens, scope, end)
        {
            return Some(opener);
        }
        match first.kind {
            TokenKind::For => return Some(self.for_loop(tokens, scope)),
            TokenKind::If
            | TokenKind::While
            | TokenKind::Once
            | TokenKind::Else
            | TokenKind::Switch => {
                let body = if tokens.iter().any(|t| t.kind == TokenKind::Switch) {
                    Body::Switch
                } else {
                    Body::Code
                };
                self.uses(tokens, scope);
                return Some(Pending { scope: None, body });
            }
            _ => {}
        }
        let switch = if tokens.iter().any(|t| t.kind == TokenKind::Switch) {
            Body::Switch
        } else {
            Body::Code
        };
        self.assignment(tokens, scope, end);
        Some(Pending {
            scope: None,
            body: switch,
        })
    }

    fn import(&mut self, tokens: &[Tok]) {
        if let Some(position) = tokens.iter().position(|t| t.is_word("as"))
            && let Some(alias) = tokens.get(position + 1)
            && let Some(name) = alias.name()
        {
            self.declare(
                0,
                name,
                Kind::ImportAlias,
                alias.range.clone(),
                alias.range.end,
            );
        }
    }

    /// A field line of a `type`: the field name is not a symbol, its type and default are uses.
    fn field(&mut self, tokens: &[Tok], scope: ScopeId) {
        let equals = tokens
            .iter()
            .position(|t| t.kind == TokenKind::Assign)
            .unwrap_or(tokens.len());
        let name = equals
            .checked_sub(1)
            .filter(|i| tokens[*i].name().is_some());
        let rest: Vec<Tok> = tokens
            .iter()
            .enumerate()
            .filter(|(i, _)| Some(*i) != name)
            .map(|(_, t)| t.clone())
            .collect();
        self.uses(&rest, scope);
    }

    /// An enum member line: the member is not a symbol, a title string may follow.
    fn member(&mut self, _tokens: &[Tok], _scope: ScopeId) {}

    /// `[method] name(parameters) => body`, when `tokens` is one.
    fn function(&mut self, tokens: &[Tok], scope: ScopeId, end: usize) -> Option<Pending> {
        let method = tokens[0].is_word("method");
        let name_index = usize::from(method);
        let name = tokens.get(name_index)?;
        let text = name.name()?;
        if tokens.get(name_index + 1)?.kind != TokenKind::LeftParen {
            return None;
        }
        let mut depth = 0usize;
        let mut close = None;
        for (offset, token) in tokens.iter().enumerate().skip(name_index + 1) {
            match token.kind {
                TokenKind::LeftParen | TokenKind::LeftBracket => depth += 1,
                TokenKind::RightParen | TokenKind::RightBracket => {
                    depth = depth.checked_sub(1)?;
                    if depth == 0 {
                        close = Some(offset);
                        break;
                    }
                }
                _ => {}
            }
        }
        let close = close?;
        if tokens.get(close + 1)?.kind != TokenKind::Arrow {
            return None;
        }
        let kind = if method { Kind::Method } else { Kind::Function };
        let body = self.new_scope(Some(scope));
        let id = self.declare(scope, text, kind, name.range.clone(), end);
        self.table.symbols[id].body = Some(body);

        let mut start = name_index + 2;
        let mut depth = 0usize;
        for offset in name_index + 2..close {
            match tokens[offset].kind {
                TokenKind::LeftParen | TokenKind::LeftBracket => depth += 1,
                TokenKind::RightParen | TokenKind::RightBracket => {
                    depth = depth.saturating_sub(1);
                }
                TokenKind::Comma if depth == 0 => {
                    self.parameter(&tokens[start..offset], scope, body);
                    start = offset + 1;
                }
                _ => {}
            }
        }
        self.parameter(&tokens[start..close], scope, body);
        // An inline body: `f(x) => x * 2`.
        self.uses(&tokens[close + 2..], body);
        Some(Pending {
            scope: Some(body),
            body: Body::Code,
        })
    }

    /// One parameter: `[qualifier] [type] name [= default]`.
    fn parameter(&mut self, tokens: &[Tok], outer: ScopeId, body: ScopeId) {
        if tokens.is_empty() {
            return;
        }
        let equals = tokens
            .iter()
            .position(|t| t.kind == TokenKind::Assign)
            .unwrap_or(tokens.len());
        let Some(name) = equals
            .checked_sub(1)
            .and_then(|i| tokens[i].name().map(|n| (i, n)))
        else {
            return;
        };
        // The type words and the default value read in the enclosing scope.
        let outside: Vec<Tok> = tokens
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != name.0)
            .map(|(_, t)| t.clone())
            .collect();
        self.uses(&outside, outer);
        let range = tokens[name.0].range.clone();
        self.declare(body, name.1, Kind::Parameter, range.clone(), range.end);
    }

    fn for_loop(&mut self, tokens: &[Tok], scope: ScopeId) -> Pending {
        let loop_scope = self.new_scope(Some(scope));
        let mut depth = 0usize;
        let mut split = None;
        for (offset, token) in tokens.iter().enumerate().skip(1) {
            match token.kind {
                TokenKind::LeftParen | TokenKind::LeftBracket => depth += 1,
                TokenKind::RightParen | TokenKind::RightBracket => {
                    depth = depth.saturating_sub(1);
                }
                TokenKind::Assign if depth == 0 => {
                    split = Some(offset);
                    break;
                }
                _ if depth == 0 && token.is_word("in") => {
                    split = Some(offset);
                    break;
                }
                _ => {}
            }
        }
        let Some(split) = split else {
            self.uses(&tokens[1..], scope);
            return Pending {
                scope: None,
                body: Body::Code,
            };
        };
        // The names before `=` / `in`: one identifier, or a tuple of them.
        let names: Vec<usize> = if tokens[split - 1].kind == TokenKind::RightBracket {
            (1..split)
                .filter(|i| tokens[*i].name().is_some() && tokens[*i - 1].kind != TokenKind::Dot)
                .collect()
        } else {
            tokens[split - 1]
                .name()
                .map(|_| split - 1)
                .into_iter()
                .collect()
        };
        let header_end = tokens.last().map_or(0, |t| t.range.end);
        for name in &names {
            let token = &tokens[*name];
            if let Some(text) = token.name() {
                self.declare(
                    loop_scope,
                    text,
                    Kind::LoopVariable,
                    token.range.clone(),
                    header_end,
                );
            }
        }
        let rest: Vec<Tok> = tokens[split + 1..]
            .iter()
            .filter(|t| !(t.is_word("to") || t.is_word("by")))
            .cloned()
            .collect();
        self.uses(&rest, scope);
        Pending {
            scope: Some(loop_scope),
            body: Body::Code,
        }
    }

    /// Declarations (`x = …`, `var float x = …`, `[a, b] = …`), reassignments and plain
    /// expressions.
    fn assignment(&mut self, tokens: &[Tok], scope: ScopeId, end: usize) {
        let mut depth = 0usize;
        let mut operator = None;
        for (offset, token) in tokens.iter().enumerate() {
            match token.kind {
                TokenKind::LeftParen | TokenKind::LeftBracket => depth += 1,
                TokenKind::RightParen | TokenKind::RightBracket => {
                    depth = depth.saturating_sub(1);
                }
                TokenKind::Assign
                | TokenKind::Reassign
                | TokenKind::PlusAssign
                | TokenKind::MinusAssign
                | TokenKind::StarAssign
                | TokenKind::SlashAssign
                | TokenKind::PercentAssign
                    if depth == 0 =>
                {
                    operator = Some(offset);
                    break;
                }
                _ => {}
            }
        }
        let Some(operator) = operator else {
            self.uses(tokens, scope);
            return;
        };
        let (left, right) = (&tokens[..operator], &tokens[operator + 1..]);
        self.uses(right, scope);
        if tokens[operator].kind != TokenKind::Assign {
            self.uses(left, scope);
            return;
        }
        let tuple = left
            .first()
            .is_some_and(|t| t.kind == TokenKind::LeftBracket);
        if tuple {
            for token in left {
                if let Some(name) = token.name() {
                    self.declare(scope, name, Kind::Variable, token.range.clone(), end);
                }
            }
            return;
        }
        let target = left.len().checked_sub(1).filter(|i| {
            left[*i].name().is_some() && (*i == 0 || left[*i - 1].kind != TokenKind::Dot)
        });
        match target {
            Some(position) => {
                // Type words before the name are uses; the name is the declaration.
                let before: Vec<Tok> = left[..position].to_vec();
                self.uses(&before, scope);
                let token = &left[position];
                if let Some(name) = token.name() {
                    self.declare(scope, name, Kind::Variable, token.range.clone(), end);
                }
            }
            None => self.uses(left, scope),
        }
    }
}

/// The names a script declares, by kind, for tests and tooling.
#[cfg(test)]
fn declared(table: &SymbolTable) -> std::collections::HashMap<String, Kind> {
    table
        .symbols
        .iter()
        .map(|s| (s.name.clone(), s.kind))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(source: &str, needle: &str, occurrence: usize) -> usize {
        let chars: Vec<char> = source.chars().collect();
        let needle: Vec<char> = needle.chars().collect();
        let mut seen = 0;
        for start in 0..chars.len() {
            if chars[start..].starts_with(&needle) {
                if seen == occurrence {
                    return start;
                }
                seen += 1;
            }
        }
        panic!("no occurrence {occurrence} of {needle:?}");
    }

    /// Where the name at the `occurrence`th `needle` is declared, as its character start.
    fn definition(source: &str, needle: &str, occurrence: usize) -> Option<usize> {
        let table = analyze(source);
        table
            .definition_of(at(source, needle, occurrence))
            .map(|range| range.start)
    }

    fn renamed(
        source: &str,
        needle: &str,
        occurrence: usize,
        to: &str,
    ) -> Result<String, RenameError> {
        let table = analyze(source);
        let id = table
            .symbol_at(at(source, needle, occurrence))
            .expect("a symbol there");
        let edits = table.rename(source, id, to)?;
        let mut chars: Vec<char> = source.chars().collect();
        for (range, text) in edits.iter().rev() {
            chars.splice(range.clone(), text.chars());
        }
        Ok(chars.into_iter().collect())
    }

    const HEADER: &str = "//@version=6\nindicator(\"T\")\n";

    fn script(body: &str) -> String {
        format!("{HEADER}{body}")
    }

    #[test]
    fn a_use_resolves_to_its_declaration() {
        let source = script("len = 14\nplot(ta.sma(close, len))\n");
        assert_eq!(definition(&source, "len", 1), Some(at(&source, "len", 0)));
        assert_eq!(
            definition(&source, "len", 0),
            None,
            "the declaration itself"
        );
    }

    #[test]
    fn the_right_hand_side_reads_the_previous_binding() {
        let source = script("x = 1\nif close > open\n    x = x + 1\n    plot(x)\nplot(x)\n");
        // The inner `x = x + 1`: its right-hand `x` is the outer one.
        assert_eq!(definition(&source, "x + 1", 0), Some(at(&source, "x", 0)));
        // `plot(x)` inside the block reads the inner declaration; after it, the outer one.
        assert_eq!(definition(&source, "x)", 0), Some(at(&source, "x = x", 0)));
        assert_eq!(definition(&source, "x)", 1), Some(at(&source, "x", 0)));
    }

    #[test]
    fn reassignment_refers_to_the_declaration() {
        let source = script("var total = 0.0\ntotal := total + close\n");
        assert_eq!(
            definition(&source, "total :=", 0),
            Some(at(&source, "total", 0))
        );
    }

    #[test]
    fn a_use_before_the_declaration_is_unresolved() {
        let source = script("plot(later)\nlater = 1\n");
        assert_eq!(definition(&source, "later", 0), None);
    }

    #[test]
    fn functions_and_values_are_separate_namespaces() {
        let source =
            script("kama(src, len) =>\n    src * len\nkama = kama(close, 3)\nplot(kama)\n");
        let function = at(&source, "kama", 0);
        let variable = at(&source, "kama = ", 0);
        assert_eq!(definition(&source, "kama(close", 0), Some(function));
        assert_eq!(definition(&source, "kama)", 0), Some(variable));
    }

    #[test]
    fn parameters_are_local_to_the_function() {
        let source = script("f(a, b = 2) =>\n    a + b\nplot(f(1))\nplot(a)\n");
        assert_eq!(definition(&source, "a + b", 0), Some(at(&source, "a, ", 0)));
        assert_eq!(definition(&source, "b\n", 0), Some(at(&source, "b = 2", 0)));
        assert_eq!(definition(&source, "a)", 0), None, "outside the body");
    }

    #[test]
    fn a_function_body_sees_earlier_globals_not_later_ones() {
        let source = script("g = 2\nf(x) =>\n    x * g * h\nh = 3\n");
        assert_eq!(definition(&source, "g * h", 0), Some(at(&source, "g", 0)));
        assert_eq!(definition(&source, "h\nh", 0), None);
    }

    #[test]
    fn an_inline_function_body_sees_its_parameters() {
        let source = script("sq(val) => val * val\nplot(sq(close))\n");
        assert_eq!(
            definition(&source, "val * val", 0),
            Some(at(&source, "val", 0))
        );
        assert_eq!(
            definition(&source, "sq(close", 0),
            Some(at(&source, "sq", 0))
        );
    }

    #[test]
    fn typed_parameters_and_defaults_read_in_the_outer_scope() {
        let source = script("base = 3\nf(int arg = base) =>\n    arg\n");
        assert_eq!(
            definition(&source, "base)", 0),
            Some(at(&source, "base", 0))
        );
        assert_eq!(
            definition(&source, "arg\n", 0),
            Some(at(&source, "arg =", 0))
        );
    }

    #[test]
    fn loop_variables_live_in_the_loop() {
        let source = script("sum = 0.0\nfor idx = 0 to 9\n    sum += idx\nplot(idx)\n");
        assert_eq!(
            definition(&source, "idx\n", 0),
            Some(at(&source, "idx =", 0))
        );
        assert_eq!(definition(&source, "idx)", 0), None);
        assert_eq!(
            definition(&source, "sum +=", 0),
            Some(at(&source, "sum", 0))
        );
    }

    #[test]
    fn typed_loops_and_for_in_declare_their_names() {
        let source = script(
            "a = array.new_float()\nfor int i = 0 to 3\n    plot(i)\nfor v in a\n    plot(v)\nfor [k, w] in a\n    plot(k + w)\n",
        );
        assert_eq!(definition(&source, "i)", 0), Some(at(&source, "i =", 0)));
        assert_eq!(definition(&source, "v)", 0), Some(at(&source, "v in", 0)));
        assert_eq!(definition(&source, "k +", 0), Some(at(&source, "k,", 0)));
        assert_eq!(definition(&source, "w)", 0), Some(at(&source, "w]", 0)));
        assert_eq!(definition(&source, "in a", 1), None, "`in` is not a name");
        assert_eq!(definition(&source, "a\n", 0), Some(at(&source, "a =", 0)));
    }

    #[test]
    fn tuple_declarations_declare_each_name() {
        let source = script("[m, s, h] = ta.macd(close, 12, 26, 9)\nplot(m + s + h)\n");
        assert_eq!(definition(&source, "m +", 0), Some(at(&source, "m,", 0)));
        assert_eq!(definition(&source, "h)", 0), Some(at(&source, "h]", 0)));
    }

    #[test]
    fn a_field_assignment_is_a_use_of_the_object() {
        let source = script("type Box\n    float top = 0\nb = Box.new()\nb.top := 5\n");
        assert_eq!(
            definition(&source, "b.top", 0),
            Some(at(&source, "b = ", 0))
        );
        assert_eq!(
            definition(&source, "top :=", 0),
            None,
            "fields are not resolved"
        );
    }

    #[test]
    fn types_and_enums_resolve_from_annotations_and_constructors() {
        let source = script(
            "type Zone\n    float top\nenum Mode\n    fast\n    slow\nZone z = Zone.new(1)\nm = Mode.fast\narray<Zone> zs = array.new<Zone>()\n",
        );
        assert_eq!(
            definition(&source, "Zone z", 0),
            Some(at(&source, "Zone", 0))
        );
        assert_eq!(
            definition(&source, "Zone.new", 0),
            Some(at(&source, "Zone", 0))
        );
        assert_eq!(
            definition(&source, "Mode.fast", 0),
            Some(at(&source, "Mode", 0))
        );
        assert_eq!(
            definition(&source, "Zone>", 0),
            Some(at(&source, "Zone", 0))
        );
        assert_eq!(
            definition(&source, "Zone>", 1),
            Some(at(&source, "Zone", 0))
        );
    }

    #[test]
    fn methods_match_by_name_after_a_dot() {
        let source = script(
            "type P\n    float x\nmethod bump(P this) =>\n    this.x + 1\np = P.new(1)\nplot(p.bump())\n",
        );
        assert_eq!(
            definition(&source, "bump())", 0),
            Some(at(&source, "bump", 0))
        );
        assert_eq!(
            definition(&source, "this.x", 0),
            Some(at(&source, "this)", 0))
        );
    }

    #[test]
    fn labels_on_user_functions_name_the_parameter() {
        let source =
            script("f(width, height) =>\n    width * height\nplot(f(height = 2, width = 3))\n");
        assert_eq!(
            definition(&source, "height =", 0),
            Some(at(&source, "height", 0))
        );
        assert_eq!(
            definition(&source, "width =", 0),
            Some(at(&source, "width", 0))
        );
    }

    #[test]
    fn labels_on_builtins_are_never_user_names() {
        let source = script("title = \"x\"\nplot(close, title = title)\n");
        assert_eq!(definition(&source, "title =", 1), None, "the label");
        assert_eq!(
            definition(&source, "title)", 0),
            Some(at(&source, "title", 0))
        );
    }

    #[test]
    fn strings_and_comments_hold_no_names() {
        let source = script("len = 5\n// len is here\nlabel.new(bar_index, close, \"len\")\n");
        assert_eq!(analyze(&source).occurrences(0).len(), 1);
    }

    #[test]
    fn switch_cases_are_not_function_definitions() {
        let source =
            script("sel = 2\nres = switch sel\n    1 => 10\n    2 => 20\n    => 0\nplot(res)\n");
        assert_eq!(
            definition(&source, "sel\n    1", 0),
            Some(at(&source, "sel", 0))
        );
        assert_eq!(
            definition(&source, "res)", 0),
            Some(at(&source, "res =", 0))
        );
    }

    #[test]
    fn a_value_block_declares_its_name_after_the_block() {
        let source = script("x = 1\nx2 = if close > open\n    x\nelse\n    -x\n");
        assert_eq!(definition(&source, "x\nelse", 0), Some(at(&source, "x", 0)));
    }

    #[test]
    fn wrapped_lines_stay_one_statement() {
        let source = script("total = close +\n  open +\n  high\nplot(total)\n");
        assert_eq!(
            definition(&source, "total)", 0),
            Some(at(&source, "total", 0))
        );
    }

    #[test]
    fn tabs_count_as_four_columns() {
        let source = script("if close > open\n\ty = 1\n\tplot(y)\n");
        assert_eq!(definition(&source, "y)", 0), Some(at(&source, "y", 0)));
    }

    #[test]
    fn import_aliases_are_declared() {
        let source = script("import user/lib/1 as lib\nplot(lib.f(close))\n");
        assert_eq!(
            definition(&source, "lib.f", 0),
            Some(at(&source, "lib\n", 0))
        );
    }

    #[test]
    fn crlf_and_astral_characters_do_not_shift_ranges() {
        let source = "//@version=6\r\nindicator(\"\u{1F600}\")\r\nlen = 3\r\nplot(len)\r\n";
        assert_eq!(definition(source, "len)", 0), Some(at(source, "len", 0)));
        let table = analyze(source);
        let id = table.symbol_at(at(source, "len)", 0)).unwrap();
        let ranges = table.occurrences(id);
        assert_eq!(
            ranges
                .iter()
                .map(|r| source
                    .chars()
                    .skip(r.start)
                    .take(r.len())
                    .collect::<String>())
                .collect::<Vec<_>>(),
            ["len", "len"]
        );
    }

    #[test]
    fn broken_code_resolves_what_it_can() {
        let source = script("len = 5\nplot(ta.sma(close, len\nother = len\n");
        let table = analyze(&source);
        assert!(table.symbol_at(at(&source, "len = 5", 0)).is_some());
    }

    #[test]
    fn rename_changes_the_declaration_and_every_use() {
        let source = script("len = 14\nplot(ta.sma(close, len) + len)\n");
        let out = renamed(&source, "len", 0, "period").unwrap();
        assert_eq!(
            out,
            script("period = 14\nplot(ta.sma(close, period) + period)\n")
        );
    }

    #[test]
    fn rename_respects_shadowing() {
        let source = script("x = 1\nf(v) =>\n    x = v * 2\n    x\nplot(x + f(1))\n");
        // The outer `x` only.
        let out = renamed(&source, "x", 0, "outer").unwrap();
        assert!(out.contains("outer = 1"));
        assert!(out.contains("    x = v * 2"));
        assert!(out.contains("plot(outer + f(1))"));
    }

    #[test]
    fn rename_a_parameter_changes_the_labels_too() {
        let source = script("f(width) =>\n    width * 2\nplot(f(width = 3))\n");
        let out = renamed(&source, "width", 0, "w").unwrap();
        assert_eq!(out, script("f(w) =>\n    w * 2\nplot(f(w = 3))\n"));
    }

    #[test]
    fn rename_a_function_keeps_a_variable_of_the_same_name() {
        let source = script("kama(a) =>\n    a\nkama = kama(close)\nplot(kama)\n");
        let out = renamed(&source, "kama", 0, "smooth").unwrap();
        assert_eq!(
            out,
            script("smooth(a) =>\n    a\nkama = smooth(close)\nplot(kama)\n")
        );
    }

    #[test]
    fn overloads_rename_together() {
        let source = script("f(a) =>\n    a\nf(a, b) =>\n    a + b\nplot(f(1) + f(1, 2))\n");
        let out = renamed(&source, "f(1)", 0, "g").unwrap();
        assert_eq!(out.matches("g(").count(), 4);
        assert!(!out.contains("f("));
    }

    #[test]
    fn rename_refuses_a_name_that_would_capture() {
        let source = script("aa = 1\nbb = 2\nplot(aa + bb)\n");
        assert_eq!(renamed(&source, "aa", 0, "bb"), Err(RenameError::Conflict));
    }

    #[test]
    fn rename_refuses_a_name_a_nested_use_would_hit() {
        // `inner` shadows nothing today; naming the outer `y` would make the use of `y` in the
        // function read the parameter instead.
        let source = script("x = 1\nf(y) =>\n    x + y\nplot(f(2))\n");
        assert_eq!(renamed(&source, "x", 0, "y"), Err(RenameError::Conflict));
    }

    #[test]
    fn rename_refuses_builtins_keywords_and_junk() {
        let source = script("aa = 1\nplot(aa)\n");
        for bad in [
            "close", "ta.sma", "plot", "if", "1a", "a b", "", "in", "true",
        ] {
            assert!(
                matches!(renamed(&source, "aa", 0, bad), Err(RenameError::Invalid(_))),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn rename_to_the_same_name_is_a_no_op_edit() {
        let source = script("aa = 1\nplot(aa)\n");
        assert_eq!(renamed(&source, "aa", 0, "aa").unwrap(), source);
    }

    #[test]
    fn declared_kinds_are_recorded() {
        let source = script(
            "import u/l/1 as l\ntype T\n    float f\nenum E\n    a\nm(q) =>\n    q\nfor i = 0 to 1\n    plot(i)\nv = 1\n",
        );
        let kinds = declared(&analyze(&source));
        assert_eq!(kinds["l"], Kind::ImportAlias);
        assert_eq!(kinds["T"], Kind::Type);
        assert_eq!(kinds["E"], Kind::Enum);
        assert_eq!(kinds["m"], Kind::Function);
        assert_eq!(kinds["q"], Kind::Parameter);
        assert_eq!(kinds["i"], Kind::LoopVariable);
        assert_eq!(kinds["v"], Kind::Variable);
    }

    fn calls(source: &str, needle: &str, occurrence: usize) -> Option<Vec<usize>> {
        analyze(source)
            .calls_of(at(source, needle, occurrence))
            .map(|ranges| ranges.iter().map(|r| r.start).collect())
    }

    #[test]
    fn a_declaration_lists_its_calls() {
        let source = script("half(x) => x / 2\na = half(1)\nb = half(2)\n");
        assert_eq!(
            calls(&source, "half", 0),
            Some(vec![at(&source, "half", 1), at(&source, "half", 2)])
        );
        let none = script("half(x) => x / 2\n");
        assert_eq!(calls(&none, "half", 0), Some(vec![]));
    }

    #[test]
    fn every_overload_lists_the_calls_of_all_of_them() {
        let source = script(
            "describe(int x) => 1\ndescribe(string s) => 2\na = describe(1)\nb = describe(\"x\")\n",
        );
        let both = Some(vec![at(&source, "describe", 2), at(&source, "describe", 3)]);
        assert_eq!(calls(&source, "describe", 0), both);
        assert_eq!(calls(&source, "describe", 1), both);
    }

    #[test]
    fn a_methods_calls_include_the_dotted_ones_in_other_methods() {
        let source = script(
            "type Z\n    int n\nmethod grow(Z this) => this\nz = Z.new(1)\nz.grow()\n\
             method again(Z this) =>\n    this.grow()\n",
        );
        assert_eq!(
            calls(&source, "grow", 0),
            Some(vec![at(&source, "grow", 1), at(&source, "grow", 2)])
        );
    }

    #[test]
    fn a_variable_or_a_label_of_the_same_name_is_not_a_call() {
        let source = script("f(x) => x\nf = 3\nplot(f)\ny = f(1)\n");
        assert_eq!(
            calls(&source, "f(x)", 0),
            Some(vec![at(&source, "f(1)", 0)])
        );
        let source = script("scale(x, k) => x * k\nv = scale(1, k = 2)\n");
        assert_eq!(
            calls(&source, "scale", 0),
            Some(vec![at(&source, "scale", 1)])
        );
    }

    #[test]
    fn only_a_function_declaration_answers() {
        let source = script("half(x) => x / 2\nlen = 14\ntype Z\n    int n\na = half(len)\n");
        // A call, a variable, its use, a type, a parameter: none is a function's declaration.
        assert_eq!(calls(&source, "half", 1), None);
        assert_eq!(calls(&source, "len", 0), None);
        assert_eq!(calls(&source, "len", 1), None);
        assert_eq!(calls(&source, "Z", 0), None);
        assert_eq!(calls(&source, "x", 0), None);
    }
}
