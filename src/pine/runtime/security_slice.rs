//! The statements a `request.security` expression needs to run on another series.
//!
//! `request.security(sym, tf, longTrend)` does not read the chart's `longTrend`: TradingView runs
//! the statements that compute it on the requested series. The session cannot run the whole script
//! there (it would draw and trade), so at build time each request gets the *slice* of top-level
//! statements written **before it** that declare or change a name the expression reads, directly or
//! through other slice statements and the script's functions. The slice runs in the request's own
//! state, in source order, before the expression.
//!
//! A statement changes a name when it declares it, assigns it (`x := …`, `x.field := …`), or, for a
//! collection or object, passes it to something that may fill it: `array.push(samples, x)`,
//! `samples.push(x)`, or a call of one of the script's functions that changes the parameter it
//! receives it as (a function that only reads its argument is not replayed). Calls of the script's
//! functions follow the overload the type checker resolved (`Expression::Call::target`).
//!
//! Reads and writes are found scope-aware: a block's own `level` is not the global of that name, an
//! alias of a collection (`alias = samples`, in a block or a helper too) is the collection, a
//! collection argument is found by its label (`id`) or its position, and a function that changes a
//! global collection (not only one it is handed) writes it where it is called. Aliases are per block
//! (two blocks' `alias` are different variables), a builtin namespace's functions (`ta.sma`) are never
//! the script's `sma`, and a method the type checker resolved to the script's own definition has that
//! definition's effects, not the builtin ones of its name. Known limits:
//! aliasing through a function's return value or a container, and handles other than collections and
//! objects, are not followed.
//!
//! A name the request's own blocks declare before it (`level = 20.0` inside the `if` the request is
//! in, a loop variable) is not the global of that name: the slice does not resolve it, so the
//! expression reads the chart's value as it did before slices.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::pine::compiler::is_builtin_namespace;
use crate::pine::syntax::ast::{Argument, Expression, Parameter, Statement};
use crate::pine::syntax::traversal;

/// The request functions whose third argument is the expression.
const REQUESTS: [&str; 2] = ["request.security", "request.security_lower_tf"];
/// Their parameters in order, the first three being `symbol`, `timeframe` and `expression`.
const EXPRESSION_POSITION: usize = 2;
/// The namespaces whose functions take the collection as their first argument.
const COLLECTION_NAMESPACES: [&str; 3] = ["array", "map", "matrix"];
/// Members that never change the collection they are given. Any other member is taken to, so a
/// function added to the language later errs towards running more, not towards wrong results.
const READ_ONLY_MEMBERS: [&str; 54] = [
    "get",
    "size",
    "first",
    "last",
    "includes",
    "indexof",
    "lastindexof",
    "contains",
    "join",
    "copy",
    "slice",
    "sum",
    "avg",
    "min",
    "max",
    "median",
    "keys",
    "values",
    // Array queries and the functions that return a new array.
    "abs",
    "standardize",
    "binary_search",
    "binary_search_leftmost",
    "binary_search_rightmost",
    "sort_indices",
    "covariance",
    "every",
    "some",
    "mode",
    "range",
    "variance",
    "stdev",
    "percentile_nearest_rank",
    "percentile_linear_interpolation",
    "percentrank",
    // Matrix queries and the functions that return a new matrix.
    "col",
    "columns",
    "det",
    "diff",
    "eigenvalues",
    "eigenvectors",
    "elements_count",
    "inv",
    "is_square",
    "is_stochastic",
    "kron",
    "mult",
    "pinv",
    "pow",
    "rank",
    "row",
    "rows",
    "submatrix",
    "trace",
    "transpose",
];
/// Members, besides `new…`, that return a collection of their own.
const COLLECTION_RETURNING_MEMBERS: [&str; 5] = ["from", "copy", "slice", "keys", "values"];
/// Methods that change the variable they are called on, whatever it is declared from (a collection
/// a script function returned, say).
const MUTATING_METHODS: [&str; 21] = [
    "push",
    "pop",
    "shift",
    "unshift",
    "insert",
    "remove",
    "clear",
    "set",
    "fill",
    "sort",
    "reverse",
    "put",
    "put_all",
    "add_row",
    "add_col",
    "remove_row",
    "remove_col",
    "swap_rows",
    "swap_columns",
    "reshape",
    "concat",
];

/// Where an expression sits in the source (start and end offsets), which is how the runtime finds
/// the slice of the request it evaluates.
pub(crate) type ExpressionKey = (usize, usize);

pub(crate) type Slices = HashMap<ExpressionKey, Arc<Vec<Statement>>>;

pub(crate) fn key(expression: &Expression) -> ExpressionKey {
    let range = expression.range();
    (range.start.offset, range.end.offset)
}

/// The slice of every `request.security` / `request.security_lower_tf` call outside a function
/// (inside one, a parameter may share a name with a global), for the requests that read a name the
/// script declares at its top level.
pub(crate) fn build(statements: &[Statement]) -> Slices {
    let declared = declared_names(statements);
    if declared.is_empty() {
        return Slices::new();
    }
    let references = reference_names(statements);
    let aliases = aliases(statements);
    let functions = Functions::new(statements);
    let facts: Vec<Facts> = statements
        .iter()
        .map(|statement| Facts::of(statement, &declared, &references, &aliases, &functions))
        .collect();

    let mut slices = Slices::new();
    for (index, statement) in statements.iter().enumerate() {
        if matches!(statement, Statement::Function { .. }) {
            continue;
        }
        traversal::for_each_call_with_args(
            std::slice::from_ref(statement),
            &mut |name, arguments, _| {
                let Some(expression) = expression_argument(name, arguments) else {
                    return;
                };
                let key = key(expression);
                let locals =
                    visible_locals(std::slice::from_ref(statement), key, false).unwrap_or_default();
                let slice = slice_for(expression, index, statements, &facts, &functions, &locals);
                if !slice.is_empty() {
                    slices.insert(key, Arc::new(slice));
                }
            },
        );
    }
    slices
}

