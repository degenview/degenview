//! What the expression of a request may take from the strategy.
//!
//! The strategy's state (`strategy.equity`, `strategy.position_size`, `strategy.closedtrades`, ...)
//! belongs to the chart's broker, and a request runs on another series, so TradingView refuses to
//! compile a request whose `expression` carries it: `"strategy.equity" cannot be used with any
//! parameter of the "request.*()" functions.` (CE10059). Measured on a 5-minute BTCUSDT chart,
//! Pine v6:
//!
//! - **Sources**: every `strategy.*` variable, the `strategy.closedtrades.*()` and
//!   `strategy.opentrades.*()` functions, `strategy.convert_to_account()`,
//!   `strategy.convert_to_symbol()` and `strategy.default_entry_qty()`. The constants
//!   (`strategy.long`, `strategy.short`) are not.
//! - **Carried** by whatever is computed from a source, however indirectly and wherever it is
//!   written: a variable (`b = a * 2`), also one reassigned from it *after* the request; a variable
//!   assigned inside `if strategy.position_size > 0`, its `else`, a `switch` or a loop that depends
//!   on it; every name of a tuple one value of which has it; an object or a collection it was put
//!   into; a function or method that reads it anywhere in its body, itself or through a global; a
//!   call given it as an argument, used or not; and a parameter given it, for a request inside the
//!   function.
//! - **Not carried** by the condition of an `else if`, nor of a `switch` arm after the first:
//!   TradingView follows only the first condition of a chain (and the `switch` subject). An `if`
//!   written inside an `else` block is a chain of its own. The value of `x = if …`, `switch` or
//!   `for` comes from that condition and the value each block ends in, not from a line the block
//!   does not use.
//! - **Checked**: the `expression` of `request.security`, `request.security_lower_tf` and
//!   `request.seed`. Not the symbol or timeframe, and not a request merely placed under a strategy
//!   condition. A parameter named like a global is its own, and of several overloads only the one
//!   the call picks counts.
//!
//! So this is flow-insensitive: one pass over the whole script finds every variable and function
//! that carries state, and each request's expression is then looked up in it. A collection or
//! object is changed by a field assignment or a mutating builtin (`array.push`, `m.put`), also
//! through a function that is handed it or reaches it, and through a plain copy of its name.
//! `array.from(len, …)` changes nothing, and neither it nor a copy leads back to its arguments.
//! An element taken into a name of its own (`z = zones.get(0)`, `for z in zones`) is not linked to
//! its collection: not measured, so let through. Which definition a call picks, and whether `s.size()` is a
//! builtin or the script's method, comes from the type checker. Where it cannot tell, a call
//! carries state only if every definition of that name does, so a script that may be fine is not
//! refused.
//! TradingView tells more apart: with `g(v) => f(v)` it picks the overload of `f` per call of `g`.
//!
//! The error is reported on the expression and names a source it comes from.

use std::collections::{HashMap, HashSet};

use crate::pine::compiler::{is_builtin_namespace, registry, symbol_catalog};
use crate::pine::runtime::security_slice::{
    COLLECTION_NAMESPACES, MUTATING_METHODS, collection_argument, request_expression,
};
use crate::pine::syntax::ast::{Argument, Expression, FunctionKind, Parameter, Statement};
use crate::pine::syntax::scopes::{self, Binding, Bindings, head};
use crate::pine::syntax::traversal;
use crate::pine::{Diagnostic, DiagnosticCategory};

/// The requests that run their third argument, `expression`, on another series.
const REQUESTS: [&str; 3] = [
    "request.security",
    "request.security_lower_tf",
    "request.seed",
];

/// One error for each request whose expression carries the strategy's state. `resolutions` and
/// `builtin_methods` are the type checker's: the definition or the builtin each call site picked.
pub fn validate(
    statements: &[Statement],
    resolutions: &HashMap<usize, usize>,
    builtin_methods: &HashMap<usize, String>,
) -> Vec<Diagnostic> {
    let mut expressions = Vec::new();
    traversal::for_each_call_with_args(statements, &mut |name, arguments, _| {
        if REQUESTS.contains(&name)
            && let Some(expression) = request_expression(arguments)
        {
            expressions.push(expression);
        }
    });
    if expressions.is_empty() {
        return Vec::new();
    }
    let script = Script::new(statements, resolutions, builtin_methods);
    let mut carried = Carried::new(&script);
    carried.settle(&script, Pass::Declarations);
    carried.settle(&script, Pass::Arguments);
    expressions
        .into_iter()
        .filter_map(|expression| {
            let source = carried.by(&script, expression)?;
            Some(Diagnostic::error(
                "PINE3074",
                DiagnosticCategory::Semantic,
                format!(
                    "\"{source}\" cannot be used with any parameter of the \"request.*()\" \
                     functions."
                ),
                expression.range(),
            ))
        })
        .collect()
}

/// The members that give back an element of the collection itself, not a new value.
const ELEMENT_ACCESSORS: [&str; 6] = ["get", "first", "last", "pop", "shift", "remove"];

/// The builtin types whose functions can be called as methods of a value.
const METHOD_NAMESPACES: [&str; 10] = [
    "array", "map", "matrix", "line", "label", "box", "table", "linefill", "polyline", "str",
];

/// Whether some builtin type has a method called `member`.
fn is_builtin_method(member: &str) -> bool {
    METHOD_NAMESPACES
        .iter()
        .any(|namespace| registry::is_function(&format!("{namespace}.{member}")))
}

