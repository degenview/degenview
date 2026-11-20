//! Name resolution over the syntax tree: what every variable occurrence refers to.
//!
//! The compiler resolves *calls* (`Expression::Call::target`); it never resolved *variables*, so each
//! static analysis that needs to know whether `level` is the script's global, a block's own variable
//! or a function's parameter re-derived Pine's scoping by hand. [`resolve`] does it once, over the
//! raw tree, and is the single authority for it: it has no dependency on the compiler or the
//! runtime, only on the syntax tree.
//!
//! # Scoping rules
//!
//! These are the rules the runtime's `Session::invoke_with` and the `request.security` slicer
//! implement.
//!
//! - A top-level declaration (`x = …`, `[a, b] = …`) is a **global**. Pine forbids using a name
//!   before it is declared, so every read of a top-level name is the global, wherever it sits
//!   (also in a function written above the declaration).
//! - A declaration in a block is a **local**, visible from the declaration to the end of its block
//!   (a branch, an arm, a loop body, a function body). Its own value expression is resolved before
//!   it, so `x = x + 1` in a block reads the outer `x`. A loop variable belongs to the loop body,
//!   not to the bounds or the collection.
//! - `x := …` and the compound operators bind the nearest visible `x`, which may be a local, a
//!   parameter or the global.
//! - A function's parameters and body declarations belong to the function. A function sees the
//!   globals and never its caller's locals (functions are top-level, so this is plain lexical
//!   scope). Methods are functions.
//! - The statements inside an expression (`x = if …`, `x = switch …`) are blocks too.
//! - A name that none of these declares is **free**: a builtin (`close`, `ta`, the head of
//!   `syminfo.tickerid`) or an undefined name.
//!
//! A dotted name (`zone.top`, `array.push`) resolves by its head, the part before the first dot.
//!
//! # Keys
//!
//! An occurrence is found by where it sits in the source, which survives cloning the tree (a
//! slice's statements are clones). Expressions are keyed by the start and end offset of their range,
//! the way `request.security` expressions already are: an `Identifier` or `Call` range is its name
//! token(s), so two of them never share one. Names a statement declares or assigns are keyed by the
//! statement's start offset and the name's slot in it: a [`Declaration`](Statement::Declaration)
//! or [`Assignment`](Statement::Assignment) is ranged at its name, a tuple at its `[`, a `for` at
//! its keyword, so statements never share a start either. Syntax with no real position (every
//! range zero) would collide; the parser never produces it for a valid script.

use std::collections::{HashMap, HashSet};

use super::ast::{DeclarationMode, Expression, ShadowId, Statement};
use super::traversal;
use crate::pine::SourceRange;

/// A dense id, one per declaration site (a variable, a tuple element or a loop variable), in the
/// order the sites are resolved. Locals of different blocks never share one, even when named alike.
pub type LocalId = usize;

/// What a name refers to.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Binding<'a> {
    /// A top-level declaration of the script.
    Global(&'a str),
    /// A declaration in a block or a function body, or a loop variable.
    Local(LocalId),
    /// The `position`th parameter of the `function`th top-level function, counting functions and
    /// methods together in source order: the id `Expression::Call::target` uses.
    Parameter { function: usize, position: usize },
    /// Not declared by the script: a builtin, or an undefined name.
    Free(&'a str),
}

impl<'a> Binding<'a> {
    /// The name of a global or a free name: both are keyed by their name in the runtime's variable
    /// table.
    pub fn name(self) -> Option<&'a str> {
        match self {
            Self::Global(name) | Self::Free(name) => Some(name),
            Self::Local(_) | Self::Parameter { .. } => None,
        }
    }
}

/// One declaration site of a local.
#[derive(Clone, Copy, Debug)]
pub struct LocalInfo<'a> {
    pub name: &'a str,
    /// Where it is declared: the source offset of the declared name (a tuple's `[`, a loop's
    /// keyword). Everything a local is visible to sits after it.
    pub site: usize,
}

type Span = (usize, usize);

fn span(range: SourceRange) -> Span {
    (range.start.offset, range.end.offset)
}

/// Where a statement declares or assigns its `slot`th name.
type Slot = (usize, usize);

/// What every variable occurrence of a program refers to. See the module documentation.
#[derive(Debug, Default)]
pub struct Bindings<'a> {
    /// `Identifier`s and `Call`s, by the span of their range: the binding of the name's head.
    uses: HashMap<Span, Binding<'a>>,
    /// The names declared, assigned or introduced by statements, by [`Slot`].
    names: HashMap<Slot, Binding<'a>>,
    locals: Vec<LocalInfo<'a>>,
    globals: HashSet<&'a str>,
    /// The declaration slots of a block and the counters of a loop that hide a name visible just
    /// before them: a global, a local, a parameter, or a builtin variable (see [`resolve_with`]).
    /// The first declaration of a name, a function body's own declarations and a `var` hide
    /// nothing.
    shadowing: HashSet<Slot>,
}

