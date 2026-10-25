//! Interprets a compiled Pine program bar by bar. Ported from `PineRuntimeSession.swift` +
//! `+Expressions`/`+Statements`/`+Calls`/`+Market`. Builtin handlers live in their own modules
//! (`scalar_builtins`, `array_builtins`, `strategy_builtins`, `ta`, `math`, `strings`, …).

use std::collections::{HashMap, HashSet};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

use super::ast::{
    AssignmentOperator, BinaryOperator, DeclarationMode, Expression, Literal, Statement, SwitchArm,
    UnaryOperator,
};
use super::broker::Broker;
use super::calendar;
use super::fast_hash::FastMap;
use super::operators;
use super::program::{
    BarEvent, BarPhase, CompiledProgram, InputValue, ScriptType, SymbolInfo, VisualOutput,
};
use super::runtime_support::{
    ExecutionRejection, LibraryLinkage, RuntimeResult, SecurityDataProvider,
};
use super::runtime_types::{BarFlags, Flow, RuntimeContext, RuntimeFunction};
use super::runtime_value::{RefKind, RuntimeValue};
use super::state::{AlertEvent, RuntimeState, SecuritySite};
use super::traversal::nested_blocks;
use super::{
    alert_builtins, array_builtins, drawings, map_builtins, market_values, math, matrix_builtins,
    number_format, strategy_builtins, strings, ta, visuals,
};
use crate::model::Kline;
use crate::pine::{Diagnostic, DiagnosticCategory, Limits, SourceRange};

/// The `strategy.*` readings a strategy keeps a history of, so `strategy.position_size[1]` works.
const STRATEGY_SERIES: [&str; 5] = [
    "strategy.position_size",
    "strategy.position_avg_price",
    "strategy.equity",
    "strategy.netprofit",
    "strategy.openprofit",
];

/// Namespaces whose members are values the engine computes, not named constants.
const VARIABLE_NAMESPACES: [&str; 8] = [
    "ta",
    "syminfo",
    "barstate",
    "timeframe",
    "chart",
    "request",
    "ticker",
    "math",
];

pub(crate) const INTERNAL_PREFIX: &str = "\u{0}";
const ALERT_LIMIT: usize = 200;
const DEFAULT_MINTICK: f64 = 0.01;

pub struct Session {
    pub program: CompiledProgram,
    pub inputs: HashMap<String, InputValue>,
    pub limits: Limits,
    pub symbol: SymbolInfo,
    pub functions: FastMap<String, RuntimeFunction>,
    pub methods: FastMap<String, Vec<RuntimeFunction>>,
    /// Functions declared more than once with different parameter types, by registry key.
    pub overloads: FastMap<String, Vec<RuntimeFunction>>,
    pub types: FastMap<String, Vec<super::ast::TypeField>>,
    pub enums: FastMap<String, Vec<super::ast::EnumMember>>,
    pub input_variables: HashMap<usize, String>,
    pub linkage: LibraryLinkage,
    pub security_data: Option<Box<dyn SecurityDataProvider>>,
    pub security_series:
        HashMap<super::runtime_support::SecurityKey, Option<std::sync::Arc<Vec<Kline>>>>,
    pub library_constants: FastMap<String, RuntimeValue>,
    pub constants_in_progress: HashSet<String>,
    pub committed: RuntimeState,
    /// While a history is replayed, `working` already is the committed state: cloning it into
    /// `committed` after every bar would make a run quadratic in its length. `committed` is
    /// refreshed (see `sync_committed`) when something other than a historical bar needs it.
    working_is_committed: bool,
    pub working: RuntimeState,
    pub intrabar: FastMap<String, RuntimeValue>,
    pub last_open_time: Option<i64>,
    pub security_globals: Option<FastMap<String, RuntimeValue>>,
    /// The chart's foreground and background (`chart.fg_color`, `chart.bg_color`).
    pub theme: super::program::ChartTheme,
    /// Builtin variable names (`low`) the script declares itself: removed at the start of each bar so
    /// a read before the declaration still means the builtin.
    shadowing_builtins: Vec<String>,
    /// How many `request.*` expressions the running code is inside of.
    pub security_depth: usize,
    pub last_bar_index: i64,
    pub last_bar_time: Option<i64>,
    pub first_bar_time: Option<i64>,
    pub visible_range: Option<(i64, i64)>,
    pub last_committed_open_time: Option<i64>,
    pub once_per_bar_ledger: HashSet<usize>,
    pub emitted_alerts: Vec<AlertEvent>,
    /// The messages of `log.*()` calls. Not part of the rolled-back state: a realtime bar's messages
    /// outlive the bar's rollback.
    pub logs: super::logs::LogSink,
    pub mintick: f64,
    pub bar_seconds: f64,
    /// The length of the chart's own bars when it cannot be read off their open times: a
    /// price-based chart's bars follow price, not time.
    pub fixed_bar_seconds: Option<f64>,
    drawing_limits: drawings::DrawingLimits,
    supplied_mintick: Option<f64>,
    pub(crate) deadline_at: Option<Instant>,
    cancellation: Option<Arc<AtomicBool>>,
    /// Present only in a debugging session; every hook below tests it and does nothing otherwise.
    debug: Option<Box<super::debug::DebugRuntime>>,
    /// Present only in a profiling session; see `docs/pine-profiler.md`.
    profile: Option<Box<super::profile::ProfileRuntime>>,
}

struct Step {
    flow: Flow,
    value: RuntimeValue,
}

impl Step {
    fn normal(value: RuntimeValue) -> Self {
        Self {
            flow: Flow::Normal,
            value,
        }
    }
}

#[derive(Debug)]
pub enum RunError {
    Rejection(ExecutionRejection),
    Script(Diagnostic),
}

