//! Interprets a compiled Pine program bar by bar. Ported from `PineRuntimeSession.swift` +
//! `+Expressions`/`+Statements`/`+Calls`/`+Market`. Builtin handlers live in their own modules
//! (`builtins::array`, `strategy::builtins`, `ta`, `math`, `strings`, …); the plain-value builtins
//! are `scalar`'s.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, atomic::AtomicBool};
use std::time::Instant;

use super::fast_hash::FastMap;
use super::state::{AlertEvent, RuntimeState};
use super::support::{LibraryLinkage, SecurityDataProvider};
use super::types::{Flow, RuntimeFunction};
use super::value::RuntimeValue;
use crate::model::Kline;
use crate::pine::compiler::program::{CompiledProgram, InputValue, SymbolInfo};
use crate::pine::outputs::drawings;
use crate::pine::{Diagnostic, Limits};

mod bookkeeping;
mod build;
mod calls;
mod expressions;
mod hooks;
mod running;
mod scalar;
mod statements;

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
const DEFAULT_MINTICK: f64 = 0.01;

/// What a hooked session carries: a debugger or a profiler, never both. A session without either
/// has `hooks: None`, the one thing the statement loop tests.
enum Hooks {
    Debug(crate::pine::debug::DebugRuntime),
    Profile(crate::pine::profile::ProfileRuntime),
}

pub struct Session {
    pub program: CompiledProgram,
    pub inputs: HashMap<String, InputValue>,
    pub limits: Limits,
    pub symbol: SymbolInfo,
    pub functions: FastMap<String, RuntimeFunction>,
    pub methods: FastMap<String, Vec<RuntimeFunction>>,
    /// The script's own top-level functions and methods in declaration order, which is what a call's
    /// `target` indexes.
    pub(crate) callables: Vec<RuntimeFunction>,
    /// Functions declared more than once with different parameter types, by registry key.
    pub overloads: FastMap<String, Vec<RuntimeFunction>>,
    pub types: FastMap<String, Vec<crate::pine::syntax::ast::TypeField>>,
    pub enums: FastMap<String, Vec<crate::pine::syntax::ast::EnumMember>>,
    pub input_variables: HashMap<usize, String>,
    pub linkage: LibraryLinkage,
    pub security_data: Option<Box<dyn SecurityDataProvider>>,
    pub security_series: HashMap<super::support::SecurityKey, Option<std::sync::Arc<Vec<Kline>>>>,
    pub library_constants: FastMap<String, RuntimeValue>,
    pub constants_in_progress: HashSet<String>,
    pub committed: RuntimeState,
    /// While a history is replayed, `working` already is the committed state: cloning it into
    /// `committed` after every bar would make a run quadratic in its length. `committed` is
    /// refreshed (see `sync_committed`) when something other than a historical bar needs it.
    working_is_committed: bool,
    pub working: RuntimeState,
    pub intrabar: FastMap<String, RuntimeValue>,
    /// What a `varip` variable or `varip` field holds beyond a handle, taken at the end of each
    /// unconfirmed tick and put back at the start of the next, because the objects live in the
    /// rolled-back state. Cleared with `intrabar`.
    intrabar_objects: Vec<IntrabarObject>,
    /// Scratch for the key a `varip` name is kept under, so reading it does not allocate.
    intrabar_key: String,
    /// Set when the script fails at run time: it stays stopped, as in TradingView, until `reset`.
    halted: Option<Halted>,
    /// True while `load_history` replays bars: a failed bar is then settled once, afterwards, instead
    /// of keeping a copy of the state before every bar.
    bulk_replay: bool,
    /// For each script type with a `varip` field, those fields' names. Empty for most scripts.
    varip_fields: Vec<(String, Vec<String>)>,
    pub last_open_time: Option<i64>,
    pub security_globals: Option<FastMap<String, RuntimeValue>>,
    /// The chart's foreground and background (`chart.fg_color`, `chart.bg_color`).
    pub theme: crate::pine::compiler::program::ChartTheme,
    /// Builtin variable names (`low`) the script declares itself: removed at the start of each bar so
    /// a read before the declaration still means the builtin.
    shadowing_builtins: Vec<String>,
    /// Builtin variables the program reads a history of (`hlcc4[1]`, `ta.tr[1]`,
    /// `strategy.closedtrades[1]`) that `commit_histories` does not record on its own. Each is read at
    /// the end of every confirmed bar and kept like a variable. Empty for most scripts.
    history_builtins: Vec<String>,
    /// Tests only: ignore every call's `route` and `target`, so a run can be compared with the
    /// chain's, which finds functions and methods by name.
    #[cfg(any(test, feature = "corpus"))]
    pub(crate) ignore_routes: bool,
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
    pub logs: crate::pine::logs::LogSink,
    pub mintick: f64,
    /// The footprints `request.footprint()` built. Not part of the rolled-back state: they follow
    /// from the candles, and a handle names a bar, not an object.
    pub(crate) footprints: crate::pine::footprint::call::FootprintCache,
    pub bar_seconds: f64,
    /// The chart's own bar length while a `request.*()` call evaluates on another timeframe, which is
    /// when `bar_seconds` is that timeframe's.
    pub(crate) chart_bar_seconds: Option<f64>,
    /// Whether the script runs for an alert, not on a chart (`runtime.calculation.mode`).
    pub(crate) alert_mode: bool,
    /// The length of the chart's own bars when it cannot be read off their open times: a
    /// price-based chart's bars follow price, not time.
    pub fixed_bar_seconds: Option<f64>,
    pub(crate) drawing_limits: drawings::DrawingLimits,
    supplied_mintick: Option<f64>,
    pub(crate) deadline_at: Option<Instant>,
    cancellation: Option<Arc<AtomicBool>>,
    /// Present only in a debugging or profiling session; every hook below tests it and does nothing
    /// otherwise.
    hooks: Option<Box<Hooks>>,
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
    /// `execute` refused a bar that opened before, or is, the one most recently committed. Not a
    /// script failure: the feed repeated or reordered itself.
    Rejection,
    Script(Diagnostic),
}

