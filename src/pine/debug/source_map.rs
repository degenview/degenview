//! Which statements of a program can be stopped at, and where they sit in the source.
//!
//! The interpreter walks the syntax tree, so a stop is always "before this statement". A statement
//! is identified by the UTF-16 offset where the parser anchored it: ranges are start-anchored, a
//! call's range is its first token, and the language has no `;`, so no two statements start at the
//! same offset. A line is never an identity: `x = if c` / `    a` / `else` / `    b` puts one
//! statement on each of several lines, and a statement can span many.

use crate::pine::ast::{Expression, Statement};
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
    /// `type`, `enum`, `import` and the like.
    Declaration,
}

/// The stoppable statements of one program, ordered by offset.
#[derive(Clone, Debug, Default)]
pub struct SourceMap {
    executables: Vec<Executable>,
    /// Lines that declare something that never runs, to say why a breakpoint there is refused.
    declarations: Vec<(usize, NotExecutable)>,
}

impl SourceMap {
    pub fn build(statements: &[Statement]) -> Self {
        let mut map = Self::default();
        map.visit(statements, None);
        map.executables.sort_by_key(|executable| executable.offset);
        map.executables.dedup_by_key(|executable| executable.offset);
        map
    }

    fn visit(&mut self, statements: &[Statement], function: Option<&str>) {
        for statement in statements {
            self.visit_statement(statement, function);
        }
    }

    fn visit_statement(&mut self, statement: &Statement, function: Option<&str>) {
        let range = statement.range();
        match statement {
            Statement::Function { name, body, .. } => {
                self.declarations
                    .push((range.start.line, NotExecutable::FunctionHeader));
                self.visit(body, Some(name));
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
            offset: range.start.offset,
            line: range.start.line,
            column: range.start.column,
            function: function.map(str::to_owned),
        });
        for expression in traversal::expressions(statement) {
            self.visit_expression(expression, function);
        }
        for block in traversal::nested_blocks(statement) {
            self.visit(block, function);
        }
    }

    /// `x = if …` and `x = switch …` hold statements inside an expression.
    fn visit_expression(&mut self, expression: &Expression, function: Option<&str>) {
        if let Expression::Statement(inner, _) = expression {
            self.visit_statement(inner, function);
        }
        for child in traversal::children(expression) {
            self.visit_expression(child, function);
        }
    }

    /// The stoppable statement that starts at `offset`.
    pub fn at(&self, offset: usize) -> Option<&Executable> {
        self.executables
            .binary_search_by_key(&offset, |executable| executable.offset)
            .ok()
            .map(|index| &self.executables[index])
    }

    pub fn executables(&self) -> &[Executable] {
        &self.executables
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

    #[test]
    fn statements_are_ordered_by_offset_and_carry_their_line() {
        let map = map("a = close\nb = a + 1\nplot(b)\n");
        assert!(lines(&map).ends_with(&[3, 4, 5]));
        let offsets: Vec<usize> = map.executables().iter().map(|e| e.offset).collect();
        assert!(offsets.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn function_bodies_are_stoppable_and_their_header_is_not() {
        let map = map("f(x) =>\n    y = x * 2\n    y + 1\nplot(f(close))\n");
        assert_eq!(
            map.resolve_line(3),
            LineResolution::NotExecutable(NotExecutable::FunctionHeader)
        );
        let LineResolution::Executable { offset } = map.resolve_line(4) else {
            panic!("the body's first line holds code");
        };
        assert_eq!(
            map.at(offset).and_then(|e| e.function.as_deref()),
            Some("f")
        );
        let top = map.resolve_line(6);
        let LineResolution::Executable { offset } = top else {
            panic!("plot holds code");
        };
        assert_eq!(map.at(offset).and_then(|e| e.function.as_deref()), None);
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
}
