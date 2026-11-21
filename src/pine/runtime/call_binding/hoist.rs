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
//! It runs before `bind`, so a moved call is routed like any other, and before the rename and the scope analysis
//! (`syntax::rename`, the `request.security` slices) so those see the synthetic statements.
//!
//! A *lazy* operand is the right side of `and`/`or`, either arm of `?:`, the arms of an `if` or
//! `switch` used for its value, and a `switch` arm's condition after the first. Only a *total*
//! call moves: running it early cannot fail or change anything, so it cannot change what the script
//! does on the bars its branch skips. Which builtins are total is decided by `total::is_total`; a
//! user function is total when its body, and the defaults of its parameters (which every call that
//! leaves one out evaluates), only compute with total calls, and a loop in it has a trip count that
//! is known and cannot fail (its bounds and step are literals and the step is not zero: a zero step
//! raises). A history is total only at a literal non-negative offset (`close[lag]` fails for a
//! negative `lag`, which a guard may be keeping out). Anything that could fail, change state or draw
//! (`array.get`, `array.push`, `plot`, `runtime.error`, `time`, a method, a field read, `request.*`)
//! stays where it is, as does a call whose arguments read a name the statement itself declares or
//! assigns.
//!
//! A moved call also adds its cost to every bar, and a bar has a budget (PINE8004, and a deadline)
//! that the guard may be keeping the script under: a function that costs more than
//! `RUN_AHEAD_WORK` (statements run and loop passes, the loops of the functions it calls included)
//! stays where it is, however its loops are written, and so does a call whose cost times the passes
//! of the loops around it passes it (`Hoister::passes`). A script close to the budget can still be
//! pushed over it by what does move, and how often a function holding a moved call is called is not
//! counted.
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

