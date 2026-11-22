//! What statements and expressions read, from the bindings `syntax::scopes` resolved.

use std::collections::HashSet;

use crate::pine::compiler::is_builtin_namespace;
use crate::pine::syntax::ast::{Expression, Statement};
use crate::pine::syntax::scopes::{Binding, Bindings, LocalId, head};
use crate::pine::syntax::traversal;

/// The names an expression or statement reads, and the functions it calls that the type checker
/// resolved to one definition.
#[derive(Default)]
pub(super) struct Reads<'a> {
    /// The globals and builtins it reads, and the callees the type checker did not resolve: any
    /// definition of that name may be the one called.
    pub names: HashSet<&'a str>,
    pub targets: HashSet<usize>,
    /// The names among `names` that are read through the history operator (`level[1]`).
    pub history: HashSet<&'a str>,
    /// The locals it reads, the ones it declares itself included. A function's body is the one
    /// place a local is not a read of anything the slice has.
    pub locals: HashSet<LocalId>,
}

impl<'a> Reads<'a> {
    pub fn of_statements(bindings: &Bindings<'a>, statements: &'a [Statement]) -> Self {
        let mut collector = Collector {
            bindings,
            reads: Self::default(),
        };
        for statement in statements {
            collector.statement(statement);
        }
        collector.reads
    }

    pub fn of_expression(bindings: &Bindings<'a>, expression: &'a Expression) -> Self {
        let mut collector = Collector {
            bindings,
            reads: Self::default(),
        };
        collector.expression(expression);
        collector.reads
    }

    pub fn extend(&mut self, other: &Self) {
        self.names.extend(other.names.iter().copied());
        self.targets.extend(other.targets.iter().copied());
        self.history.extend(other.history.iter().copied());
    }
}

struct Collector<'w, 'a> {
    bindings: &'w Bindings<'a>,
    reads: Reads<'a>,
}

impl<'a> Collector<'_, 'a> {
    fn statement(&mut self, statement: &'a Statement) {
        for expression in traversal::expressions(statement) {
            self.expression(expression);
        }
        for block in traversal::nested_blocks(statement) {
            for child in block {
                self.statement(child);
            }
        }
    }

    /// A read of the head of a name: a global or builtin is a name the slice may need, a local is
    /// noted, a parameter is the function's own.
    fn variable(&mut self, binding: Option<Binding<'a>>) {
        match binding {
            Some(Binding::Global(name) | Binding::Free(name)) => {
                self.reads.names.insert(name);
            }
            Some(Binding::Local(id)) => {
                self.reads.locals.insert(id);
            }
            Some(Binding::Parameter { .. }) | None => {}
        }
    }

    /// Every name an expression may depend on: variables (a dotted path counts as its head), and
    /// the callees. A call the type checker resolved names its definition; any other may be any
    /// definition of that name.
    fn expression(&mut self, expression: &'a Expression) {
        match expression {
            Expression::Identifier(..) => self.variable(self.bindings.occurrence(expression)),
            Expression::Call { name, target, .. } => {
                match target {
                    Some(id) => {
                        self.reads.targets.insert(*id);
                    }
                    None if !name.contains('.') => {
                        self.reads.names.insert(name);
                    }
                    None => {}
                }
                if let Some((receiver, member)) = name.rsplit_once('.') {
                    self.variable(self.bindings.occurrence(expression));
                    // `ta.sma` is the builtin, not a script function that happens to be called `sma`.
                    if target.is_none() && !is_builtin_namespace(head(receiver)) {
                        self.reads.names.insert(member);
                    }
                }
            }
            Expression::MethodCall { name, target, .. } => match target {
                Some(id) => {
                    self.reads.targets.insert(*id);
                }
                None => {
                    self.reads.names.insert(name);
                }
            },
            Expression::History { base, .. } => {
                if let Some(root) = self.bindings.root(base).and_then(Binding::name) {
                    self.reads.history.insert(root);
                }
            }
            Expression::Statement(statement, _) => self.statement(statement),
            _ => {}
        }
        for child in traversal::children(expression) {
            self.expression(child);
        }
    }
}
