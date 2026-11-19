//! The statements a `request.security` expression needs to run on another series.
//!
//! `request.security(sym, tf, longTrend)` does not read the chart's `longTrend`: TradingView runs
//! the statements that compute it on the requested series. The session cannot run the whole script
//! there (it would draw and trade), so at build time each request gets the *slice* of statements
//! written **before it** that declare or change a name the expression reads, directly or through
//! other slice statements and the script's functions. Those are the top-level statements before the
//! request's own, and the statements before the request in the blocks around it. The slice runs in
//! the request's own state, in source order, before the expression.
//!
//! A statement changes a name when it declares it, assigns it (`x := …`, `x.field := …`), or, for a
//! collection or object, passes it to something that may fill it: `array.push(samples, x)`,
//! `samples.push(x)`, or a call of one of the script's functions that changes the parameter it
//! receives it as (a function that only reads its argument is not replayed). Calls of the script's
//! functions follow the overload the type checker resolved (`Expression::Call::target`).
//!
//! Position matters for a name that is rebound (declared, `x := …`): a variable is computed as it
//! stands where it is read, so what is rebound after the statement that reads it is left out. Not so
//! for a name that is changed through its value (a collection filled later is the same collection),
//! for a `var` or `varip` (a read sees what the previous bar wrote after it) or for one read as
//! `level[1]`.
//!
//! Reads and writes are found scope-aware, on the names `syntax::scopes` resolved once for the whole
//! script: a block's own `level` is not the global of that name, and a function's parameter is not
//! the global it shares a name with. An alias of a collection (`alias = samples`, in a block or a
//! helper too, and one an inner block reassigns) is the collection, a collection argument is found
//! by its label (`id`) or its position, and a function that changes a global collection (not only
//! one it is handed) writes it where it is called. Aliases are per variable (two blocks' `alias`
//! are different locals), a builtin namespace's functions (`ta.sma`) are never the script's `sma`,
//! and a method the type checker resolved to the script's own definition has that definition's
//! effects, not the builtin ones of its name. Known limits: aliasing through a function's return
//! value or a container, and handles other than collections and objects, are not followed.
//!
//! A statement in a block around the request is replayed as written, whatever the block's condition
//! says: the request was reached, so the block ran. It can only be replayed when it declares nothing
//! (that would define a global) and reads no name the blocks declared (the slice has none), and when
//! no loop or `once` runs it more or fewer times than once per bar. A statement that qualifies but
//! cannot be replayed leaves the request without a slice: the expression reads the chart's values as
//! it did before slices.
//!
//! A name the request's own blocks declare before it (`level = 20.0` inside the `if` the request is
//! in, a loop variable) is not the global of that name: the slice does not resolve it, so the
//! expression reads the chart's value as it did before slices.

mod effects;
mod locate;
mod members;
mod reads;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::Arc;

use self::effects::{
    Aliases, Effect, Functions, Origin, Walker, persistent_names, reference_names,
};
use self::locate::{Location, locate};
use self::members::{expression_argument, is_sliced_request, makes_request};
use self::reads::Reads;
use crate::pine::syntax::ast::{Expression, Statement};
use crate::pine::syntax::scopes::{self, Bindings};
use crate::pine::syntax::traversal;

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
    if !has_sliced_request(statements) {
        return Slices::new();
    }
    let Some(script) = Script::new(statements) else {
        return Slices::new();
    };
    let mut cache = FactsCache::default();
    let facts: Vec<usize> = statements
        .iter()
        .map(|statement| cache.id(&script, statement))
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
                let location = locate(statement, key).unwrap_or_default();
                let slice = slice_for(
                    expression,
                    &location,
                    &statements[..index],
                    &facts[..index],
                    &mut cache,
                    &script,
                );
                if !slice.is_empty() {
                    slices.insert(key, Arc::new(slice));
                }
            },
        );
    }
    slices
}

/// Whether a statement outside the functions calls a request that gets a slice.
fn has_sliced_request(statements: &[Statement]) -> bool {
    let mut found = false;
    for statement in statements {
        if matches!(statement, Statement::Function { .. }) {
            continue;
        }
        traversal::for_each_call(std::slice::from_ref(statement), &mut |name, _| {
            found |= is_sliced_request(name);
        });
    }
    found
}