#[cfg(test)]
thread_local! {
    /// Makes the pass do nothing on this thread, to run a script as it ran before it (tests).
    pub(crate) static OFF: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Hoists the lazily evaluated history references of `program` and of the libraries it imports.
pub fn lazy_history(program: &mut CompiledProgram) {
    #[cfg(test)]
    if OFF.get() {
        return;
    }
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
        Hoister {
            calls,
            counter,
            passes: 1,
        }
        .block(&mut program.statements);
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
    /// For each of them: what a call costs, as `Work` counts it.
    work: Vec<u64>,
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
            work: Vec::new(),
        };
        // What a call of each function costs, which no other function's purity changes.
        calls.work = Work::new(&calls, &bodies).of_each();
        // Greatest fixpoint: a function is pure unless its body calls something that is not.
        loop {
            let next: Vec<bool> = bodies
                .iter()
                .zip(&calls.pure)
                .zip(&calls.work)
                .map(|(((parameters, body), pure), work)| {
                    *pure && *work <= RUN_AHEAD_WORK && calls.body_is_pure(parameters, body)
                })
                .collect();
            if next == calls.pure {
                return calls;
            }
            calls.pure = next;
        }
    }

    /// A body that only computes: it assigns nothing but its own names and calls only what is safe,
    /// and so do the defaults of its parameters, which every call that leaves one out evaluates.
    fn body_is_pure(&self, parameters: &[Parameter], body: &[Statement]) -> bool {
        let mut locals: HashSet<String> = parameters.iter().map(|p| p.slot.clone()).collect();
        for statement in body {
            visit_statements(statement, &mut |inner| declared(inner, &mut locals));
        }
        parameters.iter().all(|parameter| {
            parameter
                .default_value
                .as_ref()
                .is_none_or(|default| self.expression(default, &locals))
        }) && body
            .iter()
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
            // bar its branch was keeping out. How much a function may cost is `RUN_AHEAD_WORK`.
            Statement::ForRange {
                from,
                to,
                step,
                body,
                ..
            } => trips(from, to, step.as_ref()) != u64::MAX && all(body),
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
                let callees = self.callees(name, site);
                !callees.is_empty() && callees.iter().all(|callee| self.pure[*callee])
            }
        }
    }

    /// The script's functions a call of `name` at `site` may reach.
    fn callees(&self, name: &str, site: usize) -> &[usize] {
        match self.targets.get(&site) {
            Some(target) => std::slice::from_ref(target),
            None => self.by_name.get(name).map_or(&[][..], Vec::as_slice),
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
    /// How many times the statement being walked runs in one run of the function (or the script)
    /// it is in, from the loops around it: a call moved ahead of it runs that often. `u64::MAX`
    /// when a loop's passes are not known.
    passes: u64,
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
                let outside = self.passes;
                self.passes = outside.saturating_mul(trips(from, to, step.as_ref()));
                self.block(body);
                self.passes = outside;
            }
            Statement::ForIn {
                collection, body, ..
            } => {
                self.expression(collection, false, &written, ahead);
                let outside = std::mem::replace(&mut self.passes, u64::MAX);
                self.block(body);
                self.passes = outside;
            }
            // The condition runs again on every pass: nothing to run once ahead of it.
            Statement::WhileLoop { body, .. } => {
                let outside = std::mem::replace(&mut self.passes, u64::MAX);
                self.block(body);
                self.passes = outside;
            }
            // How often a function runs is for its callers to count: a body starts again.
            Statement::Function { body, .. } => {
                let outside = std::mem::replace(&mut self.passes, 1);
                self.block(body);
                self.passes = outside;
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
        // It runs as often as the statement does in a run of its function: the loops around it
        // multiply what it costs (a function it calls only has its own loops counted).
        if Work::known(&self.calls)
            .expression(call)
            .saturating_mul(self.passes)
            > RUN_AHEAD_WORK
        {
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

/// The most a function may cost one call, in statements run and loop passes, to be run ahead. A
/// moved call adds its cost to every bar, and a bar has a budget (`instructions_per_bar`, PINE8004)
/// that the guard may be keeping the script under; this is a small part of it (the budget is 20
/// million, each statement and each loop pass costing at least one).
const RUN_AHEAD_WORK: u64 = 100_000;

/// How many passes a `for` loop makes when its bounds and step are numbers written in the source and
/// the step is not zero; `u64::MAX` for any other loop (what the data decides is unknown, and a zero
/// step raises).
fn trips(from: &Expression, to: &Expression, step: Option<&Expression>) -> u64 {
    let (Some(from), Some(to)) = (literal_number(from), literal_number(to)) else {
        return u64::MAX;
    };
    let stride = match step.map(literal_number) {
        None => 1.0,
        Some(Some(step)) if step != 0.0 => step.abs(),
        Some(_) => return u64::MAX,
    };
    // The counter steps as a float (`i += stride`): past 2^53, or by less than the gap between the
    // floats at the end it reaches (a step of half a gap rounds back to `i` half the time), it
    // stops moving and the loop never ends. Two gaps always move it.
    let reach = from.abs().max(to.abs());
    let ulp = f64::from_bits(reach.to_bits() + 1) - reach;
    if reach > 9_007_199_254_740_992.0 || stride < 2.0 * ulp {
        return u64::MAX;
    }
    // Rounding to the floats near `reach` can take half an ulp off a step.
    let passes = ((to - from).abs() / (stride - ulp / 2.0)).floor() + 1.0;
    if passes >= u64::MAX as f64 {
        u64::MAX
    } else {
        passes as u64
    }
}

/// What running a function costs one call, counted the way `RUN_AHEAD_WORK` is: every statement and
/// expression once, a loop body once per pass, and what a user function it calls costs (the most of
/// the ones the call may reach). Saturates: a loop that is not bounded costs everything.
struct Work<'w, 'c> {
    calls: &'w Calls<'c>,
    bodies: &'w [(&'w [Parameter], &'w [Statement])],
    /// Each function's cost once known.
    memo: std::cell::RefCell<Vec<Option<u64>>>,
    /// The functions being counted, outermost first. A call may reach (by name, where the compiler
    /// did not resolve it, as in a library) the function it is in, or one that calls it: that is
    /// not a recursion, which the compiler refuses, and counts nothing.
    active: std::cell::RefCell<Vec<usize>>,
    /// The outermost function on `active` that a call skipped since the function being counted
    /// began (`usize::MAX`: none), as a position on `active`. What a function costs then lacks the
    /// cost of that callee: it is not complete for the functions above it on the stack, and is
    /// complete for the skipped one (counted again from its own start) and for those below.
    shallowest_skip: std::cell::Cell<usize>,
    /// How many function bodies have been counted. A cycle that is not real (calls resolved by name
    /// reach every overload of a name) makes counting every path through it exponential, so past
    /// `MAX_WALKS` a function is taken to cost everything, which only keeps its calls where they are.
    walks: std::cell::Cell<usize>,
}

/// The function bodies one `Work` may count; a script with functions that call each other by name
/// (a library) can need many more than it has functions, and none needs this many.
const MAX_WALKS: usize = 20_000;

impl<'w, 'c> Work<'w, 'c> {
    fn new(calls: &'w Calls<'c>, bodies: &'w [(&'w [Parameter], &'w [Statement])]) -> Self {
        Self {
            calls,
            bodies,
            memo: std::cell::RefCell::new(vec![None; bodies.len()]),
            active: std::cell::RefCell::default(),
            shallowest_skip: std::cell::Cell::new(usize::MAX),
            walks: std::cell::Cell::new(0),
        }
    }

    /// With the costs of `calls` already worked out (`Calls::work`), for what is not a function.
    fn known(calls: &'w Calls<'c>) -> Self {
        Self {
            calls,
            bodies: &[],
            memo: std::cell::RefCell::new(calls.work.iter().map(|work| Some(*work)).collect()),
            active: std::cell::RefCell::default(),
            shallowest_skip: std::cell::Cell::new(usize::MAX),
            walks: std::cell::Cell::new(0),
        }
    }

    fn of_each(&self) -> Vec<u64> {
        (0..self.bodies.len())
            .map(|index| self.function(index))
            .collect()
    }

    fn function(&self, index: usize) -> u64 {
        if let Some(known) = self.memo.borrow()[index] {
            return known;
        }
        if self.walks.get() >= MAX_WALKS {
            return u64::MAX;
        }
        self.walks.set(self.walks.get() + 1);
        let position = self.active.borrow().len();
        let outer = self.shallowest_skip.replace(usize::MAX);
        self.active.borrow_mut().push(index);
        let (parameters, body) = self.bodies[index];
        let defaults = parameters
            .iter()
            .filter_map(|parameter| parameter.default_value.as_ref())
            .map(|default| self.expression(default))
            .fold(0, u64::saturating_add);
        let cost = self.block(body).saturating_add(defaults);
        self.active.borrow_mut().pop();
        let skipped = self.shallowest_skip.get();
        // A skip of this function or of one counted below it leaves its cost complete.
        let complete = skipped >= position;
        if complete {
            self.memo.borrow_mut()[index] = Some(cost);
        }
        self.shallowest_skip
            .set(outer.min(if complete { usize::MAX } else { skipped }));
        cost
    }

    /// Whether a call to `callee` counts nothing because it is being counted already.
    fn skipped(&self, callee: usize) -> bool {
        let position = self
            .active
            .borrow()
            .iter()
            .position(|active| *active == callee);
        if let Some(position) = position {
            self.shallowest_skip
                .set(self.shallowest_skip.get().min(position));
        }
        position.is_some()
    }

    fn block(&self, statements: &[Statement]) -> u64 {
        statements
            .iter()
            .map(|statement| self.statement(statement))
            .fold(0, u64::saturating_add)
    }

    fn statement(&self, statement: &Statement) -> u64 {
        let own = expressions(statement)
            .into_iter()
            .map(|expression| self.expression(expression))
            .fold(1, u64::saturating_add);
        if let Statement::ForRange {
            from,
            to,
            step,
            body,
            ..
        } = statement
        {
            let passes = trips(from, to, step.as_ref());
            return own.saturating_add(passes.saturating_mul(self.block(body).saturating_add(1)));
        }
        nested_blocks(statement)
            .into_iter()
            .map(|block| self.block(block))
            .fold(own, u64::saturating_add)
    }

    fn expression(&self, expression: &Expression) -> u64 {
        let mut cost = children(expression)
            .into_iter()
            .map(|child| self.expression(child))
            .fold(1, u64::saturating_add);
        match expression {
            Expression::Call { name, site, .. }
                if route_for(name, self.calls.captured).is_none() =>
            {
                let callee = self
                    .calls
                    .callees(name, *site)
                    .iter()
                    .filter(|callee| !self.skipped(**callee))
                    .map(|callee| self.function(*callee))
                    .max()
                    .unwrap_or(0);
                cost = cost.saturating_add(callee);
            }
            Expression::Statement(statement, _) => {
                cost = cost.saturating_add(self.statement(statement));
            }
            _ => {}
        }
        cost
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
