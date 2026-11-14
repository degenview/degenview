//! Small runtime support types: the result, security keys/providers and the flattened library
//! linkage. Ported from `Pine/Runtime/*`.

use std::collections::HashSet;

use super::fast_hash::FastMap;
use super::types::RuntimeFunction;
use crate::model::Kline;
use crate::pine::compiler::program::{CompiledProgram, Import};
use crate::pine::syntax::ast::{EnumMember, Expression, Parameter, Statement, TypeField};

/// What `Session::evaluate` returns: the output and every bar's flags.
#[cfg(test)]
pub struct RuntimeResult {
    pub output: crate::pine::compiler::program::VisualOutput,
    pub bar_states: Vec<super::types::BarFlags>,
}

/// A series `request.security` asks for: a symbol as the script wrote it, at a bar length in seconds.
#[derive(Clone, Debug)]
pub struct SecurityKey {
    pub symbol: String,
    pub interval: f64,
    /// `calc_bars_count` of a `request.security_lower_tf` call. 0 is every bar.
    pub recent_bars: usize,
}

impl PartialEq for SecurityKey {
    fn eq(&self, other: &Self) -> bool {
        self.symbol == other.symbol
            && self.interval.to_bits() == other.interval.to_bits()
            && self.recent_bars == other.recent_bars
    }
}

impl Eq for SecurityKey {}

impl std::hash::Hash for SecurityKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.symbol.hash(state);
        self.interval.to_bits().hash(state);
        self.recent_bars.hash(state);
    }
}

/// Supplies the candles `request.security` reads from a series the chart does not carry. The engine
/// never fetches anything; whoever runs it decides what exists.
pub trait SecurityDataProvider: Send {
    /// Closed candles of `key`, oldest first, or None when the provider has nothing for it.
    fn candles(&self, key: &SecurityKey) -> Option<Vec<Kline>>;

    /// The same, for a chart that spans `chart`: a series of finer bars (the intrabars of
    /// `request.security_lower_tf`) must reach back to the chart's first bar to be of any use. A
    /// provider that does not care about the span answers as `candles` does.
    fn candles_over(&self, key: &SecurityKey, chart: ChartSpan) -> Option<Vec<Kline>> {
        let _ = chart;
        self.candles(key)
    }
}

/// The most base candles a provider fetches for one intrabar series; the newest are kept, so a long
/// chart gets intrabars for its recent part. Whoever asks for a series (`request.footprint`) picks a
/// candle size that keeps the chart's span within it.
pub const MAX_INTRABAR_CANDLES: usize = 60_000;

/// The time the chart's bars cover, in milliseconds: from the first bar's open to the end of the last.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ChartSpan {
    pub start_ms: i64,
    pub end_ms: i64,
    /// The chart's bar length.
    pub spacing_ms: i64,
}

/// What the imported libraries contribute to a run, flattened out of the compiled import graph.
#[derive(Clone, Debug, Default)]
pub struct LibraryLinkage {
    /// Scope (a library path, or `""` for the script) -> import alias -> library path.
    pub aliases: FastMap<String, FastMap<String, String>>,
    /// Library path -> the names it exports.
    pub exports: FastMap<String, HashSet<String>>,
    /// Library path -> its top-level constants (evaluated on first use).
    pub constants: FastMap<String, FastMap<String, Expression>>,
    pub functions: FastMap<String, RuntimeFunction>,
    pub methods: FastMap<String, Vec<RuntimeFunction>>,
    /// Functions declared more than once with different parameter types, by registry key.
    pub overloads: FastMap<String, Vec<RuntimeFunction>>,
    pub types: FastMap<String, Vec<TypeField>>,
    pub enums: FastMap<String, Vec<EnumMember>>,
}

impl LibraryLinkage {
    /// Whether any import is in play; without one no name has an alias to resolve.
    fn is_empty(&self) -> bool {
        self.aliases.values().all(|aliases| aliases.is_empty())
    }