impl From<Diagnostic> for RunError {
    fn from(diagnostic: Diagnostic) -> Self {
        Self::Script(diagnostic)
    }
}

impl Session {
    pub fn reset(&mut self, inputs: Option<HashMap<String, InputValue>>) {
        if let Some(inputs) = inputs {
            self.inputs = inputs;
        }
        self.restart_state();
        self.logs.restart();
        self.bar_seconds = self.fixed_bar_seconds.unwrap_or(0.0);
        self.last_bar_index = -1;
        self.last_bar_time = None;
        self.first_bar_time = None;
        self.mintick = self.supplied_mintick.unwrap_or(DEFAULT_MINTICK);
        self.deadline_at = None;
    }

    /// Back to before the first bar: the script's state, and the bookkeeping that belongs to it.
    /// Leaves the chart's own facts (bar length, mintick, the logs) as they are.
    fn restart_state(&mut self) {
        self.committed = self.fresh_state();
        self.working = self.committed.clone();
        self.working_is_committed = false;
        self.intrabar.clear();
        self.intrabar_objects.clear();
        self.halted = None;
        self.last_open_time = None;
        self.last_committed_open_time = None;
        self.once_per_bar_ledger.clear();
        self.emitted_alerts.clear();
        self.footprints.clear();
    }

    /// Installs a cooperative cancellation flag. Long history runs and instruction-heavy bars
    /// check it without requiring an async runtime.
    pub fn set_cancellation(&mut self, cancellation: Arc<AtomicBool>) {
        self.cancellation = Some(cancellation);
    }
}

/// A script that stopped on a runtime error.
struct Halted {
    error: Diagnostic,
    /// How many bars ran before the failing one.
    completed_bars: usize,
}

/// What a tick leaves for the next one on a bar that has not closed (see
/// `Session::capture_intrabar_objects`).
enum IntrabarObject {
    Array(usize, Vec<RuntimeValue>),
    Map(usize, super::containers::Map),
    Matrix(usize, super::containers::Matrix),
    Object(usize, super::containers::Object),
    Fields {
        id: usize,
        type_name: String,
        fields: Vec<(String, RuntimeValue)>,
    },
}

#[cfg(test)]
mod tests;
