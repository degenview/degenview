//! What statements do to the variables they mention: which they rebind, and which collections or
//! objects they may change.
//!
//! The effects of the script's functions come first (`Functions`, a fixpoint over the call graph),
//! then one walk over the script's own statements records the effects of each in an event log, from
//! which `Facts` reads them back per statement.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use super::members::{
    COLLECTION_NAMESPACES, MUTATING_METHODS, READ_ONLY_MEMBERS, collection_argument,
    returns_reference,
};
use super::reads::Reads;
use crate::pine::compiler::is_builtin_namespace;
use crate::pine::syntax::ast::{Argument, DeclarationMode, Expression, Parameter, Statement};
use crate::pine::syntax::scopes::{Binding, Bindings, LocalId, head};
use crate::pine::syntax::traversal;

/// A name a change reaches, once the locals that were copied from it are looked through.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum Origin<'a> {
    /// A global or a builtin, by its name in the variable table.
    Name(&'a str),
    /// A parameter of the function being walked, by position.
    Parameter(usize),
}

/// One thing a statement does.
#[derive(Clone, Copy, Debug)]
pub(super) enum Effect<'a> {
    /// A top-level declaration, or a global reassigned (`x := …`).
    Assigned(&'a str),
    /// Changed through the value: a collection filled, a field assigned, an argument or a global a
    /// function changes. A collection changed through one name is changed for every name that
    /// aliases it.
    Mutated(Origin<'a>),
}

/// The script's top-level functions and methods. A call's `target` is the position among them.
pub(super) struct Functions<'a> {
    pub bodies: Vec<(&'a [Parameter], &'a [Statement])>,
    /// For each function, what its body reads of the script's names: a parameter, or a variable the
    /// body declares before it reads it, is the function's own.
    reads: Vec<Reads<'a>>,
    by_name: HashMap<&'a str, Vec<usize>>,
    /// For each function, the positions of the parameters it may change.
    pub mutated: Vec<HashSet<usize>>,
    /// For each function, the globals it may change: a collection or object of the script's it
    /// fills or assigns a field of without having been handed it.
    pub global_writes: Vec<HashSet<&'a str>>,
}

impl<'a> Functions<'a> {
    pub fn new(bindings: &Bindings<'a>, statements: &'a [Statement]) -> Self {
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
        let reads = bodies
            .iter()
            .map(|(_, body)| Reads::of_statements(bindings, body))
            .collect();
        let mut functions = Self {
            reads,
            mutated: vec![HashSet::new(); bodies.len()],
            global_writes: vec![HashSet::new(); bodies.len()],
            bodies,
            by_name,
        };
        functions.find_effects(bindings);
        functions
    }