/// The script's top-level functions and methods. A call's `target` is the position among them.
struct Functions<'a> {
    bodies: Vec<(&'a [Parameter], &'a [Statement])>,
    by_name: HashMap<&'a str, Vec<usize>>,
    /// For each function, the positions of the parameters it may change.
    mutated: Vec<HashSet<usize>>,
    /// For each function, the globals it may change: a collection or object of the script's it
    /// fills or assigns a field of without having been handed it.
    global_writes: Vec<HashSet<String>>,
}

impl<'a> Functions<'a> {
    fn new(statements: &'a [Statement]) -> Self {
        let mut bodies = Vec::new();
        let mut by_name: HashMap<&str, Vec<usize>> = HashMap::new();
        for statement in statements {
            if let Statement::Function {
                name,
                parameters,
                body,
                ..
            } = statement
            {
                by_name.entry(name).or_default().push(bodies.len());
                bodies.push((parameters.as_slice(), body.as_slice()));
            }
        }
        let mut functions = Self {
            mutated: vec![HashSet::new(); bodies.len()],
            global_writes: vec![HashSet::new(); bodies.len()],
            bodies,
            by_name,
        };
        functions.find_effects();
        functions
    }

    /// What each function may change: the parameters it assigns a field of, calls a mutating
    /// function or method on, or hands to a parameter another function changes, and the globals it
    /// does the same to. Repeated until nothing is added, so a chain of calls settles.
    fn find_effects(&mut self) {
        loop {
            let next: Vec<(HashSet<usize>, HashSet<String>)> = (0..self.bodies.len())
                .map(|id| {
                    let (parameters, body) = self.bodies[id];
                    let names: HashSet<String> =
                        parameters.iter().map(|p| p.name.clone()).collect();
                    let walker = Walker {
                        references: &names,
                        functions: self,
                    };
                    let mut effects = Effects::default();
                    let mut scope = Scope::new();
                    for statement in body {
                        walker.statement(statement, &scope, true, &mut effects);
                        declare(statement, &mut scope);
                    }
                    let mut changed: HashSet<usize> = parameters
                        .iter()
                        .enumerate()
                        .filter(|(_, parameter)| effects.mutated.contains(&parameter.name))
                        .map(|(index, _)| index)
                        .collect();
                    changed.extend(self.mutated[id].iter().copied());
                    let mut globals = self.global_writes[id].clone();
                    globals.extend(effects.mutated.difference(&names).cloned());
                    (changed, globals)
                })
                .collect();
            let (mutated, global_writes): (Vec<_>, Vec<_>) = next.into_iter().unzip();
            if mutated == self.mutated && global_writes == self.global_writes {
                return;
            }
            self.mutated = mutated;
            self.global_writes = global_writes;
        }
    }

    /// Every function or method called `name`: all overloads, for a call nothing resolved.
    fn named(&self, name: &str) -> &[usize] {
        self.by_name.get(name).map_or(&[], Vec::as_slice)
    }

    /// The definitions a call may reach: the one the type checker chose, else every one of that
    /// name.
    fn callees(&self, name: &str, target: Option<usize>) -> Vec<usize> {
        match target {
            Some(id) => vec![id],
            None => self.named(name).to_vec(),
        }
    }
}

/// The names an expression or statement reads, and the functions it calls that the type checker
/// resolved to one definition.
#[derive(Default)]
struct Reads {
    names: HashSet<String>,
    targets: HashSet<usize>,
}

impl Reads {
    fn extend(&mut self, other: &Reads) {
        self.names.extend(other.names.iter().cloned());
        self.targets.extend(other.targets.iter().copied());
    }
}

/// What one top-level statement writes (of the names the script declares) and reads.
struct Facts {
    writes: HashSet<String>,
    reads: Reads,
    /// Whether it calls a `request.*` function. Such a statement is never part of a slice: run
    /// inside another request it would nest requests, so a name it computes keeps the chart's value.
    requests: bool,
}

impl Facts {
    fn of(
        statement: &Statement,
        declared: &HashSet<String>,
        references: &HashSet<String>,
        aliases: &Aliases,
        functions: &Functions,
    ) -> Self {
        let mut writes = HashSet::new();
        let mut reads = Reads::default();
        let mut requests = false;
        if !matches!(statement, Statement::Function { .. }) {
            let walker = Walker {
                references,
                functions,
            };
            let mut effects = Effects::default();
            walker.statement(statement, &Scope::new(), false, &mut effects);
            writes = effects.assigned;
            // A collection changed through one name is changed for every name that aliases it.
            for name in effects.mutated {
                writes.extend(aliases.get(&name).into_iter().flatten().cloned());
                writes.insert(name);
            }
            writes.retain(|name| declared.contains(name));
            collect_reads(
                std::slice::from_ref(statement),
                &HashSet::new(),
                false,
                &mut reads,
            );
            traversal::for_each_call(std::slice::from_ref(statement), &mut |name, _| {
                requests |= name.starts_with("request.");
            });
        }
        Self {
            writes,
            reads,
            requests,
        }
    }
}