/// The name TradingView prints for a read of the strategy's state, if `name` is one. `call` is
/// whether it is called.
fn state_name(name: &str, call: bool) -> Option<String> {
    let member = name.strip_prefix("strategy.")?;
    if !call {
        return symbol_catalog::is_variable(name).then(|| name.to_owned());
    }
    if member.starts_with("closedtrades.") {
        Some("strategy.closedtrades.*()".to_owned())
    } else if member.starts_with("opentrades.") {
        Some("strategy.opentrades.*()".to_owned())
    } else if member.starts_with("convert_to_") {
        Some("strategy.convert.*()".to_owned())
    } else {
        (member == "default_entry_qty").then(|| name.to_owned())
    }
}

/// A top-level function or method.
struct Function<'a> {
    name: &'a str,
    kind: FunctionKind,
    parameters: &'a [Parameter],
    body: &'a [Statement],
}

/// The expressions of `statement` that TradingView follows: all but the condition of an `else if`
/// (`chained`) and of a `switch` arm after the first.
fn followed(statement: &Statement, chained: bool) -> Vec<&Expression> {
    match statement {
        Statement::Conditional { .. } if chained => Vec::new(),
        Statement::Switch { subject, arms, .. } => subject
            .iter()
            .chain(arms.first().and_then(|arm| arm.condition.as_ref()))
            .collect(),
        _ => traversal::expressions(statement),
    }
}

/// The blocks of `statement`, each with whether it is the `if` of an `else if`.
fn blocks(statement: &Statement) -> Vec<(&[Statement], bool)> {
    match statement {
        Statement::Conditional {
            when_true,
            when_false,
            else_if,
            ..
        } => vec![(when_true, false), (when_false, *else_if)],
        _ => traversal::nested_blocks(statement)
            .into_iter()
            .map(|block| (block, false))
            .collect(),
    }
}

/// Values flowing into variables: what one statement declares or assigns, or one call puts into a
/// collection, and everything that decides what that is (its expressions and the conditions around
/// it).
struct Write<'a> {
    into: Vec<Binding<'a>>,
    from: Vec<&'a Expression>,
}

/// A call of the script's functions.
struct Call<'a> {
    /// The definitions it may run, by position among the top-level functions.
    callees: Vec<usize>,
    expression: &'a Expression,
    /// The conditions around it.
    around: Vec<&'a Expression>,
    /// The function whose body it is in.
    caller: Option<usize>,
}

/// What the script declares and does, in no particular order.
struct Script<'a> {
    bindings: Bindings<'a>,
    resolutions: &'a HashMap<usize, usize>,
    builtin_methods: &'a HashMap<usize, String>,
    /// In source order, which is how `resolutions` and parameter bindings number them.
    functions: Vec<Function<'a>>,
    /// The definitions of each name.
    named: HashMap<&'a str, Vec<usize>>,
    /// The builtin changes and field assignments found, applied once every copy is known.
    mutations: Vec<(Binding<'a>, Vec<&'a Expression>, Option<usize>)>,
    writes: Vec<Write<'a>>,
    calls: Vec<Call<'a>>,
    /// For each function, what a call of it changes: its own parameters and the globals.
    changes: Vec<HashSet<Binding<'a>>>,
    /// The variables that were plainly copied one into the other: names of one collection.
    copies: HashMap<Binding<'a>, Vec<Binding<'a>>>,
}

impl<'a> Script<'a> {
    fn new(
        statements: &'a [Statement],
        resolutions: &'a HashMap<usize, usize>,
        builtin_methods: &'a HashMap<usize, String>,
    ) -> Self {
        let functions: Vec<Function<'a>> = statements
            .iter()
            .filter_map(|statement| match statement {
                Statement::Function {
                    name,
                    kind,
                    parameters,
                    body,
                    ..
                } => Some(Function {
                    name,
                    kind: *kind,
                    parameters,
                    body,
                }),
                _ => None,
            })
            .collect();
        let mut named: HashMap<&str, Vec<usize>> = HashMap::new();
        for (id, function) in functions.iter().enumerate() {
            named.entry(function.name).or_default().push(id);
        }
        let mut script = Self {
            named,
            mutations: Vec::new(),
            bindings: scopes::resolve(statements),
            resolutions,
            builtin_methods,
            changes: vec![HashSet::new(); functions.len()],
            functions,
            writes: Vec::new(),
            calls: Vec::new(),
            copies: HashMap::new(),
        };
        let mut function = 0;
        for statement in statements {
            let within = matches!(statement, Statement::Function { .. }).then(|| {
                function += 1;
                function - 1
            });
            let one = std::slice::from_ref(statement);
            match (within, statement) {
                (Some(id), Statement::Function { body, .. }) => {
                    script.block(body, false, &mut Vec::new(), Some(id));
                }
                _ => script.block(one, false, &mut Vec::new(), None),
            }
        }
        for (holder, from, within) in std::mem::take(&mut script.mutations) {
            script.change(holder, from, within);
        }
        script.follow_calls();
        script
    }

    /// Records what `statements` write and call, under the conditions `around` them, in the body
    /// of the function `within`. `chained` says they are the `if` of an `else if`.
    fn block(
        &mut self,
        statements: &'a [Statement],
        chained: bool,
        around: &mut Vec<&'a Expression>,
        within: Option<usize>,
    ) {
        for statement in statements {
            for expression in traversal::expressions(statement) {
                self.expression(expression, around, within);
            }
            let own = followed(statement, chained);
            let from = || own.iter().chain(around.iter()).copied().collect::<Vec<_>>();
            let mut into: Vec<Binding> = self.bindings.introduced(statement).collect();
            match statement {
                Statement::Declaration { value, .. } => {
                    self.copied(self.bindings.declaration(statement), value);
                }
                Statement::Assignment { value, .. } => {
                    into.extend(self.bindings.target(statement));
                    self.copied(self.bindings.target(statement), value);
                }
                Statement::FieldAssignment { target, .. } => {
                    if let Some(holder) = self.holder(target) {
                        let from = from();
                        self.mutations.push((holder, from, within));
                    }
                }
                _ => {}
            }
            if !into.is_empty() {
                let from = from();
                self.writes.push(Write { into, from });
            }
            // Its own expressions are the conditions of its blocks.
            let depth = around.len();
            around.extend(own);
            for (block, chained) in blocks(statement) {
                self.block(block, chained, around, within);
            }
            around.truncate(depth);
        }
    }