impl From<Diagnostic> for RunError {
    fn from(diagnostic: Diagnostic) -> Self {
        Self::Script(diagnostic)
    }
}

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
        let supplied_mintick = mintick.filter(|m| *m > 0.0 && m.is_finite());
        let drawing_limits = drawings::DrawingLimits {
            max_lines: program.declaration.max_lines_count,
            max_labels: program.declaration.max_labels_count,
            max_boxes: program.declaration.max_boxes_count,
            max_polylines: program.declaration.max_polylines_count,
        };
        let mut functions = FastMap::default();
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
            last_open_time: None,
            security_globals: None,
            theme: super::program::ChartTheme::DARK,
            security_depth: 0,
            shadowing_builtins,
            last_bar_index: -1,
            last_bar_time: None,
            first_bar_time: None,
            visible_range,
            last_committed_open_time: None,
            once_per_bar_ledger: HashSet::new(),
            emitted_alerts: Vec::new(),
            logs: super::logs::LogSink::default(),
            mintick: supplied_mintick.unwrap_or(DEFAULT_MINTICK),
            bar_seconds: 0.0,
            fixed_bar_seconds: None,
            drawing_limits,
            supplied_mintick,
            deadline_at: None,
            cancellation: None,
            debug: None,
            profile: None,
        };
        session.committed = session.fresh_state();
        session.working = session.committed.clone();
        session
    }

    fn fresh_state(&self) -> RuntimeState {
        RuntimeState {
            broker: Broker::new(
                self.program
                    .declaration
                    .strategy
                    .clone()
                    .unwrap_or_default(),
            ),
            // The first bar executes as bar 0.
            bar_index: -1,
            ..RuntimeState::default()
        }
    }

    pub fn reset(&mut self, inputs: Option<HashMap<String, InputValue>>) {
        if let Some(inputs) = inputs {
            self.inputs = inputs;
        }
        self.committed = self.fresh_state();
        self.working = self.committed.clone();
        self.working_is_committed = false;
        self.intrabar.clear();
        self.last_open_time = None;
        self.last_committed_open_time = None;
        self.once_per_bar_ledger.clear();
        self.emitted_alerts.clear();
        self.logs.restart();
        self.bar_seconds = self.fixed_bar_seconds.unwrap_or(0.0);
        self.last_bar_index = -1;
        self.last_bar_time = None;
        self.first_bar_time = None;
        self.mintick = self.supplied_mintick.unwrap_or(DEFAULT_MINTICK);
        self.deadline_at = None;
    }

    /// Installs a cooperative cancellation flag. Long history runs and instruction-heavy bars
    /// check it without requiring an async runtime.
    pub fn set_cancellation(&mut self, cancellation: Arc<AtomicBool>) {
        self.cancellation = Some(cancellation);
    }

    /// Makes this session a debugging session: the script stops where the debugger says. A session
    /// is never debugged by accident; nothing but this call installs the hooks.
    pub fn attach_debugger(&mut self, debug: super::debug::DebugRuntime) {
        // Logpoints write to the Pine Logs, and the script's own `log.*()` calls are what a person
        // debugging wants to read too.
        self.logs.enable();
        self.debug = Some(Box::new(debug));
    }

    /// Makes this session a profiling session: each statement, user function call and bar is
    /// timed. A session is never profiled by accident; nothing but this call installs the hooks.
    pub fn attach_profiler(&mut self, profile: super::profile::ProfileRuntime) {
        self.profile = Some(Box::new(profile));
    }

    /// Takes the recorder back out, to read what it measured.
    pub fn detach_profiler(&mut self) -> Option<super::profile::ProfileRuntime> {
        self.profile.take().map(|profile| *profile)
    }

    // MARK: Running

    pub fn evaluate(&mut self, bars: &[Kline]) -> Result<RuntimeResult, RunError> {
        let states = self.load_history(bars, false, None)?;
        Ok(RuntimeResult {
            output: self.output(),
            diagnostics: Vec::new(),
            bar_states: states,
        })
    }

    pub fn load_history(
        &mut self,
        bars: &[Kline],
        precedes_live_bar: bool,
        seed: Option<f64>,
    ) -> Result<Vec<BarFlags>, RunError> {
        self.reset(None);
        self.start_deadline();
        self.infer_mintick(bars);
        if let Some(seed) = seed.filter(|s| *s > 0.0) {
            self.bar_seconds = seed;
        }
        if let Some(fixed) = self.fixed_bar_seconds {
            self.bar_seconds = fixed;
        } else if bars.len() > 1 {
            self.bar_seconds =
                (bars[bars.len() - 1].open_time - bars[bars.len() - 2].open_time) as f64 / 1000.0;
        }
        self.last_bar_index = bars.len() as i64 - 1 + (precedes_live_bar as i64);
        self.last_bar_time = bars.last().map(|bar| {
            bar.open_time
                + (if precedes_live_bar {
                    (self.bar_seconds * 1000.0) as i64
                } else {
                    0
                })
        });
        self.first_bar_time = bars.first().map(|bar| bar.open_time);
        let result = (|| {
            let mut states = Vec::with_capacity(bars.len());
            for (index, bar) in bars.iter().enumerate() {
                self.check_interruption()?;
                let is_last_history = index == bars.len() - 1;
                let flags = self.execute(
                    BarEvent {
                        candle: bar.clone(),
                        phase: BarPhase::Historical,
                    },
                    is_last_history && !precedes_live_bar,
                    Some(is_last_history),
                )?;
                states.push(flags);
            }
            Ok(states)
        })();
        self.deadline_at = None;
        result
    }

    pub fn inferred_mintick(bars: &[Kline]) -> f64 {
        let mut decimals = 0i32;
        for bar in bars.iter().rev().take(200) {
            for price in [bar.open, bar.high, bar.low, bar.close] {
                if !price.is_finite() {
                    continue;
                }
                while decimals < 8 {
                    let scaled = price * 10f64.powi(decimals);
                    if (scaled - scaled.round()).abs() <= (1e-6f64).max(scaled.abs() * 1e-12) {
                        break;
                    }
                    decimals += 1;
                }
            }
        }
        10f64.powi(-decimals)
    }

    /// Recompute `mintick` from the bars when the caller did not supply one. `load_history` runs
    /// this before its execute loop; the host's `run_script`/`run_script_alerts` call it directly
    /// because they drive `execute` themselves.
    pub fn infer_mintick(&mut self, bars: &[Kline]) {
        if self.supplied_mintick.is_none() {
            self.mintick = Self::inferred_mintick(bars);
        }
    }

    #[allow(clippy::result_large_err)]
    pub fn execute(
        &mut self,
        event: BarEvent,
        is_last: bool,
        is_last_history: Option<bool>,
    ) -> Result<BarFlags, RunError> {
        let owns_deadline = self.deadline_at.is_none();
        if owns_deadline {
            self.start_deadline();
        }
        let realtime = event.phase.is_realtime();
        if self.profile.is_some() {
            self.profile_begin_bar();
        }
        let result = self.execute_bar(event, is_last, is_last_history);
        if self.profile.is_some() {
            self.profile_end_bar(!matches!(result, Err(RunError::Rejection(_))), realtime);
        }
        if owns_deadline {
            self.deadline_at = None;
        }
        result
    }

    fn execute_bar(
        &mut self,
        event: BarEvent,
        is_last: bool,
        is_last_history: Option<bool>,
    ) -> Result<BarFlags, RunError> {
        self.check_interruption()?;
        let open_time = event.candle.open_time;
        if let Some(committed_time) = self.last_committed_open_time
            && open_time <= committed_time
        {
            return Err(RunError::Rejection(if open_time < committed_time {
                ExecutionRejection::StaleBar
            } else {
                ExecutionRejection::DuplicateBar
            }));
        }
        if let Some(previous) = self.last_open_time
            && open_time < previous
        {
            return Err(RunError::Rejection(ExecutionRejection::StaleBar));
        }
        let is_new = Some(open_time) != self.last_open_time;
        if is_new {
            self.intrabar.clear();
            self.once_per_bar_ledger.clear();
        }
        self.emitted_alerts.clear();
        if is_new
            && self.fixed_bar_seconds.is_none()
            && let Some(previous) = self.last_open_time
        {
            self.bar_seconds = (open_time - previous) as f64 / 1000.0;
        }
        let confirmed = event.phase.is_confirmed();
        let realtime = event.phase.is_realtime();
        // A historical bar continues from the state the previous one left; anything else starts from
        // the committed state, so an unconfirmed tick can be thrown away.
        let replaying = matches!(event.phase, BarPhase::Historical);
        if replaying && self.working_is_committed {
            self.working.bar_index += 1;
        } else {
            self.sync_committed();
            self.working = self.committed.clone();
            self.working.bar_index = self.committed.bar_index + 1;
        }
        for name in &self.shadowing_builtins {
            self.working.variables.remove(name);
        }
        if self.working.bar_index >= self.last_bar_index {
            self.last_bar_index = self.working.bar_index;
            self.last_bar_time = Some(open_time);
        }
        self.working.instructions = 0;
        self.logs
            .begin_bar(open_time, self.working.bar_index, realtime, confirmed);
        let chart_bar_index = self.working.bar_index;
        if let Some(debug) = self.debug.as_mut() {
            debug.begin_bar(chart_bar_index);
        }
        if !is_new {
            for (key, value) in self.intrabar.clone() {
                self.working.variables.insert(key, value);
            }
        }
        let flags = BarFlags {
            is_first: self.working.bar_index == 0,
            is_last,
            is_history: !realtime,
            is_realtime: realtime,
            is_new,
            is_confirmed: confirmed,
            is_last_confirmed_history: !realtime && (is_last_history.unwrap_or(is_last)),
        };
        let candle = event.candle;
        let mut context = RuntimeContext::new(candle.clone(), flags);
        if self.is_strategy() {
            self.working.broker.process(
                candle.clone(),
                self.working.bar_index as usize,
                self.mintick,
            );
        }
        let statements = std::mem::take(&mut self.program.statements);
        let outcome = self.run(&statements, &mut context);
        self.program.statements = statements;
        if let Err(error) = outcome {
            if replaying {
                // The previous bars' state was consumed in place, so a failed historical bar cannot be
                // skipped: the run starts over from nothing.
                self.committed = self.fresh_state();
                self.working = self.committed.clone();
                self.working_is_committed = false;
            }
            return Err(RunError::Script(error));
        }
        if self.is_strategy() {
            self.finish_strategy_bar(candle.clone());
        }
        if confirmed {
            self.commit_histories(candle, true);
            if replaying {
                self.working_is_committed = true;
            } else {
                self.committed = self.working.clone();
                self.working_is_committed = false;
            }
            self.intrabar.clear();
            self.last_committed_open_time = Some(open_time);
        }
        self.last_open_time = Some(open_time);
        Ok(flags)
    }

    /// Makes `committed` current when the last historical bars left their state only in `working`.
    fn sync_committed(&mut self) {
        if self.working_is_committed {
            self.committed = self.working.clone();
            self.working_is_committed = false;
        }
    }

    fn is_strategy(&self) -> bool {
        self.program.declaration.script_type == ScriptType::Strategy
    }

    fn finish_strategy_bar(&mut self, candle: Kline) {
        let close = candle.close;
        if self
            .program
            .declaration
            .strategy
            .as_ref()
            .is_some_and(|s| s.process_orders_on_close)
        {
            self.working.broker.fill_market_orders_at_close(
                candle,
                self.working.bar_index as usize,
                self.mintick,
            );
        }
        self.working.broker.record_equity(close);
    }

    pub fn emit(&mut self, alert: AlertEvent) {
        self.emitted_alerts.push(alert);
    }

    pub fn output(&self) -> VisualOutput {
        let w = &self.working;
        let linefills: Vec<_> = w
            .linefills
            .values()
            .filter(|lf| w.lines.contains_key(&lf.line1) && w.lines.contains_key(&lf.line2))
            .cloned()
            .collect();
        VisualOutput {
            overlay: self.program.declaration.overlay,
            bar_count: (w.bar_index + 1) as usize,
            plots: sorted_by_id(&w.plots)
                .into_iter()
                .map(super::outputs::PlotOutput::shifted)
                .collect(),
            hlines: sorted_by_id(&w.hlines),
            markers: sorted_by_id(&w.markers),
            backgrounds: sorted_by_id(&w.backgrounds)
                .into_iter()
                .map(super::outputs::ColorOutput::finished)
                .collect(),
            bar_colors: sorted_by_id(&w.bar_colors)
                .into_iter()
                .map(super::outputs::ColorOutput::finished)
                .collect(),
            fills: sorted_by_id(&w.fills),
            lines: sorted_by_id(&w.lines),
            labels: sorted_by_id(&w.labels),
            boxes: sorted_by_id(&w.boxes),
            linefills: {
                let mut v: Vec<_> = linefills.into_iter().collect();
                v.sort_by_key(|lf| lf.id);
                v
            },
            polylines: sorted_by_id(&w.polylines),
            tables: sorted_by_id(&w.tables),
            candles: sorted_by_id(&w.candles)
                .into_iter()
                .map(super::outputs::CandleOutput::finished)
                .collect(),
            alerts: w.alerts.clone(),
            strategy: if self.is_strategy() {
                Some(super::strategy::StrategyReport {
                    chart: self.symbol.chart,
                    ..w.broker.report()
                })
            } else {
                None
            },
            // Only a script that declares it goes behind the candles; the user can still send any
            // script's visuals to the back or the front from the chart's Scripts settings.
            behind_chart: self.program.declaration.overlay
                && self.program.declaration.behind_chart.unwrap_or(false),
        }
    }

    // MARK: Expressions

    pub(crate) fn eval(
        &mut self,
        expression: &Expression,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        self.budget()?;
        match expression {
            Expression::Literal(value, _) => Ok(match value {
                Literal::Int(n) => RuntimeValue::Int(*n),
                Literal::Float(n) => RuntimeValue::Float(*n),
                Literal::Bool(b) => RuntimeValue::Bool(*b),
                Literal::String(s) => RuntimeValue::String(s.clone()),
                Literal::Color(c) => RuntimeValue::Color(*c),
                Literal::Na => RuntimeValue::Na,
            }),
            Expression::Identifier(name, range) => self.resolve_identifier(name, *range, context),
            Expression::Unary { op, operand, range } => {
                self.eval_unary(*op, operand, *range, context)
            }
            Expression::Binary {
                lhs,
                op,
                rhs,
                range,
            } => self.eval_binary(lhs, *op, rhs, *range, context),
            Expression::Ternary {
                condition,
                when_true,
                when_false,
                range,
            } => {
                let test = Self::truth(self.eval(condition, context)?).ok_or_else(|| {
                    Diagnostic::error(
                        "PINE4005",
                        DiagnosticCategory::Runtime,
                        "Ternary condition must be bool.",
                        *range,
                    )
                })?;
                self.eval(if test { when_true } else { when_false }, context)
            }
            Expression::History {
                base,
                offset,
                range,
            } => self.eval_history(base, offset, *range, context),
            Expression::Tuple(expressions, _) => {
                let mut values = Vec::with_capacity(expressions.len());
                for e in expressions {
                    values.push(self.eval(e, context)?);
                }
                Ok(RuntimeValue::Tuple(values))
            }
            Expression::MethodCall {
                receiver,
                name,
                arguments,
                site,
                range,
            } => self.method_call_on_value(receiver, name, arguments, *site, *range, context),
            Expression::Member { base, name, range } => {
                let value = self.eval(base, context)?;
                self.read_field(name, value, *range)
            }
            Expression::Statement(statement, _) => {
                let (_, value) = self.run(std::slice::from_ref(statement.as_ref()), context)?;
                Ok(if value == RuntimeValue::Void {
                    RuntimeValue::Na
                } else {
                    value
                })
            }
            Expression::Call {
                name,
                arguments,
                site,
                range,
            } => self.call(name, arguments, *site, *range, context),
        }
    }

    fn resolve_identifier(
        &mut self,
        name: &str,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        if let Some(value) = self.working.variables.get(name) {
            return Ok(value.clone());
        }
        // The names below are all dotted; a plain `close` skips straight to the market values.
        if name.contains('.') {
            if let Some(value) = self.field_path(name, range)? {
                return Ok(value);
            }
            if let Some(value) = self.cumulative_variable(name, context) {
                return Ok(value);
            }
            if let Some(value) = self.all_drawings(name) {
                return Ok(value);
            }
        }
        if let Some(value) = self.security_globals.as_ref().and_then(|g| g.get(name)) {
            return Ok(value.clone());
        }
        if let Some(value) = self.market(name, context) {
            return Ok(value);
        }
        if let Some(flag) = context.flags.value(name) {
            return Ok(RuntimeValue::Bool(flag));
        }
        if let Some(color) = super::builtins::color(name) {
            return Ok(RuntimeValue::Color(color));
        }
        if let Some(constant) = super::builtins::constant(name) {
            return Ok(constant);
        }
        // A library constant or enum member (via an import alias).
        if let Some(value) = self.library_identifier(name, range, context)? {
            return Ok(value);
        }
        // A dotted name that matches nothing is a named constant (`size.small`) and stands for itself.
        if name.contains('.') && super::compiler::is_named_constant(name) {
            return Ok(RuntimeValue::String(name.to_owned()));
        }
        if let Some((enum_name, member)) = name.split_once('.')
            && let Some(members) = self.enums.get(enum_name)
        {
            if !members.iter().any(|m| m.name == member) {
                return Err(Diagnostic::error(
                    "PINE4025",
                    DiagnosticCategory::Runtime,
                    format!("Enum '{enum_name}' has no member '{member}'."),
                    range,
                ));
            }
            return Ok(RuntimeValue::String(name.to_owned()));
        }
        // A dotted name no table knows is a named constant (`label.style_label_up`) unless its
        // namespace holds variables: there an unknown member is a feature this engine lacks, and
        // handing the script a string would hide that.
        if let Some((namespace, _)) = name.split_once('.')
            && !VARIABLE_NAMESPACES.contains(&namespace)
        {
            return Ok(RuntimeValue::String(name.to_owned()));
        }
        Err(Diagnostic::error(
            "PINE4008",
            DiagnosticCategory::Runtime,
            format!("Undefined variable '{name}'."),
            range,
        ))
    }

    /// `line.all`, `label.all`, `box.all`, `table.all`, `linefill.all`, `polyline.all`: an array of the
    /// handles of every drawing of that kind the script has alive, oldest first.
    fn all_drawings(&mut self, name: &str) -> Option<RuntimeValue> {
        fn handles<T>(kind: RefKind, objects: &FastMap<usize, T>) -> Vec<RuntimeValue> {
            let mut ids: Vec<usize> = objects.keys().copied().collect();
            ids.sort_unstable();
            ids.into_iter()
                .map(|id| RuntimeValue::Ref(kind, id))
                .collect()
        }
        let items = match name {
            "line.all" => handles(RefKind::Line, &self.working.lines),
            "label.all" => handles(RefKind::Label, &self.working.labels),
            "box.all" => handles(RefKind::Box, &self.working.boxes),
            "table.all" => handles(RefKind::Table, &self.working.tables),
            "linefill.all" => handles(RefKind::Linefill, &self.working.linefills),
            "polyline.all" => handles(RefKind::Polyline, &self.working.polylines),
            _ => return None,
        };
        Some(array_builtins::new_array(&mut self.working, items))
    }

    /// `object.field.field`: the parser folds a dotted chain on a plain name into one identifier, so
    /// a chain whose head is a variable is read here, field by field. None when the head is no variable.
    fn field_path(
        &self,
        name: &str,
        range: SourceRange,
    ) -> Result<Option<RuntimeValue>, Diagnostic> {
        let Some((head, rest)) = name.split_once('.') else {
            return Ok(None);
        };
        // A variable that is not an object (`font = ...` shadowing the `font.*` constants) leaves the
        // name to the namespace lookup.
        let Some(mut value) = self
            .working
            .variables
            .get(head)
            .filter(|value| {
                matches!(
                    value,
                    RuntimeValue::Ref(RefKind::Object, _) | RuntimeValue::Na
                )
            })
            .cloned()
        else {
            return Ok(None);
        };
        for field in rest.split('.') {
            value = self.read_field(field, value, range)?;
        }
        Ok(Some(value))
    }

    fn market(&self, name: &str, context: &RuntimeContext) -> Option<RuntimeValue> {
        let bar = &context.bar;
        match name {
            "open" => Some(RuntimeValue::Float(bar.open)),
            "high" => Some(RuntimeValue::Float(bar.high)),
            "low" => Some(RuntimeValue::Float(bar.low)),
            "close" => Some(RuntimeValue::Float(bar.close)),
            "volume" => Some(RuntimeValue::Float(bar.volume)),
            "hl2" => Some(RuntimeValue::Float((bar.high + bar.low) / 2.0)),
            "hlc3" => Some(RuntimeValue::Float((bar.high + bar.low + bar.close) / 3.0)),
            "ohlc4" => Some(RuntimeValue::Float(
                (bar.open + bar.high + bar.low + bar.close) / 4.0,
            )),
            "hlcc4" => Some(RuntimeValue::Float(
                (bar.high + bar.low + 2.0 * bar.close) / 4.0,
            )),
            "time" => Some(RuntimeValue::Int(bar.open_time)),
            "time_close" => Some(RuntimeValue::Int(
                bar.open_time + (self.bar_seconds * 1000.0) as i64,
            )),
            "bar_index" => Some(RuntimeValue::Int(self.working.bar_index)),
            "last_bar_index" => Some(RuntimeValue::Int(self.last_bar_index)),
            "last_bar_time" => self
                .last_bar_time
                .map(RuntimeValue::Int)
                .or(Some(RuntimeValue::Na)),
            "year" | "month" | "dayofmonth" | "hour" | "minute" | "second" | "dayofweek" => {
                let c = calendar::components(bar.open_time);
                Some(RuntimeValue::Int(match name {
                    "year" => c.year,
                    "month" => c.month,
                    "dayofmonth" => c.day,
                    "hour" => c.hour,
                    "minute" => c.minute,
                    "second" => c.second,
                    "dayofweek" => c.weekday,
                    _ => unreachable!(),
                }))
            }
            _ => {
                if let Some(value) =
                    strategy_builtins::strategy_value(name, &self.working.broker, bar.close)
                {
                    return Some(value);
                }
                if let Some(value) = market_values::read(
                    name,
                    &market_values::MarketContext {
                        state: &self.working,
                        bar: &context.bar,
                        symbol: &self.symbol,
                        bar_seconds: self.bar_seconds,
                        visible_range: self.visible_range,
                        first_bar_time: self.first_bar_time,
                        last_bar_time: self.last_bar_time,
                        theme: self.theme,
                    },
                ) {
                    return Some(value);
                }
                self.symbol_value(name)
            }
        }
    }

    fn symbol_value(&self, name: &str) -> Option<RuntimeValue> {
        match name {
            "syminfo.mintick" => Some(RuntimeValue::Float(self.mintick)),
            "syminfo.ticker" => Some(RuntimeValue::String(self.symbol.ticker.clone())),
            "syminfo.tickerid" => Some(RuntimeValue::String(self.symbol.ticker_id.clone())),
            "syminfo.currency" => Some(RuntimeValue::String(self.symbol.currency.clone())),
            "syminfo.root" => Some(RuntimeValue::String(self.symbol.ticker.clone())),
            "syminfo.pointvalue" => Some(RuntimeValue::Float(1.0)),
            "syminfo.type" => Some(RuntimeValue::String(self.symbol.kind.clone())),
            "syminfo.basecurrency" => self
                .symbol
                .base_currency
                .clone()
                .map(RuntimeValue::String)
                .or(Some(RuntimeValue::Na)),
            _ => None,
        }
    }

    fn eval_unary(
        &mut self,
        op: UnaryOperator,
        operand: &Expression,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        let value = self.eval(operand, context)?;
        match op {
            UnaryOperator::Negate => Ok(operators::negate(&value)),
            UnaryOperator::Plus => Ok(value),
            UnaryOperator::Not => {
                let b = Self::truth(value).ok_or_else(|| {
                    Diagnostic::error(
                        "PINE4002",
                        DiagnosticCategory::Runtime,
                        "not requires bool.",
                        range,
                    )
                })?;
                Ok(RuntimeValue::Bool(!b))
            }
        }
    }

    fn eval_binary(
        &mut self,
        left: &Expression,
        op: BinaryOperator,
        right: &Expression,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        let lhs = self.eval(left, context)?;
        match op {
            BinaryOperator::And => self.eval_logical(true, lhs, right, range, context),
            BinaryOperator::Or => self.eval_logical(false, lhs, right, range, context),
            _ => Ok(operators::apply(op, &lhs, &self.eval(right, context)?)),
        }
    }

    fn eval_logical(
        &mut self,
        is_and: bool,
        lhs: RuntimeValue,
        right: &Expression,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        let word = if is_and { "and" } else { "or" };
        let code = if is_and { "PINE4003" } else { "PINE4004" };
        let error = |r: SourceRange| {
            Diagnostic::error(
                code,
                DiagnosticCategory::Runtime,
                format!("{word} requires bool operands."),
                r,
            )
        };
        let left = Self::truth(lhs).ok_or_else(|| error(range))?;
        if left != is_and {
            return Ok(RuntimeValue::Bool(left));
        }
        let rhs = Self::truth(self.eval(right, context)?).ok_or_else(|| error(range))?;
        Ok(RuntimeValue::Bool(rhs))
    }

    fn eval_history(
        &mut self,
        base: &Expression,
        offset: &Expression,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        // An offset that is `na` (or not a finite number) reaches no bar: the result is `na`.
        let Some(n) = self
            .eval(offset, context)?
            .number()
            .filter(|n| n.is_finite())
        else {
            return Ok(RuntimeValue::Na);
        };
        let Some(i) = RuntimeValue::integral(n).int_value() else {
            if n < 0.0 {
                return Err(Diagnostic::error(
                    "PINE4006",
                    DiagnosticCategory::Runtime,
                    "History offset cannot be negative.",
                    range,
                ));
            }
            return Ok(RuntimeValue::Na);
        };
        if i < 0 {
            return Err(Diagnostic::error(
                "PINE4006",
                DiagnosticCategory::Runtime,
                "History offset cannot be negative.",
                range,
            ));
        }
        if i == 0 {
            return self.eval(base, context);
        }
        let key = match base {
            // A local of the running function that the body reads a history of keeps one series per
            // call site, because the local itself is gone between calls.
            Expression::Identifier(name, _)
                if context
                    .history_locals
                    .as_ref()
                    .is_some_and(|locals| locals.contains(name)) =>
            {
                format!("fn:{}:{name}", context.site_prefix)
            }
            Expression::Identifier(name, _) => name.clone(),
            _ => {
                let key = format!(
                    "{INTERNAL_PREFIX}expr:{}",
                    self.site_key(range.start.offset, context)
                );
                let value = self.eval(base, context)?;
                self.working.expression_values.insert(key.clone(), value);
                key
            }
        };
        let i = i as usize;
        Ok(match self.working.histories.get(&key) {
            Some(history) if i <= history.len() => history[history.len() - i].clone(),
            _ => RuntimeValue::Na,
        })
    }

    // MARK: Statements

    fn run(
        &mut self,
        statements: &[Statement],
        context: &mut RuntimeContext,
    ) -> Result<(Flow, RuntimeValue), Diagnostic> {
        // One test per block, not per statement: a session with a debugger or a profiler takes the
        // loop below that carries their hooks, every other session this one.
        if self.hooked() {
            return self.run_hooked(statements, context);
        }
        let mut last = RuntimeValue::Void;
        for statement in statements {
            self.budget()?;
            let step = self.run_statement(statement, context)?;
            if step.value != RuntimeValue::Void {
                last = step.value;
            }
            if step.flow != Flow::Normal {
                return Ok((step.flow, last));
            }
        }
        Ok((Flow::Normal, last))
    }

    /// Whether a debugger or a profiler is attached.
    #[inline]
    fn hooked(&self) -> bool {
        self.debug.is_some() || self.profile.is_some()
    }

    /// `run` for a session with a debugger or a profiler: the same statements in the same order, with
    /// the hooks around each.
    #[inline(never)]
    fn run_hooked(
        &mut self,
        statements: &[Statement],
        context: &mut RuntimeContext,
    ) -> Result<(Flow, RuntimeValue), Diagnostic> {
        let mut last = RuntimeValue::Void;
        for statement in statements {
            self.budget()
                .map_err(|error| self.debug_failed(statement, context, error))?;
            let debugging = self.debug.is_some();
            if debugging {
                self.debug_before(statement, context)?;
            }
            let timed = self.profile_enter_statement(statement, context);
            let outcome = self.run_statement(statement, context);
            if timed {
                self.profile_exit();
            }
            let step = outcome.map_err(|error| self.debug_failed(statement, context, error))?;
            if debugging {
                self.debug_after(statement, context);
            }
            if step.value != RuntimeValue::Void {
                last = step.value;
            }
            if step.flow != Flow::Normal {
                return Ok((step.flow, last));
            }
        }
        Ok((Flow::Normal, last))
    }

    // MARK: Profiling hooks
    //
    // The recorder is taken from `self` by reference only; it never needs the session.

    /// A statement is about to run. Returns whether it is being timed, so that its end is reported.
    /// Library code is not timed: its offsets belong to another text, and its time is part of the
    /// statement that called it.
    #[inline(never)]
    fn profile_enter_statement(&mut self, statement: &Statement, context: &RuntimeContext) -> bool {
        // Walking past a function's declaration runs nothing; its calls are timed in `invoke`.
        if context.scope.is_some() || matches!(statement, Statement::Function { .. }) {
            return false;
        }
        match self.profile.as_mut() {
            Some(profile) => profile.enter_statement(statement.range().start.offset),
            None => false,
        }
    }

    #[inline(never)]
    fn profile_exit(&mut self) {
        if let Some(profile) = self.profile.as_mut() {
            profile.exit();
        }
    }

    /// A user function's body is about to run, called from the call at `call`. Returns whether it is
    /// being timed.
    #[inline(never)]
    fn profile_enter_function(&mut self, function: &RuntimeFunction, call: SourceRange) -> bool {
        if function.scope.is_some() {
            return false;
        }
        let (Some(profile), Some(first)) = (self.profile.as_mut(), function.body.first()) else {
            return false;
        };
        profile.enter_function(first.range().start.offset, call.start.offset)
    }

    #[inline(never)]
    fn profile_begin_bar(&mut self) {
        if let Some(profile) = self.profile.as_mut() {
            profile.begin_bar();
        }
    }

    /// The bar ended. What measuring it added is given back to the deadline: a profiled run is
    /// slower than a normal one, and must not time out where the normal one would not.
    #[inline(never)]
    fn profile_end_bar(&mut self, counted: bool, realtime: bool) {
        let Some(profile) = self.profile.as_mut() else {
            return;
        };
        let added = profile.end_bar(counted, realtime);
        if let Some(deadline) = self.deadline_at.as_mut() {
            *deadline += added;
        }
    }

    // MARK: Debugging hooks
    //
    // Each takes the debugger out of the session while it runs, so it can be handed `&mut self`.
    // They are cold and never inlined: a session without a debugger must not carry their code in the
    // statement loop.

    #[cold]
    #[inline(never)]
    fn debug_before(
        &mut self,
        statement: &Statement,
        context: &RuntimeContext,
    ) -> Result<(), Diagnostic> {
        let Some(mut debug) = self.debug.take() else {
            return Ok(());
        };
        let outcome = debug.before_statement(self, statement, context);
        self.debug = Some(debug);
        outcome
    }

    /// `statement` ran to its end: whatever it declares now has a value this bar.
    #[cold]
    #[inline(never)]
    fn debug_after(&mut self, statement: &Statement, context: &RuntimeContext) {
        if let Some(debug) = self.debug.as_mut() {
            debug.after_statement(statement, context);
        }
    }

    /// Passes a failure of `statement` through the debugger on its way up, and hands it back.
    #[cold]
    #[inline(never)]
    fn debug_failed(
        &mut self,
        statement: &Statement,
        context: &RuntimeContext,
        error: Diagnostic,
    ) -> Diagnostic {
        if let Some(mut debug) = self.debug.take() {
            debug.failed(self, context, statement, &error);
            self.debug = Some(debug);
        }
        error
    }

    #[cold]
    #[inline(never)]
    fn debug_enter(
        &mut self,
        call_name: &str,
        range: SourceRange,
        scope: Option<&str>,
        shadowed: &[(String, Option<RuntimeValue>)],
    ) {
        if let Some(debug) = self.debug.as_mut() {
            debug.enter_function(call_name, range, scope, shadowed.to_vec());
        }
    }

    #[inline]
    pub(crate) fn debug_is_attached(&self) -> bool {
        self.debug.is_some()
    }

    /// A `request.security()` expression is about to run on `symbol` at `interval_seconds`.
    #[cold]
    #[inline(never)]
    pub(crate) fn debug_enter_security(&mut self, symbol: &str, interval_seconds: f64) {
        if let Some(debug) = self.debug.as_mut() {
            debug.enter_security(symbol, interval_seconds);
        }
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn debug_leave(&mut self) {
        if let Some(debug) = self.debug.as_mut() {
            debug.leave_function();
        }
    }

    fn run_statement(
        &mut self,
        statement: &Statement,
        context: &mut RuntimeContext,
    ) -> Result<Step, Diagnostic> {
        // A name a block declares shadows an outer variable of the same name only inside the block;
        // the variable table is flat, so the outer value is put back when the statement ends.
        let shadowed = self.shadowed_by(statement);
        let result = self.run_statement_unscoped(statement, context);
        for (name, value) in shadowed {
            match value {
                Some(value) => self.working.variables.insert(name, value),
                None => self.working.variables.remove(&name),
            };
        }
        result
    }

    /// The current values of the variables that `statement`'s blocks declare anew (a loop counter,
    /// an ordinary `x = …` in a branch or body), for those that already exist outside it.
    fn shadowed_by(&self, statement: &Statement) -> Vec<(String, Option<RuntimeValue>)> {
        let mut names: Vec<&str> = Vec::new();
        match statement {
            Statement::ForRange { variable, .. } => names.push(variable),
            Statement::ForIn { index, value, .. } => {
                names.extend(index.as_deref());
                names.push(value);
            }
            Statement::Conditional { .. }
            | Statement::WhileLoop { .. }
            | Statement::Switch { .. } => {}
            _ => return Vec::new(),
        }
        for block in super::traversal::nested_blocks(statement) {
            for inner in block {
                match inner {
                    Statement::Declaration {
                        name,
                        mode: DeclarationMode::Ordinary,
                        ..
                    } => names.push(name),
                    Statement::TupleDeclaration { names: tuple, .. } => {
                        names.extend(tuple.iter().map(String::as_str));
                    }
                    _ => {}
                }
            }
        }
        // A name that is a builtin (`low`) has no variable outside the block, so it is removed after.
        names
            .into_iter()
            .filter_map(|name| match self.working.variables.get(name) {
                Some(value) => Some((name.to_owned(), Some(value.clone()))),
                None if self.shadowing_builtins.iter().any(|shadow| shadow == name) => {
                    Some((name.to_owned(), None))
                }
                None => None,
            })
            .collect()
    }

    fn run_statement_unscoped(
        &mut self,
        statement: &Statement,
        context: &mut RuntimeContext,
    ) -> Result<Step, Diagnostic> {
        match statement {
            Statement::Declaration {
                name, mode, value, ..
            } => self.run_declaration(name, *mode, value, context),
            Statement::Assignment {
                name, op, value, ..
            } => self.run_assignment(name, *op, value, context),
            Statement::Expression(expression) => Ok(Step::normal(self.eval(expression, context)?)),
            Statement::Conditional {
                condition,
                when_true,
                when_false,
                ..
            } => self.run_conditional(condition, when_true, when_false, context),
            Statement::ForRange {
                variable,
                from,
                to,
                step,
                body,
                range,
            } => self.run_for_range(variable, from, to, step.as_ref(), body, *range, context),
            Statement::ForIn {
                index,
                value,
                collection,
                body,
                range,
            } => self.run_for_in(index.as_deref(), value, collection, body, *range, context),
            Statement::WhileLoop {
                condition, body, ..
            } => self.run_while(condition, body, context),
            Statement::Switch {
                subject,
                arms,
                range,
            } => self.run_switch(subject.as_ref(), arms, *range, context),
            Statement::TupleDeclaration { names, value, .. } => {
                self.run_tuple_declaration(names, value, context)
            }
            Statement::LoopControl(control, _) => Ok(Step {
                flow: if *control == super::ast::LoopControl::Break {
                    Flow::Break
                } else {
                    Flow::Continue
                },
                value: RuntimeValue::Void,
            }),
            Statement::FieldAssignment {
                target,
                op,
                value,
                range,
            } => self.run_field_assignment(target, *op, value, *range, context),
            Statement::Function { .. }
            | Statement::TypeDeclaration { .. }
            | Statement::EnumDeclaration { .. } => Ok(Step {
                flow: Flow::Normal,
                value: RuntimeValue::Void,
            }),
        }
    }

    fn run_declaration(
        &mut self,
        name: &str,
        mode: DeclarationMode,
        expression: &Expression,
        context: &mut RuntimeContext,
    ) -> Result<Step, Diagnostic> {
        if mode == DeclarationMode::Variable
            && let Some(existing) = self.working.variables.get(name)
        {
            return Ok(Step::normal(existing.clone()));
        }
        if mode == DeclarationMode::Intrabar
            && let Some(value) = self
                .intrabar
                .get(name)
                .or_else(|| self.working.variables.get(name))
        {
            let value = value.clone();
            self.working
                .variables
                .insert(name.to_owned(), value.clone());
            self.intrabar.insert(name.to_owned(), value.clone());
            context.record(mode, name);
            return Ok(Step::normal(value));
        }
        let value = self.eval(expression, context)?;
        self.set_variable(name, value.clone());
        context.record(mode, name);
        if mode == DeclarationMode::Intrabar {
            self.intrabar.insert(name.to_owned(), value.clone());
        }
        Ok(Step::normal(value))
    }

    /// Sets a variable, allocating its name only the first time it is seen.
    fn set_variable(&mut self, name: &str, value: RuntimeValue) {
        match self.working.variables.get_mut(name) {
            Some(slot) => *slot = value,
            None => {
                self.working.variables.insert(name.to_owned(), value);
            }
        }
    }

    fn run_assignment(
        &mut self,
        name: &str,
        op: AssignmentOperator,
        expression: &Expression,
        context: &mut RuntimeContext,
    ) -> Result<Step, Diagnostic> {
        let rhs = self.eval(expression, context)?;
        let old = self
            .working
            .variables
            .get(name)
            .cloned()
            .unwrap_or(RuntimeValue::Na);
        let value = operators::apply_assignment(op, &old, &rhs);
        self.set_variable(name, value.clone());
        if context.intrabar_names.contains(name) || self.intrabar.contains_key(name) {
            self.intrabar.insert(name.to_owned(), value.clone());
        }
        Ok(Step::normal(value))
    }

    fn run_tuple_declaration(
        &mut self,
        names: &[String],
        expression: &Expression,
        context: &mut RuntimeContext,
    ) -> Result<Step, Diagnostic> {
        let value = self.eval(expression, context)?;
        let items = match &value {
            RuntimeValue::Tuple(items) => items.clone(),
            _ => Vec::new(),
        };
        for (i, name) in names.iter().enumerate() {
            if name != "_" {
                self.set_variable(name, items.get(i).cloned().unwrap_or(RuntimeValue::Na));
            }
        }
        Ok(Step::normal(value))
    }

    fn run_conditional(
        &mut self,
        condition: &Expression,
        when_true: &[Statement],
        when_false: &[Statement],
        context: &mut RuntimeContext,
    ) -> Result<Step, Diagnostic> {
        let value = self.eval(condition, context)?;
        let test = self.require_bool(
            value,
            "if condition must be bool; numeric-to-bool coercion is not allowed in v6.",
            condition.range(),
        )?;
        let (flow, value) = self.run(if test { when_true } else { when_false }, context)?;
        Ok(Step { flow, value })
    }

    #[allow(clippy::too_many_arguments)]
    fn run_for_range(
        &mut self,
        variable: &str,
        from: &Expression,
        to: &Expression,
        step_expression: Option<&Expression>,
        body: &[Statement],
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<Step, Diagnostic> {
        let start_value = self.eval(from, context)?;
        let Some(start) = start_value.number() else {
            return Ok(Step {
                flow: Flow::Normal,
                value: RuntimeValue::Void,
            });
        };
        let Some(stop) = self.eval(to, context)?.number() else {
            return Ok(Step {
                flow: Flow::Normal,
                value: RuntimeValue::Void,
            });
        };
        let mut stride = 1.0;
        let mut integral = start_value.is_int();
        if let Some(step_expression) = step_expression {
            let step_value = self.eval(step_expression, context)?;
            let Some(size) = step_value.number() else {
                return Err(Diagnostic::error(
                    "PINE4014",
                    DiagnosticCategory::Runtime,
                    "for loop step must be a non-zero number.",
                    range,
                ));
            };
            if size == 0.0 || !size.is_finite() {
                return Err(Diagnostic::error(
                    "PINE4014",
                    DiagnosticCategory::Runtime,
                    "for loop step must be a non-zero number.",
                    range,
                ));
            }
            stride = size.abs();
            if !step_value.is_int() {
                integral = false;
            }
        }
        let direction = if stop >= start { 1.0 } else { -1.0 };
        let mut last = RuntimeValue::Void;
        let mut i = start;
        while if direction > 0.0 {
            i <= stop
        } else {
            i >= stop
        } {
            self.budget()?;
            self.set_variable(
                variable,
                if integral {
                    RuntimeValue::integral(i)
                } else {
                    RuntimeValue::Float(i)
                },
            );
            let (flow, value) = self.run(body, context)?;
            last = value;
            if flow == Flow::Break {
                break;
            }
            i += direction * stride;
        }
        Ok(Step {
            flow: Flow::Normal,
            value: last,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn run_for_in(
        &mut self,
        index_name: Option<&str>,
        value_name: &str,
        collection: &Expression,
        body: &[Statement],
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<Step, Diagnostic> {
        let target = self.eval(collection, context)?;
        if target == RuntimeValue::Na {
            return Ok(Step {
                flow: Flow::Normal,
                value: RuntimeValue::Void,
            });
        }
        let RuntimeValue::Ref(RefKind::Array, id) = target else {
            return Err(Diagnostic::error(
                "PINE4011",
                DiagnosticCategory::Runtime,
                "for...in requires an array or a map.",
                range,
            ));
        };
        let items = self.working.arrays.get(&id).cloned().unwrap_or_default();
        let mut last = RuntimeValue::Void;
        for (offset, item) in items.into_iter().enumerate() {
            self.budget()?;
            if let Some(index_name) = index_name {
                self.set_variable(index_name, RuntimeValue::Int(offset as i64));
            }
            self.set_variable(value_name, item);
            let (flow, value) = self.run(body, context)?;
            last = value;
            if flow == Flow::Break {
                break;
            }
        }
        Ok(Step {
            flow: Flow::Normal,
            value: last,
        })
    }

    fn run_while(
        &mut self,
        condition: &Expression,
        body: &[Statement],
        context: &mut RuntimeContext,
    ) -> Result<Step, Diagnostic> {
        let mut last = RuntimeValue::Void;
        loop {
            self.budget()?;
            let value = self.eval(condition, context)?;
            let test =
                self.require_bool(value, "while condition must be bool.", condition.range())?;
            if !test {
                break;
            }
            let (flow, value) = self.run(body, context)?;
            last = value;
            if flow == Flow::Break {
                break;
            }
        }
        Ok(Step {
            flow: Flow::Normal,
            value: last,
        })
    }

    fn run_switch(
        &mut self,
        subject: Option<&Expression>,
        arms: &[SwitchArm],
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<Step, Diagnostic> {
        let target = match subject {
            Some(subject) => Some(self.eval(subject, context)?),
            None => None,
        };
        let Some(chosen) = self.choose_arm(arms, target.as_ref(), range, context)? else {
            return Ok(Step {
                flow: Flow::Normal,
                value: RuntimeValue::Na,
            });
        };
        let (flow, value) = self.run(&arms[chosen].body, context)?;
        Ok(Step { flow, value })
    }

    fn choose_arm(
        &mut self,
        arms: &[SwitchArm],
        target: Option<&RuntimeValue>,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<Option<usize>, Diagnostic> {
        let mut fallback = None;
        for (index, arm) in arms.iter().enumerate() {
            let Some(condition) = &arm.condition else {
                fallback = fallback.or(Some(index));
                continue;
            };
            let value = self.eval(condition, context)?;
            let matches = if let Some(target) = target {
                operators::apply(BinaryOperator::Equal, target, &value) == RuntimeValue::Bool(true)
            } else {
                self.require_bool(value, "switch arm condition must be bool.", range)?
            };
            if matches {
                return Ok(Some(index));
            }
        }
        Ok(fallback)
    }

    fn require_bool(
        &self,
        value: RuntimeValue,
        message: &str,
        range: SourceRange,
    ) -> Result<bool, Diagnostic> {
        Self::truth(value).ok_or_else(|| {
            Diagnostic::error("PINE4001", DiagnosticCategory::Runtime, message, range)
        })
    }

    pub fn truth(value: RuntimeValue) -> Option<bool> {
        match value {
            RuntimeValue::Bool(b) => Some(b),
            RuntimeValue::Na => Some(false),
            _ => None,
        }
    }

    // MARK: Calls

    pub(crate) fn call(
        &mut self,
        name: &str,
        arguments: &[super::ast::Argument],
        site: usize,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        // A script-defined (or library) type constructor: `Type.new(...)` / `alias.Type.new(...)`.
        if let Some(object) = self.constructor_call(name, arguments, range, context)? {
            return Ok(object);
        }
        // `area(rect)`: a method called as a function runs the overload that fits its first argument.
        if self.methods.contains_key(name)
            && let Some(first) = arguments.first().filter(|argument| argument.name.is_none())
        {
            let receiver = self.eval(&first.value, context)?;
            if let Some(result) =
                self.call_user_method(name, receiver, &arguments[1..], site, range, context)?
            {
                return Ok(result);
            }
        }
        // A function declared several times with different parameter types picks one per call.
        if let Some(key) = self.scoped_function_key(name, range, context)?
            && let Some(candidates) = self.overloads.get(&key).cloned()
        {
            return self.call_overloaded(name, &candidates, arguments, site, range, context);
        }
        // User function (own, or a library's via an import alias).
        if let Some(function) = self.scoped_function(name, range, context)?
            && self.methods.get(name).is_none_or(|m| m.len() <= 1)
        {
            return self.invoke(function, name, arguments, site, range, context);
        }
        // The declaration and input builtins are no-ops here (their values are read at compile time).
        match name {
            "indicator" | "strategy" | "library" => return Ok(RuntimeValue::Na),
            _ if name.starts_with("input.") || name == "input" => {
                return self.input_value(site, range, context);
            }
            _ => {}
        }
        if let Some(value) = self.scalar_call(name, arguments, range, context)? {
            return Ok(value);
        }
        if let Some(value) = self.collection_call(name, arguments, range, context)? {
            return Ok(value);
        }
        if name == "str.split" {
            let values = self.eval_all(arguments, context)?;
            let subject = values
                .first()
                .and_then(RuntimeValue::text_value)
                .unwrap_or_default();
            let separator = values
                .get(1)
                .and_then(RuntimeValue::text_value)
                .unwrap_or_default();
            let pieces = strings::split(subject, separator)
                .into_iter()
                .map(RuntimeValue::String)
                .collect();
            return Ok(array_builtins::new_array(&mut self.working, pieces));
        }
        if math::SUPPORTED.contains(&name) && name != "math.sum" {
            let values = self.eval_all(arguments, context)?;
            return Ok(math::call(name, &values, self.mintick));
        }
        if strings::SUPPORTED.contains(&name) && name != "str.tostring" {
            let values = self.eval_all(arguments, context)?;
            return strings::call(name, &values, range, self.mintick);
        }
        if matches!(name, "ta.sar" | "ta.supertrend") {
            let values = self.eval_all(arguments, context)?;
            let key = self.site_key(site, context);
            let state = self.working.ta_states.entry(key).or_default();
            return Ok(ta::evaluate_price(name, &values, &context.bar, state));
        }
        if name == "fixnan" {
            let source = self.arg_value(arguments, 0, Some("source"), context)?;
            let key = self.site_key(site, context);
            let state = self.working.ta_states.entry(key).or_default();
            return Ok(ta::evaluate_fixnan(&source, state));
        }
        if name == "ta.tr" {
            // `ta.tr(handle_na)` reads like the `ta.tr` variable.
            return Ok(self.market("ta.tr", context).unwrap_or(RuntimeValue::Na));
        }
        if name == "runtime.error" {
            let message = self.arg_value(arguments, 0, Some("message"), context)?;
            let text = match message {
                RuntimeValue::String(text) => text,
                _ => "runtime.error()".to_owned(),
            };
            return Err(Diagnostic::error(
                "PINE4030",
                DiagnosticCategory::Runtime,
                text,
                range,
            ));
        }
        if name == "ta.barssince" {
            let condition = self.arg_value(arguments, 0, Some("condition"), context)?;
            let key = self.site_key(site, context);
            let state = self.working.ta_states.entry(key).or_default();
            return Ok(ta::evaluate_barssince(&condition, state));
        }
        if name == "ta.vwap" {
            let bound = self.bind_named(arguments, &["source", "anchor", "stdev_mult"], context)?;
            let price = bound
                .get("source")
                .and_then(RuntimeValue::number)
                .unwrap_or(context.bar.close);
            let anchor = bound.get("anchor").and_then(RuntimeValue::bool);
            let multiplier = bound.get("stdev_mult").and_then(RuntimeValue::number);
            let key = self.site_key(site, context);
            let state = self.working.ta_states.entry(key).or_default();
            return Ok(ta::evaluate_vwap(
                price,
                &context.bar,
                anchor,
                multiplier,
                state,
            ));
        }
        if name == "ta.dmi" {
            let di = self.arg_value(arguments, 0, Some("diLength"), context)?;
            let smoothing = self.arg_value(arguments, 1, Some("adxSmoothing"), context)?;
            let key = self.site_key(site, context);
            return Ok(ta::evaluate_dmi(
                &mut self.working.ta_states,
                key,
                &context.bar,
                di.int_value().unwrap_or(0),
                smoothing.int_value().unwrap_or(0),
            ));
        }
        if name == "ta.cum" {
            let source = self.arg_value(arguments, 0, Some("source"), context)?;
            let key = self.site_key(site, context);
            let state = self.working.ta_states.entry(key).or_default();
            return Ok(ta::evaluate_cum(&source, state));
        }
        if name == "ta.correlation" {
            let first = self.arg_value(arguments, 0, Some("source1"), context)?;
            let second = self.arg_value(arguments, 1, Some("source2"), context)?;
            let length = self.arg_value(arguments, 2, Some("length"), context)?;
            let key = self.site_key(site, context);
            let state = self.working.ta_states.entry(key).or_default();
            return Ok(ta::evaluate_correlation(
                &first,
                &second,
                length.int_value().unwrap_or(0),
                state,
            ));
        }
        if name == "ta.atr" {
            let length = self.arg_value(arguments, 0, Some("length"), context)?;
            let key = self.site_key(site, context);
            let state = self.working.ta_states.entry(key).or_default();
            return Ok(ta::evaluate_atr(
                state,
                &context.bar,
                length.int_value().unwrap_or(14),
            ));
        }
        if name == "ta.bb" {
            let source = self.arg_value(arguments, 0, Some("series"), context)?;
            let length = self.arg_value(arguments, 1, Some("length"), context)?;
            let multiplier = self.arg_value(arguments, 2, Some("mult"), context)?;
            let key = self.site_key(site, context);
            let state = self.working.ta_states.entry(key).or_default();
            return Ok(ta::evaluate_bb(
                &source,
                length.int_value().unwrap_or(0),
                multiplier.number().unwrap_or(2.0),
                state,
            ));
        }
        if matches!(name, "ta.pivothigh" | "ta.pivotlow") {
            let high = name == "ta.pivothigh";
            // `(source, leftbars, rightbars)`, or `(leftbars, rightbars)` reading the bar's own
            // high or low.
            let positional = arguments.iter().filter(|a| a.name.is_none()).count();
            let (source, left, right) = if positional >= 3
                || arguments
                    .iter()
                    .any(|a| a.name.as_deref() == Some("source"))
            {
                (
                    self.arg_value(arguments, 0, Some("source"), context)?,
                    self.arg_value(arguments, 1, Some("leftbars"), context)?,
                    self.arg_value(arguments, 2, Some("rightbars"), context)?,
                )
            } else {
                let own = if high {
                    context.bar.high
                } else {
                    context.bar.low
                };
                (
                    RuntimeValue::Float(own),
                    self.arg_value(arguments, 0, Some("leftbars"), context)?,
                    self.arg_value(arguments, 1, Some("rightbars"), context)?,
                )
            };
            let key = self.site_key(site, context);
            let state = self.working.ta_states.entry(key).or_default();
            return Ok(ta::evaluate_pivot(
                &source,
                left.int_value().unwrap_or(0),
                right.int_value().unwrap_or(0),
                high,
                state,
            ));
        }
        if name == "ta.linreg" {
            let bound = self.bind_named(arguments, &["source", "length", "offset"], context)?;
            let source = bound.get("source").cloned().unwrap_or(RuntimeValue::Na);
            let length = bound
                .get("length")
                .and_then(RuntimeValue::int_value)
                .unwrap_or(0);
            let offset = bound.get("offset").cloned().unwrap_or(RuntimeValue::Na);
            let key = self.site_key(site, context);
            let state = self.working.ta_states.entry(key).or_default();
            return Ok(ta::evaluate(name, &source, &offset, length, state));
        }
        if ta::is_supported(name) {
            let default_source = match name {
                "ta.highest" | "ta.highestbars" => Some("high"),
                "ta.lowest" | "ta.lowestbars" => Some("low"),
                _ => None,
            };
            let uses_default_source = default_source.is_some()
                && arguments.len() == 1
                && !arguments
                    .iter()
                    .any(|argument| argument.name.as_deref() == Some("source"));
            let (source, second) = if uses_default_source {
                (
                    self.market(default_source.unwrap_or_default(), context)
                        .unwrap_or(RuntimeValue::Na),
                    self.arg_value(arguments, 0, Some("length"), context)?,
                )
            } else {
                (
                    self.arg_value(arguments, 0, Some("source"), context)?,
                    self.arg_value(arguments, 1, Some("length"), context)?,
                )
            };
            let length = second.int_value().unwrap_or(0);
            let key = self.site_key(site, context);
            let state = self.working.ta_states.entry(key).or_default();
            return Ok(ta::evaluate(name, &source, &second, length, state));
        }
        if let Some(value) = self.extra_ta_call(name, arguments, site, range, context)? {
            return Ok(value);
        }
        if let Some(value) = self.misc_call(name, arguments, range, context)? {
            return Ok(value);
        }
        if drawings::handles(name) {
            let args = self.eval_args(arguments, context)?;
            return drawings::call(
                name,
                &args,
                &mut self.working,
                &context.bar,
                self.bar_seconds,
                &self.drawing_limits,
                range,
            );
        }
        if visuals::handles(name) {
            let args = self.eval_args(arguments, context)?;
            let key = self.site_key(site, context);
            return visuals::call(name, &args, key, &mut self.working, range);
        }
        if name.starts_with("strategy.closedtrades.") || name.starts_with("strategy.opentrades.") {
            let number = self.arg_value(arguments, 0, Some("trade_num"), context)?;
            return strategy_builtins::trade_property(
                name,
                number.int_value(),
                &self.working.broker,
                context.bar.close,
            )
            .ok_or_else(|| {
                Diagnostic::error(
                    "PINE4007",
                    DiagnosticCategory::Runtime,
                    format!("Unknown or unsupported function '{name}'."),
                    range,
                )
            });
        }
        if strategy_builtins::handles(name) {
            let args = self.eval_args(arguments, context)?;
            let is_strategy =
                self.program.declaration.script_type == super::program::ScriptType::Strategy;
            return strategy_builtins::call(name, &args, &mut self.working, is_strategy, range);
        }
        if alert_builtins::handles(name) {
            let args = self.eval_args(arguments, context)?;
            let key = self.site_key(site, context);
            if let Some(event) = alert_builtins::call(
                name,
                &args,
                key,
                &mut self.working,
                &context.flags,
                &mut self.once_per_bar_ledger,
                &self.symbol,
                self.mintick,
                self.bar_seconds,
                &context.bar,
                range,
            )? {
                self.emit(event);
            }
            return Ok(RuntimeValue::Void);
        }
        if name == "request.security" {
            return self.security_call(arguments, site, range, context);
        }
        if name == "request.security_lower_tf" {
            return self.security_lower_tf_call(arguments, site, range, context);
        }
        // `levels.push(x)`: the lexer reads `name.member` as one dotted name, so a variable (or
        // parameter) holding an array, map or matrix reaches here as a call to `levels.push`.
        if let Some((receiver, member)) = name.rsplit_once('.') {
            let expression = Expression::Identifier(receiver.to_owned(), range);
            // `l.delete()` and `l.set_x1(…)` on a drawing variable that is still `na` do nothing, as
            // the same calls on a missing id do in Pine; a `get_*` gives `na`.
            if let Ok(RuntimeValue::Na) = self.eval(&expression, context)
                && (member == "delete" || member.starts_with("set_") || member.starts_with("get_"))
                && !self.methods.contains_key(member)
            {
                return Ok(if member.starts_with("get_") {
                    RuntimeValue::Na
                } else {
                    RuntimeValue::Void
                });
            }
            if let Ok(value) = self.eval(&expression, context)
                && (matches!(value, RuntimeValue::Ref(..)) || self.methods.contains_key(member))
            {
                return self.method_call_on_value(
                    &expression,
                    member,
                    arguments,
                    site,
                    range,
                    context,
                );
            }
        }
        // Any name still unhandled is genuinely unknown.
        Err(Diagnostic::error(
            "PINE4007",
            DiagnosticCategory::Runtime,
            format!("Unknown or unsupported function '{name}'."),
            range,
        ))
    }

    /// The value an `input.*` call stands for: the user's override, or the declared default, of
    /// the variable it is assigned to. A call that isn't a plain declaration reads `na`.
    fn input_value(
        &mut self,
        site: usize,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        let Some(variable) = self.input_variables.get(&site) else {
            return Ok(RuntimeValue::Na);
        };
        Ok(match self.inputs.get(variable).cloned() {
            Some(InputValue::Int(value)) => RuntimeValue::Int(value),
            Some(InputValue::Float(value)) => RuntimeValue::Float(value),
            Some(InputValue::Bool(value)) => RuntimeValue::Bool(value),
            Some(InputValue::String(value)) => RuntimeValue::String(value),
            Some(InputValue::Color(value)) => RuntimeValue::Color(value),
            Some(InputValue::Source(name)) => self.resolve_identifier(&name, range, context)?,
            None => RuntimeValue::Na,
        })
    }

    pub(crate) fn arg_value(
        &mut self,
        arguments: &[super::ast::Argument],
        index: usize,
        key: Option<&str>,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        if let Some(key) = key
            && let Some(found) = arguments.iter().find(|a| a.name.as_deref() == Some(key))
        {
            return self.eval(&found.value, context);
        }
        if index < arguments.len() {
            return self.eval(&arguments[index].value, context);
        }
        Ok(RuntimeValue::Na)
    }

    pub(crate) fn eval_all(
        &mut self,
        arguments: &[super::ast::Argument],
        context: &mut RuntimeContext,
    ) -> Result<Vec<RuntimeValue>, Diagnostic> {
        arguments
            .iter()
            .map(|argument| self.eval(&argument.value, context))
            .collect()
    }

    /// Evaluates arguments as source-ordered `drawings::Arg`s (name + value), for the handlers
    /// that bind overloads themselves.
    fn eval_args(
        &mut self,
        arguments: &[super::ast::Argument],
        context: &mut RuntimeContext,
    ) -> Result<Vec<drawings::Arg>, Diagnostic> {
        arguments
            .iter()
            .map(|argument| {
                Ok(drawings::Arg {
                    name: argument.name.clone(),
                    value: self.eval(&argument.value, context)?,
                })
            })
            .collect()
    }

    /// Binds arguments by parameter name list (positional first, then named, then `na`), the
    /// builtin-handler analogue of `bind_parameters`.
    pub(crate) fn bind_named(
        &mut self,
        arguments: &[super::ast::Argument],
        parameters: &[&str],
        context: &mut RuntimeContext,
    ) -> Result<FastMap<String, RuntimeValue>, Diagnostic> {
        let positional: Vec<&super::ast::Argument> = arguments
            .iter()
            .filter(|argument| argument.name.is_none())
            .collect();
        let mut bound = FastMap::default();
        for (index, name) in parameters.iter().enumerate() {
            if let Some(named) = arguments
                .iter()
                .find(|argument| argument.name.as_deref() == Some(*name))
            {
                bound.insert((*name).to_owned(), self.eval(&named.value, context)?);
            } else if index < positional.len() {
                bound.insert(
                    (*name).to_owned(),
                    self.eval(&positional[index].value, context)?,
                );
            } else {
                bound.insert((*name).to_owned(), RuntimeValue::Na);
            }
        }
        Ok(bound)
    }

    /// Runs one `array.*`/`map.*`/`matrix.*` call, or `None` when the name is none of them.
    fn collection_call(
        &mut self,
        name: &str,
        arguments: &[super::ast::Argument],
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<Option<RuntimeValue>, Diagnostic> {
        // `array.from` is variadic, so it cannot use the named-parameter binder. Evaluate its
        // arguments exactly once, in source order, and construct the array directly.
        if name == "array.from" {
            let values = self.eval_all(arguments, context)?;
            let value = array_builtins::call(
                name,
                &FastMap::default(),
                &values,
                &mut self.working,
                self.mintick,
                range,
            )?;
            return Ok(Some(value));
        }
        let parameters = if array_builtins::handles(name) {
            array_builtins::parameters(name)
        } else if map_builtins::handles(name) {
            map_builtins::parameters(name)
        } else if matrix_builtins::handles(name) {
            matrix_builtins::parameters(name)
        } else {
            return Ok(None);
        };
        let bound = self.bind_named(arguments, parameters, context)?;
        let value = if array_builtins::handles(name) {
            array_builtins::call(name, &bound, &[], &mut self.working, self.mintick, range)?
        } else if map_builtins::handles(name) {
            map_builtins::call(name, &bound, &mut self.working, range)?
        } else {
            matrix_builtins::call(name, &bound, &mut self.working, range)?
        };
        Ok(Some(value))
    }

    fn scalar_call(
        &mut self,
        name: &str,
        arguments: &[super::ast::Argument],
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<Option<RuntimeValue>, Diagnostic> {
        match name {
            "na" => Ok(Some(RuntimeValue::Bool(
                self.arg_value(arguments, 0, None, context)? == RuntimeValue::Na,
            ))),
            "nz" => {
                let v = self.arg_value(arguments, 0, None, context)?;
                if v != RuntimeValue::Na {
                    return Ok(Some(v));
                }
                let replacement = self.arg_value(arguments, 1, None, context)?;
                Ok(Some(if replacement == RuntimeValue::Na {
                    RuntimeValue::Float(0.0)
                } else {
                    replacement
                }))
            }
            // `int(x)` truncates toward zero; only `na` and non-finite values have no integer.
            "int" => Ok(Some(
                match self.arg_value(arguments, 0, None, context)?.number() {
                    Some(number) if number.is_finite() => RuntimeValue::Int(number.trunc() as i64),
                    _ => RuntimeValue::Na,
                },
            )),
            "float" => Ok(Some(
                self.arg_value(arguments, 0, None, context)?
                    .number()
                    .map(RuntimeValue::Float)
                    .unwrap_or(RuntimeValue::Na),
            )),
            "bool" => {
                let v = self.arg_value(arguments, 0, None, context)?;
                Ok(Some(match &v {
                    RuntimeValue::Bool(_) => v,
                    _ => RuntimeValue::Bool(v.number().is_some_and(|n| n != 0.0)),
                }))
            }
            "string" | "color" => {
                let v = self.arg_value(arguments, 0, None, context)?;
                let keeps = matches!(
                    (&v, name),
                    (RuntimeValue::String(_), "string") | (RuntimeValue::Color(_), "color")
                );
                Ok(Some(if keeps { v } else { RuntimeValue::Na }))
            }
            "color.new" => {
                let c = self.arg_value(arguments, 0, None, context)?;
                let transparency = self.arg_value(arguments, 1, None, context)?.number();
                Ok(Some(match (&c, transparency) {
                    (RuntimeValue::Color(c), Some(t)) => {
                        RuntimeValue::Color(super::builtins::with_transparency(*c, t))
                    }
                    _ => RuntimeValue::Na,
                }))
            }
            "str.format_time" => {
                let bound = self.bind_named(arguments, &["time", "format", "timezone"], context)?;
                let text = |key: &str| match bound.get(key) {
                    Some(RuntimeValue::String(text)) => Some(text.clone()),
                    _ => None,
                };
                Ok(Some(
                    match bound.get("time").and_then(RuntimeValue::int_value) {
                        Some(ms) => RuntimeValue::String(super::time::formatted(
                            ms,
                            text("format").as_deref(),
                            text("timezone").as_deref(),
                        )),
                        None => RuntimeValue::Na,
                    },
                ))
            }
            "color.r" | "color.g" | "color.b" | "color.t" => {
                let color = self.arg_value(arguments, 0, Some("color"), context)?;
                Ok(Some(match color {
                    RuntimeValue::Color(c) => RuntimeValue::Float(
                        super::builtins::color_component(c, name.chars().last().unwrap_or('r')),
                    ),
                    _ => RuntimeValue::Na,
                }))
            }
            "color.from_gradient" => {
                let value = self.arg_value(arguments, 0, Some("value"), context)?;
                let bottom = self.arg_value(arguments, 1, Some("bottom_value"), context)?;
                let top = self.arg_value(arguments, 2, Some("top_value"), context)?;
                let from = self.arg_value(arguments, 3, Some("bottom_color"), context)?;
                let to = self.arg_value(arguments, 4, Some("top_color"), context)?;
                Ok(Some(
                    match (value.number(), bottom.number(), top.number(), from, to) {
                        (
                            Some(v),
                            Some(b),
                            Some(t),
                            RuntimeValue::Color(f),
                            RuntimeValue::Color(c),
                        ) => RuntimeValue::Color(super::builtins::gradient(v, b, t, f, c)),
                        _ => RuntimeValue::Na,
                    },
                ))
            }
            "timeframe.in_seconds" => {
                let timeframe = self.arg_value(arguments, 0, Some("timeframe"), context)?;
                let seconds = match timeframe {
                    RuntimeValue::String(text) => super::time::seconds_of_timeframe(&text),
                    // No argument: the chart's own bar length.
                    RuntimeValue::Na if self.bar_seconds > 0.0 => Some(self.bar_seconds),
                    _ => None,
                };
                Ok(Some(match seconds {
                    Some(seconds) => RuntimeValue::Int(seconds.round() as i64),
                    None => RuntimeValue::Na,
                }))
            }
            "timeframe.change" => {
                let timeframe = self.arg_value(arguments, 0, Some("timeframe"), context)?;
                let seconds = match timeframe {
                    RuntimeValue::String(text) => super::time::seconds_of_timeframe(&text),
                    _ => None,
                };
                Ok(Some(match seconds {
                    Some(seconds) => RuntimeValue::Bool(super::time::timeframe_changed(
                        self.last_committed_open_time,
                        context.bar.open_time,
                        seconds,
                    )),
                    None => RuntimeValue::Na,
                }))
            }
            "timeframe.from_seconds" => {
                let seconds = self.arg_value(arguments, 0, Some("seconds"), context)?;
                Ok(Some(
                    match seconds
                        .int_value()
                        .and_then(super::time::timeframe_from_seconds)
                    {
                        Some(text) => RuntimeValue::String(text),
                        None => RuntimeValue::Na,
                    },
                ))
            }
            "color.rgb" => {
                let r = self
                    .arg_value(arguments, 0, None, context)?
                    .number()
                    .unwrap_or(0.0);
                let g = self
                    .arg_value(arguments, 1, None, context)?
                    .number()
                    .unwrap_or(0.0);
                let b = self
                    .arg_value(arguments, 2, None, context)?
                    .number()
                    .unwrap_or(0.0);
                let transparency = self
                    .arg_value(arguments, 3, None, context)?
                    .number()
                    .unwrap_or(0.0);
                Ok(Some(RuntimeValue::Color(super::builtins::rgb(
                    r,
                    g,
                    b,
                    transparency,
                ))))
            }
            // `hour(time, timezone)` and the other calendar parts: of `time` (the bar's own when
            // there are no arguments), read in `timezone` (UTC when omitted).
            _ if super::time::PART_NAMES.contains(&name) => {
                let bound = self.bind_named(arguments, &["time", "timezone"], context)?;
                let stamp = if arguments.is_empty() {
                    Some(context.bar.open_time)
                } else {
                    bound.get("time").and_then(RuntimeValue::int_value)
                };
                let zone = match bound.get("timezone") {
                    Some(RuntimeValue::String(zone)) => Some(zone.as_str()),
                    _ => None,
                };
                let offset = super::time::zone_offset_secs(zone, stamp.unwrap_or_default());
                Ok(Some(match stamp {
                    Some(stamp) => super::time::part(name, stamp, offset),
                    None => RuntimeValue::Na,
                }))
            }
            // `time(timeframe, session, timezone)`: the open time of the `timeframe` bar that
            // contains the current bar. With a `session` it is `na` unless that bar opens inside
            // the session, read in `timezone` (UTC when omitted). Without a usable timeframe it is
            // the bar's own `time`.
            "time" => {
                let bound =
                    self.bind_named(arguments, &["timeframe", "session", "timezone"], context)?;
                let mut start = context.bar.open_time;
                if let Some(RuntimeValue::String(text)) = bound.get("timeframe")
                    && !text.is_empty()
                {
                    let Some(seconds) = super::time::seconds_of_timeframe(text) else {
                        return Err(Diagnostic::error(
                            "PINE4021",
                            DiagnosticCategory::Runtime,
                            "time() needs a timeframe such as \"60\" or \"1D\".",
                            range,
                        ));
                    };
                    // The chart's own timeframe keeps the bar's real open.
                    if seconds != self.bar_seconds {
                        start = super::security_call::bucket_start(start, seconds);
                    }
                }
                if let Some(RuntimeValue::String(text)) = bound.get("session")
                    && !text.is_empty()
                {
                    let Some(session) = super::trading_session::TradingSession::parse(text) else {
                        return Ok(Some(RuntimeValue::Na));
                    };
                    let zone = match bound.get("timezone") {
                        Some(RuntimeValue::String(zone)) => Some(zone.as_str()),
                        _ => None,
                    };
                    let offset = super::time::zone_offset_secs(zone, start);
                    if !session.contains(&super::time::components(start, offset)) {
                        return Ok(Some(RuntimeValue::Na));
                    }
                }
                Ok(Some(RuntimeValue::Int(start)))
            }
            // `time_close(timeframe)`: the close of the `timeframe` bar that contains the
            // current bar.
            "time_close" => {
                let timeframe = self.arg_value(arguments, 0, Some("timeframe"), context)?;
                let RuntimeValue::String(text) = timeframe else {
                    return Ok(Some(
                        self.market("time_close", context)
                            .unwrap_or(RuntimeValue::Na),
                    ));
                };
                if text.is_empty() {
                    return Ok(Some(
                        self.market("time_close", context)
                            .unwrap_or(RuntimeValue::Na),
                    ));
                }
                let Some(seconds) = super::time::seconds_of_timeframe(&text) else {
                    return Err(Diagnostic::error(
                        "PINE4021",
                        DiagnosticCategory::Runtime,
                        "time_close() needs a timeframe such as \"60\" or \"1D\".",
                        range,
                    ));
                };
                let start = super::security_call::bucket_start(context.bar.open_time, seconds);
                Ok(Some(RuntimeValue::Int(super::security_call::bucket_end(
                    start, seconds,
                ))))
            }
            "timestamp" => {
                let mut positional = Vec::new();
                let mut named = HashMap::new();
                for argument in arguments {
                    let v = self.eval(&argument.value, context)?;
                    if let Some(label) = &argument.name {
                        named.insert(label.clone(), v);
                    } else {
                        positional.push(v);
                    }
                }
                Ok(Some(
                    super::timestamp::evaluate(&positional, &named)
                        .map(RuntimeValue::Int)
                        .unwrap_or(RuntimeValue::Na),
                ))
            }
            "str.tostring" => {
                let v = self.arg_value(arguments, 0, None, context)?;
                let pattern = self
                    .arg_value(arguments, 1, Some("format"), context)?
                    .text_value()
                    .map(str::to_owned);
                Ok(Some(RuntimeValue::String(number_format::format(
                    &v,
                    pattern.as_deref(),
                    self.mintick,
                ))))
            }
            _ => Ok(None),
        }
    }

    fn method_call_on_value(
        &mut self,
        receiver: &Expression,
        member: &str,
        arguments: &[super::ast::Argument],
        site: usize,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        let value = self.eval(receiver, context)?;
        if self.methods.contains_key(member)
            && let Some(result) =
                self.call_user_method(member, value.clone(), arguments, site, range, context)?
        {
            return Ok(result);
        }
        // `point.copy()`: a new object with the same field values (nested objects are shared).
        if member == "copy"
            && let RuntimeValue::Ref(RefKind::Object, id) = &value
            && let Some(object) = self.working.instances.get(id).cloned()
        {
            self.working.next_reference += 1;
            let copy = self.working.next_reference;
            self.working.instances.insert(copy, object);
            return Ok(RuntimeValue::Ref(RefKind::Object, copy));
        }
        // `ln.set_extend(...)` on a drawing handle is `line.set_extend(ln, ...)`: the receiver
        // becomes the first argument of the namespaced builtin.
        let drawing = match &value {
            RuntimeValue::Ref(RefKind::Line, _) => Some("line"),
            RuntimeValue::Ref(RefKind::Label, _) => Some("label"),
            RuntimeValue::Ref(RefKind::Box, _) => Some("box"),
            RuntimeValue::Ref(RefKind::Table, _) => Some("table"),
            RuntimeValue::Ref(RefKind::Linefill, _) => Some("linefill"),
            RuntimeValue::Ref(RefKind::Polyline, _) => Some("polyline"),
            _ => None,
        };
        if let Some(namespace) = drawing {
            let full = format!("{namespace}.{member}");
            if drawings::handles(&full) {
                let mut args = vec![drawings::Arg {
                    name: None,
                    value: value.clone(),
                }];
                args.extend(self.eval_args(arguments, context)?);
                return drawings::call(
                    &full,
                    &args,
                    &mut self.working,
                    &context.bar,
                    self.bar_seconds,
                    &self.drawing_limits,
                    range,
                );
            }
        }
        if value == RuntimeValue::Na {
            return Err(Diagnostic::error(
                "PINE4018",
                DiagnosticCategory::Runtime,
                format!("Cannot call '{member}' on a value that is na."),
                range,
            ));
        }
        let prefix = match &value {
            RuntimeValue::Ref(RefKind::Array, _) => "array",
            RuntimeValue::Ref(RefKind::Map, _) => "map",
            RuntimeValue::Ref(RefKind::Matrix, _) => "matrix",
            _ => {
                return Err(Diagnostic::error(
                    "PINE4007",
                    DiagnosticCategory::Runtime,
                    format!("Unknown or unsupported method '{member}'."),
                    range,
                ));
            }
        };
        let full = format!("{prefix}.{member}");
        let parameters = match prefix {
            "array" => array_builtins::parameters(&full),
            "map" => map_builtins::parameters(&full),
            _ => matrix_builtins::parameters(&full),
        };
        let mut bound = FastMap::default();
        bound.insert("id".to_owned(), value.clone());
        let positional: Vec<&super::ast::Argument> = arguments
            .iter()
            .filter(|argument| argument.name.is_none())
            .collect();
        let mut position = 0;
        for parameter in parameters {
            if *parameter == "id" {
                continue;
            }
            if let Some(named) = arguments
                .iter()
                .find(|argument| argument.name.as_deref() == Some(*parameter))
            {
                bound.insert((*parameter).to_owned(), self.eval(&named.value, context)?);
            } else if position < positional.len() {
                bound.insert(
                    (*parameter).to_owned(),
                    self.eval(&positional[position].value, context)?,
                );
                position += 1;
            } else {
                bound.insert((*parameter).to_owned(), RuntimeValue::Na);
            }
        }
        let result = match prefix {
            "array" => {
                array_builtins::call(&full, &bound, &[], &mut self.working, self.mintick, range)?
            }
            "map" => map_builtins::call(&full, &bound, &mut self.working, range)?,
            _ => matrix_builtins::call(&full, &bound, &mut self.working, range)?,
        };
        Ok(result)
    }

    fn read_field(
        &self,
        name: &str,
        base: RuntimeValue,
        range: SourceRange,
    ) -> Result<RuntimeValue, Diagnostic> {
        if let RuntimeValue::Ref(RefKind::Object, id) = base
            && let Some(object) = self.working.instances.get(&id)
            && let Some(value) = object.fields.get(name)
        {
            return Ok(value.clone());
        }
        Err(Diagnostic::error(
            "PINE4010",
            DiagnosticCategory::Runtime,
            format!("Field '{name}' not found on this value."),
            range,
        ))
    }

    fn run_field_assignment(
        &mut self,
        target: &Expression,
        op: AssignmentOperator,
        value: &Expression,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<Step, Diagnostic> {
        // `a.b := x` reaches here as the dotted name `a.b`; `f().b := x` as a member of a call.
        let (base, name) = match target {
            Expression::Member { base, name, .. } => (self.eval(base, context)?, name.clone()),
            Expression::Identifier(path, _) if path.contains('.') => {
                let (owner, field) = path.rsplit_once('.').unwrap_or_default();
                let owner = self.resolve_identifier(owner, range, context)?;
                (owner, field.to_owned())
            }
            _ => {
                return Err(Diagnostic::error(
                    "PINE4007",
                    DiagnosticCategory::Runtime,
                    "Assignment to this target is not yet supported.",
                    range,
                ));
            }
        };
        let RuntimeValue::Ref(RefKind::Object, id) = base else {
            return Err(Diagnostic::error(
                "PINE4007",
                DiagnosticCategory::Runtime,
                "Field assignment requires an object.",
                range,
            ));
        };
        let name = &name;
        let rhs = self.eval(value, context)?;
        let assigned = match op {
            AssignmentOperator::Reassign => rhs,
            AssignmentOperator::Compound(_) => {
                let old = self.read_field(name, RuntimeValue::Ref(RefKind::Object, id), range)?;
                operators::apply_assignment(op, &old, &rhs)
            }
        };
        if let Some(object) = self.working.instances.get_mut(&id) {
            object.fields.insert(name.clone(), assigned);
        }
        Ok(Step::normal(RuntimeValue::Void))
    }

    pub(crate) fn site_key(&self, site: usize, context: &RuntimeContext) -> usize {
        if context.site_prefix == 0 {
            site
        } else {
            context
                .site_prefix
                .wrapping_mul(1_000_003)
                .wrapping_add(site)
        }
    }

    pub(crate) fn invoke(
        &mut self,
        function: RuntimeFunction,
        call_name: &str,
        arguments: &[super::ast::Argument],
        site: usize,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        if context.depth >= self.limits.call_depth {
            return Err(Diagnostic::error(
                "PINE8005",
                DiagnosticCategory::Resource,
                format!("Call depth limit exceeded in '{call_name}'."),
                range,
            ));
        }
        let bound = self.bind_parameters(&function, arguments, context)?;
        let prefix = self.site_key(site, context);
        let stash_key = |local: &str| format!("{INTERNAL_PREFIX}fn:{prefix}:{local}");

        let saved: Vec<(String, Option<RuntimeValue>)> = function
            .locals
            .iter()
            .map(|local| (local.clone(), self.working.variables.get(local).cloned()))
            .collect();
        for local in function.locals.iter() {
            self.working.variables.remove(local);
        }
        // A `var` local that has run before resumes with its value; on the first call it is left
        // unset so its declaration initialises it.
        for local in function.persistent_locals.iter() {
            if let Some(value) = self.working.variables.get(&stash_key(local)).cloned() {
                self.working.variables.insert(local.clone(), value);
            }
        }
        for (parameter, value) in bound {
            self.working.variables.insert(parameter, value);
        }

        let caller_prefix = context.site_prefix;
        let caller_scope = context.scope.clone();
        let caller_locals = context.locals.clone();
        let caller_history_locals = context
            .history_locals
            .replace(function.history_locals.clone());
        context.site_prefix = prefix;
        context.depth += 1;
        context.scope = function.scope.clone();
        context.locals = if function.scope.is_none() {
            None
        } else {
            Some(function.locals.clone())
        };
        let hooked = self.hooked();
        let mut timed = false;
        if hooked {
            if self.debug.is_some() {
                self.debug_enter(call_name, range, function.scope.as_deref(), &saved);
            }
            timed = self.profile_enter_function(&function, range);
        }
        let result = self.run(&function.body, context);
        if hooked {
            if timed {
                self.profile_exit();
            }
            if self.debug.is_some() {
                self.debug_leave();
            }
        }
        context.site_prefix = caller_prefix;
        context.depth -= 1;
        context.scope = caller_scope;
        context.locals = caller_locals;
        context.history_locals = caller_history_locals;

        // What the call leaves in a local that is read as `local[n]` becomes that call site's next
        // value of the series, one per bar.
        if result.is_ok() {
            let bar = self.working.bar_index;
            for local in function.history_locals.iter() {
                let value = self
                    .working
                    .variables
                    .get(local)
                    .cloned()
                    .unwrap_or(RuntimeValue::Na);
                let key = format!("fn:{prefix}:{local}");
                let series = self.working.histories.entry(key.clone()).or_default();
                if self.working.local_history_bar.get(&key) == Some(&bar) {
                    if let Some(last) = series.last_mut() {
                        *last = value;
                    }
                } else {
                    series.push(value);
                    self.working.local_history_bar.insert(key, bar);
                }
            }
        }
        let result = result.inspect_err(|_| {
            for (local, previous) in &saved {
                match previous {
                    Some(previous) => {
                        self.working
                            .variables
                            .insert(local.clone(), previous.clone());
                    }
                    None => {
                        self.working.variables.remove(local);
                    }
                }
            }
        });
        let (_, value) = result?;
        for local in function.persistent_locals.iter() {
            let value = self
                .working
                .variables
                .get(local)
                .cloned()
                .unwrap_or(RuntimeValue::Na);
            self.working.variables.insert(stash_key(local), value);
        }
        for (local, previous) in saved {
            if let Some(previous) = previous {
                self.working.variables.insert(local, previous);
            } else {
                self.working.variables.remove(&local);
            }
        }
        Ok(value)
    }

    fn bind_parameters(
        &mut self,
        function: &RuntimeFunction,
        arguments: &[super::ast::Argument],
        context: &mut RuntimeContext,
    ) -> Result<FastMap<String, RuntimeValue>, Diagnostic> {
        let positional: Vec<&super::ast::Argument> =
            arguments.iter().filter(|a| a.name.is_none()).collect();
        let mut bound = FastMap::default();
        for (i, parameter) in function.parameters.iter().enumerate() {
            if let Some(named) = arguments
                .iter()
                .find(|a| a.name.as_deref() == Some(&parameter.name))
            {
                bound.insert(parameter.name.clone(), self.eval(&named.value, context)?);
            } else if i < positional.len() {
                bound.insert(
                    parameter.name.clone(),
                    self.eval(&positional[i].value, context)?,
                );
            } else if let Some(fallback) = &parameter.default_value {
                bound.insert(parameter.name.clone(), self.eval(fallback, context)?);
            } else {
                bound.insert(parameter.name.clone(), RuntimeValue::Na);
            }
        }
        Ok(bound)
    }

    // MARK: Bookkeeping

    pub(crate) fn commit_histories(&mut self, bar: Kline, releasing_committed: bool) {
        let mut series: FastMap<String, RuntimeValue> = FastMap::default();
        series.insert("open".to_owned(), RuntimeValue::Float(bar.open));
        series.insert("high".to_owned(), RuntimeValue::Float(bar.high));
        series.insert("low".to_owned(), RuntimeValue::Float(bar.low));
        series.insert("close".to_owned(), RuntimeValue::Float(bar.close));
        series.insert("volume".to_owned(), RuntimeValue::Float(bar.volume));
        series.insert(
            "hl2".to_owned(),
            RuntimeValue::Float((bar.high + bar.low) / 2.0),
        );
        series.insert(
            "hlc3".to_owned(),
            RuntimeValue::Float((bar.high + bar.low + bar.close) / 3.0),
        );
        series.insert(
            "ohlc4".to_owned(),
            RuntimeValue::Float((bar.open + bar.high + bar.low + bar.close) / 4.0),
        );
        series.insert("time".to_owned(), RuntimeValue::Int(bar.open_time));
        series.insert(
            "time_close".to_owned(),
            RuntimeValue::Int(bar.open_time + (self.bar_seconds * 1000.0) as i64),
        );
        series.insert(
            "bar_index".to_owned(),
            RuntimeValue::Int(self.working.bar_index),
        );
        if self.program.declaration.script_type == super::program::ScriptType::Strategy {
            for name in STRATEGY_SERIES {
                if let Some(value) =
                    strategy_builtins::strategy_value(name, &self.working.broker, bar.close)
                {
                    series.insert(name.to_owned(), value);
                }
            }
        }

        let mut histories = std::mem::take(&mut self.working.histories);
        if releasing_committed {
            self.committed.histories.clear();
        }
        let builtin: HashSet<String> = series.keys().cloned().collect();
        let own = self
            .working
            .variables
            .iter()
            .filter(|(name, _)| !builtin.contains(*name))
            .map(|(name, value)| (name.clone(), value.clone()));
        for (name, value) in series.into_iter().chain(own) {
            if !name.starts_with(INTERNAL_PREFIX) {
                histories.entry(name).or_default().push(value);
            }
        }
        let expression_keys: HashSet<String> = histories
            .keys()
            .filter(|k| k.starts_with(INTERNAL_PREFIX))
            .cloned()
            .chain(self.working.expression_values.keys().cloned())
            .collect();
        for key in expression_keys {
            let value = self
                .working
                .expression_values
                .get(&key)
                .cloned()
                .unwrap_or(RuntimeValue::Na);
            histories.entry(key).or_default().push(value);
        }
        self.working.histories = histories;
        self.working.expression_values.clear();
    }

    fn budget(&mut self) -> Result<(), Diagnostic> {
        self.working.instructions += 1;
        if self.working.instructions == 1 || self.working.instructions.is_multiple_of(1024) {
            self.check_interruption().map_err(|error| match error {
                RunError::Script(diagnostic) => diagnostic,
                RunError::Rejection(_) => unreachable!("interruption is always a diagnostic"),
            })?;
        }
        if self.working.instructions > self.limits.instructions_per_bar {
            return Err(Diagnostic::error(
                "PINE8004",
                DiagnosticCategory::Resource,
                "Per-bar instruction limit exceeded.",
                SourceRange::ZERO,
            ));
        }
        Ok(())
    }

    fn start_deadline(&mut self) {
        let seconds = if self.limits.deadline_seconds.is_finite() {
            self.limits.deadline_seconds.max(0.0)
        } else {
            0.0
        };
        self.deadline_at = Instant::now().checked_add(Duration::from_secs_f64(seconds));
    }

    fn check_interruption(&self) -> Result<(), RunError> {
        if self
            .cancellation
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::Acquire))
        {
            return Err(RunError::Script(Diagnostic::error(
                "PINE8008",
                DiagnosticCategory::Cancellation,
                "Evaluation cancelled.",
                SourceRange::ZERO,
            )));
        }
        if self
            .deadline_at
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            return Err(RunError::Script(Diagnostic::error(
                "PINE8007",
                DiagnosticCategory::Resource,
                "Evaluation deadline exceeded.",
                SourceRange::ZERO,
            )));
        }
        Ok(())
    }

    fn allocate(&mut self) -> usize {
        self.working.next_reference += 1;
        self.working.next_reference
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
                if super::symbol_catalog::is_variable(name) && !found.iter().any(|n| n == name) {
                    found.push(name.to_owned());
                }
            }
            for block in super::traversal::nested_blocks(statement) {
                visit(block, found);
            }
        }
    }
    let mut found = Vec::new();
    visit(statements, &mut found);
    found
}

fn sorted_by_id<T: Clone>(map: &FastMap<usize, T>) -> Vec<T> {
    let mut entries: Vec<_> = map.iter().collect();
    entries.sort_by_key(|(id, _)| **id);
    entries.into_iter().map(|(_, v)| v.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::Limits;

    fn bar() -> Kline {
        Kline {
            open_time: 1_000,
            close_time: 2_000,
            open: 100.0,
            high: 110.0,
            low: 90.0,
            close: 105.0,
            volume: 1.0,
        }
    }

    #[test]
    fn evaluates_a_declaration() {
        let program = super::super::compiler::compile(
            "//@version=6\nindicator(\"T\")\nx = 1 + 2\n",
            Limits::default(),
            &super::super::compiler::NoLibraries,
        );
        let mut session = Session::new(
            program,
            HashMap::new(),
            Limits::default(),
            None,
            SymbolInfo::default(),
            None,
            None,
        );
        let result = session.evaluate(std::slice::from_ref(&bar())).unwrap();
        assert_eq!(result.bar_states.len(), 1);
        assert_eq!(
            session.working.variables.get("x"),
            Some(&RuntimeValue::Int(3))
        );
    }

    #[test]
    fn evaluates_control_flow() {
        let program = super::super::compiler::compile(
            "//@version=6\nindicator(\"T\")\nx = 0\nif close > open\n    x := 1\n",
            Limits::default(),
            &super::super::compiler::NoLibraries,
        );
        let mut session = Session::new(
            program,
            HashMap::new(),
            Limits::default(),
            None,
            SymbolInfo::default(),
            None,
            None,
        );
        session.evaluate(std::slice::from_ref(&bar())).unwrap();
        assert_eq!(
            session.working.variables.get("x"),
            Some(&RuntimeValue::Int(1))
        );
    }

    /// `source` run over `bars`, one confirmed bar at a time, leaving the session to inspect.
    fn run_over(source: &str, bars: &[Kline]) -> Session {
        let program = super::super::compiler::compile(
            source,
            Limits::default(),
            &super::super::compiler::NoLibraries,
        );
        assert!(program.is_valid(), "{:?}", program.diagnostics);
        let mut session = Session::new(
            program,
            HashMap::new(),
            Limits::default(),
            None,
            SymbolInfo::default(),
            None,
            None,
        );
        session.evaluate(bars).unwrap();
        session
    }

    fn hourly(open_time: i64) -> Kline {
        Kline {
            open_time,
            close_time: open_time + 3_600_000,
            ..bar()
        }
    }

    #[test]
    fn time_call_buckets_to_the_timeframe_and_gates_on_a_session() {
        const DAY_MS: i64 = 86_400_000;
        // 2023-11-14 12:00 UTC (a Tuesday) and the bar before it.
        let noon = 1_699_963_200_000;
        let source = "//@version=6\nindicator(\"T\")\n\
            day = time(\"D\")\n\
            own = time(timeframe.period)\n\
            inside = time(timeframe.period, \"0930-1600\")\n\
            outside = time(timeframe.period, \"1300-1600\")\n\
            shifted = time(timeframe.period, \"1300-1600\", \"UTC+2\")";
        let session = run_over(source, &[hourly(noon - 3_600_000), hourly(noon)]);
        let variable = |name: &str| session.working.variables.get(name).cloned();
        assert_eq!(
            variable("day"),
            Some(RuntimeValue::Int(noon.div_euclid(DAY_MS) * DAY_MS))
        );
        assert_eq!(variable("own"), Some(RuntimeValue::Int(noon)));
        assert_eq!(variable("inside"), Some(RuntimeValue::Int(noon)));
        assert_eq!(variable("outside"), Some(RuntimeValue::Na));
        // 12:00 UTC is 14:00 in UTC+2, inside 13:00-16:00 there.
        assert_eq!(variable("shifted"), Some(RuntimeValue::Int(noon)));
    }

    #[test]
    fn calendar_parts_take_a_time_and_a_zone() {
        let noon = 1_699_963_200_000;
        let source = "//@version=6\nindicator(\"T\")\n\
            own = hour\n\
            utc = hour(time)\n\
            shifted = hour(time, \"UTC+3\")";
        let session = run_over(source, &[hourly(noon)]);
        let variable = |name: &str| session.working.variables.get(name).cloned();
        assert_eq!(variable("own"), Some(RuntimeValue::Int(12)));
        assert_eq!(variable("utc"), Some(RuntimeValue::Int(12)));
        assert_eq!(variable("shifted"), Some(RuntimeValue::Int(15)));
    }

    #[test]
    fn drawing_methods_work_on_variables_and_call_results() {
        let source = "//@version=6\nindicator(\"T\", overlay = true)\n\
            make() =>\n    line.new(bar_index, close, bar_index + 1, close)\n\
            ln = line.new(bar_index, open, bar_index + 1, open)\n\
            ln.set_extend(extend.right)\n\
            make().set_extend(extend.both)\n\
            line.new(bar_index, low, bar_index + 1, low).set_extend(extend.left)";
        let session = run_over(source, &[hourly(1_000_000)]);
        let mut extends: Vec<_> = session
            .output()
            .lines
            .iter()
            .map(|line| format!("{:?}", line.extend))
            .collect();
        extends.sort();
        assert_eq!(extends, ["Both", "Left", "Right"]);
    }

    #[test]
    fn drawing_calls_honor_the_limits_from_the_script_declaration() {
        let source = "//@version=6\nindicator(\"T\", overlay = true, max_lines_count = 2)\n\
            line.new(bar_index, close, bar_index + 1, close)";
        let session = run_over(source, &[hourly(0), hourly(3_600_000), hourly(7_200_000)]);

        assert_eq!(session.output().lines.len(), 2);
        assert_eq!(session.output().lines[0].x1, 1);
        assert_eq!(session.output().lines[1].x1, 2);
    }

    #[test]
    fn array_call_arguments_are_evaluated_once() {
        let source = "//@version=6\nindicator(\"T\")\n\
            var side = array.new_int()\n\
            var target = array.new_int()\n\
            value() =>\n    array.push(side, 1)\n    7\n\
            array.push(target, value())\n\
            plot(array.size(side))";
        let session = run_over(source, &[hourly(0)]);

        assert_eq!(session.output().plots[0].values, [Some(1.0)]);
    }

    #[test]
    fn canonical_math_and_string_builtins_are_reachable_from_scripts() {
        let source = "//@version=6\nindicator(\"T\")\n\
            matched = str.match(\"price=123\", \"[0-9]+\")\n\
            parts = str.split(\"a,b,c\", \",\")\n\
            plot(str.length(matched))\n\
            plot(array.size(parts))\n\
            plot(math.sum(close, 2))";
        let session = run_over(source, &[hourly(0), hourly(3_600_000)]);
        let plots = session.output().plots;

        assert_eq!(plots[0].values.last(), Some(&Some(3.0)));
        assert_eq!(plots[1].values.last(), Some(&Some(3.0)));
        assert_eq!(plots[2].values.last(), Some(&Some(210.0)));
    }

    #[test]
    fn ta_overloads_use_default_sources_and_linreg_offset() {
        let source = "//@version=6\nindicator(\"T\")\n\
            plot(ta.highest(2))\n\
            plot(ta.lowest(length = 2))\n\
            plot(ta.highestbars(2))\n\
            plot(ta.linreg(close, 4, 1))";
        let bars = [
            Kline {
                high: 2.0,
                low: 1.0,
                close: 2.0,
                ..hourly(0)
            },
            Kline {
                high: 4.0,
                low: 3.0,
                close: 4.0,
                ..hourly(3_600_000)
            },
            Kline {
                high: 7.0,
                low: 2.0,
                close: 6.0,
                ..hourly(7_200_000)
            },
            Kline {
                high: 5.0,
                low: 4.0,
                close: 8.0,
                ..hourly(10_800_000)
            },
        ];
        let plots = run_over(source, &bars).output().plots;

        assert_eq!(plots[0].values.last(), Some(&Some(7.0)));
        assert_eq!(plots[1].values.last(), Some(&Some(2.0)));
        assert_eq!(plots[2].values.last(), Some(&Some(-1.0)));
        assert_eq!(plots[3].values.last(), Some(&Some(6.0)));
    }

    #[test]
    fn realtime_ticks_do_not_advance_committed_ta_history() {
        let program = super::super::compiler::compile(
            "//@version=6\nindicator(\"T\")\nplot(ta.sma(close, 2))",
            Limits::default(),
            &super::super::compiler::NoLibraries,
        );
        let mut session = Session::new(
            program,
            HashMap::new(),
            Limits::default(),
            None,
            SymbolInfo::default(),
            None,
            None,
        );
        session
            .load_history(
                &[Kline {
                    close: 10.0,
                    ..hourly(0)
                }],
                true,
                None,
            )
            .unwrap();
        for close in [20.0, 30.0] {
            session
                .execute(
                    BarEvent {
                        candle: Kline {
                            close,
                            ..hourly(3_600_000)
                        },
                        phase: BarPhase::RealtimeTick {
                            is_new: close == 20.0,
                        },
                    },
                    true,
                    None,
                )
                .unwrap();
        }
        session
            .execute(
                BarEvent {
                    candle: Kline {
                        close: 40.0,
                        ..hourly(3_600_000)
                    },
                    phase: BarPhase::RealtimeClose { is_new: false },
                },
                true,
                None,
            )
            .unwrap();

        assert_eq!(session.output().plots[0].values.last(), Some(&Some(25.0)));
        assert!(
            session
                .working
                .ta_states
                .values()
                .all(|state| state.inputs.len() == 2),
            "only the historical bar and confirmed realtime bar should be committed"
        );
    }

    #[test]
    fn evaluation_enforces_deadlines_and_cooperative_cancellation() {
        let source = "//@version=6\nindicator(\"T\")\nplot(close)";
        let program = super::super::compiler::compile(
            source,
            Limits::default(),
            &super::super::compiler::NoLibraries,
        );
        let mut deadline_session = Session::new(
            program.clone(),
            HashMap::new(),
            Limits {
                deadline_seconds: 0.0,
                ..Limits::default()
            },
            None,
            SymbolInfo::default(),
            None,
            None,
        );
        let Err(RunError::Script(deadline)) = deadline_session.evaluate(&[hourly(0)]) else {
            panic!("a zero deadline must stop evaluation");
        };
        assert_eq!(deadline.code, "PINE8007");

        let mut cancelled_session = Session::new(
            program,
            HashMap::new(),
            Limits::default(),
            None,
            SymbolInfo::default(),
            None,
            None,
        );
        cancelled_session.set_cancellation(Arc::new(AtomicBool::new(true)));
        let Err(RunError::Script(cancelled)) = cancelled_session.evaluate(&[hourly(0)]) else {
            panic!("a raised cancellation flag must stop evaluation");
        };
        assert_eq!(cancelled.code, "PINE8008");
        assert_eq!(cancelled.category, DiagnosticCategory::Cancellation);
    }

    #[test]
    fn strategy_readings_follow_the_broker_and_keep_a_history() {
        let source = "//@version=6\nstrategy(\"T\")\n\
            if bar_index == 1\n    strategy.entry(\"L\", strategy.long)\n\
            size = strategy.position_size\n\
            before = strategy.position_size[1]\n\
            avg = strategy.position_avg_price";
        let session = run_over(
            source,
            &[
                hourly(0),
                hourly(3_600_000),
                hourly(7_200_000),
                hourly(10_800_000),
            ],
        );
        let variable = |name: &str| session.working.variables.get(name).cloned();
        // Entered on bar 1, filled at bar 2's open; bar 3 sees the position and its history.
        assert!(matches!(variable("size"), Some(RuntimeValue::Float(size)) if size > 0.0));
        assert!(matches!(variable("before"), Some(RuntimeValue::Float(size)) if size > 0.0));
        assert!(matches!(variable("avg"), Some(RuntimeValue::Float(_))));
    }
}
