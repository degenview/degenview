//! Building a session from a compiled program: its functions, methods, types and inputs, the
//! builtin names the script declares itself, and the state before the first bar.

use std::collections::{HashMap, HashSet};

use super::{DEFAULT_MINTICK, STRATEGY_SERIES, Session};
use crate::pine::Limits;
use crate::pine::compiler::program::{CompiledProgram, InputValue, SymbolInfo};
use crate::pine::outputs::drawings;
use crate::pine::runtime::fast_hash::FastMap;
use crate::pine::runtime::state::RuntimeState;
use crate::pine::runtime::support::{LibraryLinkage, SecurityDataProvider};
use crate::pine::runtime::types::RuntimeFunction;
use crate::pine::strategy::broker::Broker;
use crate::pine::syntax::ast::{Expression, Statement};

impl Session {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        program: CompiledProgram,
        inputs: HashMap<String, InputValue>,
        limits: Limits,
        mintick: Option<f64>,
        symbol: SymbolInfo,
        security_data: Option<Box<dyn SecurityDataProvider>>,
        visible_range: Option<(i64, i64)>,
    ) -> Self {
        let mut program = program;
        crate::pine::runtime::call_binding::bind(&mut program);
        let supplied_mintick = mintick.filter(|m| *m > 0.0 && m.is_finite());
        let drawing_limits = drawings::DrawingLimits {
            max_lines: program.declaration.max_lines_count,
            max_labels: program.declaration.max_labels_count,
            max_boxes: program.declaration.max_boxes_count,
            max_polylines: program.declaration.max_polylines_count,
        };
        let mut functions = FastMap::default();
        let mut callables = Vec::new();
        let mut methods: FastMap<String, Vec<RuntimeFunction>> = FastMap::default();
        let mut script_overloads: FastMap<String, Vec<RuntimeFunction>> = FastMap::default();
        let mut input_variables = HashMap::new();
        let mut types = FastMap::default();
        let mut enums = FastMap::default();
        let linkage = LibraryLinkage::new(&program);
        for statement in &program.statements {
            match statement {
                Statement::TypeDeclaration { name, fields, .. } => {
                    types.insert(name.clone(), fields.clone());
                }
                Statement::EnumDeclaration { name, members, .. } => {
                    enums.insert(name.clone(), members.clone());
                }
                Statement::Function {
                    name,
                    parameters,
                    body,
                    ..
                } => {
                    let function = RuntimeFunction::new(
                        parameters
                            .iter()
                            .map(|p| linkage.canonical(p, "", &HashSet::new()))
                            .collect(),
                        body.clone(),
                    );
                    callables.push(function.clone());
                    if let Some(previous) = functions.insert(name.clone(), function.clone()) {
                        let known: &mut Vec<RuntimeFunction> =
                            script_overloads.entry(name.clone()).or_default();
                        if known.is_empty() {
                            known.push(previous);
                        }
                        known.push(function.clone());
                    }
                    if program.method_names.contains(name) {
                        methods.entry(name.clone()).or_default().push(function);
                    }
                }
                Statement::Declaration {
                    name,
                    value: Expression::Call { site, .. },
                    ..
                } => {
                    input_variables.insert(*site, name.clone());
                }
                _ => {}
            }
        }
        let shadowing_builtins = declared_builtin_names(&program.statements);
        // Over the script's types and the imported ones, which are keyed `"<library>::<Name>"` here
        // and on the objects built from them.
        let varip_fields: Vec<(String, Vec<String>)> = types
            .iter()
            .chain(linkage.types.iter())
            .filter_map(|(name, fields)| {
                let kept: Vec<String> = fields
                    .iter()
                    .filter(|field| field.persistent)
                    .map(|field| field.name.clone())
                    .collect();
                (!kept.is_empty()).then(|| (name.clone(), kept))
            })
            .collect();
        let history_builtins =
            history_builtin_names(&program.statements, &linkage, &shadowing_builtins);
        let mut inputs = inputs;
        for input in &program.input_schema.inputs {
            inputs
                .entry(input.id.clone())
                .or_insert_with(|| input.default_value.clone());
        }
        let mut session = Self {
            program,
            inputs,
            limits,
            symbol,
            callables,
            functions: functions
                .into_iter()
                .chain(linkage.functions.clone())
                .collect(),
            overloads: script_overloads
                .into_iter()
                .chain(linkage.overloads.clone())
                .collect(),
            methods: {
                let mut merged = methods;
                for (name, funcs) in &linkage.methods {
                    merged
                        .entry(name.clone())
                        .or_default()
                        .extend(funcs.clone());
                }
                merged
            },
            types: {
                let mut merged = types;
                for (name, fields) in &linkage.types {
                    merged.entry(name.clone()).or_insert_with(|| fields.clone());
                }
                merged
            },
            enums: {
                let mut merged = enums;
                for (name, members) in &linkage.enums {
                    merged
                        .entry(name.clone())
                        .or_insert_with(|| members.clone());
                }
                merged
            },
            input_variables,
            linkage,
            security_data,
            security_series: HashMap::new(),
            library_constants: FastMap::default(),
            constants_in_progress: HashSet::new(),
            committed: RuntimeState::default(),
            working_is_committed: false,
            working: RuntimeState::default(),
            intrabar: FastMap::default(),
            intrabar_objects: Vec::new(),
            intrabar_key: String::new(),
            halted: None,
            bulk_replay: false,
            varip_fields,
            last_open_time: None,
            security_globals: None,
            theme: crate::pine::compiler::program::ChartTheme::DARK,
            security_depth: 0,
            shadowing_builtins,
            history_builtins,
            #[cfg(any(test, feature = "corpus"))]
            ignore_routes: false,
            last_bar_index: -1,
            last_bar_time: None,
            first_bar_time: None,
            visible_range,
            last_committed_open_time: None,
            once_per_bar_ledger: HashSet::new(),
            emitted_alerts: Vec::new(),
            logs: crate::pine::logs::LogSink::default(),
            mintick: supplied_mintick.unwrap_or(DEFAULT_MINTICK),
            footprints: Default::default(),
            bar_seconds: 0.0,
            chart_bar_seconds: None,
            alert_mode: false,
            fixed_bar_seconds: None,
            drawing_limits,
            supplied_mintick,
            deadline_at: None,
            cancellation: None,
            hooks: None,
        };
        session.committed = session.fresh_state();
        session.working = session.committed.clone();
        session
    }

    pub(super) fn fresh_state(&self) -> RuntimeState {
        let mut broker = Broker::new(
            self.program
                .declaration
                .strategy
                .clone()
                .unwrap_or_default(),
        );
        // The trading days the risk rules count are the exchange's.
        broker.zone = Some(
            crate::pine::time::trading_session::MarketSessions::for_kind(&self.symbol.kind).zone(),
        );
        RuntimeState {
            broker,
            // The first bar executes as bar 0.
            bar_index: -1,
            ..RuntimeState::default()
        }
    }
}