    fn expression(
        &mut self,
        expression: &'a Expression,
        around: &mut Vec<&'a Expression>,
        within: Option<usize>,
    ) {
        if let Expression::Statement(statement, _) = expression {
            let held = std::slice::from_ref(statement.as_ref());
            self.block(held, false, around, within);
        }
        if let Some(holder) = self.changed_by_builtin(expression) {
            let from = std::iter::once(expression)
                .chain(around.iter().copied())
                .collect();
            self.mutations.push((holder, from, within));
        }
        let callees = self.callees(expression);
        if !callees.is_empty() {
            self.calls.push(Call {
                callees,
                expression,
                around: around.clone(),
                caller: within,
            });
        }
        for child in traversal::children(expression) {
            self.expression(child, around, within);
        }
    }

    /// A collection or object is changed, under every name it has. In a function, a global or a
    /// parameter is changed for whoever calls it; a global is so only then, with what that call
    /// passes.
    fn change(&mut self, holder: Binding<'a>, from: Vec<&'a Expression>, within: Option<usize>) {
        let mut into = self.names_of(holder);
        if let Some(function) = within {
            self.changes[function].extend(into.iter().copied().filter(is_seen_by_callers));
            into.retain(|name| !matches!(name, Binding::Global(_)));
        }
        self.writes.push(Write { into, from });
    }

    /// Carries what functions change to the calls of them, through however many functions.
    fn follow_calls(&mut self) {
        let calls = std::mem::take(&mut self.calls);
        let mut grown = true;
        while grown {
            grown = false;
            for call in &calls {
                let Some(caller) = call.caller else { continue };
                for holder in self.changed_by_call(call) {
                    for name in self.names_of(holder) {
                        if is_seen_by_callers(&name) {
                            grown |= self.changes[caller].insert(name);
                        }
                    }
                }
            }
        }
        for call in &calls {
            for holder in self.changed_by_call(call) {
                let from = std::iter::once(call.expression)
                    .chain(call.around.iter().copied())
                    .collect();
                let mut into = self.names_of(holder);
                if call.caller.is_some() {
                    into.retain(|name| !matches!(name, Binding::Global(_)));
                }
                self.writes.push(Write { into, from });
            }
        }
        self.calls = calls;
    }

    /// The variables of the caller that `call` changes.
    fn changed_by_call(&self, call: &Call<'a>) -> Vec<Binding<'a>> {
        let mut changed = Vec::new();
        for &callee in &call.callees {
            for &holder in &self.changes[callee] {
                let Binding::Parameter { position, .. } = holder else {
                    changed.push(holder);
                    continue;
                };
                let receiver = match call.expression {
                    Expression::Call { name, .. } if name.contains('.') => {
                        self.variable(call.expression)
                    }
                    Expression::MethodCall { receiver, .. } => self.holder(receiver),
                    _ => None,
                };
                let given = match receiver {
                    Some(receiver) if position == 0 => Some(receiver),
                    _ => self
                        .arguments(call.expression, callee)
                        .into_iter()
                        .find(|(index, _)| *index == position)
                        .and_then(|(_, value)| self.holder(value)),
                };
                changed.extend(given);
            }
        }
        changed
    }

    /// The script's variable `expression` starts with, not a builtin or a namespace.
    fn variable(&self, expression: &Expression) -> Option<Binding<'a>> {
        self.bindings
            .occurrence(expression)
            .filter(|binding| !matches!(binding, Binding::Free(_)))
    }