/// What the script declares and does, as `build` needs it.
struct Script<'a> {
    bindings: Bindings<'a>,
    /// The `var` and `varip` names: a read of one sees what the previous bar wrote after it, too.
    persistent: HashSet<&'a str>,
    aliases: Aliases<'a>,
    functions: Functions<'a>,
    /// What the statements outside the functions do, in source order, and where each one's effects
    /// are in it.
    effects: Vec<Effect<'a>>,
    spans: HashMap<*const Statement, Range<usize>>,
}

impl<'a> Script<'a> {
    /// None when the script declares nothing at its top level: no request can read a name it
    /// computes.
    fn new(statements: &'a [Statement]) -> Option<Self> {
        let bindings = scopes::resolve(statements);
        if bindings.globals().is_empty() {
            return None;
        }
        let references = reference_names(statements);
        let functions = Functions::new(&bindings, statements);
        let mut walker = Walker::new(&bindings, &references, &functions);
        walker.spans = Some(HashMap::new());
        for statement in statements {
            if !matches!(statement, Statement::Function { .. }) {
                walker.statement(statement);
            }
        }
        let Walker { effects, spans, .. } = walker;
        Some(Self {
            persistent: persistent_names(statements),
            aliases: Aliases::new(&bindings, statements),
            functions,
            effects,
            spans: spans.unwrap_or_default(),
            bindings,
        })
    }
}

/// What one statement writes (of the names the script declares) and reads.
#[derive(Default)]
struct Facts<'a> {
    /// Names it rebinds: declared at the top level, or reassigned (`x := …`).
    assigned: HashSet<&'a str>,
    /// Names it changes through the value: a collection filled, a field assigned, an argument or a
    /// global a function changes. A collection changed through one name is changed for every name
    /// that aliases it.
    mutated: HashSet<&'a str>,
    /// What it reads, with the functions it calls expanded.
    reads: Reads<'a>,
    /// Whether it reads a name that a block around it declared, which a slice does not have.
    reads_local: bool,
    /// Whether it calls a `request.*` function. Such a statement is never part of a slice: run
    /// inside another request it would nest requests, so a name it computes keeps the chart's value.
    requests: bool,
}

impl<'a> Facts<'a> {
    /// The facts of `statement`, a top-level statement or one in a block.
    fn of(script: &Script<'a>, statement: &'a Statement) -> Self {
        let mut facts = Self::default();
        if matches!(statement, Statement::Function { .. }) {
            return facts;
        }
        let bindings = &script.bindings;
        let span = script
            .spans
            .get(&std::ptr::from_ref(statement))
            .cloned()
            .unwrap_or_default();
        for effect in &script.effects[span] {
            match *effect {
                Effect::Assigned(name) => {
                    facts.assigned.insert(name);
                }
                Effect::Mutated(Origin::Name(name)) => {
                    facts.mutated.extend(script.aliases.group(name));
                    facts.mutated.insert(name);
                }
                Effect::Mutated(Origin::Parameter(_)) => {}
            }
        }
        facts
            .mutated
            .retain(|name| bindings.globals().contains(name));

        let reads = Reads::of_statements(bindings, std::slice::from_ref(statement));
        let start = statement.range().start.offset;
        // A local it reads that was declared before it, not one it declares itself.
        facts.reads_local = reads
            .locals
            .iter()
            .any(|id| bindings.local(*id).site < start);
        facts.reads = script.functions.expanded(reads);
        traversal::for_each_call(std::slice::from_ref(statement), &mut |name, _| {
            facts.requests |= makes_request(name);
        });
        facts
    }

    /// Whether it writes a name the slice needs: it rebinds one that is read after it (`limits` has
    /// the position a name's readers are at), or changes one in any way, since a collection
    /// filled later is the same collection.
    fn writes_needed(&self, position: usize, limits: &HashMap<&str, usize>) -> bool {
        self.assigned
            .iter()
            .any(|name| limits.get(name).is_some_and(|limit| position < *limit))
            || self.mutated.iter().any(|name| limits.contains_key(name))
    }
}

/// The facts of the statements a slice may hold, each worked out once however many requests it
/// precedes.
#[derive(Default)]
struct FactsCache<'a> {
    index: HashMap<*const Statement, usize>,
    facts: Vec<Facts<'a>>,
}