impl<'a> Bindings<'a> {
    /// The names declared at the top level.
    pub fn globals(&self) -> &HashSet<&'a str> {
        &self.globals
    }

    /// How many locals the program declares: [`LocalId`]s are `0..local_count()`.
    pub fn local_count(&self) -> usize {
        self.locals.len()
    }

    pub fn local(&self, id: LocalId) -> &LocalInfo<'a> {
        &self.locals[id]
    }

    /// The binding of the name an `Identifier` or a `Call` starts with (`a` for `a.b.c`, and for
    /// the call `a.b()`).
    pub fn occurrence(&self, expression: &Expression) -> Option<Binding<'a>> {
        match expression {
            Expression::Identifier(_, range) | Expression::Call { range, .. } => {
                self.uses.get(&span(*range)).copied()
            }
            _ => None,
        }
    }

    /// The variable at the start of `a.b[1].c`, `f().x` or `a.method().y`: the binding of `a`, or
    /// of the callee `f` of a call whose name is not dotted.
    pub fn root(&self, expression: &Expression) -> Option<Binding<'a>> {
        match expression {
            Expression::Identifier(..) | Expression::Call { .. } => self.occurrence(expression),
            Expression::Member { base, .. } | Expression::History { base, .. } => self.root(base),
            Expression::MethodCall { receiver, .. } => self.root(receiver),
            _ => None,
        }
    }

    /// What a [`Declaration`](Statement::Declaration) declares: a global at the top level, a local
    /// in a block.
    pub fn declaration(&self, statement: &Statement) -> Option<Binding<'a>> {
        match statement {
            Statement::Declaration { range, .. } => {
                self.names.get(&(range.start.offset, 0)).copied()
            }
            _ => None,
        }
    }

    /// What an [`Assignment`](Statement::Assignment) assigns: the nearest visible name.
    pub fn target(&self, statement: &Statement) -> Option<Binding<'a>> {
        match statement {
            Statement::Assignment { range, .. } => {
                self.names.get(&(range.start.offset, 0)).copied()
            }
            _ => None,
        }
    }

    /// What `statement` itself introduces: a declaration's name, a tuple's names, a `for` loop's
    /// variables (which belong to its body). Nothing for any other statement.
    pub fn introduced<'s>(
        &'s self,
        statement: &'s Statement,
    ) -> impl Iterator<Item = Binding<'a>> + 's {
        let (start, count) = match statement {
            Statement::Declaration { range, .. } | Statement::ForRange { range, .. } => {
                (range.start.offset, 1)
            }
            Statement::TupleDeclaration { names, range, .. } => (range.start.offset, names.len()),
            Statement::ForIn { range, .. } => (range.start.offset, 2),
            _ => (0, 0),
        };
        (0..count).filter_map(move |slot| self.names.get(&(start, slot)).copied())
    }
}

/// The names `statements` declare at their own level: a variable and the elements of a tuple
/// declaration. A block's declarations are its own and are not included.
pub fn top_level_declarations(statements: &[Statement]) -> HashSet<String> {
    declared_names(statements)
        .into_iter()
        .map(str::to_owned)
        .collect()
}

fn declared_names(statements: &[Statement]) -> HashSet<&str> {
    let mut names = HashSet::new();
    for statement in statements {
        match statement {
            Statement::Declaration { name, .. } => {
                names.insert(name.as_str());
            }
            Statement::TupleDeclaration { names: tuple, .. } => {
                names.extend(tuple.iter().map(String::as_str));
            }
            _ => {}
        }
    }
    names
}

/// The variable a dotted name starts with.
pub fn head(name: &str) -> &str {
    name.split_once('.').map_or(name, |(head, _)| head)
}

/// Resolves every variable occurrence in the program `statements`.
pub fn resolve(statements: &[Statement]) -> Bindings<'_> {
    resolve_with(statements, &|_| false)
}

/// [`resolve`], where `is_builtin` says which free names are builtin variables (`low`, `time`): a
/// block's declaration of one hides it ([`Bindings::shadows`]).
pub fn resolve_with<'a>(
    statements: &'a [Statement],
    is_builtin: &dyn Fn(&str) -> bool,
) -> Bindings<'a> {
    let mut resolver = Resolver {
        bindings: Bindings {
            globals: declared_names(statements),
            ..Bindings::default()
        },
        visible: HashMap::new(),
        trail: Vec::new(),
        functions: 0,
        is_builtin,
    };
    for statement in statements {
        resolver.statement(statement, Level::Top);
    }
    resolver.bindings
}

