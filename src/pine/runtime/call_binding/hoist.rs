//! Runs a call on every bar when a lazily evaluated operand takes a history of it.
//!
//! `cond ? ta.sma(close, 3)[1] : na` reads the call's value one bar back. On TradingView that is
//! the value the call has when it runs on every bar (measured for `ta.sma`, `ta.ema`, an offset of
//! 2 and a user function), although the same call without `[]` keeps its own state and runs only
//! when its branch does. The interpreter stores an `expr:` history value only on the bars the `[]`
//! runs, so it read `na` (offset 1) or the call's own previous run (offset 2).
//!
//! This pass gives such a call a variable of its own. The call moves, unchanged, into a synthetic
//! declaration placed before the statement it is in, and the operand reads that variable's history:
//!
//! ```text
//! \u{1}h0 = ta.sma(close, 3)
//! r = cond ? \u{1}h0[1] : na
//! ```
//!
//! The ordinary history of a variable then does the rest; the interpreter has nothing new to run.
//! It runs before `bind`, so a moved call is routed like any other, and before the scope analysis
//! (`mark_shadowing`, the `request.security` slices) so those see the synthetic statements.
//!
//! A *lazy* operand is the right side of `and`/`or`, either arm of `?:`, the arms of an `if` or
//! `switch` used for its value, and a `switch` arm's condition after the first. Only a *total*
//! call moves: running it early cannot fail or change anything, so it cannot change what the script
//! does on the bars its branch skips. Which builtins are total is decided by `total::is_total`; a
//! user function is total when its body only computes with total calls, and a loop in it has a trip
//! count that is known and cannot fail (its bounds and step are literals and the step is not zero:
//! a zero step raises, and a bound the data sets can run past the per-bar instruction budget). A
//! history is total only at a literal non-negative offset (`close[lag]` fails for a negative `lag`,
//! which a guard may be keeping out). Anything that could fail, change state or draw (`array.get`,
//! `array.push`, `plot`, `runtime.error`, `time`, a method, a field read, `request.*`) stays where
//! it is, as does a call whose arguments read a name the statement itself declares or assigns.
//!
//! The scope is the script's own statements and the libraries it imports, in blocks and functions
//! alike. A call inside a `request.*` expression is not moved: that expression runs on the requested
//! series, so a lazy `call[]` written there keeps its old reading (`na` on the bars its branch
//! skips). A call to a library's function is never moved either, since a library's functions are not
//! in the script's resolutions.
//!
//! The synthetic declaration starts where the call does, so its offset is unique among
//! declarations, and `SYNTHETIC` marks its name: the debugger skips it, and the profiler charges
//! its time to the statement it was taken from, which stays the unit they measure and stop at.

use std::collections::{HashMap, HashSet};

use super::total::is_total;
use super::{Targets, collect_captured, route_for};
use crate::pine::SourceRange;
use crate::pine::compiler::program::CompiledProgram;
use crate::pine::syntax::ast::{
    BinaryOperator, DeclarationMode, Expression, FunctionKind, Literal, Parameter, SYNTHETIC,
    Statement, TypeAnnotation, UnaryOperator,
};
use crate::pine::syntax::traversal::{children, expressions, nested_blocks};

/// Hoists the lazily evaluated history references of `program` and of the libraries it imports.
pub fn lazy_history(program: &mut CompiledProgram) {
    // Most scripts take no history of a call: they pay for this scan and nothing else.
    if !any_history_of_call(program) {
        return;
    }
    let mut captured = HashSet::new();
    collect_captured(program, &mut captured);
    // `bind` takes the resolutions next; they are only read here.
    let targets = std::mem::take(&mut program.resolutions);
    let mut counter = 0;
    hoist_program(program, &captured, &targets, &mut counter);
    program.resolutions = targets;
}

fn hoist_program(
    program: &mut CompiledProgram,
    captured: &HashSet<String>,
    targets: &Targets,
    counter: &mut usize,
) {
    if takes_history_of_call(&program.statements) {
        let calls = Calls::new(&program.statements, captured, targets);
        Hoister { calls, counter }.block(&mut program.statements);
    }
    for import in &mut program.imports {
        if let Some(library) = &mut import.library {
            // A library's call sites are numbered apart from the script's.
            hoist_program(library, captured, &Targets::new(), counter);
        }
    }
}

