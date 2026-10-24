//! Persistent Pine execution over a changing candle snapshot. The controller decides when a
//! snapshot is history, a realtime tick, or a close; [`Session`](super::session::Session) remains
//! responsible for evaluating the program. This is the Rust counterpart of Swift's execution
//! controller and scheduler.

use std::collections::HashMap;
use std::sync::{Arc, atomic::AtomicBool};

use crate::model::Kline;

use super::compiler::{self, LibraryResolver};
use super::logs::LogBatch;
use super::program::{BarEvent, BarPhase, InputValue, ScriptType, SymbolInfo, VisualOutput};
use super::runtime_support::SecurityDataProvider;
use super::session::{RunError, Session};
use super::state::AlertEvent;
use super::{Diagnostic, DiagnosticCategory, DiagnosticSeverity, Limits, SourceRange};

#[derive(Clone, Debug)]
pub struct ExecutionUpdate {
    pub output: VisualOutput,
    pub alerts: Vec<AlertEvent>,
    pub error: Option<Diagnostic>,
    /// True when the complete history had to be replayed rather than applying an incremental
    /// tick/close. Alert routers use this to avoid notifying for the initial snapshot.
    pub rebuilt: bool,
    /// False when the snapshot and visible range were identical to the prior update.
    pub changed: bool,
    /// The `log.*()` messages written during this update; empty unless
    /// [`ExecutionController::with_logs`] was asked for.
    pub logs: LogBatch,
}

pub struct ExecutionController {
    session: Session,
    bars: Vec<Kline>,
    visible_range: Option<(i64, i64)>,
    live_open_time: Option<i64>,
}