impl<'a> FactsCache<'a> {
    /// Where the facts of `statement` are in `facts`.
    fn id(&mut self, script: &Script<'a>, statement: &'a Statement) -> usize {
        let key = std::ptr::from_ref(statement);
        if let Some(id) = self.index.get(&key) {
            return *id;
        }
        let id = self.facts.len();
        self.facts.push(Facts::of(script, statement));
        self.index.insert(key, id);
        id
    }
}

/// A statement that may be part of a slice, in source order: the top-level ones before the request,
/// then those in the blocks around it.
struct Unit<'a, 'f> {
    statement: &'a Statement,
    facts: &'f Facts<'a>,
    /// Whether the slice can run it on its own, once per bar.
    replayable: bool,
}

/// The slice of the request whose `expression` sits in the top-level statement after `before`
/// (those, with their ids in `cache`) and in the blocks `location` describes.
fn slice_for<'a>(
    expression: &'a Expression,
    location: &Location<'a>,
    before: &'a [Statement],
    before_facts: &[usize],
    cache: &mut FactsCache<'a>,
    script: &Script<'a>,
) -> Vec<Statement> {
    let bindings = &script.bindings;
    let needed = Reads::of_expression(bindings, expression);
    // A name the request's own blocks declare is not the global of that name.
    let start = expression.range().start.offset;
    let local_reads: HashSet<&str> = needed
        .locals
        .iter()
        .map(|id| bindings.local(*id))
        .filter(|local| local.site < start)
        .map(|local| local.name)
        .collect();
    let needed = script.functions.expanded(needed);

    // Only what is written before the request: TradingView computes a variable as it stands where
    // the request is, and the request's own statement would run the request again from inside itself.
    let preceding_facts: Vec<usize> = location
        .preceding
        .iter()
        .map(|before| cache.id(script, before.statement))
        .collect();
    let cache = &*cache;
    let units: Vec<Unit> = before
        .iter()
        .zip(before_facts)
        .map(|(statement, facts)| Unit {
            statement,
            facts: &cache.facts[*facts],
            replayable: true,
        })
        .chain(
            location
                .preceding
                .iter()
                .zip(&preceding_facts)
                .map(|(before, facts)| {
                    let facts = &cache.facts[*facts];
                    Unit {
                        statement: before.statement,
                        facts,
                        // A declaration there would define a global in the slice, and a local it reads is
                        // not defined there.
                        replayable: before.replayable
                            && !facts.reads_local
                            && !matches!(
                                before.statement,
                                Statement::Declaration { .. } | Statement::TupleDeclaration { .. }
                            ),
                    }
                }),
        )
        .collect();

    // For each name the slice needs, the position its readers are at: only what is rebound before
    // that matters, except where a read also sees a later write (a `var`, or a history read, sees
    // the previous bar's).
    let end = units.len();
    let mut limits: HashMap<&str, usize> = needed.names.iter().map(|name| (*name, end)).collect();
    let mut included = vec![false; end];
    loop {
        let mut changed = false;
        for (position, unit) in units.iter().enumerate() {
            if included[position]
                || unit.facts.requests
                || !unit.facts.writes_needed(position, &limits)
            {
                continue;
            }
            // The slice cannot run it, so it cannot compute what the expression needs: the
            // expression reads the chart's values as it did before slices.
            if !unit.replayable {
                return Vec::new();
            }
            included[position] = true;
            changed = true;
            let reads = &unit.facts.reads;
            for name in &reads.names {
                let limit = if reads.history.contains(name) || script.persistent.contains(name) {
                    end
                } else {
                    position
                };
                let slot = limits.entry(name).or_insert(limit);
                *slot = (*slot).max(limit);
            }
        }
        if !changed {
            break;
        }
    }
    // The slice would define a global of that name, which the expression would read in place of
    // the local: no slice, and the request reads the chart's value as before.
    let shadows = included.iter().zip(&units).any(|(included, unit)| {
        *included
            && (!unit.facts.assigned.is_disjoint(&local_reads)
                || !unit.facts.mutated.is_disjoint(&local_reads))
    });
    if shadows {
        return Vec::new();
    }
    units
        .iter()
        .zip(&included)
        .filter(|(_, included)| **included)
        .map(|(unit, _)| unit.statement.clone())
        .collect()
}