fn slice_for(
    expression: &Expression,
    own_statement: usize,
    statements: &[Statement],
    facts: &[Facts],
    functions: &Functions,
    locals: &HashSet<String>,
) -> Vec<Statement> {
    let mut needed = Reads::default();
    read_names(expression, &HashSet::new(), &mut needed);
    // A name the request's own blocks declare is not the global of that name.
    let local_reads: HashSet<String> = needed.names.intersection(locals).cloned().collect();
    for name in &local_reads {
        needed.names.remove(name);
    }
    let mut expanded = HashSet::new();
    expand_functions(&mut needed, &mut expanded, functions);

    // Only what is written before the request: TradingView computes a variable as it stands where
    // the request is, and the request's own statement would run the request again from inside itself.
    let mut included = vec![false; own_statement];
    loop {
        let mut changed = false;
        for (index, fact) in facts.iter().enumerate().take(own_statement) {
            if included[index] || fact.requests || fact.writes.is_disjoint(&needed.names) {
                continue;
            }
            included[index] = true;
            changed = true;
            needed.extend(&fact.reads);
            expand_functions(&mut needed, &mut expanded, functions);
        }
        if !changed {
            break;
        }
    }
    // The slice would define a global of that name, which the expression would read in place of
    // the local: no slice, and the request reads the chart's value as before.
    let shadows = included
        .iter()
        .zip(facts)
        .any(|(included, fact)| *included && !fact.writes.is_disjoint(&local_reads));
    if shadows {
        return Vec::new();
    }
    statements
        .iter()
        .zip(&included)
        .filter(|(_, included)| **included)
        .map(|(statement, _)| statement.clone())
        .collect()
}

/// The names the blocks around the expression `key` declare before it, loop variables included, in
/// `block` (`nested` is false for the top level, whose declarations are the globals). None when
/// `key` is not inside `block`.
fn visible_locals(
    block: &[Statement],
    key: ExpressionKey,
    nested: bool,
) -> Option<HashSet<String>> {
    let mut locals = HashSet::new();
    for statement in block {
        if traversal::expressions(statement)
            .into_iter()
            .any(|expression| holds(expression, key))
        {
            return Some(locals);
        }
        for inner in traversal::nested_blocks(statement) {
            if let Some(mut found) = visible_locals(inner, key, true) {
                found.extend(locals);
                declared_by_loop(statement, &mut found);
                return Some(found);
            }
        }
        if nested {
            declared_by(statement, &mut locals);
        }
    }
    None
}

/// Whether the expression, or one inside it, is the one at `key`.
fn holds(expression: &Expression, key: ExpressionKey) -> bool {
    if self::key(expression) == key {
        return true;
    }
    if let Expression::Statement(statement, _) = expression
        && statement_holds(statement, key)
    {
        return true;
    }
    traversal::children(expression)
        .into_iter()
        .any(|child| holds(child, key))
}

fn statement_holds(statement: &Statement, key: ExpressionKey) -> bool {
    traversal::expressions(statement)
        .into_iter()
        .any(|expression| holds(expression, key))
        || traversal::nested_blocks(statement)
            .into_iter()
            .flatten()
            .any(|inner| statement_holds(inner, key))
}

/// The names a statement declares in the block it is in.
fn declared_by(statement: &Statement, names: &mut HashSet<String>) {
    match statement {
        Statement::Declaration { name, .. } => {
            names.insert(name.clone());
        }
        Statement::TupleDeclaration { names: tuple, .. } => names.extend(tuple.iter().cloned()),
        _ => {}
    }
}

/// The variables of a `for` loop, which its body sees.
fn declared_by_loop(statement: &Statement, names: &mut HashSet<String>) {
    match statement {
        Statement::ForRange { variable, .. } => {
            names.insert(variable.clone());
        }
        Statement::ForIn { index, value, .. } => {
            names.extend(index.iter().cloned());
            names.insert(value.clone());
        }
        _ => {}
    }
}

/// The names a script declares at its top level: variables and tuple elements.
fn declared_names(statements: &[Statement]) -> HashSet<String> {
    let mut names = HashSet::new();
    for statement in statements {
        match statement {
            Statement::Declaration { name, .. } => {
                names.insert(name.clone());
            }
            Statement::TupleDeclaration { names: tuple, .. } => names.extend(tuple.iter().cloned()),
            _ => {}
        }
    }
    names
}

/// The declared names known to hold a collection or an object: those declared from a constructor
/// (`array.new_float()`, `Point.new()`). Other names may hold one too (a script function returned
/// it); a call that changes it still shows up as a write through the call's own shape.
fn reference_names(statements: &[Statement]) -> HashSet<String> {
    statements
        .iter()
        .filter_map(|statement| match statement {
            Statement::Declaration {
                name,
                value: Expression::Call { name: callee, .. },
                ..
            } if returns_reference(callee) => Some(name.clone()),
            _ => None,
        })
        .collect()
}

fn returns_reference(callee: &str) -> bool {
    let (namespace, member) = callee.rsplit_once('.').unwrap_or(("", callee));
    member.starts_with("new")
        || (COLLECTION_NAMESPACES.contains(&namespace)
            && COLLECTION_RETURNING_MEMBERS.contains(&member))
}

/// The `expression` argument of a request call, found the way the call binds its arguments: by
/// name, or as the third argument that has none.
fn expression_argument<'a>(name: &str, arguments: &'a [Argument]) -> Option<&'a Expression> {
    if !REQUESTS.contains(&name) {
        return None;
    }
    let mut position = 0;
    for argument in arguments {
        match argument.name.as_deref() {
            Some("expression") => return Some(&argument.value),
            Some(_) => {}
            None => {
                if position == EXPRESSION_POSITION {
                    return Some(&argument.value);
                }
                position += 1;
            }
        }
    }
    None
}

/// The names the blocks around a statement have declared so far, each with the outer names (globals
/// or parameters) it was copied from: `alias = samples` in a block gives `alias -> {samples}`, a
/// plain local an empty set. Aliases are per block: two blocks' `alias` are different variables.
type Scope = HashMap<String, HashSet<String>>;

/// The outer names `value` stands for when it is copied: itself, or what a local it names was copied
/// from.
fn copied_from(value: &Expression, scope: &Scope) -> HashSet<String> {
    match value {
        Expression::Identifier(other, _) if !other.contains('.') => scope
            .get(other)
            .cloned()
            .unwrap_or_else(|| HashSet::from([other.clone()])),
        _ => HashSet::new(),
    }
}