impl ExecutionController {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        source: &str,
        inputs: HashMap<String, InputValue>,
        symbol: SymbolInfo,
        visible_range: Option<(i64, i64)>,
        limits: Limits,
        libraries: &dyn LibraryResolver,
        security_data: Option<Box<dyn SecurityDataProvider>>,
        cancellation: Option<Arc<AtomicBool>>,
    ) -> Result<Self, Diagnostic> {
        let program = compiler::compile(source, limits, libraries);
        if !program.is_valid() || program.statements.is_empty() {
            return Err(program
                .diagnostics
                .iter()
                .find(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
                .cloned()
                .unwrap_or_else(|| {
                    Diagnostic::error(
                        "PINE3001",
                        DiagnosticCategory::Semantic,
                        "A script must contain an executable declaration.",
                        SourceRange::ZERO,
                    )
                }));
        }
        let mut session = Session::new(
            program,
            inputs,
            limits,
            None,
            symbol,
            security_data,
            visible_range,
        );
        if let Some(cancellation) = cancellation {
            session.set_cancellation(cancellation);
        }
        Ok(Self {
            session,
            bars: Vec::new(),
            visible_range,
            live_open_time: None,
        })
    }

    /// Runs the script against a chart with this foreground and background colour.
    pub fn with_theme(mut self, theme: super::program::ChartTheme) -> Self {
        self.session.theme = theme;
        self
    }

    /// Keeps the messages of `log.*()` calls and hands them over with each update. Off by default:
    /// a run nobody reads formats and keeps nothing.
    pub fn with_logs(mut self) -> Self {
        self.session.logs.enable();
        self
    }

    /// Makes the run a debugging session: the script stops where the debugger says. Nothing else
    /// installs the hooks, so a controller built without this runs the code it always did.
    pub fn with_debugger(mut self, debug: super::debug::DebugRuntime) -> Self {
        self.session.attach_debugger(debug);
        self
    }

    /// Fixes the bar length scripts read as `timeframe.in_seconds()`. A price-based chart's bars
    /// follow price, so it cannot be told from their times; the chart passes its source
    /// candles' length.
    pub fn with_bar_seconds(mut self, seconds: f64) -> Self {
        self.session.fixed_bar_seconds = (seconds > 0.0).then_some(seconds);
        self
    }

    pub fn update(
        &mut self,
        bars: &[Kline],
        visible_range: Option<(i64, i64)>,
        now_ms: i64,
    ) -> ExecutionUpdate {
        self.session.logs.set_clock(now_ms);
        let live_bar_has_closed =
            self.live_open_time.is_some() && bars.last().is_some_and(|bar| is_closed(bar, now_ms));
        if self.bars == bars && self.visible_range == visible_range && !live_bar_has_closed {
            return self.result(Vec::new(), None, false, false);
        }
        if bars.is_empty() {
            self.bars.clear();
            self.live_open_time = None;
            self.visible_range = visible_range;
            self.session.visible_range = visible_range;
            self.session.reset(None);
            return self.result(Vec::new(), None, true, true);
        }

        let must_rebuild = self.bars.is_empty()
            || self.visible_range != visible_range
            || !self.can_apply_incrementally(bars);
        self.visible_range = visible_range;
        self.session.visible_range = visible_range;
        if must_rebuild {
            return self.rebuild(bars, now_ms);
        }

        let mut alerts = Vec::new();
        let mut error = None;
        let old_last = self.bars.last().cloned().expect("non-empty snapshot");
        let new_last = bars.last().cloned().expect("non-empty snapshot");
        if old_last.open_time == new_last.open_time {
            let phase = if is_closed(&new_last, now_ms) {
                BarPhase::RealtimeClose { is_new: false }
            } else {
                BarPhase::RealtimeTick { is_new: false }
            };
            self.execute_event(new_last.clone(), phase, &mut alerts, &mut error);
            if phase.is_confirmed() {
                self.live_open_time = None;
            }
        } else {
            if self.live_open_time == Some(old_last.open_time) {
                let closed = bars
                    .iter()
                    .rev()
                    .find(|bar| bar.open_time == old_last.open_time)
                    .cloned()
                    .unwrap_or(old_last);
                self.execute_event(
                    closed,
                    BarPhase::RealtimeClose { is_new: false },
                    &mut alerts,
                    &mut error,
                );
                self.live_open_time = None;
            }
            if error.is_none() {
                if is_closed(&new_last, now_ms) {
                    self.execute_event(
                        new_last.clone(),
                        BarPhase::Historical,
                        &mut alerts,
                        &mut error,
                    );
                } else {
                    self.execute_event(
                        new_last.clone(),
                        BarPhase::RealtimeTick { is_new: true },
                        &mut alerts,
                        &mut error,
                    );
                    self.live_open_time = Some(new_last.open_time);
                }
            }
        }
        self.bars = bars.to_vec();
        self.result(alerts, error, false, true)
    }

    fn rebuild(&mut self, bars: &[Kline], now_ms: i64) -> ExecutionUpdate {
        let mut alerts = Vec::new();
        let mut error = None;
        let last_is_live = bars.last().is_some_and(|bar| !is_closed(bar, now_ms));
        let history_len = bars.len().saturating_sub(usize::from(last_is_live));
        let history = &bars[..history_len];
        let seed = bars
            .last()
            .map(|bar| (bar.close_time - bar.open_time).max(0) as f64 / 1000.0);
        if let Err(run_error) = self.session.load_history(history, last_is_live, seed) {
            error = script_error(run_error);
        } else if last_is_live {
            let live = bars.last().cloned().expect("live bar");
            self.execute_event(
                live.clone(),
                BarPhase::RealtimeTick { is_new: true },
                &mut alerts,
                &mut error,
            );
            self.live_open_time = Some(live.open_time);
        } else {
            self.live_open_time = None;
        }
        self.bars = bars.to_vec();
        self.result(alerts, error, true, true)
    }

    fn can_apply_incrementally(&self, next: &[Kline]) -> bool {
        if self.bars.is_empty() || next.is_empty() {
            return false;
        }
        let old_last = self.bars.last().expect("old last");
        let new_last = next.last().expect("new last");
        if old_last.open_time == new_last.open_time {
            return self.bars.len() == next.len()
                && self.bars[..self.bars.len() - 1] == next[..next.len() - 1]
                && self.live_open_time == Some(old_last.open_time);
        }
        next.len() == self.bars.len() + 1
            && self.bars[..self.bars.len() - 1] == next[..self.bars.len() - 1]
            && next[next.len() - 2].open_time == old_last.open_time
            && (self.live_open_time == Some(old_last.open_time)
                || next[next.len() - 2] == *old_last)
    }

    fn execute_event(
        &mut self,
        candle: Kline,
        phase: BarPhase,
        alerts: &mut Vec<AlertEvent>,
        error: &mut Option<Diagnostic>,
    ) {
        if !should_execute(&self.session, phase) {
            return;
        }
        match self.session.execute(BarEvent { candle, phase }, true, None) {
            Ok(_) => alerts.extend(self.session.emitted_alerts.clone()),
            Err(run_error) => *error = script_error(run_error),
        }
    }

    fn result(
        &mut self,
        alerts: Vec<AlertEvent>,
        error: Option<Diagnostic>,
        rebuilt: bool,
        changed: bool,
    ) -> ExecutionUpdate {
        ExecutionUpdate {
            output: self.session.output(),
            alerts,
            error,
            rebuilt,
            changed,
            logs: self.session.logs.take(),
        }
    }
}