/// Whether the program or a library it imports has a `call(…)[n]` anywhere.
fn any_history_of_call(program: &CompiledProgram) -> bool {
    takes_history_of_call(&program.statements)
        || program
            .imports
            .iter()
            .filter_map(|import| import.library.as_deref())
            .any(any_history_of_call)
}

/// Whether anything in `statements` is `call(…)[n]`.
fn takes_history_of_call(statements: &[Statement]) -> bool {
    fn in_expression(expression: &Expression) -> bool {
        matches!(expression, Expression::History { base, .. } if matches!(**base, Expression::Call { .. }))
            || children(expression).into_iter().any(in_expression)
    }
    let mut found = false;
    for statement in statements {
        visit_statements(statement, &mut |inner| {
            found = found || expressions(inner).into_iter().any(in_expression);
        });
    }
    found
}

/// Which calls may run on a bar their branch skips.
struct Calls<'c> {
    captured: &'c HashSet<String>,
    targets: &'c Targets,
    /// The top-level functions by name (methods are never called by a plain name).
    by_name: HashMap<String, Vec<usize>>,
    /// For each top-level function or method: whether running it changes nothing and cannot fail.
    pure: Vec<bool>,
}

impl<'c> Calls<'c> {
    fn new(statements: &[Statement], captured: &'c HashSet<String>, targets: &'c Targets) -> Self {
        let mut bodies: Vec<(&[Parameter], &[Statement])> = Vec::new();
        let mut by_name: HashMap<String, Vec<usize>> = HashMap::new();
        for statement in statements {
            if let Statement::Function {
                name,
                kind,
                parameters,
                body,
                ..
            } = statement
            {
                if *kind == FunctionKind::Function {
                    by_name.entry(name.clone()).or_default().push(bodies.len());
                }
                bodies.push((parameters, body));
            }
        }
        let mut calls = Self {
            captured,
            targets,
            by_name,
            pure: vec![true; bodies.len()],
        };
        // Greatest fixpoint: a function is pure unless its body calls something that is not.
        loop {
            let next: Vec<bool> = bodies
                .iter()
                .zip(&calls.pure)
                .map(|((parameters, body), pure)| *pure && calls.body_is_pure(parameters, body))
                .collect();
            if next == calls.pure {
                return calls;
            }
            calls.pure = next;
        }
    }

    /// A body that only computes: it assigns nothing but its own names and calls only what is safe.
    fn body_is_pure(&self, parameters: &[Parameter], body: &[Statement]) -> bool {
        let mut locals: HashSet<String> = parameters.iter().map(|p| p.name.clone()).collect();
        for statement in body {
            visit_statements(statement, &mut |inner| declared(inner, &mut locals));
        }
        body.iter()
            .all(|statement| self.statement(statement, &locals))
    }

    fn statement(&self, statement: &Statement, locals: &HashSet<String>) -> bool {
        let all = |block: &[Statement]| block.iter().all(|s| self.statement(s, locals));
        match statement {
            Statement::Declaration { value, .. } | Statement::TupleDeclaration { value, .. } => {
                self.expression(value, locals)
            }
            Statement::Assignment { name, value, .. } => {
                locals.contains(name) && self.expression(value, locals)
            }
            Statement::Expression(expression) => self.expression(expression, locals),
            Statement::Conditional {
                condition,
                when_true,
                when_false,
                ..
            } => self.expression(condition, locals) && all(when_true) && all(when_false),
            // A loop is total when its trip count is known and cannot fail: a zero step raises, and
            // a bound the data sets can run past the per-bar instruction budget (PINE8004), on a
            // bar its branch was keeping out.
            Statement::ForRange {
                from,
                to,
                step,
                body,
                ..
            } => {
                literal_number(from).is_some()
                    && literal_number(to).is_some()
                    && step
                        .iter()
                        .all(|step| literal_number(step).is_some_and(|step| step != 0.0))
                    && all(body)
            }
            // How often it runs is the collection's size.
            Statement::ForIn { .. } => false,
            Statement::Switch { subject, arms, .. } => {
                subject
                    .iter()
                    .all(|subject| self.expression(subject, locals))
                    && arms.iter().all(|arm| {
                        arm.condition
                            .iter()
                            .all(|condition| self.expression(condition, locals))
                            && all(&arm.body)
                    })
            }
            Statement::LoopControl(..) => true,
            // A loop whose end the data decides could run away on a bar it was never meant to run.
            Statement::WhileLoop { .. }
            | Statement::Once { .. }
            | Statement::FieldAssignment { .. }
            | Statement::TypeDeclaration { .. }
            | Statement::EnumDeclaration { .. }
            | Statement::Function { .. } => false,
        }
    }