/// What a statement adds to the scope of the block it is in.
fn declare(statement: &Statement, scope: &mut Scope) {
    match statement {
        Statement::Declaration { name, value, .. } => {
            let sources = copied_from(value, scope);
            scope.insert(name.clone(), sources);
        }
        Statement::TupleDeclaration { names, .. } => {
            for name in names {
                scope.insert(name.clone(), HashSet::new());
            }
        }
        Statement::Assignment { name, value, .. } if scope.contains_key(name) => {
            let sources = copied_from(value, scope);
            scope.entry(name.clone()).or_default().extend(sources);
        }
        _ => {}
    }
}

/// The variables of a `for` loop, which its body sees.
fn declare_loop_variables(statement: &Statement, scope: &mut Scope) {
    let mut names = HashSet::new();
    declared_by_loop(statement, &mut names);
    for name in names {
        scope.insert(name, HashSet::new());
    }
}

/// What statements do to the variables they mention.
#[derive(Default)]
struct Effects {
    /// Declared at the top level, or reassigned (`x := …`).
    assigned: HashSet<String>,
    /// Changed through the value: a collection filled, a field assigned, an argument a function
    /// changes, a global a function changes.
    mutated: HashSet<String>,
}

/// Finds the effects of statements. `references` are the names known to hold a collection, for
/// which any method that is not read-only counts as a change.
struct Walker<'w, 'f> {
    references: &'w HashSet<String>,
    functions: &'w Functions<'f>,
}

impl Walker<'_, '_> {
    /// The effects of `statement`. `scope` holds the names the blocks around it have declared so far
    /// (loop variables included): those are the blocks' own, not the globals of the same name.
    /// `nested` is false for a top-level statement, whose declarations are the globals.
    fn statement(&self, statement: &Statement, scope: &Scope, nested: bool, out: &mut Effects) {
        match statement {
            Statement::Declaration { name, .. } if !nested => {
                out.assigned.insert(name.clone());
            }
            Statement::TupleDeclaration { names, .. } if !nested => {
                out.assigned.extend(names.iter().cloned())
            }
            Statement::Assignment { name, .. } if !scope.contains_key(name) => {
                out.assigned.insert(name.clone());
            }
            Statement::FieldAssignment { target, .. } => {
                if let Some(root) = root_name(target) {
                    self.mutate(root, scope, out);
                }
            }
            _ => {}
        }
        for expression in traversal::expressions(statement) {
            self.expression(expression, scope, out);
        }
        for block in traversal::nested_blocks(statement) {
            let mut inner = scope.clone();
            declare_loop_variables(statement, &mut inner);
            for child in block {
                self.statement(child, &inner, true, out);
                declare(child, &mut inner);
            }
        }
    }

    /// `root` is changed. A name a block declared stands for what it was copied from; any other is
    /// a global or a parameter, whose aliases the caller adds.
    fn mutate(&self, root: String, scope: &Scope, out: &mut Effects) {
        match scope.get(&root) {
            Some(sources) => out.mutated.extend(sources.iter().cloned()),
            None => {
                out.mutated.insert(root);
            }
        }
    }

    fn expression(&self, expression: &Expression, scope: &Scope, out: &mut Effects) {
        match expression {
            Expression::Call {
                name,
                arguments,
                target,
                ..
            } => self.call(name, arguments, *target, scope, out),
            Expression::MethodCall {
                receiver,
                name,
                arguments,
                target,
                ..
            } => {
                let root = root_name(receiver);
                // A method the type checker resolved to the script's own definition has that
                // definition's effects, whatever it is called.
                if target.is_none()
                    && MUTATING_METHODS.contains(&name.as_str())
                    && let Some(root) = root.clone()
                {
                    self.mutate(root, scope, out);
                }
                let callees = self.functions.callees(name, *target);
                self.user_call(&callees, root, arguments, scope, out);
            }
            Expression::Statement(statement, _) => self.statement(statement, scope, true, out),
            _ => {}
        }
        for child in traversal::children(expression) {
            self.expression(child, scope, out);
        }
    }

    /// A call by name: `array.push(samples, x)`, `samples.push(x)`, `f(x)` or `samples.add(x)`.
    fn call(
        &self,
        name: &str,
        arguments: &[Argument],
        target: Option<usize>,
        scope: &Scope,
        out: &mut Effects,
    ) {
        let (receiver, member) = match name.rsplit_once('.') {
            Some((receiver, member)) => (Some(receiver), member),
            None => (None, name),
        };
        let changes = !READ_ONLY_MEMBERS.contains(&member);
        match receiver {
            Some(namespace) if COLLECTION_NAMESPACES.contains(&namespace) => {
                if changes
                    && let Some(collection) = collection_argument(arguments)
                    && let Some(root) = root_name(collection)
                {
                    self.mutate(root, scope, out);
                }
            }
            Some(variable) => {
                let root = head(variable);
                if target.is_none()
                    && ((changes && self.references.contains(root))
                        || MUTATING_METHODS.contains(&member))
                {
                    self.mutate(root.to_owned(), scope, out);
                }
                let callees = self.functions.callees(member, target);
                self.user_call(&callees, Some(root.to_owned()), arguments, scope, out);
            }
            None => {
                let callees = self.functions.callees(name, target);
                self.user_call(&callees, None, arguments, scope, out);
            }
        }
    }