fn should_execute(session: &Session, phase: BarPhase) -> bool {
    if session.program.declaration.script_type != ScriptType::Strategy
        || !phase.is_realtime()
        || phase.is_confirmed()
    {
        return true;
    }
    session
        .program
        .declaration
        .strategy
        .as_ref()
        .is_some_and(|settings| settings.calc_on_every_tick)
}

fn script_error(error: RunError) -> Option<Diagnostic> {
    match error {
        RunError::Script(diagnostic) => Some(diagnostic),
        RunError::Rejection(_) => None,
    }
}

fn is_closed(bar: &Kline, now_ms: i64) -> bool {
    bar.close_time <= now_ms
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::alert_frequency::AlertFrequency;
    use crate::pine::compiler::NoLibraries;

    fn bar(open_time: i64, close: f64) -> Kline {
        Kline {
            open_time,
            close_time: open_time + 60_000,
            open: close,
            high: close,
            low: close,
            close,
            volume: 1.0,
        }
    }

    fn controller(source: &str) -> ExecutionController {
        ExecutionController::new(
            source,
            HashMap::new(),
            SymbolInfo::default(),
            None,
            Limits::default(),
            &NoLibraries,
            None,
            None,
        )
        .expect("valid controller")
    }

    #[test]
    fn forming_bar_updates_recompute_output_without_growing_history() {
        let mut controller = controller("//@version=6\nindicator(\"T\")\nplot(close)");
        let first = controller.update(&[bar(0, 10.0)], None, 30_000);
        assert!(first.rebuilt);
        assert_eq!(first.output.plots[0].values.last(), Some(&Some(10.0)));

        let second = controller.update(&[bar(0, 20.0)], None, 40_000);
        assert!(!second.rebuilt);
        assert_eq!(second.output.bar_count, 1);
        assert_eq!(second.output.plots[0].values.last(), Some(&Some(20.0)));
    }

    #[test]
    fn alerts_follow_tick_once_per_bar_and_close_frequency() {
        let source = "//@version=6\nindicator(\"T\")\n\
            alert(\"all\", alert.freq_all)\n\
            alert(\"once\", alert.freq_once_per_bar)\n\
            alert(\"close\", alert.freq_once_per_bar_close)";
        let mut controller = controller(source);
        let initial = controller.update(&[bar(0, 10.0)], None, 10_000);
        assert_eq!(initial.alerts.len(), 2);

        let tick = controller.update(&[bar(0, 11.0)], None, 20_000);
        assert_eq!(tick.alerts.len(), 1);
        assert_eq!(tick.alerts[0].frequency, AlertFrequency::All);

        let close = controller.update(&[bar(0, 12.0)], None, 60_000);
        assert_eq!(close.alerts.len(), 2);
        assert!(close.alerts.iter().any(|alert| {
            alert.frequency == AlertFrequency::OncePerBarClose && alert.is_confirmed
        }));
    }

    #[test]
    fn default_strategies_skip_ticks_but_run_on_close() {
        let mut controller =
            controller("//@version=6\nstrategy(\"T\", calc_on_every_tick = false)\nplot(close)");
        let tick = controller.update(&[bar(0, 10.0)], None, 10_000);
        assert!(tick.output.plots.is_empty());
        let close = controller.update(&[bar(0, 12.0)], None, 60_000);
        assert_eq!(close.output.plots[0].values.last(), Some(&Some(12.0)));
    }
}