    /// Whether `expression` is *total*: it changes nothing and cannot fail, so running it on a bar
    /// its branch skips cannot change what the script does. Every variant is decided here, and a
    /// new one has to be.
    fn expression(&self, expression: &Expression, locals: &HashSet<String>) -> bool {
        let total = |operand: &Expression| self.expression(operand, locals);
        match expression {
            Expression::Literal(..) | Expression::Identifier(..) => true,
            Expression::Unary { operand, .. } => total(operand),
            Expression::Binary { lhs, rhs, .. } => total(lhs) && total(rhs),
            Expression::Ternary {
                condition,
                when_true,
                when_false,
                ..
            } => total(condition) && total(when_true) && total(when_false),
            Expression::Tuple(values, _) => values.iter().all(total),
            Expression::Call {
                name,
                arguments,
                site,
                ..
            } => self.call(name, *site) && arguments.iter().all(|argument| total(&argument.value)),
            // A history fails for a negative offset (PINE4006): only one written as a
            // non-negative number is known not to, whatever a guard around it says.
            Expression::History { base, offset, .. } => {
                matches!(**offset, Expression::Literal(Literal::Int(n), _) if n >= 0) && total(base)
            }
            // A field read of an object that is not there, a method that changes its receiver.
            Expression::Member { .. } | Expression::MethodCall { .. } => false,
            Expression::Statement(statement, _) => self.statement(statement, locals),
        }
    }

    /// Whether running the call on a bar its branch skips is harmless.
    fn call(&self, name: &str, site: usize) -> bool {
        match route_for(name, self.captured) {
            Some(_) => is_total(name),
            None => {
                let callees = match self.targets.get(&site) {
                    Some(target) => std::slice::from_ref(target),
                    None => self.by_name.get(name).map_or(&[][..], Vec::as_slice),
                };
                !callees.is_empty() && callees.iter().all(|callee| self.pure[*callee])
            }
        }
    }

    /// Whether `expression` calls something that is not a builtin.
    fn calls_user_function(&self, expression: &Expression) -> bool {
        if let Expression::Call { name, .. } = expression
            && route_for(name, self.captured).is_none()
        {
            return true;
        }
        children(expression)
            .into_iter()
            .any(|child| self.calls_user_function(child))
    }
}

/// What the statement a lazy operand sits in declares or changes, besides its own result: the
/// names a hoisted call must not read, and whether anything in it has an effect a user function
/// could read.
#[derive(Clone, Default)]
struct Written {
    names: HashSet<String>,
    effects: bool,
}

struct Hoister<'h, 'c> {
    calls: Calls<'c>,
    counter: &'h mut usize,
}

impl Hoister<'_, '_> {
    /// Hoists in each statement of a block, which gets the synthetic declarations it needs just
    /// before the statement they came from.
    fn block(&mut self, statements: &mut Vec<Statement>) {
        let mut index = 0;
        while index < statements.len() {
            let mut ahead = Vec::new();
            self.statement(&mut statements[index], &mut ahead);
            let moved = ahead.len();
            if moved > 0 {
                statements.splice(index..index, ahead);
            }
            index += moved + 1;
        }
    }