    /// The variable holding what `expression` reaches: `zones` for `zones`, `zones.first().hit`
    /// and `array.get(zones, 0)`. A new value (`array.from(a, b)`, `zones.copy()`) is held by
    /// none.
    fn holder(&self, expression: &Expression) -> Option<Binding<'a>> {
        match expression {
            Expression::Identifier(..) => self.variable(expression),
            Expression::Member { base, .. } | Expression::History { base, .. } => self.holder(base),
            Expression::MethodCall { receiver, name, .. } => ELEMENT_ACCESSORS
                .contains(&name.as_str())
                .then(|| self.holder(receiver))
                .flatten(),
            Expression::Call {
                name, arguments, ..
            } => {
                let (receiver, member) = name.rsplit_once('.')?;
                if !ELEMENT_ACCESSORS.contains(&member) {
                    None
                } else if COLLECTION_NAMESPACES.contains(&receiver) {
                    self.holder(collection_argument(arguments)?)
                } else {
                    self.variable(expression)
                }
            }
            _ => None,
        }
    }

    /// The variable a builtin call changes: `array.push(samples, x)`, `samples.push(x)`.
    fn changed_by_builtin(&self, expression: &Expression) -> Option<Binding<'a>> {
        match expression {
            Expression::Call {
                name,
                arguments,
                site,
                ..
            } => {
                let (receiver, member) = name.rsplit_once('.')?;
                if !MUTATING_METHODS.contains(&member) || self.resolutions.contains_key(site) {
                    None
                } else if COLLECTION_NAMESPACES.contains(&receiver) {
                    self.holder(collection_argument(arguments)?)
                } else {
                    self.variable(expression)
                }
            }
            Expression::MethodCall {
                receiver,
                name,
                site,
                ..
            } => (MUTATING_METHODS.contains(&name.as_str())
                && !self.resolutions.contains_key(site))
            .then(|| self.holder(receiver))
            .flatten(),
            _ => None,
        }
    }

    /// Notes `into = from`, a plain copy of one variable into another.
    fn copied(&mut self, into: Option<Binding<'a>>, from: &Expression) {
        if let Some(into) = into
            && let Expression::Identifier(name, _) = from
            && !name.contains('.')
            && let Some(from) = self.variable(from)
        {
            self.copies.entry(into).or_default().push(from);
            self.copies.entry(from).or_default().push(into);
        }
    }

    /// `holder` and every variable it was copied from or into, however many copies apart.
    fn names_of(&self, holder: Binding<'a>) -> Vec<Binding<'a>> {
        let mut names = vec![holder];
        let mut seen = HashSet::from([holder]);
        let mut next = 0;
        while next < names.len() {
            for &other in self.copies.get(&names[next]).into_iter().flatten() {
                if seen.insert(other) {
                    names.push(other);
                }
            }
            next += 1;
        }
        names
    }

    /// The script's definitions the call `expression` may run: the one the type checker picked,
    /// else every one it could name. None for anything else.
    fn callees(&self, expression: &Expression) -> Vec<usize> {
        let (name, site, method) = match expression {
            Expression::Call { name, site, .. } => match name.rsplit_once('.') {
                // `ta.sma` is the builtin, not a method that happens to be called `sma`.
                Some((receiver, member)) => {
                    if !self.resolutions.contains_key(site) && is_builtin_namespace(head(receiver))
                    {
                        return Vec::new();
                    }
                    (member, site, true)
                }
                None => (name.as_str(), site, false),
            },
            Expression::MethodCall { name, site, .. } => (name.as_str(), site, true),
            _ => return Vec::new(),
        };
        if let Some(id) = self.resolutions.get(site) {
            return vec![*id];
        }
        // A builtin method, or on a receiver of unknown type possibly one.
        if self.builtin_methods.contains_key(site) || (method && is_builtin_method(name)) {
            return Vec::new();
        }
        self.named
            .get(name)
            .into_iter()
            .flatten()
            .copied()
            .filter(|id| !method || self.functions[*id].kind == FunctionKind::Method)
            .collect()
    }

    /// The arguments of `call` by the parameter of definition `id` each is given to; the receiver
    /// of a method written `receiver.name(…)` is the first, and is returned apart.
    fn arguments(&self, call: &'a Expression, id: usize) -> Vec<(usize, &'a Expression)> {
        let parameters = self.functions[id].parameters;
        let (arguments, mut position): (&[Argument], usize) = match call {
            Expression::Call {
                name, arguments, ..
            } => (arguments, usize::from(name.contains('.'))),
            Expression::MethodCall { arguments, .. } => (arguments, 1),
            _ => return Vec::new(),
        };
        arguments
            .iter()
            .filter_map(|argument| {
                let index = match &argument.name {
                    Some(label) => parameters.iter().position(|p| &p.name == label)?,
                    None => {
                        position += 1;
                        position - 1
                    }
                };
                Some((index, &argument.value))
            })
            .collect()
    }
}

/// Whether a change of `name` in a function is one its callers see.
fn is_seen_by_callers(name: &Binding<'_>) -> bool {
    matches!(name, Binding::Global(_) | Binding::Parameter { .. })
}

#[derive(Clone, Copy, PartialEq)]
enum Pass {
    /// What the script's variables and functions carry by themselves.
    Declarations,
    /// And what parameters are given at the call sites, for the requests inside functions. What a
    /// function carries by itself stays: each call is judged by its own arguments.
    Arguments,
}

/// The variables and functions that carry the strategy's state, each with a source it comes from.
struct Carried<'a> {
    variables: HashMap<Binding<'a>, String>,
    functions: Vec<Option<String>>,
}

impl<'a> Carried<'a> {
    fn new(script: &Script<'a>) -> Self {
        Self {
            variables: HashMap::new(),
            functions: vec![None; script.functions.len()],
        }
    }

    /// Follows the writes until nothing more carries state.
    fn settle(&mut self, script: &Script<'a>, pass: Pass) {
        let mut changed = true;
        while changed {
            changed = false;
            for write in &script.writes {
                if write.into.iter().all(|b| self.variables.contains_key(b)) {
                    continue;
                }
                if let Some(source) = write.from.iter().find_map(|from| self.by(script, from)) {
                    for binding in &write.into {
                        self.variables
                            .entry(*binding)
                            .or_insert_with(|| source.clone());
                    }
                    changed = true;
                }
            }
            match pass {
                Pass::Declarations => changed |= self.follow_functions(script),
                Pass::Arguments => changed |= self.follow_arguments(script),
            }
        }
    }

    fn follow_functions(&mut self, script: &Script<'a>) -> bool {
        let mut changed = false;
        for (id, function) in script.functions.iter().enumerate() {
            if self.functions[id].is_none()
                && let Some(source) = self.anywhere_in(script, function.body, false)
            {
                self.functions[id] = Some(source);
                changed = true;
            }
        }
        changed
    }

    fn follow_arguments(&mut self, script: &Script<'a>) -> bool {
        let mut found = Vec::new();
        for call in &script.calls {
            for &function in &call.callees {
                let receiver = self.receiver(script, call.expression);
                let given = script
                    .arguments(call.expression, function)
                    .into_iter()
                    .filter_map(|(position, value)| Some((position, self.by(script, value)?)));
                found.extend(
                    receiver
                        .map(|source| (0, source))
                        .into_iter()
                        .chain(given)
                        .map(|(position, source)| {
                            (Binding::Parameter { function, position }, source)
                        }),
                );
            }
        }
        let known = self.variables.len();
        for (parameter, source) in found {
            self.variables.entry(parameter).or_insert(source);
        }
        self.variables.len() > known
    }