    pub fn new(program: &CompiledProgram) -> Self {
        let mut linkage = Self::default();
        linkage.aliases.insert(
            String::new(),
            program
                .imports
                .iter()
                .map(|entry| (entry.alias.clone(), entry.path.clone()))
                .collect(),
        );
        let mut seen = HashSet::new();
        linkage.visit(&program.imports, &mut seen);
        linkage
    }

    fn visit(&mut self, imports: &[Import], seen: &mut HashSet<String>) {
        for entry in imports {
            let Some(library) = &entry.library else {
                continue;
            };
            if !seen.insert(entry.path.clone()) {
                continue;
            }
            self.aliases.insert(
                entry.path.clone(),
                library
                    .imports
                    .iter()
                    .map(|inner| (inner.alias.clone(), inner.path.clone()))
                    .collect(),
            );
            self.register(library, &entry.path);
            self.visit(&library.imports, seen);
        }
    }

    fn register(&mut self, library: &CompiledProgram, key: &str) {
        self.exports
            .insert(key.to_owned(), library.exported_names.clone());
        let declared_types: HashSet<&str> = library
            .statements
            .iter()
            .filter_map(|statement| match statement {
                Statement::TypeDeclaration { name, .. }
                | Statement::EnumDeclaration { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        for statement in &library.statements {
            match statement {
                Statement::TypeDeclaration { name, fields, .. } => {
                    self.types.insert(format!("{key}::{name}"), fields.clone());
                }
                Statement::EnumDeclaration { name, members, .. } => {
                    self.enums.insert(format!("{key}::{name}"), members.clone());
                }
                Statement::Declaration {
                    name,
                    mode: crate::pine::syntax::ast::DeclarationMode::Ordinary,
                    value,
                    ..
                } => {
                    self.constants
                        .entry(key.to_owned())
                        .or_default()
                        .insert(name.clone(), value.clone());
                }
                Statement::Function {
                    name,
                    parameters,
                    body,
                    ..
                } => {
                    let mut function = RuntimeFunction::new(
                        parameters
                            .iter()
                            .map(|p| self.canonical(p, key, &declared_types))
                            .collect(),
                        body.clone(),
                    );
                    function.scope = Some(key.to_owned());
                    function.is_exported = library.exported_names.contains(name);
                    let registry_key = format!("{key}::{name}");
                    if self.functions.contains_key(&registry_key) {
                        let known = self.overloads.entry(registry_key.clone()).or_default();
                        if known.is_empty() {
                            known.push(self.functions[&registry_key].clone());
                        }
                        known.push(function.clone());
                    }
                    self.functions.insert(registry_key, function.clone());
                    if library.method_names.contains(name) {
                        self.methods.entry(name.clone()).or_default().push(function);
                    }
                }
                _ => {}
            }
        }
    }

    /// A parameter whose type name is spelled as the importing code spells it, canonicalized.
    pub fn canonical(
        &self,
        parameter: &Parameter,
        scope: &str,
        declared: &HashSet<&str>,
    ) -> Parameter {
        let mut result = parameter.clone();
        if let Some(name) = parameter.type_name()
            && let Some(reference) = result.type_ref.as_mut()
        {
            if let Some((alias, rest)) = name.split_once('.') {
                if let Some(path) = self.aliases.get(scope).and_then(|m| m.get(alias)) {
                    reference.name = Some(format!("{path}::{rest}"));
                }
            } else if declared.contains(name) && !scope.is_empty() {
                reference.name = Some(format!("{scope}::{name}"));
            }
        }
        result
    }

    /// `alias.rest` as seen from `scope`: the library path and what follows the alias.
    pub fn resolve(&self, name: &str, scope: &str) -> Option<(String, String)> {
        if self.is_empty() {
            return None;
        }
        let (alias, rest) = name.split_once('.')?;
        let path = self.aliases.get(scope)?.get(alias)?;
        Some((path.clone(), rest.to_owned()))
    }
}