    fn statement(&mut self, statement: &mut Statement, ahead: &mut Vec<Statement>) {
        let written = Written::default();
        match statement {
            Statement::Declaration { value, .. }
            | Statement::Assignment { value, .. }
            | Statement::TupleDeclaration { value, .. }
            | Statement::Expression(value) => self.expression(value, false, &written, ahead),
            Statement::FieldAssignment { target, value, .. } => {
                self.expression(target, false, &written, ahead);
                self.expression(value, false, &written, ahead);
            }
            Statement::Conditional {
                condition,
                when_true,
                when_false,
                ..
            } => {
                self.expression(condition, false, &written, ahead);
                self.block(when_true);
                self.block(when_false);
            }
            Statement::ForRange {
                from,
                to,
                step,
                body,
                ..
            } => {
                self.expression(from, false, &written, ahead);
                self.expression(to, false, &written, ahead);
                if let Some(step) = step {
                    self.expression(step, false, &written, ahead);
                }
                self.block(body);
            }
            Statement::ForIn {
                collection, body, ..
            } => {
                self.expression(collection, false, &written, ahead);
                self.block(body);
            }
            // The condition runs again on every pass: nothing to run once ahead of it.
            Statement::WhileLoop { body, .. } | Statement::Function { body, .. } => {
                self.block(body);
            }
            Statement::Once {
                condition, body, ..
            } => {
                if let Some(condition) = condition {
                    self.expression(condition, false, &written, ahead);
                }
                self.block(body);
            }
            Statement::Switch { subject, arms, .. } => {
                if let Some(subject) = subject {
                    self.expression(subject, false, &written, ahead);
                }
                for (index, arm) in arms.iter_mut().enumerate() {
                    if let Some(condition) = &mut arm.condition {
                        self.expression(condition, index > 0, &written, ahead);
                    }
                    self.block(&mut arm.body);
                }
            }
            Statement::LoopControl(..)
            | Statement::TypeDeclaration { .. }
            | Statement::EnumDeclaration { .. } => {}
        }
    }

    /// `lazy`: whether the expression runs only on some of the bars the statement does.
    fn expression(
        &mut self,
        expression: &mut Expression,
        lazy: bool,
        written: &Written,
        ahead: &mut Vec<Statement>,
    ) {
        match expression {
            Expression::Literal(..) | Expression::Identifier(..) => {}
            Expression::Unary { operand, .. } => self.expression(operand, lazy, written, ahead),
            Expression::Binary { lhs, op, rhs, .. } => {
                let short_circuits = matches!(op, BinaryOperator::And | BinaryOperator::Or);
                self.expression(lhs, lazy, written, ahead);
                self.expression(rhs, lazy || short_circuits, written, ahead);
            }
            Expression::Ternary {
                condition,
                when_true,
                when_false,
                ..
            } => {
                self.expression(condition, lazy, written, ahead);
                self.expression(when_true, true, written, ahead);
                self.expression(when_false, true, written, ahead);
            }
            Expression::Call {
                name, arguments, ..
            } => {
                // A request's expression runs on the requested series, where a variable of the
                // chart's is not the call's.
                if name.starts_with("request.") {
                    return;
                }
                for argument in arguments {
                    self.expression(&mut argument.value, lazy, written, ahead);
                }
            }
            Expression::MethodCall {
                receiver,
                arguments,
                ..
            } => {
                self.expression(receiver, lazy, written, ahead);
                for argument in arguments {
                    self.expression(&mut argument.value, lazy, written, ahead);
                }
            }
            Expression::Member { base, .. } => self.expression(base, lazy, written, ahead),
            Expression::Tuple(values, _) => {
                for value in values {
                    self.expression(value, lazy, written, ahead);
                }
            }
            Expression::History { base, offset, .. } => {
                self.expression(offset, lazy, written, ahead);
                if lazy
                    && matches!(**base, Expression::Call { .. })
                    && self.can_run_ahead(base, written)
                {
                    self.run_ahead(base, ahead);
                } else {
                    self.expression(base, lazy, written, ahead);
                }
            }
            Expression::Statement(inner, _) => {
                let written = self.written_in(inner, written);
                self.held(inner, lazy, &written, ahead);
            }
        }
    }

    /// A statement used for its value (`x = if …`): its condition runs when the expression does,
    /// its branches only when taken. What it declares stays inside it, so it cannot move out.
    fn held(
        &mut self,
        statement: &mut Statement,
        lazy: bool,
        written: &Written,
        ahead: &mut Vec<Statement>,
    ) {
        match statement {
            Statement::Conditional {
                condition,
                when_true,
                when_false,
                ..
            } => {
                self.expression(condition, lazy, written, ahead);
                for inner in when_true.iter_mut().chain(when_false) {
                    self.held(inner, true, written, ahead);
                }
            }
            Statement::Switch { subject, arms, .. } => {
                if let Some(subject) = subject {
                    self.expression(subject, lazy, written, ahead);
                }
                for (index, arm) in arms.iter_mut().enumerate() {
                    if let Some(condition) = &mut arm.condition {
                        self.expression(condition, lazy || index > 0, written, ahead);
                    }
                    for inner in &mut arm.body {
                        self.held(inner, true, written, ahead);
                    }
                }
            }
            Statement::Declaration { value, .. }
            | Statement::Assignment { value, .. }
            | Statement::TupleDeclaration { value, .. }
            | Statement::Expression(value) => self.expression(value, lazy, written, ahead),
            Statement::FieldAssignment { target, value, .. } => {
                self.expression(target, lazy, written, ahead);
                self.expression(value, lazy, written, ahead);
            }
            // A loop runs its body more or fewer times than once: nothing moves out of it.
            _ => {}
        }
    }