    /// The source the variable at the start of `expression` carries.
    fn variable(&self, script: &Script<'a>, expression: &Expression) -> Option<String> {
        let binding = script.bindings.occurrence(expression)?;
        self.variables.get(&binding).cloned()
    }

    /// The source carried by what a method is called on: `a.id()`, `strategy.equity.m()`.
    fn receiver(&self, script: &Script<'a>, call: &Expression) -> Option<String> {
        match call {
            Expression::Call { name, .. } => {
                let (receiver, _) = name.rsplit_once('.')?;
                state_name(receiver, false).or_else(|| self.variable(script, call))
            }
            Expression::MethodCall { receiver, .. } => self.by(script, receiver),
            _ => None,
        }
    }

    /// The source `expression` carries, if it carries the strategy's state at all.
    fn by(&self, script: &Script<'a>, expression: &Expression) -> Option<String> {
        let here = match expression {
            Expression::Identifier(name, _) => {
                state_name(name, false).or_else(|| self.variable(script, expression))
            }
            // `strategy.opentrades.size(0)` is the builtin; `strategy.opentrades.m()` resolved to
            // the script's method is that method on the variable.
            Expression::Call { name, site, .. } => (!script.resolutions.contains_key(site))
                .then(|| state_name(name, true))
                .flatten()
                .or_else(|| self.called(script, expression))
                .or_else(|| self.receiver(script, expression)),
            Expression::MethodCall { .. } => self.called(script, expression),
            Expression::Statement(statement, _) => {
                return self.value_of(script, statement, false);
            }
            _ => None,
        };
        here.or_else(|| {
            traversal::children(expression)
                .into_iter()
                .find_map(|child| self.by(script, child))
        })
    }

    /// What the script's function a call runs carries: all it may run must.
    fn called(&self, script: &Script<'a>, expression: &Expression) -> Option<String> {
        let callees = script.callees(expression);
        let carried = callees.iter().all(|id| self.functions[*id].is_some());
        callees
            .first()
            .filter(|_| carried)
            .and_then(|id| self.functions[*id].clone())
    }

    /// What the value of `statement`, used as one, carries: its followed conditions and what each
    /// of its blocks ends in. A line the value does not use does not count.
    fn value_of(
        &self,
        script: &Script<'a>,
        statement: &Statement,
        chained: bool,
    ) -> Option<String> {
        let of = |binding| self.variables.get(&binding).cloned();
        match statement {
            Statement::Declaration { .. } | Statement::TupleDeclaration { .. } => {
                script.bindings.introduced(statement).find_map(of)
            }
            Statement::Assignment { .. } => script.bindings.target(statement).and_then(of),
            Statement::FieldAssignment { value, .. } | Statement::Expression(value) => {
                self.by(script, value)
            }
            _ => followed(statement, chained)
                .into_iter()
                .find_map(|expression| self.by(script, expression))
                .or_else(|| {
                    blocks(statement)
                        .into_iter()
                        .find_map(|(block, chained)| self.value_of(script, block.last()?, chained))
                }),
        }
    }

    /// What a line held by `expression` carries (`y = if …` in a function), used or not.
    fn held_by(&self, script: &Script<'a>, expression: &Expression) -> Option<String> {
        match expression {
            Expression::Statement(statement, _) => {
                self.anywhere_in(script, std::slice::from_ref(statement.as_ref()), false)
            }
            _ => traversal::children(expression)
                .into_iter()
                .find_map(|child| self.held_by(script, child)),
        }
    }