/// The names a block's declaration may hide, numbered in the order they were first met: what the
/// `shadows` of a declaration index.
#[derive(Default)]
pub struct ShadowNames {
    names: Vec<String>,
    ids: HashMap<String, ShadowId>,
}

impl ShadowNames {
    fn id(&mut self, name: &str) -> ShadowId {
        if let Some(id) = self.ids.get(name) {
            return *id;
        }
        let id = ShadowId::try_from(self.names.len()).unwrap_or(ShadowId::MAX);
        self.names.push(name.to_owned());
        self.ids.insert(name.to_owned(), id);
        id
    }

    pub fn into_names(self) -> Vec<String> {
        self.names
    }
}

/// Sets `shadows` on every declaration of `statements` that [`Bindings::shadows`] says hides a
/// name, numbering the names in `names`. `statements` is one program: the script or a library.
pub fn mark_shadowing(
    statements: &mut [Statement],
    is_builtin: &dyn Fn(&str) -> bool,
    names: &mut ShadowNames,
) {
    let sites = resolve_with(statements, is_builtin).shadowing;
    if sites.is_empty() {
        return;
    }
    traversal::for_each_statement_mut(statements, &mut |statement| match statement {
        Statement::Declaration {
            name,
            range,
            shadows,
            ..
        } => {
            if sites.contains(&(range.start.offset, 0)) {
                *shadows = Some(names.id(name));
            }
        }
        Statement::TupleDeclaration {
            names: tuple,
            range,
            shadows,
            ..
        } => {
            let site = range.start.offset;
            if (0..tuple.len()).any(|slot| sites.contains(&(site, slot))) {
                *shadows = tuple
                    .iter()
                    .enumerate()
                    .map(|(slot, name)| sites.contains(&(site, slot)).then(|| names.id(name)))
                    .collect();
            }
        }
        Statement::ForRange {
            variable,
            range,
            shadows,
            ..
        } => {
            if sites.contains(&(range.start.offset, 0)) {
                *shadows = Some(names.id(variable));
            }
        }
        Statement::ForIn {
            index,
            value,
            range,
            shadows,
            ..
        } => {
            let site = range.start.offset;
            if sites.contains(&(site, 0))
                && let Some(index) = index
            {
                shadows[0] = Some(names.id(index));
            }
            if sites.contains(&(site, 1)) {
                shadows[1] = Some(names.id(value));
            }
        }
        _ => {}
    });
}

/// Where a statement sits.
#[derive(Clone, Copy, PartialEq)]
enum Level {
    /// One of the program's own: its declarations are the globals.
    Top,
    /// Directly in a function's body: its declarations belong to the call.
    Function,
    /// In a branch, an arm or a loop body.
    Block,
}

struct Resolver<'a, 'f> {
    bindings: Bindings<'a>,
    is_builtin: &'f dyn Fn(&str) -> bool,
    /// The names the blocks around the point being resolved have declared so far (and the
    /// function's parameters): they hide the globals.
    visible: HashMap<&'a str, Binding<'a>>,
    /// What each entry of `visible` replaced, to put it back when its block ends.
    trail: Vec<(&'a str, Option<Binding<'a>>)>,
    /// How many top-level functions were seen.
    functions: usize,
}

