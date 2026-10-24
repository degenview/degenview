//! Which statements of a program can be stopped at, where they sit in the source, and which names
//! are in scope at each.
//!
//! The interpreter walks the syntax tree, so a stop is always "before this statement". A statement
//! is identified by the UTF-16 offset where the parser anchored it: ranges are start-anchored, a
//! call's range is its first token, and the language has no `;`, so no two statements start at the
//! same offset. A line is never an identity: `x = if c` / `    a` / `else` / `    b` puts one
//! statement on each of several lines, and a statement can span many.
//!
//! The interpreter's variable table is flat and keeps names after their block ends, so what a
//! stopped script may show is decided here, from the syntax, never from what the table holds.

use crate::pine::ast::{DeclarationMode, Expression, Statement};
use crate::pine::traversal;

/// A statement the interpreter runs, in source order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Executable {
    /// UTF-16 offset of the statement's start in the compiled (CRLF-folded) source.
    pub offset: usize,
    /// One-based line and column of that offset.
    pub line: usize,
    pub column: usize,
    /// The user function whose body holds the statement; `None` at the top level.
    pub function: Option<String>,
    /// The newest name visible when the statement starts, as an index into the scope table.
    scope: Option<u32>,
}

/// How a name came to exist.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeclarationKind {
    Ordinary,
    /// `var`: initialised once, kept across bars.
    Var,
    /// `varip`: kept across realtime ticks as well.
    Varip,
    /// A parameter of a user function.
    Parameter,
    /// The counter or element of a `for` loop.
    Loop,
    /// One name of `[a, b] = …`.
    Tuple,
}

impl DeclarationKind {
    pub fn is_persistent(self) -> bool {
        matches!(self, Self::Var | Self::Varip)
    }
}

/// A name in scope. Entries form chains through `parent`: each statement points at the newest name
/// visible to it, and its older siblings and the names of the blocks around it follow. A block's own
/// names are not reachable from after it, because the chain after the block is the one from before.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScopeEntry {
    pub name: String,
    pub kind: DeclarationKind,
    /// The statement that gives the name a value: its declaration, the `for` it counts, or the
    /// function declaration for a parameter.
    pub offset: usize,
    /// Declared at the top level of the script, as opposed to inside a function or a block.
    pub global: bool,
    parent: Option<u32>,
}

/// Where a breakpoint on a line lands.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LineResolution {
    /// The first statement that starts on the line.
    Executable { offset: usize },
    /// Nothing runs from this line.
    NotExecutable(NotExecutable),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NotExecutable {
    /// A blank line, a comment, a continuation of a statement that began earlier, or a line past
    /// the end.
    NoCode,
    /// `name(args) =>`: declaring a function runs nothing; its body does.
    FunctionHeader,
    /// `type`, `enum`, `indicator()` and the like.
    Declaration,
}

impl NotExecutable {
    pub fn describe(self) -> &'static str {
        match self {
            Self::NoCode => "There is no code to stop at on this line.",
            Self::FunctionHeader => {
                "Declaring a function runs nothing; stop inside its body instead."
            }
            Self::Declaration => "This declaration runs nothing.",
        }
    }
}

/// The stoppable statements of one program, ordered by offset, and the names in scope at each.
#[derive(Clone, Debug, Default)]
pub struct SourceMap {
    executables: Vec<Executable>,
    scope: Vec<ScopeEntry>,
    /// Lines that declare something that never runs, to say why a breakpoint there is refused.
    declarations: Vec<(usize, NotExecutable)>,
}

impl SourceMap {
    pub fn build(statements: &[Statement]) -> Self {
        let mut map = Self::default();
        map.block(statements, None, None, true);
        map.executables.sort_by_key(|executable| executable.offset);
        map.executables.dedup_by_key(|executable| executable.offset);
        map
    }

    fn declare(
        &mut self,
        head: &mut Option<u32>,
        name: &str,
        kind: DeclarationKind,
        offset: usize,
        global: bool,
    ) {
        self.scope.push(ScopeEntry {
            name: name.to_owned(),
            kind,
            offset,
            global,
            parent: *head,
        });
        *head = Some((self.scope.len() - 1) as u32);
    }

