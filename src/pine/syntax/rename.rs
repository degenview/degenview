//! Gives every local and parameter that shares a name with something else a name of its own.
//!
//! The interpreter keeps its variables in one table keyed by name, and histories by the same name.
//! Pine's scopes nest: a block's `x` hides the global `x` until the block ends, two sibling blocks
//! may both declare `x`, a function's parameter may be called `close`. A table keyed by name cannot
//! tell those apart, so the interpreter used to save and put back values around every block and call
//! that hid something. That left the cases it did not cover wrong: the history of a hiding local read
//! the global's, sibling blocks shared one history, a function called from a hiding block saw the
//! caller's local.
//!
//! This pass removes the problem at its root. Before the session runs anything, every declaration
//! whose name is also a global, a builtin variable, a parameter or another local becomes
//! `name#site` (`name#L2_site` in the 2nd imported library), where `site` is the offset of the
//! declaration. Every use that [`scopes::resolve`] bound to it follows. A name nothing else shares
//! stays as written, so the runtime pays for the pass only where a script reuses names, and a global
//! always keeps its own name (plots, alerts and the variables a script exposes are named by it).
//!
//! Display names (the debugger's variables, error text) drop everything from the mark on:
//! [`display_name`].
//!
//! A [`Renames`] is a plan over a tree that is not touched until [`Renames::apply`], so the debugger
//! can ask what a script's names will be called without running one.

use std::borrow::Cow;
use std::collections::HashMap;

use super::ast::{Expression, SYNTHETIC, Statement};
use super::scopes::{self, Binding, Slot, Span, span};
use super::traversal;

/// Starts the part of a renamed declaration's name that a person never sees.
pub const MARK: char = '#';

/// The name to show for a variable of the interpreter's table: `level` for `level#42`.
pub fn display_name(name: &str) -> &str {
    name.split_once(MARK).map_or(name, |(shown, _)| shown)
}

/// [`display_name`] for a name that may be dotted: `zone.top` for `zone#42.top`. A name nothing
/// renamed is handed back as it is. Every message a person can read that holds a variable's name,
/// or the written name of a call on one, goes through this.
pub fn display_text(name: &str) -> Cow<'_, str> {
    match name.split_once(MARK) {
        None => Cow::Borrowed(name),
        Some((shown, renamed)) => Cow::Owned(match renamed.split_once('.') {
            Some((_, rest)) => format!("{shown}.{rest}"),
            None => shown.to_owned(),
        }),
    }
}

/// The program a tree belongs to: names are only unique within one, so a library's carry its
/// number.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Program {
    Script,
    /// The nth library the script imports, depth first.
    Library(usize),
}

/// What the declarations and uses of a program are renamed to.
#[derive(Debug, Default)]
pub struct Renames {
    /// The new head of an identifier or call that reads a renamed declaration, by span.
    uses: HashMap<Span, String>,
    /// The new name of what a statement declares or assigns, by slot.
    names: HashMap<Slot, String>,
    /// The new variable of a function's parameter, by the function's start offset and position.
    parameters: HashMap<(usize, usize), String>,
}