    fn written_in(&self, statement: &Statement, outer: &Written) -> Written {
        let mut written = outer.clone();
        visit_statements(statement, &mut |inner| {
            declared(inner, &mut written.names);
            if let Statement::Assignment { name, .. } = inner {
                written.names.insert(name.clone());
            }
        });
        written.effects |= !self.calls.statement(statement, &HashSet::new());
        written
    }

    fn can_run_ahead(&self, call: &Expression, written: &Written) -> bool {
        if !self.calls.expression(call, &HashSet::new()) {
            return false;
        }
        let mut reads_written = false;
        identifiers(call, &mut |name| {
            reads_written |= written.names.contains(name)
        });
        !reads_written && !(written.effects && self.calls.calls_user_function(call))
    }

    /// Moves `call` into a declaration of its own, placed in `ahead`, and leaves a read of the
    /// variable in its place. The call keeps its site, so its state and its resolution stay its own.
    fn run_ahead(&mut self, call: &mut Expression, ahead: &mut Vec<Statement>) {
        let start = call.range().start;
        let name = format!("{SYNTHETIC}h{}", *self.counter);
        *self.counter += 1;
        let read = Expression::Identifier(name.clone(), SourceRange::new(start, start));
        let mut declaration = Statement::Declaration {
            name,
            annotation: TypeAnnotation::default(),
            type_ref: None,
            mode: DeclarationMode::Ordinary,
            value: std::mem::replace(call, read),
            range: SourceRange::new(start, start),
            shadows: None,
        };
        // The call's own arguments may hold operands that run ahead too; they come first.
        let mut inner = Vec::new();
        self.statement(&mut declaration, &mut inner);
        ahead.append(&mut inner);
        ahead.push(declaration);
    }
}

/// Visits `statement`, the statements nested in its blocks and the ones its expressions hold.
fn visit_statements<'a>(statement: &'a Statement, visit: &mut impl FnMut(&'a Statement)) {
    visit(statement);
    for block in nested_blocks(statement) {
        for inner in block {
            visit_statements(inner, visit);
        }
    }
    for expression in expressions(statement) {
        held_in(expression, visit);
    }
}

fn held_in<'a>(expression: &'a Expression, visit: &mut impl FnMut(&'a Statement)) {
    if let Expression::Statement(statement, _) = expression {
        visit_statements(statement, visit);
    }
    for child in children(expression) {
        held_in(child, visit);
    }
}

/// The value of a number written in the source, with its sign (`3`, `-1`, `2.5`).
fn literal_number(expression: &Expression) -> Option<f64> {
    match expression {
        Expression::Literal(Literal::Int(n), _) => Some(*n as f64),
        Expression::Literal(Literal::Float(n), _) => Some(*n),
        Expression::Unary {
            op: UnaryOperator::Negate,
            operand,
            ..
        } => literal_number(operand).map(|n| -n),
        Expression::Unary {
            op: UnaryOperator::Plus,
            operand,
            ..
        } => literal_number(operand),
        _ => None,
    }
}

/// The names `statement` declares itself (not the blocks nested in it).
fn declared(statement: &Statement, names: &mut HashSet<String>) {
    match statement {
        Statement::Declaration { name, .. } | Statement::ForRange { variable: name, .. } => {
            names.insert(name.clone());
        }
        Statement::TupleDeclaration {
            names: declared, ..
        } => names.extend(declared.iter().cloned()),
        Statement::ForIn { index, value, .. } => {
            names.extend(index.iter().cloned());
            names.insert(value.clone());
        }
        _ => {}
    }
}

/// Every plain or dotted name `expression` reads.
fn identifiers(expression: &Expression, visit: &mut impl FnMut(&str)) {
    match expression {
        Expression::Identifier(name, _) => visit(name),
        Expression::Statement(statement, _) => visit_statements(statement, &mut |inner| {
            for owned in expressions(inner) {
                identifiers(owned, visit);
            }
        }),
        _ => {}
    }
    for child in children(expression) {
        identifiers(child, visit);
    }
}
