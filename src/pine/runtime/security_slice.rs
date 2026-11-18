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
//! `samples.push(x)`, or a call of one of the script's functions. Calls of the script's functions
//! follow the overload the type checker resolved (`Expression::Call::target`).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

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
const READ_ONLY_MEMBERS: [&str; 18] = [
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
];
/// Members, besides `new…`, that return a collection of their own.
const COLLECTION_RETURNING_MEMBERS: [&str; 5] = ["from", "copy", "slice", "keys", "values"];

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
    let functions = Functions::new(statements);
    let references = reference_names(statements, &functions);
    let facts: Vec<Facts> = statements
        .iter()
        .map(|statement| Facts::of(statement, &declared, &references, &functions))
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
                let slice = slice_for(expression, index, statements, &facts, &functions);
                if !slice.is_empty() {
                    slices.insert(key(expression), Arc::new(slice));
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
        Self { bodies, by_name }
    }

    /// Every function or method called `name`: all overloads, for a call nothing resolved.
    fn named(&self, name: &str) -> &[usize] {
        self.by_name.get(name).map_or(&[], Vec::as_slice)
    }

    fn is_function(&self, name: &str) -> bool {
        self.by_name.contains_key(name)
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
        functions: &Functions,
    ) -> Self {
        let mut writes = HashSet::new();
        let mut reads = Reads::default();
        let mut requests = false;
        if !matches!(statement, Statement::Function { .. }) {
            collect_writes(statement, false, &mut writes);
            collect_call_writes(statement, references, functions, &mut writes);
            writes.retain(|name| declared.contains(name));
            collect_reads(std::slice::from_ref(statement), &mut reads);
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
) -> Vec<Statement> {
    let mut needed = Reads::default();
    read_names(expression, &mut needed);
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
    statements
        .iter()
        .zip(&included)
        .filter(|(_, included)| **included)
        .map(|(statement, _)| statement.clone())
        .collect()
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

/// The declared names that hold a collection or an object, so that passing one to a call may
/// change it: those declared from a constructor (`array.new_float()`, `Point.new()`), a function
/// that returns a collection, or a call of one of the script's own functions.
fn reference_names(statements: &[Statement], functions: &Functions) -> HashSet<String> {
    statements
        .iter()
        .filter_map(|statement| match statement {
            Statement::Declaration {
                name,
                value: Expression::Call { name: callee, .. },
                ..
            } if returns_reference(callee, functions) => Some(name.clone()),
            _ => None,
        })
        .collect()
}

fn returns_reference(callee: &str, functions: &Functions) -> bool {
    let (namespace, member) = callee.rsplit_once('.').unwrap_or(("", callee));
    functions.is_function(callee)
        || member.starts_with("new")
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

/// Names the statement declares at its top level, or reassigns anywhere inside it (`x := …`,
/// `x.field := …`); a declaration inside a block belongs to the block.
fn collect_writes(statement: &Statement, nested: bool, writes: &mut HashSet<String>) {
    match statement {
        Statement::Declaration { name, .. } if !nested => {
            writes.insert(name.clone());
        }
        Statement::TupleDeclaration { names, .. } if !nested => {
            writes.extend(names.iter().cloned())
        }
        Statement::Assignment { name, .. } => {
            writes.insert(name.clone());
        }
        Statement::FieldAssignment { target, .. } => {
            if let Some(root) = root_name(target) {
                writes.insert(root);
            }
        }
        _ => {}
    }
    for block in traversal::nested_blocks(statement) {
        for inner in block {
            collect_writes(inner, true, writes);
        }
    }
}

/// The collections and objects (`references`) the calls in the statement may change.
fn collect_call_writes(
    statement: &Statement,
    references: &HashSet<String>,
    functions: &Functions,
    writes: &mut HashSet<String>,
) {
    let touch = |expression: &Expression, writes: &mut HashSet<String>| {
        if let Some(root) = root_name(expression)
            && references.contains(&root)
        {
            writes.insert(root);
        }
    };
    traversal::for_each_call_with_args(
        std::slice::from_ref(statement),
        &mut |name, arguments, _| {
            let (receiver, member) = match name.rsplit_once('.') {
                Some((receiver, member)) => (Some(receiver), member),
                None => (None, name),
            };
            let changes = !READ_ONLY_MEMBERS.contains(&member);
            match receiver {
                // `array.push(samples, x)`: the collection is the first argument.
                Some(namespace) if COLLECTION_NAMESPACES.contains(&namespace) => {
                    if changes && let Some(first) = arguments.first() {
                        touch(&first.value, writes);
                    }
                }
                // `samples.push(x)`, or a method of the script's on the variable.
                Some(variable) => {
                    let root = head(variable);
                    if changes && references.contains(root) {
                        writes.insert(root.to_owned());
                    }
                    if functions.is_function(member) {
                        for argument in arguments {
                            touch(&argument.value, writes);
                        }
                    }
                }
                // A function of the script may fill anything it is given.
                None => {
                    if functions.is_function(name) {
                        for argument in arguments {
                            touch(&argument.value, writes);
                        }
                    }
                }
            }
        },
    );
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

fn collect_reads(statements: &[Statement], reads: &mut Reads) {
    for statement in statements {
        for expression in traversal::expressions(statement) {
            read_names(expression, reads);
        }
        for block in traversal::nested_blocks(statement) {
            collect_reads(block, reads);
        }
    }
}

/// Every name an expression may depend on: variables (a dotted path counts as its head), and the
/// callees. A call the type checker resolved names its definition; any other may be any
/// definition of that name.
fn read_names(expression: &Expression, reads: &mut Reads) {
    match expression {
        Expression::Identifier(name, _) => {
            reads.names.insert(name.clone());
            reads.names.insert(head(name).to_owned());
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
                reads.names.insert(head(receiver).to_owned());
                if target.is_none() {
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
            collect_reads(std::slice::from_ref(statement.as_ref()), reads);
        }
        _ => {}
    }
    for child in traversal::children(expression) {
        read_names(child, reads);
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
            collect_reads(body, &mut inner);
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
}
