//! Everything a script run accumulates, plus the alert event and `request.security` site the
//! state holds. Ported from `PineRuntimeState.swift`, `PineAlertEvent.swift`, `PineSecuritySite.swift`.

use std::collections::HashMap;

use super::fast_hash::FastMap;

use super::alert_frequency::AlertFrequency;
use super::broker::Broker;
use super::containers::{Map, Matrix, Object};
use super::outputs::{
    BoxOutput, CandleOutput, ColorOutput, FillOutput, HorizontalLine, LabelOutput, LineOutput,
    LinefillOutput, MarkerOutput, PlotOutput, PolylineOutput, TableOutput,
};
use super::runtime_value::RuntimeValue;
use super::ta;
use crate::model::Kline;

/// An `alert()` / `alertcondition()` that fired.
#[derive(Clone, Debug, PartialEq)]
pub struct AlertEvent {
    pub id: usize,
    /// Call-site key, so `alert.freq_once_per_bar` can fire once per bar per call.
    pub site: usize,
    pub bar: usize,
    pub time: i64,
    pub message: String,
    pub frequency: AlertFrequency,
    /// Whether the execution ran on a realtime bar. Historical executions never notify.
    pub is_realtime: bool,
    /// Whether the execution was the bar's closing one.
    pub is_confirmed: bool,
}

/// What one `request.security` call site remembers between chart bars.
#[derive(Clone, Debug)]
pub struct SecuritySite {
    /// Open of the higher-timeframe bucket the chart bars are currently in.
    pub bucket: Option<i64>,
    /// The confirmed chart bars of that bucket so far, folded into one candle.
    pub accumulated: Option<Kline>,
    /// Whether `accumulated` holds bars whose bucket has not been completed yet.
    pub has_pending_bars: bool,
    /// Whether the history from before the chart's first bar has been fed in.
    pub is_warmed_up: bool,
    /// Another symbol's series: how many of its candles have been completed and evaluated.
    pub completed_candles: usize,
    /// The expression's own state, not shared with the chart script.
    pub state: Box<RuntimeState>,
    /// The expression's value on the last completed higher-timeframe bar.
    pub last_result: RuntimeValue,
    /// The chart bar this site last answered, and what it answered, for a second call.
    pub processed_bar: Option<i64>,
    pub bar_result: RuntimeValue,
}

impl Default for SecuritySite {
    fn default() -> Self {
        Self {
            bucket: None,
            accumulated: None,
            has_pending_bars: false,
            is_warmed_up: false,
            completed_candles: 0,
            state: Box::default(),
            last_result: RuntimeValue::Na,
            processed_bar: None,
            bar_result: RuntimeValue::Na,
        }
    }
}

/// Everything a script run accumulates. The session keeps a `committed` and a `working` copy:
/// a realtime tick runs on `working` and is thrown away unless the bar confirms.
#[derive(Clone, Debug, Default)]
pub struct RuntimeState {
    pub variables: FastMap<String, RuntimeValue>,
    pub histories: FastMap<String, Vec<RuntimeValue>>,
    /// The bar a function-local series (`fn:<site>:<name>` in `histories`) was last written on, so
    /// several calls in one bar leave one value.
    pub local_history_bar: FastMap<String, i64>,
    /// This bar's value of every `expression[n]` whose base is not a plain name, by history key.
    pub expression_values: FastMap<String, RuntimeValue>,
    pub calls: FastMap<usize, Vec<RuntimeValue>>,
    pub call_inputs: FastMap<usize, Vec<RuntimeValue>>,
    /// Complete state for each `ta.*` call site. It belongs to the working state so an
    /// unconfirmed realtime tick is rolled back together with variables and containers.
    pub ta_states: FastMap<usize, ta::State>,
    /// The `once` structures (by site key) whose block has run; they never run it again. A scan of a
    /// short list: a script has a handful of them, and a set here made `RuntimeState` large enough to
    /// slow four sessions running at once by 11% (`simultaneous_x4`).
    pub once_finished: Vec<usize>,
    pub plots: FastMap<usize, PlotOutput>,
    pub hlines: FastMap<usize, HorizontalLine>,
    pub markers: FastMap<usize, MarkerOutput>,
    pub backgrounds: FastMap<usize, ColorOutput>,
    pub bar_colors: FastMap<usize, ColorOutput>,
    pub fills: FastMap<usize, FillOutput>,
    // Reference-typed objects live in the state so realtime rollback restores them
    // together with the variables that point at them.
    pub arrays: FastMap<usize, Vec<RuntimeValue>>,
    pub maps: FastMap<usize, Map>,
    pub matrices: FastMap<usize, Matrix>,
    /// Instances of script-defined types, by handle.
    pub instances: FastMap<usize, Object>,
    /// `request.security` call sites, by call-site key. Each holds a state of its own.
    pub securities: FastMap<usize, SecuritySite>,
    pub lines: FastMap<usize, LineOutput>,
    pub labels: FastMap<usize, LabelOutput>,
    pub boxes: FastMap<usize, BoxOutput>,
    pub linefills: FastMap<usize, LinefillOutput>,
    pub polylines: FastMap<usize, PolylineOutput>,
    pub tables: FastMap<usize, TableOutput>,
    pub candles: FastMap<usize, CandleOutput>,
    pub alerts: Vec<AlertEvent>,
    pub broker: Broker,
    pub next_reference: usize,
    pub bar_index: i64,
    pub instructions: usize,
}