    /// What a function's body carries: any line of it counts, used or not.
    fn anywhere_in(
        &self,
        script: &Script<'a>,
        statements: &[Statement],
        chained: bool,
    ) -> Option<String> {
        statements.iter().find_map(|statement| {
            followed(statement, chained)
                .into_iter()
                .find_map(|expression| {
                    self.by(script, expression)
                        .or_else(|| self.held_by(script, expression))
                })
                .or_else(|| {
                    blocks(statement)
                        .into_iter()
                        .find_map(|(block, chained)| self.anywhere_in(script, block, chained))
                })
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::pine::compiler::{self, NoLibraries};
    use crate::pine::syntax::token::Limits;

    const REQUEST: &str = "request.security(syminfo.tickerid, \"5\", ";

    /// The messages of the `PINE3074` errors of a strategy that ends in `body`.
    fn errors(body: &str) -> Vec<String> {
        let source = format!("//@version=6\nstrategy(\"S\", overlay = true)\n{body}\nplot(1)\n");
        let program = compiler::compile(&source, Limits::default(), &NoLibraries);
        program
            .diagnostics
            .into_iter()
            .filter(|diagnostic| diagnostic.code == "PINE3074")
            .map(|diagnostic| diagnostic.message)
            .collect()
    }

    fn message(name: &str) -> String {
        format!("\"{name}\" cannot be used with any parameter of the \"request.*()\" functions.")
    }

    fn assert_refused(body: &str, name: &str) {
        assert_eq!(errors(body), [message(name)], "{body}");
    }

    fn assert_compiles(body: &str) {
        let found = errors(body);
        assert!(found.is_empty(), "{body}: {found:?}");
    }

    /// Every case in this test and the ones named `measured_*` was compiled on TradingView.
    #[test]
    fn measured_reads_in_the_expression_are_refused() {
        for name in [
            "equity",
            "position_size",
            "closedtrades",
            "opentrades",
            "netprofit",
            "initial_capital",
            "account_currency",
        ] {
            let name = format!("strategy.{name}");
            assert_refused(&format!("x = {REQUEST}{name})"), &name);
        }
        for body in [
            format!("x = {REQUEST}strategy.equity * 2)"),
            format!("x = {REQUEST}strategy.equity[1])"),
            format!("[x, y] = {REQUEST}[strategy.equity, close])"),
            format!("x = {REQUEST}close > 0 ? strategy.equity : 0.0)"),
            "x = request.security(syminfo.tickerid, \"5\", expression = strategy.equity)".into(),
        ] {
            assert_refused(&body, "strategy.equity");
        }
        for (call, printed) in [
            (
                "strategy.closedtrades.profit(0)",
                "strategy.closedtrades.*()",
            ),
            ("strategy.opentrades.size(0)", "strategy.opentrades.*()"),
            ("strategy.convert_to_account(close)", "strategy.convert.*()"),
            ("strategy.convert_to_symbol(close)", "strategy.convert.*()"),
            (
                "strategy.default_entry_qty(close)",
                "strategy.default_entry_qty",
            ),
        ] {
            assert_refused(&format!("x = {REQUEST}{call})"), printed);
        }
    }

    #[test]
    fn measured_variables_carry_it() {
        let a = "a = strategy.equity\n";
        for body in [
            format!("{a}x = {REQUEST}a)"),
            format!("{a}x = {REQUEST}(a))"),
            format!("{a}x = {REQUEST}a + 1)"),
            format!("{a}x = {REQUEST}a[1])"),
            format!("{a}x = {REQUEST}math.abs(a))"),
            format!("{a}b = a * 2\nx = {REQUEST}b)"),
            format!("{a}b = a\nx = {REQUEST}b)"),
            format!("{a}[x, y] = {REQUEST}[a, close])"),
            format!("{a}[x, y] = {REQUEST}[a + 1, close])"),
            format!("{a}[x, y] = {REQUEST}[close, a + 1])"),
            // Reassigned, before or after the request.
            format!("a = close\na := strategy.equity\nx = {REQUEST}a)"),
            format!("a = close\nx = {REQUEST}a)\na := strategy.equity"),
            // One value of a tuple taints its other names.
            format!("f() => [strategy.equity, close]\n[p, q] = f()\nx = {REQUEST}q)"),
            // A collection it was put into.
            format!(
                "samples = array.new_float()\narray.push(samples, strategy.equity)\n\
                 x = {REQUEST}array.size(samples))"
            ),
        ] {
            assert_refused(&body, "strategy.equity");
        }
        assert_refused(
            &format!("a = close\nif strategy.position_size > 0\n    a := high\nx = {REQUEST}a)"),
            "strategy.position_size",
        );
    }

    #[test]
    fn measured_functions_and_methods_carry_it() {
        let method = "method m(float this) => this + strategy.equity\n";
        for body in [
            format!("f() => strategy.equity\nx = {REQUEST}f())"),
            format!("f() => strategy.equity\ng() => f()\nx = {REQUEST}g())"),
            // Through a global the function reads, or an argument, used or not.
            format!("a = strategy.equity\nf() => a\nx = {REQUEST}f())"),
            format!("a = strategy.equity\nid(v) => v\nx = {REQUEST}id(a))"),
            format!("f(v) => close\nx = {REQUEST}f(strategy.equity))"),
            // A parameter given it, for the request inside the function.
            format!("f(s) => {REQUEST}s)\nx = f(strategy.equity)"),
            // Methods, both ways of calling them, and on a variable that has it.
            format!("{method}x = {REQUEST}close.m())"),
            format!("{method}x = {REQUEST}m(close))"),
            format!("a = strategy.equity\nmethod id(float this) => this\nx = {REQUEST}a.id())"),
            // The overload the call picks.
            format!(
                "f(float v) => strategy.equity + v\nf(int v, int w) => v + w\nx = {REQUEST}f(close))"
            ),
        ] {
            assert_refused(&body, "strategy.equity");
        }
    }

    #[test]
    fn measured_conditions_and_loops_carry_it() {
        let ask = format!("x = {REQUEST}a)");
        let both = "a = 0\nb = 0\n";
        for body in [
            // Loops.
            format!("a = 0\nfor i = 0 to strategy.opentrades\n    a += 1\n{ask}"),
            format!("a = 0\nwhile a < strategy.opentrades\n    a += 1\n{ask}"),
            // The first condition of a chain decides every branch, the `else` included.
            format!("{both}if strategy.opentrades > 0\n    b := 1\nelse\n    a := 1\n{ask}"),
            // So does the `switch` subject and its first arm.
            format!("a = 0\nswitch\n    strategy.opentrades > 0 => a := 1\n{ask}"),
            format!("{both}switch\n    strategy.opentrades > 0 => b := 1\n    => a := 1\n{ask}"),
            format!("{both}switch strategy.opentrades\n    1 => b := 1\n    => a := 1\n{ask}"),
            // An `if` inside a block is its own chain.
            format!(
                "a = 0\nif close > open\n    if strategy.opentrades > 0\n        a := 1\n{ask}"
            ),
            format!(
                "a = 0\nif close > open\n    a := 1\nelse\n    if strategy.opentrades > 0\n        \
                 a := 2\n{ask}"
            ),
            // The value of a block: its condition, or what a branch ends in.
            format!("a = if strategy.opentrades > 0\n    close\nelse\n    open\n{ask}"),
            format!("a = switch\n    strategy.opentrades > 0 => close\n    => high\n{ask}"),
            format!("a = switch\n    close > open => close\n    => strategy.opentrades\n{ask}"),
            format!(
                "a = if close > open\n    z = strategy.opentrades\n    z + 1\nelse\n    1\n{ask}"
            ),
            // A function counts any line of its body.
            format!("f() =>\n    z = strategy.opentrades\n    close\nx = {REQUEST}f())"),
        ] {
            assert_refused(&body, "strategy.opentrades");
        }
    }

    #[test]
    fn measured_later_conditions_and_unused_lines_are_not_followed() {
        let ask = format!("x = {REQUEST}a)");
        let both = "a = 0\nb = 0\n";
        for body in [
            // `else if`, and `switch` arms after the first.
            format!(
                "a = 0\nif close > open\n    a := 1\nelse if strategy.opentrades > 0\n    a := 2\n{ask}"
            ),
            format!(
                "a = 0\nswitch\n    close > open => a := 1\n    strategy.opentrades > 0 => a := 2\n{ask}"
            ),
            format!(
                "{both}switch\n    close > open => b := 1\n    strategy.opentrades > 0 => b := 2\n    \
                 => a := 1\n{ask}"
            ),
            format!(
                "a = if close > open\n    close\nelse if strategy.opentrades > 0\n    open\n{ask}"
            ),
            format!(
                "a = switch\n    close > open => close\n    strategy.opentrades > 0 => open\n    \
                 => high\n{ask}"
            ),
            format!(
                "f() =>\n    if close > open\n        close\n    else if strategy.opentrades > 0\n        \
                 open\nx = {REQUEST}f())"
            ),
            // A line the value of the block does not use.
            format!(
                "a = if close > open\n    z = strategy.opentrades\n    close\nelse\n    open\n{ask}"
            ),
            format!("a = for i = 0 to 1\n    z = strategy.opentrades\n    close\n{ask}"),
            format!(
                "a = close\nif close > open\n    z = strategy.opentrades\n    a := high\n{ask}"
            ),
            // Placing an order is not reading the state.
            format!(
                "a = 0\nif close > open\n    strategy.entry(\"L\", strategy.long)\n    a := 1\n{ask}"
            ),
            // Another collection, another call of the same helper.
            format!(
                "s = array.new_float()\nt = array.new_float()\narray.push(t, strategy.opentrades)\n\
                 x = {REQUEST}array.size(s))"
            ),
            format!("f(v) => v\na = f(strategy.opentrades)\nb = f(close)\nx = {REQUEST}b)"),
        ] {
            assert_compiles(&body);
        }
    }

    #[test]
    fn measured_other_requests_are_checked() {
        for body in [
            "x = request.security_lower_tf(syminfo.tickerid, \"1\", strategy.equity)",
            "x = request.seed(\"seed_crypto_santiment\", \"BTC\", strategy.equity)",
        ] {
            assert_refused(body, "strategy.equity");
        }
    }

    #[test]
    fn measured_what_compiles_is_left_alone() {
        for body in [
            // A constant is not state.
            format!("x = {REQUEST}strategy.long == strategy.short ? close : open)"),
            // State the request does not use.
            format!("a = strategy.equity\nx = {REQUEST}close)\nplot(a)"),
            // The symbol and the timeframe.
            "x = request.security(strategy.equity > 0 ? syminfo.tickerid : \"BINANCE:ETHUSDT\", \"5\", close)".into(),
            "x = request.security(syminfo.tickerid, strategy.equity > 0 ? \"60\" : \"240\", close)".into(),
            // A request merely placed under a strategy condition.
            format!("x = 0.0\nif strategy.position_size > 0\n    x := {REQUEST}close)"),
            // A parameter is not the global it is named like.
            format!("a = strategy.equity\nf(a) => {REQUEST}a)\nx = f(close)\nplot(a)"),
            // An overload the call does not pick.
            format!("f(float v) => strategy.equity + v\nf(int v, int w) => v + w\nx = {REQUEST}f(1, 2))"),
            format!(
                "method m(float this) => this + strategy.equity\nmethod m(int this) => this + 1\n\
                 x = {REQUEST}bar_index.m())"
            ),
        ] {
            assert_compiles(&body);
        }
    }

    #[test]
    fn measured_objects_and_collections_carry_it() {
        let object =
            "type T\n    float v\n    float w\no = T.new(0.0, 0.0)\no.v := strategy.opentrades\n";
        for body in [
            // A field, and its neighbour: the object carries it.
            format!("{object}x = {REQUEST}o.v)"),
            format!("{object}x = {REQUEST}o.w)"),
            // Through an alias, a helper that fills, and a push under a strategy condition.
            format!(
                "s = array.new_float()\nalias = s\narray.push(alias, strategy.opentrades)\n\
                 x = {REQUEST}array.size(s))"
            ),
            format!(
                "s = array.new_float()\nfill(v) => array.push(s, v)\n\
                 fill(strategy.opentrades)\nx = {REQUEST}array.size(s))"
            ),
            format!(
                "s = array.new_float()\nif strategy.opentrades > 0\n    array.push(s, 1)\n\
                 x = {REQUEST}array.size(s))"
            ),
        ] {
            assert_refused(&body, "strategy.opentrades");
        }
    }

    #[test]
    fn collections_local_to_a_block_or_a_function_carry_it() {
        // Compiled on TradingView: a block's own array, and a line of `y = if` in a function.
        for body in [
            format!(
                "total = 0.0\nif close > 0\n    a = array.new_float()\n    \
                 array.push(a, strategy.opentrades)\n    total := array.sum(a)\n\
                 x = {REQUEST}total)"
            ),
            format!(
                "f() =>\n    y = if close > open\n        z = strategy.opentrades\n        \
                 close\n    else\n        open\n    y\nx = {REQUEST}f())"
            ),
            // Not compiled there: a function's own array, handed state through a parameter; a
            // collection reached through a method, an element, or two functions.
            format!(
                "f(v) =>\n    a = array.new_float()\n    array.push(a, v)\n    \
                 {REQUEST}array.sum(a))\nx = f(strategy.opentrades)"
            ),
            format!(
                "method add(array<float> this, float v) => this.push(v)\n\
                 s = array.new_float()\ns.add(strategy.opentrades)\nx = {REQUEST}s.size())"
            ),
            format!(
                "type Z\n    float v\nzones = array.new<Z>()\n\
                 array.get(zones, 0).v := strategy.opentrades\nx = {REQUEST}array.size(zones))"
            ),
            format!(
                "s = array.new_float()\nfill(v) => array.push(s, v)\ng(w) => fill(w)\n\
                 g(strategy.opentrades)\nx = {REQUEST}array.size(s))"
            ),
            // A method called straight on the state.
            format!("method m(float this) => this\nx = {REQUEST}strategy.opentrades.m())"),
        ] {
            assert_refused(&body, "strategy.opentrades");
        }
    }

    #[test]
    fn only_what_is_changed_carries_it() {
        for body in [
            // Compiled on TradingView: `array.from` changes none of its arguments.
            format!(
                "len = 20\na = array.from(len, strategy.opentrades)\n\
                 x = {REQUEST}ta.sma(close, len))\nplot(array.size(a))"
            ),
            // Not compiled there. A field of an element: the collection, not every `array.*`.
            format!(
                "type Z\n    bool hit\nzones = array.new<Z>()\nif strategy.opentrades > 0\n    \
                 array.get(zones, 0).hit := true\nx = {REQUEST}array.avg(array.from(high, low)))"
            ),
            // Only the overload the call picks fills its array.
            format!(
                "s = array.new_float()\nt = array.new_float()\nf(float v) => array.push(s, v)\n\
                 f(int v) => array.push(t, v)\nf(strategy.opentrades)\nx = {REQUEST}array.size(s))"
            ),
            // The builtin `size`, not the script's method for another type.
            format!(
                "type T\n    float v\nmethod size(T this) => strategy.opentrades\n\
                 s = array.new_float()\nx = {REQUEST}s.size())"
            ),
            // A push inside `x = if` gets what is pushed, not the value of the `if`.
            format!(
                "s = array.new_float()\nz = if close > open\n    array.push(s, 1.0)\n    \
                 strategy.opentrades\nelse\n    0\nx = {REQUEST}array.size(s))\nplot(z)"
            ),
        ] {
            assert_compiles(&body);
        }
    }

    #[test]
    fn new_values_and_possible_builtins_are_left_alone() {
        let med = "med(array<float> v) =>\n    v.sort()\n    v.get(0)\n";
        for body in [
            // A new array does not lead back to what it was made from.
            format!(
                "{med}a = close - open\nm = 0.0\nif strategy.opentrades > 0\n    \
                 m := med(array.from(a, high))\nx = {REQUEST}a)\nplot(m)"
            ),
            format!(
                "{med}s = array.new_float()\nm = 0.0\nif strategy.opentrades > 0\n    \
                 m := med(s.copy())\nx = {REQUEST}s.size())\nplot(m)"
            ),
            // On a receiver of unknown type the builtin may be meant.
            format!(
                "type T\n    float v\nmethod size(T this) => strategy.opentrades\n\
                 count(a) => a.size()\nx = {REQUEST}count(array.from(close)))"
            ),
            // A copy of a global in a function nothing calls leaves the global alone.
            format!(
                "s = array.new_float()\nf() =>\n    b = s\n    if strategy.opentrades > 0\n        \
                 b.push(1.0)\n    0\nx = {REQUEST}s.size())"
            ),
        ] {
            assert_compiles(&body);
        }
    }

    #[test]
    fn a_collection_is_changed_under_every_name() {
        let body = format!(
            "fill(array<float> a, float v) =>\n    b = a\n    b.push(v)\ns = array.new_float()\n\
             fill(s, strategy.opentrades)\nx = {REQUEST}array.size(s))"
        );
        assert_refused(&body, "strategy.opentrades");
    }

    /// Where the checker cannot tell which definition a call picks, one that does not carry the
    /// state is enough to leave the script alone. TradingView picks per call of `g`: it compiles
    /// `g(1.5)` and refuses `g(1)`, which is let through here.
    #[test]
    fn what_cannot_be_told_apart_is_left_alone() {
        let defined = "f(int v) => strategy.opentrades\nf(float v) => close\ng(v) => f(v)\n";
        for argument in ["1.5", "1"] {
            assert_compiles(&format!("{defined}x = {REQUEST}g({argument}))"));
        }
    }

    /// Many helpers that call the next twice are worked out once each, not once per path.
    #[test]
    fn a_long_chain_of_helpers_is_walked_once() {
        let mut body = String::from("f0() => close\n");
        for step in 1..40 {
            let previous = step - 1;
            body.push_str(&format!("f{step}() => f{previous}() + f{previous}()\n"));
        }
        body.push_str(&format!("x = {REQUEST}f39())"));
        assert_compiles(&body);
    }

    /// The error is put where the expression starts.
    #[test]
    fn the_error_starts_where_the_expression_does() {
        for expression in ["strategy.equity * 2", "close > 0 ? strategy.equity : 0.0"] {
            let source = format!(
                "//@version=6\nstrategy(\"S\")\nx = request.security(syminfo.tickerid, \"5\", {expression})\nplot(1)\n"
            );
            let program = compiler::compile(&source, Limits::default(), &NoLibraries);
            let at = program
                .diagnostics
                .iter()
                .find(|diagnostic| diagnostic.code == "PINE3074")
                .map(|diagnostic| diagnostic.range.start.offset)
                .expect("an error");
            assert_eq!(Some(at), source.find(expression), "{expression}");
        }
    }
}