/// The builtin variable names (`low`, `time`) that `statements` declare anew anywhere: a global, a
/// local, a loop counter or a tuple element.
fn declared_builtin_names(statements: &[Statement]) -> Vec<String> {
    fn visit(statements: &[Statement], found: &mut Vec<String>) {
        for statement in statements {
            let names: Vec<&str> = match statement {
                Statement::Declaration { name, .. } => vec![name],
                Statement::TupleDeclaration { names, .. } => {
                    names.iter().map(String::as_str).collect()
                }
                Statement::ForRange { variable, .. } => vec![variable],
                Statement::ForIn { index, value, .. } => index
                    .as_deref()
                    .into_iter()
                    .chain([value.as_str()])
                    .collect(),
                _ => Vec::new(),
            };
            for name in names {
                if crate::pine::compiler::symbol_catalog::is_variable(name)
                    && !found.iter().any(|n| n == name)
                {
                    found.push(name.to_owned());
                }
            }
            for block in crate::pine::syntax::traversal::nested_blocks(statement) {
                visit(block, found);
            }
        }
    }
    let mut found = Vec::new();
    visit(statements, &mut found);
    found
}

/// The builtin variables `statements` read a history of that `commit_histories` does not record
/// itself: `open`, `close` and the other series it always keeps are left out, and so is a name the
/// script declares anew (it is then an ordinary variable). The bodies of the functions and methods
/// the script imports are read too, and the constants of the libraries it links: a library reads the
/// same builtin trails as the script.
fn history_builtin_names(
    statements: &[Statement],
    linkage: &LibraryLinkage,
    shadowed: &[String],
) -> Vec<String> {
    const ALWAYS_RECORDED: [&str; 11] = [
        "open",
        "high",
        "low",
        "close",
        "volume",
        "hl2",
        "hlc3",
        "ohlc4",
        "time",
        "time_close",
        "bar_index",
    ];
    let mut names: Vec<String> = Vec::new();
    let mut note = |name: &str| {
        if crate::pine::compiler::symbol_catalog::is_variable(name)
            && !ALWAYS_RECORDED.contains(&name)
            && !STRATEGY_SERIES.contains(&name)
            && !shadowed.iter().any(|shadow| shadow == name)
            && !names.iter().any(|known| known == name)
        {
            names.push(name.to_owned());
        }
    };
    crate::pine::syntax::traversal::for_each_history_identifier(statements, &mut note);
    let functions = linkage
        .functions
        .values()
        .chain(linkage.methods.values().flatten())
        .chain(linkage.overloads.values().flatten());
    for function in functions {
        crate::pine::syntax::traversal::for_each_history_identifier(&function.body, &mut note);
    }
    for constants in linkage.constants.values() {
        for expression in constants.values() {
            crate::pine::syntax::traversal::for_each_history_identifier_in_expression(
                expression, &mut note,
            );
        }
    }
    names
}