/// Plans the renames of the program `statements`. `is_builtin` says which free names are builtin
/// variables (`low`, `time`), `keeps_name` which written call names are answered without reading a
/// variable (`log.info`, `Zone.new`, `lib.f`): a call of one keeps its name, whatever a local
/// called like its head is.
pub fn plan(
    statements: &[Statement],
    is_builtin: &dyn Fn(&str) -> bool,
    keeps_name: &dyn Fn(&str) -> bool,
    program: Program,
) -> Renames {
    let bindings = scopes::resolve(statements);
    // The parameters of the top-level functions, numbered the way `Binding::Parameter` numbers them.
    let mut parameters: Vec<(usize, usize, &str)> = Vec::new();
    let mut function_offsets = Vec::new();
    for statement in statements {
        if let Statement::Function {
            parameters: declared,
            range,
            ..
        } = statement
        {
            let function = function_offsets.len();
            function_offsets.push(range.start.offset);
            for (position, parameter) in declared.iter().enumerate() {
                parameters.push((function, position, parameter.slot.as_str()));
            }
        }
    }

    let mut declarations: HashMap<&str, usize> = HashMap::new();
    for id in 0..bindings.local_count() {
        *declarations.entry(bindings.local(id).name).or_default() += 1;
    }
    for (_, _, name) in &parameters {
        *declarations.entry(name).or_default() += 1;
    }
    let contested = |name: &str| {
        !name.starts_with(SYNTHETIC)
            && (program != Program::Script
                || bindings.globals().contains(name)
                || is_builtin(name)
                || declarations.get(name).is_some_and(|count| *count > 1))
    };
    let renamed = |name: &str, site: usize| match program {
        Program::Script => format!("{name}{MARK}{site}"),
        Program::Library(number) => format!("{name}{MARK}L{number}_{site}"),
    };

    let locals: Vec<Option<String>> = (0..bindings.local_count())
        .map(|id| {
            let local = bindings.local(id);
            contested(local.name).then(|| renamed(local.name, local.site))
        })
        .collect();
    let mut renames = Renames::default();
    let mut slots: HashMap<(usize, usize), String> = HashMap::new();
    for (function, position, name) in &parameters {
        if contested(name) {
            let offset = function_offsets[*function];
            let new = renamed(name, offset);
            slots.insert((*function, *position), new.clone());
            renames.parameters.insert((offset, *position), new);
        }
    }
    let new_name = |binding: Binding<'_>| match binding {
        Binding::Local(id) => locals[id].clone(),
        Binding::Parameter { function, position } => slots.get(&(function, position)).cloned(),
        Binding::Global(_) | Binding::Free(_) => None,
    };
    for (key, binding) in bindings.uses() {
        if let Some(new) = new_name(binding) {
            renames.uses.insert(key, new);
        }
    }
    for (slot, binding) in bindings.names() {
        if let Some(new) = new_name(binding) {
            renames.names.insert(slot, new);
        }
    }
    if !renames.uses.is_empty() {
        traversal::for_each_call(statements, &mut |name, range| {
            if keeps_name(name) {
                renames.uses.remove(&span(range));
            }
        });
    }
    renames
}

impl Renames {
    pub fn is_empty(&self) -> bool {
        self.names.is_empty() && self.parameters.is_empty() && self.uses.is_empty()
    }

    /// What the `slot`th name of the statement starting at `site` is called now, when it changed.
    pub fn declared(&self, site: usize, slot: usize) -> Option<&str> {
        self.names.get(&(site, slot)).map(String::as_str)
    }

    /// What the parameter at `position` of the function starting at `function` is called now.
    pub fn parameter(&self, function: usize, position: usize) -> Option<&str> {
        self.parameters
            .get(&(function, position))
            .map(String::as_str)
    }

    /// What the identifier or call spanning `key` reads now, when it changed.
    pub fn read(&self, key: Span) -> Option<&str> {
        self.uses.get(&key).map(String::as_str)
    }

    /// Renames the declarations and the uses of `statements`, the tree the plan was made for.
    pub fn apply(&self, statements: &mut [Statement]) {
        if self.is_empty() {
            return;
        }
        traversal::for_each_statement_mut(statements, &mut |statement| {
            self.declarations(statement);
        });
        traversal::for_each_expression_mut(statements, &mut |expression| self.uses(expression));
    }

    fn declarations(&self, statement: &mut Statement) {
        let site = statement.range().start.offset;
        let rename = |name: &mut String, slot: usize| {
            if let Some(new) = self.declared(site, slot) {
                new.clone_into(name);
            }
        };
        match statement {
            Statement::Declaration { name, .. } | Statement::Assignment { name, .. } => {
                rename(name, 0);
            }
            Statement::TupleDeclaration { names, .. } => {
                for (slot, name) in names.iter_mut().enumerate() {
                    rename(name, slot);
                }
            }
            Statement::ForRange { variable, .. } => rename(variable, 0),
            Statement::ForIn { index, value, .. } => {
                if let Some(index) = index {
                    rename(index, 0);
                }
                rename(value, 1);
            }
            Statement::Function { parameters, .. } => {
                for (position, parameter) in parameters.iter_mut().enumerate() {
                    if let Some(new) = self.parameter(site, position) {
                        new.clone_into(&mut parameter.slot);
                    }
                }
            }
            _ => {}
        }
    }

    fn uses(&self, expression: &mut Expression) {
        match expression {
            Expression::Identifier(name, range) => {
                if let Some(new) = self.read(span(*range)) {
                    replace_head(name, new);
                }
            }
            // A plain name is a function, never a variable. A call that reads no variable was left
            // out of the plan: it keeps its name, even where a variable is called like its head.
            Expression::Call { name, range, .. } if name.contains('.') => {
                if let Some(new) = self.read(span(*range)) {
                    replace_head(name, new);
                }
            }
            _ => {}
        }
    }
}