    /// A call of the script's functions: what each hands to a parameter it changes (the receiver is
    /// parameter 0, then the arguments in order or by label) and the globals it changes.
    fn user_call(
        &self,
        callees: &[usize],
        receiver: Option<String>,
        arguments: &[Argument],
        scope: &Scope,
        out: &mut Effects,
    ) {
        for &id in callees {
            let changed = &self.functions.mutated[id];
            let (parameters, _) = self.functions.bodies[id];
            out.mutated
                .extend(self.functions.global_writes[id].iter().cloned());
            if let Some(receiver) = &receiver
                && changed.contains(&0)
            {
                self.mutate(receiver.clone(), scope, out);
            }
            let mut position = usize::from(receiver.is_some());
            for argument in arguments {
                let index = match &argument.name {
                    Some(label) => parameters.iter().position(|p| &p.name == label),
                    None => {
                        position += 1;
                        Some(position - 1)
                    }
                };
                if index.is_some_and(|index| changed.contains(&index))
                    && let Some(root) = root_name(&argument.value)
                {
                    self.mutate(root, scope, out);
                }
            }
        }
    }
}

/// The collection a function of `array.*`, `map.*` or `matrix.*` is given: the argument labelled
/// `id` (`id1` for `array.concat`), else the first one without a label.
fn collection_argument(arguments: &[Argument]) -> Option<&Expression> {
    arguments
        .iter()
        .find(|argument| matches!(argument.name.as_deref(), Some("id" | "id1")))
        .or_else(|| arguments.iter().find(|argument| argument.name.is_none()))
        .map(|argument| &argument.value)
}

/// The globals that refer to the same value because one was copied from the other
/// (`alias = samples`, `alias := samples`): each name with every name in its group. A collection
/// changed through one is changed for all. Only globals: a name a block declares is its own, and
/// the walker resolves it per block.
type Aliases = HashMap<String, HashSet<String>>;