    /// What each function may change: the parameters it assigns a field of, calls a mutating
    /// function or method on, or hands to a parameter another function changes, and the globals it
    /// does the same to. Repeated until nothing is added, so a chain of calls settles.
    fn find_effects(&mut self, bindings: &Bindings<'a>) {
        loop {
            let next: Vec<(HashSet<usize>, HashSet<&'a str>)> = (0..self.bodies.len())
                .map(|id| {
                    let (parameters, body) = self.bodies[id];
                    // Inside a function, a collection is known by being handed in.
                    let references: HashSet<&str> =
                        parameters.iter().map(|p| p.name.as_str()).collect();
                    let mut walker = Walker::new(bindings, &references, self);
                    for statement in body {
                        walker.statement(statement);
                    }
                    let mut changed = self.mutated[id].clone();
                    let mut globals = self.global_writes[id].clone();
                    for effect in walker.effects {
                        let Effect::Mutated(origin) = effect else {
                            continue;
                        };
                        match origin {
                            Origin::Parameter(position) => {
                                changed.insert(position);
                            }
                            // A global's name that a parameter is also called is that parameter:
                            // the variable table is keyed by name.
                            Origin::Name(name) => {
                                let mut handed_in = false;
                                for (position, parameter) in parameters.iter().enumerate() {
                                    if parameter.name == name {
                                        changed.insert(position);
                                        handed_in = true;
                                    }
                                }
                                if !handed_in {
                                    globals.insert(name);
                                }
                            }
                        }
                    }
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

    /// `reads` and what the bodies of the functions it reaches read, however deep the calls go.
    pub fn expanded(&self, mut reads: Reads<'a>) -> Reads<'a> {
        let mut seen = HashSet::new();
        loop {
            let mut pending: Vec<usize> = reads.targets.iter().copied().collect();
            for name in &reads.names {
                pending.extend_from_slice(self.named(name));
            }
            pending.retain(|id| seen.insert(*id));
            if pending.is_empty() {
                return reads;
            }
            for id in pending {
                reads.extend(&self.reads[id]);
            }
        }
    }

    /// Every function or method called `name`: all overloads, for a call nothing resolved.
    fn named(&self, name: &str) -> &[usize] {
        self.by_name.get(name).map_or(&[], Vec::as_slice)
    }

    /// The definitions a call may reach: the one the type checker chose, else every one of that
    /// name.
    fn callees<'s>(&'s self, name: &str, target: &'s Option<usize>) -> &'s [usize] {
        match target {
            Some(id) => std::slice::from_ref(id),
            None => self.named(name),
        }
    }
}

/// Finds the effects of statements, in source order. `references` are the names known to hold a
/// collection, for which any method that is not read-only counts as a change.
///
/// A local that is a plain copy of another name (`alias = samples`) stands for what it was copied
/// from, so changing it changes that. What a local was copied from (`sources`) changes as the walk
/// goes: a reassignment (`alias := other`) adds a source. One in a block counts for the rest of the
/// block and, once the statement the block belongs to is done, for the enclosing one, never for a
/// sibling block.
pub(super) struct Walker<'w, 'a> {
    bindings: &'w Bindings<'a>,
    references: &'w HashSet<&'a str>,
    functions: &'w Functions<'a>,
    /// For each local, the outer names it was copied from: an empty list for a plain value.
    sources: Vec<Vec<Origin<'a>>>,
    /// The sources added to locals since the walk began, innermost block last.
    additions: Vec<(LocalId, Origin<'a>)>,
    /// The additions made by the blocks of the statements being walked, to apply when the
    /// statement ends.
    pending: Vec<(LocalId, Origin<'a>)>,
    pub effects: Vec<Effect<'a>>,
    /// Where in `effects` the effects of each statement of a block are. Kept when asked for.
    pub spans: Option<HashMap<*const Statement, Range<usize>>>,
}

impl<'w, 'a> Walker<'w, 'a> {
    pub fn new(
        bindings: &'w Bindings<'a>,
        references: &'w HashSet<&'a str>,
        functions: &'w Functions<'a>,
    ) -> Self {
        Self {
            bindings,
            references,
            functions,
            sources: vec![Vec::new(); bindings.local_count()],
            additions: Vec::new(),
            pending: Vec::new(),
            effects: Vec::new(),
            spans: None,
        }
    }

    /// The effects of `statement`, one of a block or the program.
    pub fn statement(&mut self, statement: &'a Statement) {
        let start = self.effects.len();
        let pending = self.pending.len();
        self.visit(statement);
        while self.pending.len() > pending {
            if let Some((local, origin)) = self.pending.pop() {
                self.add(local, origin);
            }
        }
        self.copy(statement);
        let end = self.effects.len();
        if let Some(spans) = &mut self.spans {
            spans.insert(std::ptr::from_ref(statement), start..end);
        }
    }

    fn visit(&mut self, statement: &'a Statement) {
        match statement {
            Statement::Declaration { .. } | Statement::TupleDeclaration { .. } => {
                for binding in self.bindings.introduced(statement) {
                    if let Binding::Global(name) = binding {
                        self.effects.push(Effect::Assigned(name));
                    }
                }
            }
            Statement::Assignment { .. } => {
                if let Some(Binding::Global(name)) = self.bindings.target(statement) {
                    self.effects.push(Effect::Assigned(name));
                }
            }
            Statement::FieldAssignment { target, .. } => {
                if let Some(root) = self.bindings.root(target) {
                    self.mutate(root);
                }
            }
            _ => {}
        }
        for expression in traversal::expressions(statement) {
            self.expression(expression);
        }
        for block in traversal::nested_blocks(statement) {
            self.block(statement, block);
        }
    }

    /// A block of `owner`: the sources it adds to the locals outside `owner` wait for `owner` to be
    /// done, and the walk of the next block starts from where this one began.
    fn block(&mut self, owner: &'a Statement, block: &'a [Statement]) {
        let mark = self.additions.len();
        for binding in self.bindings.introduced(owner) {
            if let Binding::Local(id) = binding {
                self.sources[id].clear();
            }
        }
        for statement in block {
            self.statement(statement);
        }
        if self.additions.len() == mark {
            return;
        }
        let outside = owner.range().start.offset;
        let redeclared = self.redeclared(owner, block);
        while self.additions.len() > mark {
            if let Some((local, origin)) = self.additions.pop() {
                self.sources[local].retain(|source| *source != origin);
                let info = self.bindings.local(local);
                if info.site < outside && !redeclared.contains(info.name) {
                    self.pending.push((local, origin));
                }
            }
        }
    }

    /// The names `block`, or the loop `owner`, declares variables of its own for, anywhere in it.
    /// The sources a block adds to an outer local of such a name are dropped, even those added
    /// before the declaration: the name-keyed walk this replaces took the name as hidden for the
    /// whole block, and which statements a request gets must not change with the rewrite.
    fn redeclared(&self, owner: &'a Statement, block: &'a [Statement]) -> HashSet<&'a str> {
        let declared = block
            .iter()
            .filter(|statement| {
                matches!(
                    statement,
                    Statement::Declaration { .. } | Statement::TupleDeclaration { .. }
                )
            })
            .flat_map(|statement| self.bindings.introduced(statement));
        self.bindings
            .introduced(owner)
            .chain(declared)
            .filter_map(|binding| match binding {
                Binding::Local(id) => Some(self.bindings.local(id).name),
                _ => None,
            })
            .collect()
    }

    fn add(&mut self, local: LocalId, origin: Origin<'a>) {
        if !self.sources[local].contains(&origin) {
            self.sources[local].push(origin);
            self.additions.push((local, origin));
        }
    }

    /// What a statement that copies a name does to the local it assigns.
    fn copy(&mut self, statement: &'a Statement) {
        match statement {
            Statement::Declaration { value, .. } => {
                if let Some(Binding::Local(id)) = self.bindings.declaration(statement) {
                    self.sources[id] = self.copied_from(value);
                }
            }
            Statement::TupleDeclaration { .. } => {
                for binding in self.bindings.introduced(statement) {
                    if let Binding::Local(id) = binding {
                        self.sources[id].clear();
                    }
                }
            }
            Statement::Assignment { value, .. } => {
                if let Some(Binding::Local(id)) = self.bindings.target(statement) {
                    for origin in self.copied_from(value) {
                        self.add(id, origin);
                    }
                }
            }
            _ => {}
        }
    }

    /// The outer names `value` stands for when it is copied: itself, or what a local it names was
    /// copied from.
    fn copied_from(&self, value: &Expression) -> Vec<Origin<'a>> {
        match (value, self.bindings.occurrence(value)) {
            (Expression::Identifier(name, _), Some(binding)) if !name.contains('.') => {
                match binding {
                    Binding::Local(id) => self.sources[id].clone(),
                    Binding::Parameter { position, .. } => vec![Origin::Parameter(position)],
                    Binding::Global(name) | Binding::Free(name) => vec![Origin::Name(name)],
                }
            }
            _ => Vec::new(),
        }
    }

    /// `root` is changed. A local stands for what it was copied from; any other is a global or a
    /// parameter, whose aliases the caller adds.
    fn mutate(&mut self, root: Binding<'a>) {
        match root {
            Binding::Local(id) => {
                for index in 0..self.sources[id].len() {
                    let origin = self.sources[id][index];
                    self.effects.push(Effect::Mutated(origin));
                }
            }
            Binding::Parameter { position, .. } => {
                self.effects
                    .push(Effect::Mutated(Origin::Parameter(position)));
            }
            Binding::Global(name) | Binding::Free(name) => {
                self.effects.push(Effect::Mutated(Origin::Name(name)));
            }
        }
    }

    fn expression(&mut self, expression: &'a Expression) {
        match expression {
            Expression::Call {
                name,
                arguments,
                target,
                ..
            } => self.call(expression, name, arguments, target),
            Expression::MethodCall {
                receiver,
                name,
                arguments,
                target,
                ..
            } => {
                let root = self.bindings.root(receiver);
                // A method the type checker resolved to the script's own definition has that
                // definition's effects, whatever it is called.
                if target.is_none()
                    && MUTATING_METHODS.contains(&name.as_str())
                    && let Some(root) = root
                {
                    self.mutate(root);
                }
                let functions = self.functions;
                self.user_call(functions.callees(name, target), root, arguments);
            }
            Expression::Statement(statement, _) => self.visit(statement),
            _ => {}
        }
        for child in traversal::children(expression) {
            self.expression(child);
        }
    }

    /// A call by name: `array.push(samples, x)`, `samples.push(x)`, `f(x)` or `samples.add(x)`.
    fn call(
        &mut self,
        call: &'a Expression,
        name: &'a str,
        arguments: &'a [Argument],
        target: &'a Option<usize>,
    ) {
        let functions = self.functions;
        let (receiver, member) = match name.rsplit_once('.') {
            Some((receiver, member)) => (Some(receiver), member),
            None => (None, name),
        };
        let changes = !READ_ONLY_MEMBERS.contains(&member);
        match receiver {
            Some(namespace) if COLLECTION_NAMESPACES.contains(&namespace) => {
                if changes
                    && let Some(collection) = collection_argument(arguments)
                    && let Some(root) = self.bindings.root(collection)
                {
                    self.mutate(root);
                }
            }
            // `ta.sma` is the builtin, not a script function that happens to be called `sma`.
            Some(variable) if target.is_none() && is_builtin_namespace(head(variable)) => {}
            Some(variable) => {
                let root = self.bindings.occurrence(call);
                if target.is_none()
                    && ((changes && self.references.contains(head(variable)))
                        || MUTATING_METHODS.contains(&member))
                    && let Some(root) = root
                {
                    self.mutate(root);
                }
                self.user_call(functions.callees(member, target), root, arguments);
            }
            None => {
                self.user_call(functions.callees(name, target), None, arguments);
            }
        }
    }

    /// A call of the script's functions: what each hands to a parameter it changes (the receiver is
    /// parameter 0, then the arguments in order or by label) and the globals it changes.
    fn user_call(
        &mut self,
        callees: &[usize],
        receiver: Option<Binding<'a>>,
        arguments: &'a [Argument],
    ) {
        let functions = self.functions;
        for &id in callees {
            let changed = &functions.mutated[id];
            let (parameters, _) = functions.bodies[id];
            for global in &functions.global_writes[id] {
                self.effects.push(Effect::Mutated(Origin::Name(global)));
            }
            if let Some(receiver) = receiver
                && changed.contains(&0)
            {
                self.mutate(receiver);
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
                    && let Some(root) = self.bindings.root(&argument.value)
                {
                    self.mutate(root);
                }
            }
        }
    }
}

/// The globals that refer to the same value because one was copied from the other
/// (`alias = samples`, `alias := samples`): each name with every name in its group. A collection
/// changed through one is changed for all. Only globals: a name a block declares is its own, and
/// the walker follows it per block.
pub(super) struct Aliases<'a> {
    group_of: HashMap<&'a str, usize>,
    groups: Vec<Vec<&'a str>>,
}

impl<'a> Aliases<'a> {
    pub fn new(bindings: &Bindings<'a>, statements: &'a [Statement]) -> Self {
        let mut pairs = Vec::new();
        copies(bindings, statements, &mut pairs);
        // Union-find over the names the copies mention.
        let mut names: Vec<&'a str> = Vec::new();
        let mut ids: HashMap<&'a str, usize> = HashMap::new();
        let mut parent: Vec<usize> = Vec::new();
        for (alias, original) in pairs {
            let mut ends = [alias, original].map(|name| {
                *ids.entry(name).or_insert_with(|| {
                    names.push(name);
                    parent.push(parent.len());
                    parent.len() - 1
                })
            });
            for end in &mut ends {
                *end = find(&mut parent, *end);
            }
            parent[ends[0]] = ends[1];
        }
        let mut groups: Vec<Vec<&'a str>> = Vec::new();
        let mut group_of = HashMap::new();
        let mut group_ids: HashMap<usize, usize> = HashMap::new();
        for (id, name) in names.iter().enumerate() {
            let top = find(&mut parent, id);
            let group = *group_ids.entry(top).or_insert_with(|| {
                groups.push(Vec::new());
                groups.len() - 1
            });
            groups[group].push(name);
            group_of.insert(*name, group);
        }
        Self { group_of, groups }
    }

    /// `name` and every name it aliases; empty for a name that was never copied.
    pub fn group(&self, name: &str) -> &[&'a str] {
        self.group_of
            .get(name)
            .map_or(&[], |group| self.groups[*group].as_slice())
    }
}

fn find(parent: &mut [usize], mut id: usize) -> usize {
    while parent[id] != id {
        parent[id] = parent[parent[id]];
        id = parent[id];
    }
    id
}

/// The copies of one global name into another: `alias = samples` at the top level, `alias := other`
/// anywhere outside the functions. A name a block declares is not a global, and neither side of a
/// copy may be one. The statements an expression holds (`x = if …`) are not searched.
fn copies<'a>(
    bindings: &Bindings<'a>,
    statements: &'a [Statement],
    pairs: &mut Vec<(&'a str, &'a str)>,
) {
    for statement in statements {
        let (value, target) = match statement {
            Statement::Function { .. } => continue,
            Statement::Declaration { value, .. } => (Some(value), bindings.declaration(statement)),
            Statement::Assignment { value, .. } => (Some(value), bindings.target(statement)),
            _ => (None, None),
        };
        if let Some(value) = value
            && let Expression::Identifier(other, _) = value
            && !other.contains('.')
            && let Some(alias) = target.and_then(Binding::name)
            && let Some(original) = bindings.occurrence(value).and_then(Binding::name)
        {
            pairs.push((alias, original));
        }
        for block in traversal::nested_blocks(statement) {
            copies(bindings, block, pairs);
        }
    }
}

/// The declared names known to hold a collection or an object: those declared from a constructor
/// (`array.new_float()`, `Point.new()`). Other names may hold one too (a script function returned
/// it); a call that changes it still shows up as a write through the call's own shape.
pub(super) fn reference_names(statements: &[Statement]) -> HashSet<&str> {
    statements
        .iter()
        .filter_map(|statement| match statement {
            Statement::Declaration {
                name,
                value: Expression::Call { name: callee, .. },
                ..
            } if returns_reference(callee) => Some(name.as_str()),
            _ => None,
        })
        .collect()
}

/// The names a script declares at its top level with `var` or `varip`.
pub(super) fn persistent_names(statements: &[Statement]) -> HashSet<&str> {
    statements
        .iter()
        .filter_map(|statement| match statement {
            Statement::Declaration { name, mode, .. } if *mode != DeclarationMode::Ordinary => {
                Some(name.as_str())
            }
            _ => None,
        })
        .collect()
}