/// `zone.top` with `zone` renamed.
fn replace_head(name: &mut String, head: &str) {
    *name = match name.split_once('.') {
        Some((_, rest)) => format!("{head}.{rest}"),
        None => head.to_owned(),
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::{Lexer, Limits, Parser};
    use std::collections::HashSet;

    fn parse(source: &str) -> Vec<Statement> {
        let lexed = Lexer::new(source).lex();
        assert!(lexed.diagnostics.is_empty(), "{:?}", lexed.diagnostics);
        let mut parser = Parser::new(lexed.tokens, Limits::default(), Default::default());
        let (statements, diagnostics) = parser.parse();
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        statements
    }

    /// What a session counts as a builtin function here.
    fn is_log(name: &str) -> bool {
        name.starts_with("log.") || name.starts_with("request.") || name.starts_with("ticker.")
    }

    fn renamed(source: &str, program: Program) -> Vec<Statement> {
        let mut statements = parse(source);
        plan(&statements, &|name| name == "low", &is_log, program).apply(&mut statements);
        statements
    }

    /// Every name a declaration or a read in `statements` carries, in source order: declared names,
    /// parameter slots and identifier reads, as `text`.
    fn names(statements: &[Statement]) -> Vec<String> {
        let mut found = Vec::new();
        let mut statements = statements.to_vec();
        traversal::for_each_statement_mut(&mut statements, &mut |statement| match statement {
            Statement::Declaration { name, .. } | Statement::Assignment { name, .. } => {
                found.push(name.clone());
            }
            Statement::TupleDeclaration { names, .. } => found.extend(names.iter().cloned()),
            Statement::ForRange { variable, .. } => found.push(variable.clone()),
            Statement::ForIn { index, value, .. } => {
                found.extend(index.iter().cloned());
                found.push(value.clone());
            }
            Statement::Function { parameters, .. } => {
                found.extend(parameters.iter().map(|p| p.slot.clone()));
            }
            _ => {}
        });
        found
    }

    fn reads(statements: &[Statement]) -> Vec<String> {
        let mut found = Vec::new();
        traversal::for_each_identifier(statements, &mut |name, _| found.push(name.to_owned()));
        found
    }

    #[test]
    fn a_name_nothing_else_shares_is_left_alone() {
        let statements = renamed(
            "a = 1\nif a > 0\n    b = a\nf(p) =>\n    q = p\n    q\n",
            Program::Script,
        );
        assert_eq!(names(&statements), ["a", "b", "p", "q"]);
        assert_eq!(reads(&statements), ["a", "a", "p", "q"]);
    }

    #[test]
    fn a_local_that_hides_a_global_is_renamed_and_the_global_is_not() {
        let statements = renamed(
            "x = 1\nif close > 0\n    x = x + 1\n    y = x\nz = x\n",
            Program::Script,
        );
        let declared = names(&statements);
        assert_eq!(declared[0], "x", "the global keeps its name");
        assert!(declared[1].starts_with("x#"), "{declared:?}");
        assert_eq!(declared[2], "y");
        // The value of the declaration reads the outer `x`; what follows it reads the local; after
        // the block the global again.
        let read = reads(&statements);
        assert_eq!(read[1], "x", "{read:?}");
        assert_eq!(read[2], declared[1], "{read:?}");
        assert_eq!(read[3], "x", "{read:?}");
    }

    #[test]
    fn sibling_blocks_and_a_function_get_names_of_their_own() {
        let statements = renamed(
            "if close > 0\n    t = 1\nif close < 0\n    t = 2\nf() =>\n    t = 3\n    t\n",
            Program::Script,
        );
        let declared = names(&statements);
        assert_eq!(declared.len(), 3);
        assert!(
            declared.iter().all(|name| name.starts_with("t#")),
            "{declared:?}"
        );
        let unique: HashSet<&String> = declared.iter().collect();
        assert_eq!(unique.len(), 3, "{declared:?}");
    }

    #[test]
    fn a_parameter_is_renamed_in_its_slot_and_keeps_its_label() {
        let mut statements = parse("len = 10\nf(len, shift) =>\n    len + shift\n");
        plan(&statements, &|_| false, &is_log, Program::Script).apply(&mut statements);
        let Statement::Function {
            parameters, body, ..
        } = &statements[1]
        else {
            panic!("a function");
        };
        assert_eq!(parameters[0].name, "len", "the label named arguments use");
        assert!(parameters[0].slot.starts_with("len#"), "{parameters:?}");
        assert_eq!(parameters[1].slot, "shift");
        let Statement::Expression(Expression::Binary { lhs, .. }) = &body[0] else {
            panic!("a sum");
        };
        assert_eq!(
            **lhs,
            Expression::Identifier(parameters[0].slot.clone(), lhs.range())
        );
    }

    #[test]
    fn a_local_named_like_a_builtin_variable_is_renamed() {
        let statements = renamed("if close > 0\n    low = 1\n    a = low\n", Program::Script);
        let declared = names(&statements);
        assert!(declared[0].starts_with("low#"), "{declared:?}");
        assert_eq!(reads(&statements)[1], declared[0]);
    }

    #[test]
    fn a_dotted_name_renames_its_head_only() {
        let statements = renamed(
            "zone = 1\nif close > 0\n    zone = 2\n    a = zone.top\n    b = zone.get(0)\n",
            Program::Script,
        );
        let read = reads(&statements);
        assert!(
            read[1].starts_with("zone#") && read[1].ends_with(".top"),
            "{read:?}"
        );
        let mut called = Vec::new();
        traversal::for_each_call(&statements, &mut |name, _| called.push(name.to_owned()));
        assert!(
            called[0].starts_with("zone#") && called[0].ends_with(".get"),
            "{called:?}"
        );
    }

    #[test]
    fn a_call_of_a_builtin_keeps_its_name_where_a_variable_has_its_namespace() {
        let statements = renamed(
            "log = 1.0\nrequest = 1.0\nticker = 1.0\nlevels = 1.0\nf() =>\n    log = 2.0\n    request = 2.0\n    ticker = 2.0\n    levels = array.new_float(0)\n    log.info(\"hi\")\n    request.security(syminfo.tickerid, \"D\", close)\n    ticker.new(\"a\", \"b\")\n    levels.push(1.0)\n    color.red\n",
            Program::Script,
        );
        let mut called = Vec::new();
        traversal::for_each_call(&statements, &mut |name, _| called.push(name.to_owned()));
        assert!(called.contains(&"log.info".to_owned()), "{called:?}");
        assert!(
            called.contains(&"request.security".to_owned()),
            "{called:?}"
        );
        assert!(called.contains(&"ticker.new".to_owned()), "{called:?}");
        // A variable's own member is its receiver: renamed with it.
        assert!(
            called
                .iter()
                .any(|name| name.starts_with("levels#") && name.ends_with(".push")),
            "{called:?}"
        );
    }

    #[test]
    fn a_read_of_a_dotted_name_renames_its_head_so_a_variables_field_wins() {
        let statements = renamed(
            "color = 1\nf() =>\n    color = 2\n    color.red\n",
            Program::Script,
        );
        let read = reads(&statements);
        assert!(
            read.iter()
                .any(|name| name.starts_with("color#") && name.ends_with(".red")),
            "{read:?}"
        );
    }

    #[test]
    fn a_library_names_every_local_after_its_number() {
        let statements = renamed("export f(x) =>\n    y = x\n    y\n", Program::Library(2));
        assert_eq!(names(&statements).len(), 2);
        assert!(
            names(&statements).iter().all(|name| name.contains("#L2_")),
            "{:?}",
            names(&statements)
        );
    }

    #[test]
    fn the_names_a_session_does_not_see_are_the_display_names() {
        assert_eq!(display_name("level#42"), "level");
        assert_eq!(display_name("level#L2_42"), "level");
        assert_eq!(display_name("level"), "level");
        assert_eq!(display_text("zone#42.top"), "zone.top");
        assert_eq!(display_text("zone#42"), "zone");
    }

    #[test]
    fn a_plan_names_what_it_will_apply_without_touching_the_tree() {
        let statements = parse("x = 1\nif close > 0\n    x = 2\n");
        let renames = plan(&statements, &|_| false, &is_log, Program::Script);
        let Statement::Conditional { when_true, .. } = &statements[1] else {
            panic!("an if");
        };
        let site = when_true[0].range().start.offset;
        assert_eq!(
            renames.declared(site, 0),
            Some(format!("x#{site}").as_str())
        );
        assert_eq!(names(&statements), ["x", "x"]);
    }
}