    /// Visits the statements of one block. `head` is what is in scope on entry; names the block
    /// declares are visible to its later statements and to nothing after the block.
    fn block(
        &mut self,
        statements: &[Statement],
        mut head: Option<u32>,
        function: Option<&str>,
        global: bool,
    ) {
        for statement in statements {
            self.statement(statement, &mut head, function, global);
        }
    }

    fn statement(
        &mut self,
        statement: &Statement,
        head: &mut Option<u32>,
        function: Option<&str>,
        global: bool,
    ) {
        let range = statement.range();
        let offset = range.start.offset;
        match statement {
            Statement::Function {
                name,
                parameters,
                body,
                ..
            } => {
                self.declarations
                    .push((range.start.line, NotExecutable::FunctionHeader));
                // A function sees the names declared before it, plus its own parameters.
                let mut inner = *head;
                for parameter in parameters {
                    self.declare(
                        &mut inner,
                        &parameter.name,
                        DeclarationKind::Parameter,
                        offset,
                        false,
                    );
                }
                self.block(body, inner, Some(name), false);
                return;
            }
            Statement::TypeDeclaration { .. } | Statement::EnumDeclaration { .. } => {
                self.declarations
                    .push((range.start.line, NotExecutable::Declaration));
                return;
            }
            // `indicator(…)`, `strategy(…)` and `library(…)` configure the script; the interpreter
            // walks past them on every bar, but there is nothing in them to stop at.
            Statement::Expression(Expression::Call { name, .. })
                if matches!(name.as_str(), "indicator" | "strategy" | "library") =>
            {
                self.declarations
                    .push((range.start.line, NotExecutable::Declaration));
                return;
            }
            _ => {}
        }
        self.executables.push(Executable {
            offset,
            line: range.start.line,
            column: range.start.column,
            function: function.map(str::to_owned),
            scope: *head,
        });
        // Value blocks (`x = if …`) run before the name they produce exists.
        for expression in traversal::expressions(statement) {
            self.expression(expression, *head, function);
        }
        match statement {
            Statement::ForRange { variable, body, .. } => {
                let mut inner = *head;
                self.declare(&mut inner, variable, DeclarationKind::Loop, offset, false);
                self.block(body, inner, function, false);
            }
            Statement::ForIn {
                index, value, body, ..
            } => {
                let mut inner = *head;
                if let Some(index) = index {
                    self.declare(&mut inner, index, DeclarationKind::Loop, offset, false);
                }
                self.declare(&mut inner, value, DeclarationKind::Loop, offset, false);
                self.block(body, inner, function, false);
            }
            _ => {
                for body in traversal::nested_blocks(statement) {
                    self.block(body, *head, function, false);
                }
            }
        }
        match statement {
            Statement::Declaration { name, mode, .. } => {
                let kind = match mode {
                    DeclarationMode::Ordinary => DeclarationKind::Ordinary,
                    DeclarationMode::Variable => DeclarationKind::Var,
                    DeclarationMode::Intrabar => DeclarationKind::Varip,
                };
                self.declare(head, name, kind, offset, global);
            }
            Statement::TupleDeclaration { names, .. } => {
                for name in names {
                    self.declare(head, name, DeclarationKind::Tuple, offset, global);
                }
            }
            _ => {}
        }
    }

    /// `x = if …` and `x = switch …` hold statements inside an expression.
    fn expression(&mut self, expression: &Expression, head: Option<u32>, function: Option<&str>) {
        if let Expression::Statement(inner, _) = expression {
            let mut inner_head = head;
            self.statement(inner, &mut inner_head, function, false);
        }
        for child in traversal::children(expression) {
            self.expression(child, head, function);
        }
    }

    /// The names a statement can see when it starts, nearest declaration first, each name once (an
    /// inner declaration hides an outer one of the same name).
    pub fn visible(&self, offset: usize) -> Vec<&ScopeEntry> {
        let mut seen = std::collections::HashSet::new();
        let mut visible = Vec::new();
        let mut next = self.at(offset).and_then(|executable| executable.scope);
        while let Some(index) = next {
            let entry = &self.scope[index as usize];
            if seen.insert(entry.name.as_str()) {
                visible.push(entry);
            }
            next = entry.parent;
        }
        visible
    }