fn aliases(statements: &[Statement]) -> Aliases {
    fn copies(
        statements: &[Statement],
        scope: &HashSet<String>,
        nested: bool,
        pairs: &mut Vec<(String, String)>,
    ) {
        let mut scope = scope.clone();
        for statement in statements {
            if matches!(statement, Statement::Function { .. }) {
                continue;
            }
            // A declaration in a block is the block's own variable.
            let copy = match statement {
                Statement::Declaration { name, value, .. } if !nested => Some((name, value)),
                Statement::Assignment { name, value, .. } => Some((name, value)),
                _ => None,
            };
            if let Some((name, value)) = copy
                && let Expression::Identifier(other, _) = value
                && !other.contains('.')
                && !scope.contains(name)
                && !scope.contains(other)
            {
                pairs.push((name.clone(), other.clone()));
            }
            for block in traversal::nested_blocks(statement) {
                let mut inner = scope.clone();
                declared_by_loop(statement, &mut inner);
                copies(block, &inner, true, pairs);
            }
            if nested {
                declared_by(statement, &mut scope);
            }
        }
    }
    let mut pairs = Vec::new();
    copies(statements, &HashSet::new(), false, &mut pairs);
    let mut groups: Vec<HashSet<String>> = Vec::new();
    for (a, b) in pairs {
        let mut merged: HashSet<String> = HashSet::from([a.clone(), b.clone()]);
        groups.retain(|group| {
            let joins = group.contains(&a) || group.contains(&b);
            if joins {
                merged.extend(group.iter().cloned());
            }
            !joins
        });
        groups.push(merged);
    }
    groups
        .into_iter()
        .flat_map(|group| {
            group
                .iter()
                .map(|name| (name.clone(), group.clone()))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// The variable at the start of `a.b[1].c`: `a`.
fn root_name(expression: &Expression) -> Option<String> {
    match expression {
        Expression::Identifier(name, _) | Expression::Call { name, .. } => {
            Some(head(name).to_owned())
        }
        Expression::Member { base, .. } | Expression::History { base, .. } => root_name(base),
        Expression::MethodCall { receiver, .. } => root_name(receiver),
        _ => None,
    }
}

fn head(name: &str) -> &str {
    name.split_once('.').map_or(name, |(head, _)| head)
}

/// What `statements` read. `scope` holds the names the blocks around them declared; a name read
/// from it is the block's own, not a global. `nested` is false for the top level, whose declarations
/// are the globals.
fn collect_reads(
    statements: &[Statement],
    scope: &HashSet<String>,
    nested: bool,
    reads: &mut Reads,
) {
    let mut scope = scope.clone();
    for statement in statements {
        for expression in traversal::expressions(statement) {
            read_names(expression, &scope, reads);
        }
        for block in traversal::nested_blocks(statement) {
            let mut inner = scope.clone();
            declared_by_loop(statement, &mut inner);
            collect_reads(block, &inner, true, reads);
        }
        if nested {
            declared_by(statement, &mut scope);
        }
    }
}

/// Every name an expression may depend on: variables (a dotted path counts as its head), and the
/// callees. A call the type checker resolved names its definition; any other may be any
/// definition of that name.
fn read_names(expression: &Expression, scope: &HashSet<String>, reads: &mut Reads) {
    match expression {
        Expression::Identifier(name, _) => {
            if !scope.contains(name) && !scope.contains(head(name)) {
                reads.names.insert(name.clone());
                reads.names.insert(head(name).to_owned());
            }
        }
        Expression::Call { name, target, .. } => {
            match target {
                Some(id) => {
                    reads.targets.insert(*id);
                }
                None => {
                    reads.names.insert(name.clone());
                }
            }
            if let Some((receiver, member)) = name.rsplit_once('.') {
                if !scope.contains(head(receiver)) {
                    reads.names.insert(head(receiver).to_owned());
                }
                // `ta.sma` is the builtin, not a script function that happens to be called `sma`.
                if target.is_none() && !is_builtin_namespace(head(receiver)) {
                    reads.names.insert(member.to_owned());
                }
            }
        }
        Expression::MethodCall { name, target, .. } => match target {
            Some(id) => {
                reads.targets.insert(*id);
            }
            None => {
                reads.names.insert(name.clone());
            }
        },
        Expression::Statement(statement, _) => {
            collect_reads(std::slice::from_ref(statement.as_ref()), scope, true, reads);
        }
        _ => {}
    }
    for child in traversal::children(expression) {
        read_names(child, scope, reads);
    }
}

/// Adds what the bodies of the functions in `reads` read, however deep the calls go. A function's
/// own parameters and locals are not reads of a global that happens to share the name.
fn expand_functions(reads: &mut Reads, expanded: &mut HashSet<usize>, functions: &Functions) {
    loop {
        let mut pending: Vec<usize> = reads.targets.iter().copied().collect();
        for name in &reads.names {
            pending.extend_from_slice(functions.named(name));
        }
        pending.retain(|id| expanded.insert(*id));
        if pending.is_empty() {
            return;
        }
        for id in pending {
            let (parameters, body) = functions.bodies[id];
            let mut inner = Reads::default();
            collect_reads(body, &HashSet::new(), true, &mut inner);
            let mut locals: HashSet<String> = parameters.iter().map(|p| p.name.clone()).collect();
            collect_locals(body, &mut locals);
            reads.names.extend(inner.names.difference(&locals).cloned());
            reads.targets.extend(inner.targets);
        }
    }
}

/// The names the statements declare, at any depth: variables, tuple elements and loop variables.
fn collect_locals(statements: &[Statement], locals: &mut HashSet<String>) {
    for statement in statements {
        match statement {
            Statement::Declaration { name, .. } => {
                locals.insert(name.clone());
            }
            Statement::TupleDeclaration { names, .. } => locals.extend(names.iter().cloned()),
            Statement::ForRange { variable, .. } => {
                locals.insert(variable.clone());
            }
            Statement::ForIn { index, value, .. } => {
                locals.extend(index.iter().cloned());
                locals.insert(value.clone());
            }
            _ => {}
        }
        for block in traversal::nested_blocks(statement) {
            collect_locals(block, locals);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::compiler::{self, NoLibraries};
    use crate::pine::syntax::token::Limits;

    /// How many statements each request's slice has, in source order of the requests.
    fn slice_sizes(source: &str) -> Vec<usize> {
        let mut program = compiler::compile(source, Limits::default(), &NoLibraries);
        // The session binds calls to their definitions before it builds the slices.
        crate::pine::runtime::call_binding::bind(&mut program);
        let slices = build(&program.statements);
        let mut sizes: Vec<(ExpressionKey, usize)> = slices
            .iter()
            .map(|(key, slice)| (*key, slice.len()))
            .collect();
        sizes.sort();
        sizes.into_iter().map(|(_, size)| size).collect()
    }

    #[test]
    fn a_variable_pulls_in_the_statements_that_compute_it() {
        let sizes = slice_sizes(
            "//@version=6\nindicator(\"T\")\nlength = 3\nbase = ta.sma(close, length)\nunused = open\ntrend = base - 1\nplot(request.security(syminfo.tickerid, \"D\", trend))",
        );
        // `length`, `base`, `trend`; not `unused`.
        assert_eq!(sizes, [3]);
    }

    #[test]
    fn a_reassignment_and_a_called_function_are_followed() {
        let sizes = slice_sizes(
            "//@version=6\nindicator(\"T\")\nfloor_ = 1.0\nlevel = 0.0\nif close > 2\n    level := close\nscaled(x) =>\n    x * floor_\nplot(request.security(syminfo.tickerid, \"D\", scaled(level)))",
        );
        // `floor_` (read by `scaled`), `level`, the `if` that reassigns it.
        assert_eq!(sizes, [3]);
    }

    #[test]
    fn only_code_before_the_request_counts() {
        let sizes = slice_sizes(
            "//@version=6\nindicator(\"T\")\nvalue = close\nx = request.security(syminfo.tickerid, \"D\", value)\nvalue := value * 2\nplot(x)",
        );
        assert_eq!(sizes, [1], "the reassignment below the request is left out");
    }

    #[test]
    fn a_call_that_may_fill_a_collection_is_a_write() {
        let request = "plot(request.security(syminfo.tickerid, \"D\", n))";
        for filler in [
            "array.push(samples, close)",
            "samples.push(close)",
            "fill(samples)",
        ] {
            let sizes = slice_sizes(&format!(
                "//@version=6\nindicator(\"T\")\nfill(a) =>\n    array.push(a, 1.0)\nvar samples = array.new_float()\n{filler}\nn = array.size(samples)\n{request}"
            ));
            // The declaration, the call, `n`.
            assert_eq!(sizes, [3], "{filler}");
        }
        // Reading it is not a write, and a plain number passed to a function is not a collection.
        let sizes = slice_sizes(&format!(
            "//@version=6\nindicator(\"T\")\nid(a) =>\n    a\nlevel = 1.0\nseen = id(level)\nn = level\n{request}"
        ));
        assert_eq!(sizes, [2], "`level` and `n`, not `seen`");
        let sizes = slice_sizes(&format!(
            "//@version=6\nindicator(\"T\")\nsamples = array.new_float()\nfirst = array.get(samples, 0)\nn = array.size(samples)\n{request}"
        ));
        assert_eq!(sizes, [2], "`samples` and `n`; `first` only reads it");
    }

    #[test]
    fn a_function_that_only_reads_its_argument_is_not_a_write() {
        let sizes = slice_sizes(
            "//@version=6\nindicator(\"T\")\nid(a) =>\n    a\nchecked(v) =>\n    runtime.error(\"x\")\n    v\nlevel = id(close)\nunused = checked(level)\nplot(request.security(syminfo.tickerid, \"D\", level))",
        );
        assert_eq!(sizes, [1], "only `level`");
        // One that fills it, directly or through another function, is.
        let sizes = slice_sizes(
            "//@version=6\nindicator(\"T\")\nmakeArray() =>\n    array.new_float()\nadd(a) =>\n    a.push(1.0)\nfill(a) =>\n    add(a)\nsamples = makeArray()\nfill(samples)\nn = array.size(samples)\nplot(request.security(syminfo.tickerid, \"D\", n))",
        );
        assert_eq!(sizes, [3], "`samples`, `fill(samples)` and `n`");
    }

    #[test]
    fn a_name_the_requests_blocks_declare_is_not_the_global() {
        let request = "request.security(syminfo.tickerid, \"D\", level)";
        let sizes = slice_sizes(&format!(
            "//@version=6\nindicator(\"T\")\nlevel = 10.0\nvar float x = na\nif close > 0\n    level = 20.0\n    x := {request}\nplot(x)"
        ));
        assert!(sizes.is_empty(), "{sizes:?}: the block's own `level`");
        // Declared after the request, in the block, it is not visible yet: the global is.
        let sizes = slice_sizes(&format!(
            "//@version=6\nindicator(\"T\")\nlevel = 10.0\nvar float x = na\nif close > 0\n    x := {request}\n    level = 20.0\nplot(x)"
        ));
        assert_eq!(sizes, [1]);
        // A loop variable is local to its loop, whatever the globals are called.
        let sizes = slice_sizes(
            "//@version=6\nindicator(\"T\")\ni = 5\nvar float x = na\nfor i = 0 to 2\n    x := request.security(syminfo.tickerid, \"D\", close[i])\nplot(x)",
        );
        assert!(sizes.is_empty(), "{sizes:?}");
        // At the top level nothing is local.
        let sizes = slice_sizes(&format!(
            "//@version=6\nindicator(\"T\")\nlevel = 10.0\nplot({request})"
        ));
        assert_eq!(sizes, [1]);
    }

    #[test]
    fn a_global_that_would_shadow_the_local_cancels_the_slice() {
        // `other` needs the global `level`, which the expression's own `level` must not see.
        let sizes = slice_sizes(
            "//@version=6\nindicator(\"T\")\nlevel = 10.0\nother = level * 2\nvar float x = na\nif close > 0\n    level = 20.0\n    x := request.security(syminfo.tickerid, \"D\", level + other)\nplot(x)",
        );
        assert!(sizes.is_empty(), "{sizes:?}");
    }

    #[test]
    fn a_block_local_assignment_is_not_a_write_of_the_global() {
        let sizes = slice_sizes(
            "//@version=6\nindicator(\"T\")\nlevel = close\nif close > 50\n    level = 5.0\n    level += 1\nplot(request.security(syminfo.tickerid, \"D\", level))",
        );
        assert_eq!(sizes, [1], "only the global `level`");
        // `:=` on a name the block did not declare is the global's.
        let sizes = slice_sizes(
            "//@version=6\nindicator(\"T\")\nlevel = close\nif close > 50\n    level += 1\nplot(request.security(syminfo.tickerid, \"D\", level))",
        );
        assert_eq!(sizes, [2]);
        // The same for the collection a block makes for itself, and for a loop variable.
        let sizes = slice_sizes(
            "//@version=6\nindicator(\"T\")\nsamples = array.new_float()\nif close > 50\n    samples = array.new_float()\n    samples.push(1.0)\nplot(request.security(syminfo.tickerid, \"D\", array.size(samples)))",
        );
        assert_eq!(sizes, [1]);
    }

    #[test]
    fn a_mutation_through_an_alias_is_a_write_of_the_collection() {
        let sizes = slice_sizes(
            "//@version=6\nindicator(\"T\")\nvar samples = array.new_float()\nalias = samples\nalias.push(close)\nplot(request.security(syminfo.tickerid, \"D\", array.size(samples)))",
        );
        assert_eq!(sizes, [3], "`samples`, `alias = samples`, the push");
        // A number copied from another is a value, not an alias.
        let sizes = slice_sizes(
            "//@version=6\nindicator(\"T\")\nlevel = close\ncopy = level\ncopy += 1\nplot(request.security(syminfo.tickerid, \"D\", level))",
        );
        assert_eq!(sizes, [1]);
    }

    #[test]
    fn a_function_that_changes_a_global_is_a_write_of_it() {
        let sizes = slice_sizes(
            "//@version=6\nindicator(\"T\")\nvar samples = array.new_float()\naddSample() =>\n    array.push(samples, close)\nrecord() =>\n    addSample()\nrecord()\nplot(request.security(syminfo.tickerid, \"D\", array.size(samples)))",
        );
        assert_eq!(sizes, [2], "`samples` and `record()`");
        // One that only reads it is not.
        let sizes = slice_sizes(
            "//@version=6\nindicator(\"T\")\nvar samples = array.new_float()\npeek() =>\n    array.size(samples)\nunused = peek()\nplot(request.security(syminfo.tickerid, \"D\", array.size(samples)))",
        );
        assert_eq!(sizes, [1]);
    }

    #[test]
    fn only_the_overload_called_decides_what_is_written() {
        let sizes = slice_sizes(
            "//@version=6\nindicator(\"T\")\nchecked(float v) =>\n    v\nchecked(array<float> a) =>\n    array.push(a, 1.0)\n    1.0\nlevel = close\nunused = checked(level)\nplot(request.security(syminfo.tickerid, \"D\", level))",
        );
        assert_eq!(sizes, [1]);
    }

    #[test]
    fn a_block_reading_its_own_variable_is_not_a_read_of_the_global() {
        let sizes = slice_sizes(
            "//@version=6\nindicator(\"T\")\nbad = close * 3\nlevel = close\nif close > 0\n    bad = close\n    level := bad\nplot(request.security(syminfo.tickerid, \"D\", level))",
        );
        assert_eq!(sizes, [2], "`level` and the block, not the global `bad`");
        // Read before the block declares it, it is the global's.
        let sizes = slice_sizes(
            "//@version=6\nindicator(\"T\")\nbad = close * 3\nlevel = close\nif close > 0\n    level := bad\n    bad = close\nplot(request.security(syminfo.tickerid, \"D\", level))",
        );
        assert_eq!(sizes, [3]);
    }

    #[test]
    fn a_local_alias_of_a_collection_writes_the_collection() {
        let sizes = slice_sizes(
            "//@version=6\nindicator(\"T\")\nvar samples = array.new_float()\nif close > 0\n    alias = samples\n    alias.push(close)\nplot(request.security(syminfo.tickerid, \"D\", array.size(samples)))",
        );
        assert_eq!(sizes, [2], "`samples` and the block");
        let sizes = slice_sizes(
            "//@version=6\nindicator(\"T\")\nvar samples = array.new_float()\nfill(a) =>\n    alias = a\n    alias.push(close)\nfill(samples)\nplot(request.security(syminfo.tickerid, \"D\", array.size(samples)))",
        );
        assert_eq!(sizes, [2], "`samples` and `fill(samples)`");
    }

    #[test]
    fn the_collection_of_a_builtin_is_found_by_label_or_position() {
        for call in [
            "array.push(value = close, id = samples)",
            "array.push(samples, value = close)",
        ] {
            let sizes = slice_sizes(&format!(
                "//@version=6\nindicator(\"T\")\nvar samples = array.new_float()\n{call}\nplot(request.security(syminfo.tickerid, \"D\", array.size(samples)))"
            ));
            assert_eq!(sizes, [2], "{call}");
        }
    }

    #[test]
    fn a_call_follows_the_overload_the_type_checker_chose() {
        let sizes = slice_sizes(
            "//@version=6\nindicator(\"T\")\nbad = close\nf(float v) =>\n    v * 2\nf(string s) =>\n    str.tostring(bad)\nplot(request.security(syminfo.tickerid, \"D\", f(close)))",
        );
        assert!(
            sizes.is_empty(),
            "{sizes:?}: `bad` belongs to the string one"
        );
        let sizes = slice_sizes(
            "//@version=6\nindicator(\"T\")\nbad = close\nf(float v) =>\n    v * 2\nf(string s) =>\n    str.tostring(bad)\nplot(request.security(syminfo.tickerid, \"D\", f(\"a\")))",
        );
        assert_eq!(sizes, [1]);
    }

    #[test]
    fn builtins_and_the_requests_own_statement_need_no_slice() {
        assert!(
            slice_sizes(
                "//@version=6\nindicator(\"T\")\nx = request.security(syminfo.tickerid, \"D\", close)\nplot(x)"
            )
            .is_empty()
        );
        assert!(
            slice_sizes(
                "//@version=6\nindicator(\"T\")\nx = close\ny = request.security(syminfo.tickerid, \"D\", y[1])\nplot(y)"
            )
            .is_empty(),
            "a statement is not its own dependency"
        );
    }

    #[test]
    fn a_function_local_is_not_a_read_of_the_global_with_its_name() {
        let sizes = slice_sizes(
            "//@version=6\nindicator(\"T\")\nup = close > open\nf() =>\n    up = close < open\n    up ? 1 : 0\nplot(request.security(syminfo.tickerid, \"D\", f()))",
        );
        assert!(sizes.is_empty(), "{sizes:?}");
    }

    #[test]
    fn a_statement_that_makes_a_request_is_left_out() {
        let sizes = slice_sizes(
            "//@version=6\nindicator(\"T\")\ndaily = request.security(syminfo.tickerid, \"D\", close)\nscaled = daily * 2\nplot(request.security(syminfo.tickerid, \"W\", scaled))",
        );
        // `scaled` is computed on the weekly series from the chart's `daily`.
        assert_eq!(sizes, [1]);
    }

    #[test]
    fn the_expression_is_found_by_name_too() {
        let sizes = slice_sizes(
            "//@version=6\nindicator(\"T\")\nx = close * 2\nplot(request.security(syminfo.tickerid, \"D\", expression = x))",
        );
        assert_eq!(sizes, [1]);
    }

    #[test]
    fn two_blocks_alias_names_do_not_merge() {
        let sizes = slice_sizes(
            "//@version=6\nindicator(\"T\")\nvar samples = array.new_float()\nvar other = array.new_float()\nif close > 0\n    alias = samples\n    alias.push(close)\nif close > 0\n    alias = other\n    alias.push(array.get(other, 5))\nplot(request.security(syminfo.tickerid, \"D\", array.size(samples)))",
        );
        assert_eq!(sizes, [2], "`samples` and the first block only");
    }

    #[test]
    fn a_builtin_is_not_a_script_function_of_the_same_name() {
        let sizes = slice_sizes(
            "//@version=6\nindicator(\"T\")\nbad = close\nsma(float src, int len) =>\n    src + bad\nplot(request.security(syminfo.tickerid, \"D\", ta.sma(close, 2)))",
        );
        assert!(sizes.is_empty(), "{sizes:?}");
    }

    #[test]
    fn a_read_only_query_is_not_a_write_of_the_collection() {
        for query in [
            "array.percentrank(samples, 0)",
            "array.stdev(samples)",
            "array.percentile_linear_interpolation(samples, 50)",
        ] {
            let sizes = slice_sizes(&format!(
                "//@version=6\nindicator(\"T\")\nvar samples = array.new_float()\narray.push(samples, close)\nunused = {query}\nplot(request.security(syminfo.tickerid, \"D\", array.size(samples)))"
            ));
            assert_eq!(sizes, [2], "{query}: the declaration and the push");
        }
    }

    #[test]
    fn a_user_method_has_its_own_effects_not_the_builtin_ones_of_its_name() {
        let sizes = slice_sizes(
            "//@version=6\nindicator(\"T\")\nbad = close\nlevel = close\nmethod push(float this, float v) =>\n    v + bad\nr = level.push(1.0)\nplot(request.security(syminfo.tickerid, \"D\", level))",
        );
        assert_eq!(sizes, [1], "`level` alone");
    }
}