impl<'a> Resolver<'a, '_> {
    fn lookup(&self, name: &'a str) -> Binding<'a> {
        if let Some(binding) = self.visible.get(name) {
            return *binding;
        }
        match self.bindings.globals.get(name) {
            Some(global) => Binding::Global(global),
            None => Binding::Free(name),
        }
    }

    fn show(&mut self, name: &'a str, binding: Binding<'a>) {
        let previous = self.visible.insert(name, binding);
        self.trail.push((name, previous));
    }

    /// Declares a block's `slot`th name of the statement at `site` (a loop's counters are declared
    /// the same way, once the bounds or the collection are resolved), noting whether it hides a
    /// name that is visible now.
    fn declare_in_block(&mut self, name: &'a str, site: usize, slot: usize) -> Binding<'a> {
        let hides = match self.lookup(name) {
            Binding::Free(free) => (self.is_builtin)(free),
            _ => true,
        };
        if hides {
            self.bindings.shadowing.insert((site, slot));
        }
        self.declare(name, site)
    }

    fn declare(&mut self, name: &'a str, site: usize) -> Binding<'a> {
        let binding = Binding::Local(self.bindings.locals.len());
        self.bindings.locals.push(LocalInfo { name, site });
        self.show(name, binding);
        binding
    }

    /// Records the `slot`th name of the statement starting at `start`.
    fn name(&mut self, start: usize, slot: usize, binding: Binding<'a>) {
        self.bindings.names.insert((start, slot), binding);
    }

    fn enter(&self) -> usize {
        self.trail.len()
    }

    fn leave(&mut self, mark: usize) {
        while self.trail.len() > mark {
            if let Some((name, previous)) = self.trail.pop() {
                match previous {
                    Some(binding) => self.visible.insert(name, binding),
                    None => self.visible.remove(name),
                };
            }
        }
    }

    /// A block: what it declares is gone when it ends.
    fn block(&mut self, statements: &'a [Statement]) {
        let mark = self.enter();
        for statement in statements {
            self.statement(statement, Level::Block);
        }
        self.leave(mark);
    }

    /// Declares the `slot`th name of a declaration at `site`, by where the statement sits.
    fn declare_at(&mut self, name: &'a str, site: usize, slot: usize, level: Level) -> Binding<'a> {
        match level {
            Level::Top => self.lookup(name),
            Level::Function => self.declare(name, site),
            Level::Block => self.declare_in_block(name, site, slot),
        }
    }

    fn statement(&mut self, statement: &'a Statement, level: Level) {
        match statement {
            Statement::Declaration {
                name,
                value,
                range,
                mode,
                ..
            } => {
                self.expression(value);
                let site = range.start.offset;
                // `var` and `varip` keep their value: they are not saved around a block.
                let level = if *mode == DeclarationMode::Ordinary || level != Level::Block {
                    level
                } else {
                    Level::Function
                };
                let binding = self.declare_at(name, site, 0, level);
                self.name(site, 0, binding);
            }
            Statement::TupleDeclaration {
                names,
                value,
                range,
                ..
            } => {
                self.expression(value);
                let site = range.start.offset;
                for (slot, name) in names.iter().enumerate() {
                    let binding = self.declare_at(name, site, slot, level);
                    self.name(site, slot, binding);
                }
            }
            Statement::Assignment {
                name, value, range, ..
            } => {
                self.expression(value);
                let binding = self.lookup(name);
                self.name(range.start.offset, 0, binding);
            }
            Statement::ForRange {
                variable,
                from,
                to,
                step,
                body,
                range,
                ..
            } => {
                self.expression(from);
                self.expression(to);
                if let Some(step) = step {
                    self.expression(step);
                }
                let mark = self.enter();
                let site = range.start.offset;
                let binding = self.declare_in_block(variable, site, 0);
                self.name(site, 0, binding);
                for statement in body {
                    self.statement(statement, Level::Block);
                }
                self.leave(mark);
            }
            Statement::ForIn {
                index,
                value,
                collection,
                body,
                range,
                ..
            } => {
                self.expression(collection);
                let mark = self.enter();
                let site = range.start.offset;
                if let Some(index) = index {
                    let binding = self.declare_in_block(index, site, 0);
                    self.name(site, 0, binding);
                }
                let binding = self.declare_in_block(value, site, 1);
                self.name(site, 1, binding);
                for statement in body {
                    self.statement(statement, Level::Block);
                }
                self.leave(mark);
            }
            Statement::Function {
                parameters, body, ..
            } => {
                let index = (level == Level::Top).then(|| {
                    self.functions += 1;
                    self.functions - 1
                });
                // Defaults are constants: only the globals are in sight.
                for parameter in parameters {
                    if let Some(default) = &parameter.default_value {
                        self.expression(default);
                    }
                }
                let mark = self.enter();
                for (position, parameter) in parameters.iter().enumerate() {
                    let name = parameter.name.as_str();
                    match index {
                        Some(function) => {
                            self.show(name, Binding::Parameter { function, position });
                        }
                        None => {
                            self.declare(name, statement.range().start.offset);
                        }
                    }
                }
                for statement in body {
                    self.statement(statement, Level::Function);
                }
                self.leave(mark);
            }
            _ => {
                for expression in traversal::expressions(statement) {
                    self.expression(expression);
                }
                for block in traversal::nested_blocks(statement) {
                    self.block(block);
                }
            }
        }
    }

    fn expression(&mut self, expression: &'a Expression) {
        match expression {
            Expression::Identifier(name, range) | Expression::Call { name, range, .. } => {
                let binding = self.lookup(head(name));
                self.bindings.uses.insert(span(*range), binding);
            }
            Expression::Statement(statement, _) => self.statement(statement, Level::Block),
            _ => {}
        }
        for child in traversal::children(expression) {
            self.expression(child);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::{Lexer, Limits, Parser};

    fn parse(source: &str) -> Vec<Statement> {
        let lexed = Lexer::new(source).lex();
        assert!(lexed.diagnostics.is_empty(), "{:?}", lexed.diagnostics);
        let mut parser = Parser::new(lexed.tokens, Limits::default(), HashSet::new());
        let (statements, diagnostics) = parser.parse();
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        statements
    }

    /// The binding of every plain or dotted identifier read, in source order, as `name: binding`.
    fn reads<'a>(
        statements: &'a [Statement],
        bindings: &Bindings<'a>,
    ) -> Vec<(String, Binding<'a>)> {
        let mut found = Vec::new();
        traversal::for_each_identifier(statements, &mut |name, range| {
            found.push((name.to_owned(), bindings.uses[&span(range)]));
        });
        found
    }

    /// The bindings of the reads of `name`, in source order.
    fn of<'a>(found: &[(String, Binding<'a>)], name: &str) -> Vec<Binding<'a>> {
        found
            .iter()
            .filter(|(read, _)| read == name)
            .map(|(_, binding)| *binding)
            .collect()
    }

    fn global(name: &'static str) -> Binding<'static> {
        Binding::Global(name)
    }

    fn free(name: &'static str) -> Binding<'static> {
        Binding::Free(name)
    }

    fn is_local(binding: Binding<'_>) -> bool {
        matches!(binding, Binding::Local(_))
    }

    #[test]
    fn a_top_level_declaration_is_a_global_wherever_it_is_read() {
        let statements = parse("f() =>\n    x * 2\nx = close\ny = x + z\n");
        let bindings = resolve(&statements);
        let found = reads(&statements, &bindings);
        // Also in the function above the declaration; `close` and the undeclared `z` are free.
        assert_eq!(of(&found, "x"), [global("x"), global("x")]);
        assert_eq!(of(&found, "close"), [free("close")]);
        assert_eq!(of(&found, "z"), [free("z")]);
        let expected: HashSet<&str> = ["x", "y"].into();
        assert_eq!(bindings.globals(), &expected);
        assert_eq!(
            bindings.declaration(&statements[1]),
            Some(Binding::Global("x"))
        );
    }

    #[test]
    fn tuple_elements_are_globals_at_the_top_level() {
        let statements = parse("[a, b] = f(close)\nc = a + b\n");
        let bindings = resolve(&statements);
        let found = reads(&statements, &bindings);
        assert_eq!(of(&found, "a"), [global("a")]);
        assert_eq!(
            bindings.introduced(&statements[0]).collect::<Vec<_>>(),
            [global("a"), global("b")]
        );
    }

    #[test]
    fn a_block_declaration_is_visible_to_the_end_of_its_block_only() {
        let statements = parse("if close > 0\n    level = 1.0\n    y = level\nz = level\n");
        let bindings = resolve(&statements);
        let found = reads(&statements, &bindings);
        let levels = of(&found, "level");
        assert!(is_local(levels[0]), "{levels:?}");
        assert_eq!(
            levels[1],
            free("level"),
            "after the block it is not declared"
        );
        let Statement::Conditional { when_true, .. } = &statements[0] else {
            panic!("an if");
        };
        assert_eq!(
            bindings.introduced(&when_true[0]).collect::<Vec<_>>(),
            [levels[0]]
        );
        let Binding::Local(id) = levels[0] else {
            panic!("a local");
        };
        assert_eq!(bindings.local(id).name, "level");
    }

    #[test]
    fn a_name_shadowed_in_a_nested_block_is_the_outer_one_after_it() {
        let statements = parse(
            "level = 10.0\nif close > 0\n    level = 20.0\n    if close > 1\n        a = level\n    b = level\nc = level\n",
        );
        let bindings = resolve(&statements);
        let found = reads(&statements, &bindings);
        let levels = of(&found, "level");
        assert_eq!(levels.len(), 3);
        assert!(is_local(levels[0]) && levels[0] == levels[1], "{levels:?}");
        assert_eq!(
            levels[2],
            global("level"),
            "the block's `level` ended with it"
        );
    }

    #[test]
    fn a_declaration_reads_the_outer_name_in_its_own_value() {
        let statements = parse("x = 1.0\nif close > 0\n    x = x + 1\n    y = x\n");
        let bindings = resolve(&statements);
        let found = reads(&statements, &bindings);
        let xs = of(&found, "x");
        assert_eq!(
            xs[0],
            global("x"),
            "the value is resolved before the declaration"
        );
        assert!(is_local(xs[1]), "{xs:?}");
    }

    #[test]
    fn the_branches_of_an_if_do_not_see_each_other() {
        let statements = parse("if close > 0\n    a = 1\nelse\n    b = a\n");
        let bindings = resolve(&statements);
        let found = reads(&statements, &bindings);
        assert_eq!(of(&found, "a"), [free("a")]);
    }

    #[test]
    fn a_loop_variable_belongs_to_the_loop_body() {
        let statements = parse(
            "i = 5\nfor i = 0 to i\n    a = i\nb = i\nfor [k, v] in arr\n    c = k + v\nd = k\n",
        );
        let bindings = resolve(&statements);
        let found = reads(&statements, &bindings);
        let is = of(&found, "i");
        assert_eq!(
            is[0],
            global("i"),
            "the bound is evaluated outside the loop"
        );
        assert!(is_local(is[1]), "{is:?}");
        assert_eq!(is[2], global("i"));
        let loop_variables: Vec<_> = bindings.introduced(&statements[1]).collect();
        assert_eq!(loop_variables, [is[1]]);
        let ks = of(&found, "k");
        assert!(is_local(ks[0]));
        assert_eq!(ks[1], free("k"));
        assert_eq!(bindings.introduced(&statements[3]).count(), 2);
        assert_eq!(bindings.local(0).name, "i");
        // The collection is read outside the loop's variables.
        assert_eq!(of(&found, "arr"), [free("arr")]);
    }

    #[test]
    fn a_value_loop_has_only_its_value_variable() {
        let statements = parse("for v in arr\n    a = v\n");
        let bindings = resolve(&statements);
        let found = reads(&statements, &bindings);
        assert!(is_local(of(&found, "v")[0]));
        assert_eq!(bindings.introduced(&statements[0]).count(), 1);
    }

    #[test]
    fn an_assignment_binds_the_nearest_visible_name() {
        let statements = parse(
            "level = 1.0\nlevel := 2.0\nif close > 0\n    level := 3.0\n    level = 4.0\n    level += 1\n    ghost := 1\n",
        );
        let bindings = resolve(&statements);
        let top = bindings.target(&statements[1]).unwrap();
        assert_eq!(top, global("level"));
        let Statement::Conditional { when_true, .. } = &statements[2] else {
            panic!("an if");
        };
        // Before the block declares its own `level` the assignment is the global's; after, the local's.
        assert_eq!(bindings.target(&when_true[0]), Some(global("level")));
        let declared = bindings.declaration(&when_true[1]).unwrap();
        assert!(is_local(declared));
        assert_eq!(bindings.target(&when_true[2]), Some(declared));
        assert_eq!(bindings.target(&when_true[3]), Some(free("ghost")));
    }

    #[test]
    fn a_parameter_is_the_parameter_of_its_function_by_position() {
        let statements = parse(
            "level = 1.0\nf(a, b) =>\n    a + b + level\nmethod g(x) =>\n    x\nh(a) =>\n    a\n",
        );
        let bindings = resolve(&statements);
        let found = reads(&statements, &bindings);
        let parameter = |function, position| Binding::Parameter { function, position };
        assert_eq!(of(&found, "a"), [parameter(0, 0), parameter(2, 0)]);
        assert_eq!(of(&found, "b"), [parameter(0, 1)]);
        // Methods count with the functions, in source order.
        assert_eq!(of(&found, "x"), [parameter(1, 0)]);
        assert_eq!(of(&found, "level"), [global("level")]);
    }

    #[test]
    fn a_function_body_has_its_own_locals_and_never_the_callers() {
        let statements = parse(
            "up = close > open\nf() =>\n    up = close < open\n    up\ng() =>\n    if close > 0\n        w = 1\n    w\nif close > 0\n    w = 2\n    q = w\n    f()\n    g()\n",
        );
        let bindings = resolve(&statements);
        let found = reads(&statements, &bindings);
        let ups = of(&found, "up");
        assert!(is_local(ups[0]), "{ups:?}");
        let ws = of(&found, "w");
        // `g` reads `w` after the `if` that declared it: not the block's, and not the caller's.
        assert_eq!(ws[0], free("w"));
        assert_eq!(bindings.local(0).name, "up");
        assert!(is_local(ws[1]), "the caller's own `w`: {ws:?}");
    }

    #[test]
    fn a_function_local_hides_a_global_only_from_its_declaration() {
        let statements = parse("v = 1.0\nf() =>\n    a = v\n    v = 2.0\n    v\n");
        let bindings = resolve(&statements);
        let found = reads(&statements, &bindings);
        let vs = of(&found, "v");
        assert_eq!(vs[0], global("v"));
        assert!(is_local(vs[1]), "{vs:?}");
    }

    #[test]
    fn the_statements_inside_an_expression_are_blocks() {
        let statements = parse(
            "level = close\nx = if close > 0\n    level = 20.0\n    level\nelse\n    0.0\ny = level\n",
        );
        let bindings = resolve(&statements);
        let found = reads(&statements, &bindings);
        let levels = of(&found, "level");
        assert!(is_local(levels[0]), "{levels:?}");
        assert_eq!(levels[1], global("level"));
        let switched = parse("z = switch close\n    1 => q = 1\n    => q\nq2 = q\n");
        let bindings = resolve(&switched);
        let found = reads(&switched, &bindings);
        let qs = of(&found, "q");
        assert_eq!(
            qs,
            [free("q"), free("q")],
            "an arm does not see another's locals"
        );
    }

    #[test]
    fn a_dotted_name_resolves_by_its_head() {
        let statements = parse(
            "zone = 1\nif close > 0\n    zone = 2\n    a = zone.top\nb = zone.top\nc = syminfo.tickerid\n",
        );
        let bindings = resolve(&statements);
        let found = reads(&statements, &bindings);
        let zones = of(&found, "zone.top");
        assert!(is_local(zones[0]));
        assert_eq!(zones[1], global("zone"));
        assert_eq!(of(&found, "syminfo.tickerid"), [free("syminfo")]);
    }

    #[test]
    fn the_call_of_a_method_on_a_variable_resolves_the_variable() {
        let statements = parse(
            "samples = array.new_float()\nif close > 0\n    samples = array.new_float()\n    samples.push(1.0)\nsamples.push(2.0)\n",
        );
        let bindings = resolve(&statements);
        let mut calls = Vec::new();
        traversal::for_each_call(&statements, &mut |name, range| {
            if name == "samples.push" {
                calls.push(bindings.uses[&span(range)]);
            }
        });
        assert!(is_local(calls[0]), "{calls:?}");
        assert_eq!(calls[1], global("samples"));
    }

    #[test]
    fn the_root_of_a_path_is_its_first_variable() {
        let statements =
            parse("zones = array.new<Zone>()\nx = zones.get(0).top\nh = zones.get(0).top[1]\n");
        let bindings = resolve(&statements);
        for statement in &statements[1..] {
            let Statement::Declaration { value, .. } = statement else {
                panic!("a declaration");
            };
            assert_eq!(bindings.root(value), Some(global("zones")), "{value:?}");
        }
        let literal = parse("a = 1 + 2\n");
        let bindings = resolve(&literal);
        let Statement::Declaration { value, .. } = &literal[0] else {
            panic!("a declaration");
        };
        assert_eq!(bindings.root(value), None);
    }

    #[test]
    fn locals_know_their_declaration() {
        let statements = parse("if close > 0\n    var n = 0\n    [p, q] = f()\n");
        let bindings = resolve(&statements);
        assert_eq!(bindings.local_count(), 3);
        let n = bindings.local(0);
        assert_eq!(n.name, "n");
        assert_eq!(bindings.local(1).name, "p");
        assert_eq!(bindings.local(2).name, "q");
        assert_eq!(bindings.local(1).site, bindings.local(2).site, "one tuple");
        assert!(n.site < bindings.local(1).site);
    }

    #[test]
    fn top_level_declarations_are_the_variables_and_tuple_elements() {
        let statements = parse("a = 1\n[b, c] = f()\nif true\n    d = 1\nf() =>\n    e = 1\n");
        let names = top_level_declarations(&statements);
        let expected: HashSet<String> = ["a", "b", "c"].map(str::to_owned).into();
        assert_eq!(names, expected);
    }

    /// The declarations of `source` that hide a name, as `name` (or `[a, b]` for a tuple's flags).
    fn shadowing(source: &str) -> Vec<String> {
        let mut statements = parse(source);
        let mut names = ShadowNames::default();
        mark_shadowing(&mut statements, &|name| name == "low", &mut names);
        let mut found = Vec::new();
        traversal::for_each_statement_mut(&mut statements, &mut |statement| match statement {
            Statement::Declaration {
                name,
                shadows: Some(_),
                ..
            } => found.push(name.clone()),
            Statement::TupleDeclaration { names, shadows, .. } if !shadows.is_empty() => {
                let hidden: Vec<&str> = names
                    .iter()
                    .zip(shadows.iter())
                    .filter_map(|(name, shadow)| shadow.map(|_| name.as_str()))
                    .collect();
                found.push(format!("[{}]", hidden.join(", ")));
            }
            _ => {}
        });
        found
    }

    #[test]
    fn a_block_declaration_hides_what_is_visible_before_it() {
        // Not the first declaration of a name, nor a top-level one.
        assert_eq!(shadowing("x = 1\nif true\n    x = 2\n    y = 3\n"), ["x"]);
        // A local, a loop variable's name and a parameter are visible too.
        assert_eq!(
            shadowing("if true\n    x = 1\n    if true\n        x = 2\n"),
            ["x"]
        );
        assert_eq!(
            shadowing("f(p) =>\n    for i = 0 to 1\n        p = i\n    p\n"),
            ["p"]
        );
        // The name is a global of the script even above its declaration.
        assert_eq!(shadowing("if true\n    x = 2\nx = 1\n"), ["x"]);
    }

    #[test]
    fn what_hides_nothing_is_not_flagged() {
        // A name no block around has, a function body's own declarations, and a `var`.
        assert_eq!(
            shadowing("if true\n    x = 2\nif true\n    x = 3\n"),
            Vec::<String>::new()
        );
        assert_eq!(
            shadowing("x = 1\nf() =>\n    x = 2\n    x\n"),
            Vec::<String>::new()
        );
        assert_eq!(
            shadowing("x = 1\nif true\n    var x = 2\n"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_builtin_variable_is_hidden_by_a_block_declaration_of_its_name() {
        assert_eq!(shadowing("if true\n    low = 1\n    foo = 2\n"), ["low"]);
    }

    /// The loops of `source` whose counters hide a name, as `for i` or `for [i, v]` with only the
    /// flagged counters named.
    fn hidden_counters(source: &str) -> Vec<String> {
        let mut statements = parse(source);
        let mut names = ShadowNames::default();
        mark_shadowing(&mut statements, &|name| name == "low", &mut names);
        let mut found = Vec::new();
        traversal::for_each_statement_mut(&mut statements, &mut |statement| match statement {
            Statement::ForRange {
                variable,
                shadows: Some(_),
                ..
            } => found.push(format!("for {variable}")),
            Statement::ForIn {
                index,
                value,
                shadows,
                ..
            } if shadows.iter().any(Option::is_some) => {
                let hidden: Vec<&str> = [index.as_deref(), Some(value.as_str())]
                    .into_iter()
                    .zip(shadows)
                    .filter_map(|(name, shadow)| shadow.and(name))
                    .collect();
                found.push(format!("for [{}]", hidden.join(", ")));
            }
            _ => {}
        });
        found
    }

    #[test]
    fn a_loop_counter_hides_what_is_visible_at_the_loop() {
        // A global, also one declared below the loop.
        assert_eq!(
            hidden_counters("i = 5\nfor i = 0 to 2\n    a = i\n"),
            ["for i"]
        );
        assert_eq!(
            hidden_counters("for i = 0 to 2\n    a = i\ni = 5\n"),
            ["for i"]
        );
        // A local of an enclosing block, and the counter of an enclosing loop.
        assert_eq!(
            hidden_counters("if true\n    n = 1\n    for n = 0 to 2\n        a = n\n"),
            ["for n"]
        );
        assert_eq!(
            hidden_counters("for j = 0 to 2\n    for j = 0 to 1\n        a = j\n"),
            ["for j"]
        );
        // A parameter, and a global seen from a function body.
        assert_eq!(
            hidden_counters("f(p) =>\n    for p = 0 to 1\n        a = p\n    1\n"),
            ["for p"]
        );
        assert_eq!(
            hidden_counters("g = 1\nf() =>\n    for g = 0 to 1\n        a = g\n    1\n"),
            ["for g"]
        );
        // A builtin variable.
        assert_eq!(
            hidden_counters("for low = 0 to 2\n    a = low\n"),
            ["for low"]
        );
    }

    #[test]
    fn a_loop_counter_that_hides_nothing_is_not_flagged() {
        assert_eq!(
            hidden_counters("for i = 0 to 2\n    a = i\nfor i = 0 to 2\n    b = i\n"),
            Vec::<String>::new()
        );
        assert_eq!(
            hidden_counters("f() =>\n    for i = 0 to 2\n        a = i\n    1\n"),
            Vec::<String>::new()
        );
        assert_eq!(
            hidden_counters("for [i, v] in arr\n    a = i + v\n"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_for_in_flags_its_index_and_value_apart() {
        assert_eq!(
            hidden_counters("i = 1\nv = 2\nfor [i, v] in arr\n    a = i + v\n"),
            ["for [i, v]"]
        );
        assert_eq!(
            hidden_counters("v = 2\nfor [i, v] in arr\n    a = i + v\n"),
            ["for [v]"]
        );
        assert_eq!(
            hidden_counters("i = 1\nfor [i, v] in arr\n    a = i + v\n"),
            ["for [i]"]
        );
        assert_eq!(
            hidden_counters("v = 2\nfor v in arr\n    a = v\n"),
            ["for [v]"]
        );
    }

    #[test]
    fn tuples_and_expression_blocks_are_flagged_per_name() {
        assert_eq!(shadowing("a = 1\nif true\n    [a, b] = f()\n"), ["[a]"]);
        assert_eq!(
            shadowing("x = 1\ny = if true\n    x = 2\n    x\nelse\n    0\n"),
            ["x"]
        );
    }
}