    /// The position of the statement among the stoppable ones, for per-statement bit sets.
    pub fn index_of(&self, offset: usize) -> Option<usize> {
        self.executables
            .binary_search_by_key(&offset, |executable| executable.offset)
            .ok()
    }

    /// The stoppable statement that starts at `offset`.
    pub fn at(&self, offset: usize) -> Option<&Executable> {
        self.index_of(offset).map(|index| &self.executables[index])
    }

    pub fn executables(&self) -> &[Executable] {
        &self.executables
    }

    pub fn len(&self) -> usize {
        self.executables.len()
    }

    pub fn is_empty(&self) -> bool {
        self.executables.is_empty()
    }

    /// Where a breakpoint set on `line` (one-based) goes.
    pub fn resolve_line(&self, line: usize) -> LineResolution {
        if let Some(executable) = self
            .executables
            .iter()
            .find(|executable| executable.line == line)
        {
            return LineResolution::Executable {
                offset: executable.offset,
            };
        }
        let reason = self
            .declarations
            .iter()
            .find(|(declared, _)| *declared == line)
            .map_or(NotExecutable::NoCode, |(_, reason)| *reason);
        LineResolution::NotExecutable(reason)
    }

    /// The nearest line at or after `line` that has code, for "move the breakpoint to the next
    /// line" offers. `None` past the last statement.
    pub fn next_executable_line(&self, line: usize) -> Option<usize> {
        self.executables
            .iter()
            .map(|executable| executable.line)
            .filter(|candidate| *candidate >= line)
            .min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::compiler::{self, NoLibraries};
    use crate::pine::token::Limits;

    fn map(body: &str) -> SourceMap {
        let source = format!("//@version=6\nindicator(\"T\")\n{body}");
        let program = compiler::compile(&source, Limits::default(), &NoLibraries);
        assert!(program.is_valid(), "{:?}", program.diagnostics);
        SourceMap::build(&program.statements)
    }

    fn lines(map: &SourceMap) -> Vec<usize> {
        map.executables().iter().map(|e| e.line).collect()
    }

    fn offset_of(map: &SourceMap, line: usize) -> usize {
        match map.resolve_line(line) {
            LineResolution::Executable { offset } => offset,
            other => panic!("line {line}: {other:?}"),
        }
    }

    fn names_at(map: &SourceMap, line: usize) -> Vec<String> {
        map.visible(offset_of(map, line))
            .into_iter()
            .map(|entry| entry.name.clone())
            .collect()
    }

    #[test]
    fn statements_are_ordered_by_offset_and_carry_their_line() {
        let map = map("a = close\nb = a + 1\nplot(b)\n");
        assert_eq!(lines(&map), vec![3, 4, 5]);
        let offsets: Vec<usize> = map.executables().iter().map(|e| e.offset).collect();
        assert!(offsets.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn the_indicator_declaration_is_not_a_stop() {
        let map = map("plot(close)\n");
        assert_eq!(
            map.resolve_line(2),
            LineResolution::NotExecutable(NotExecutable::Declaration)
        );
    }

    #[test]
    fn function_bodies_are_stoppable_and_their_header_is_not() {
        let map = map("f(x) =>\n    y = x * 2\n    y + 1\nplot(f(close))\n");
        assert_eq!(
            map.resolve_line(3),
            LineResolution::NotExecutable(NotExecutable::FunctionHeader)
        );
        let inside = map.at(offset_of(&map, 4)).unwrap();
        assert_eq!(inside.function.as_deref(), Some("f"));
        let top = map.at(offset_of(&map, 6)).unwrap();
        assert_eq!(top.function, None);
    }

    #[test]
    fn blank_comment_and_continuation_lines_are_refused_explicitly() {
        let map = map("// note\n\nx = math.max(\n    close,\n    open)\nplot(x)\n");
        assert_eq!(
            map.resolve_line(3),
            LineResolution::NotExecutable(NotExecutable::NoCode)
        );
        assert_eq!(
            map.resolve_line(4),
            LineResolution::NotExecutable(NotExecutable::NoCode)
        );
        assert!(matches!(
            map.resolve_line(5),
            LineResolution::Executable { .. }
        ));
        // The continuation lines belong to the statement that started on line 5.
        assert_eq!(
            map.resolve_line(6),
            LineResolution::NotExecutable(NotExecutable::NoCode)
        );
        assert_eq!(
            map.resolve_line(99),
            LineResolution::NotExecutable(NotExecutable::NoCode)
        );
        assert_eq!(map.next_executable_line(3), Some(5));
        assert_eq!(map.next_executable_line(99), None);
    }

    #[test]
    fn block_bodies_and_value_blocks_are_stoppable() {
        let map = map(
            "x = 0.0\nif close > open\n    x := 1.0\nelse\n    x := 2.0\ny = if x > 1\n    10\nelse\n    20\nplot(y)\n",
        );
        let found = lines(&map);
        for line in [3, 4, 5, 7, 8, 9, 11] {
            assert!(found.contains(&line), "line {line} in {found:?}");
        }
    }

    #[test]
    fn type_declarations_are_refused_as_declarations() {
        let map = map("type Box\n    float top\nplot(close)\n");
        assert_eq!(
            map.resolve_line(3),
            LineResolution::NotExecutable(NotExecutable::Declaration)
        );
    }

    #[test]
    fn a_name_is_visible_after_its_declaration_and_not_before_or_in_its_own_value() {
        let map = map("a = close\nb = a + 1\nplot(b)\n");
        assert_eq!(names_at(&map, 3), Vec::<String>::new());
        assert_eq!(names_at(&map, 4), vec!["a"]);
        assert_eq!(names_at(&map, 5), vec!["b", "a"]);
    }

    #[test]
    fn a_blocks_names_end_with_the_block() {
        let map = map("x = 0.0\nif close > open\n    inner = 1.0\n    x := inner\nplot(x)\n");
        assert_eq!(names_at(&map, 5), vec!["x"], "inner is not yet declared");
        assert_eq!(names_at(&map, 6), vec!["inner", "x"]);
        assert_eq!(names_at(&map, 7), vec!["x"], "inner is gone after the if");
    }

    #[test]
    fn loop_counters_are_visible_in_the_loop_only() {
        let map = map("total = 0.0\nfor i = 0 to 2\n    total += i\nplot(total)\n");
        let in_loop = map.visible(offset_of(&map, 5));
        assert_eq!(in_loop[0].name, "i");
        assert_eq!(in_loop[0].kind, DeclarationKind::Loop);
        assert_eq!(names_at(&map, 6), vec!["total"]);
    }

    #[test]
    fn a_function_sees_its_parameters_its_locals_and_earlier_globals() {
        let map = map(
            "early = 1.0\nf(x) =>\n    local = x + early\n    local * 2\nlate = 2.0\nplot(f(close))\n",
        );
        let body = map.visible(offset_of(&map, 6));
        let names: Vec<&str> = body.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, vec!["local", "x", "early"]);
        assert_eq!(body[1].kind, DeclarationKind::Parameter);
        assert!(body[2].global && !body[0].global);
        assert!(!names.contains(&"late"), "declared after the function");
    }

    #[test]
    fn an_inner_name_hides_an_outer_one_of_the_same_name() {
        let map = map("x = 1.0\nf(x) =>\n    x * 2\nplot(f(close))\n");
        let body = map.visible(offset_of(&map, 5));
        assert_eq!(body.len(), 1);
        assert_eq!(body[0].kind, DeclarationKind::Parameter);
    }

    #[test]
    fn var_and_varip_are_persistent_and_tuples_declare_each_name() {
        let map = map("var a = 0.0\nvarip b = 0.0\n[m, n] = [close, open]\nplot(a + b + m + n)\n");
        let entries = map.visible(offset_of(&map, 6));
        let kinds: Vec<(&str, DeclarationKind)> = entries
            .iter()
            .map(|entry| (entry.name.as_str(), entry.kind))
            .collect();
        assert_eq!(
            kinds,
            vec![
                ("n", DeclarationKind::Tuple),
                ("m", DeclarationKind::Tuple),
                ("b", DeclarationKind::Varip),
                ("a", DeclarationKind::Var),
            ]
        );
        assert!(entries[2].kind.is_persistent() && entries[3].kind.is_persistent());
    }
}
