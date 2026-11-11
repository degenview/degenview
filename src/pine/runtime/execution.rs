//! Persistent Pine execution over a changing candle snapshot. The controller decides when a
//! snapshot is history, a realtime tick, or a close; [`Session`](super::session::Session) remains
//! responsible for evaluating the program. This is the Rust counterpart of Swift's execution
//! controller and scheduler.

use std::collections::HashMap;
use std::sync::{Arc, atomic::AtomicBool};

use crate::model::Kline;

use super::session::{RunError, Session};
use super::state::AlertEvent;
use super::support::SecurityDataProvider;
use crate::pine::compiler::program::{
    BarEvent, BarPhase, InputValue, ScriptType, SymbolInfo, VisualOutput,
};
use crate::pine::compiler::{self, LibraryResolver};
use crate::pine::logs::LogBatch;
use crate::pine::{Diagnostic, DiagnosticCategory, DiagnosticSeverity, Limits, SourceRange};

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
    /// How the script last failed, until it runs again and succeeds. An update that changes nothing
    /// does not run the script, so it must still say the script is failing: otherwise a failure
    /// would be reported once and then read as a success.
    last_error: Option<Diagnostic>,
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
            last_error: None,
        })
    }

    /// Runs the script against a chart with this foreground and background colour.
    pub fn with_theme(mut self, theme: crate::pine::compiler::program::ChartTheme) -> Self {
        self.session.theme = theme;
        self
    }

    /// Keeps the messages of `log.*()` calls and hands them over with each update. Off by default:
    /// a run nobody reads formats and keeps nothing.
    pub fn with_logs(mut self) -> Self {
        self.session.logs.enable();
        self
    }

    /// Makes the run an alert's: `runtime.calculation.mode` reads as the alert mode.
    pub fn with_alert_mode(mut self) -> Self {
        self.session.alert_mode = true;
        self
    }

    /// Makes the run a debugging session: the script stops where the debugger says. Nothing else
    /// installs the hooks, so a controller built without this runs the code it always did.
    pub fn with_debugger(mut self, debug: crate::pine::debug::DebugRuntime) -> Self {
        self.session.attach_debugger(debug);
        self
    }

    /// Makes the run a profiling session: each statement, user function call and bar is timed.
    /// Nothing else installs the hooks, so a controller built without this runs the code it always
    /// did. The recorder reports the bars it has finished to `progress`.
    pub fn with_profiler(
        mut self,
        progress: Arc<std::sync::atomic::AtomicU64>,
        waited: Arc<std::sync::atomic::AtomicU64>,
    ) -> Self {
        let table = Arc::new(crate::pine::profile::RegionTable::build(
            &self.session.program,
        ));
        self.session.attach_profiler(
            crate::pine::profile::ProfileRuntime::new(table)
                .report_progress_to(progress)
                .exclude_waiting(waited),
        );
        self
    }

    /// Like [`with_profiler`](Self::with_profiler), with a recorder the caller built (a test's, on a
    /// clock it controls).
    pub fn with_profile_runtime(mut self, runtime: crate::pine::profile::ProfileRuntime) -> Self {
        self.session.attach_profiler(runtime);
        self
    }

    /// Ends profiling and hands over what was measured; `None` if this run was not profiling.
    pub fn finish_profile(
        &mut self,
        completeness: crate::pine::profile::Completeness,
    ) -> Option<crate::pine::profile::ProfileReport> {
        self.session
            .detach_profiler()
            .map(|runtime| runtime.finish(completeness))
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
            let standing = self.last_error.clone();
            return self.result(Vec::new(), standing, false, false);
        }
        if bars.is_empty() {
            self.bars.clear();
            self.live_open_time = None;
            self.visible_range = visible_range;
            self.session.visible_range = visible_range;
            self.session.reset(None);
            self.last_error = None;
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
        // Whether the script ran at all: a tick a strategy skips tells nothing new about a failure.
        let mut ran = false;
        let old_last = self.bars.last().cloned().expect("non-empty snapshot");
        let new_last = bars.last().cloned().expect("non-empty snapshot");
        if old_last.open_time == new_last.open_time {
            let phase = if is_closed(&new_last, now_ms) {
                BarPhase::RealtimeClose { is_new: false }
            } else {
                BarPhase::RealtimeTick { is_new: false }
            };
            ran |= self.execute_event(new_last.clone(), phase, &mut alerts, &mut error);
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
                ran |= self.execute_event(
                    closed,
                    BarPhase::RealtimeClose { is_new: false },
                    &mut alerts,
                    &mut error,
                );
                self.live_open_time = None;
            }
            if error.is_none() {
                if is_closed(&new_last, now_ms) {
                    ran |= self.execute_event(
                        new_last.clone(),
                        BarPhase::Historical,
                        &mut alerts,
                        &mut error,
                    );
                } else {
                    ran |= self.execute_event(
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
        if ran {
            self.last_error = error.clone();
        } else {
            error = self.last_error.clone();
        }
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
        self.last_error = error.clone();
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
    ) -> bool {
        if !should_execute(&self.session, phase) {
            return false;
        }
        match self.session.execute(BarEvent { candle, phase }, true, None) {
            Ok(_) => alerts.extend(self.session.emitted_alerts.clone()),
            Err(run_error) => *error = script_error(run_error),
        }
        true
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
    use crate::pine::builtins::alert::frequency::AlertFrequency;
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

    /// 60-second bars, `count` of them, all long closed by the time `AFTER` comes.
    fn history(count: i64) -> Vec<Kline> {
        (0..count)
            .map(|i| bar(i * 60_000, 10.0 + i as f64))
            .collect()
    }

    const AFTER: i64 = 1_000_000_000;
    const FAILS_ON_THE_LAST_BAR: &str = "//@version=6\nindicator(\"T\")\nif barstate.islast\n    runtime.error(\"boom\")\nplot(close)";

    #[test]
    fn a_failure_is_still_reported_by_an_update_that_changes_nothing() {
        let mut controller = controller(FAILS_ON_THE_LAST_BAR);
        let bars = history(3);
        let first = controller.update(&bars, None, AFTER);
        assert_eq!(
            first.error.as_ref().map(|e| e.message.as_str()),
            Some("boom")
        );
        // The same bars again: nothing runs, and the script is still failing.
        let again = controller.update(&bars, None, AFTER);
        assert!(!again.changed);
        assert_eq!(
            again.error.as_ref().map(|e| e.message.as_str()),
            Some("boom")
        );
        let and_again = controller.update(&bars, None, AFTER);
        assert!(and_again.error.is_some());
    }

    #[test]
    fn a_failed_script_stays_stopped_and_keeps_the_bars_before_it() {
        let mut controller = controller(
            "//@version=6\nindicator(\"T\")\nif bar_index == 1\n    runtime.error(\"boom\")\nplot(close)",
        );
        let failed = controller.update(&history(2), None, AFTER);
        assert_eq!(
            failed.error.as_ref().map(|e| e.message.as_str()),
            Some("boom")
        );
        // The bar it failed on is not shown; the one before it is.
        assert_eq!(failed.output.bar_count, 1);
        assert_eq!(failed.output.plots[0].values, [Some(10.0)]);
        // Another closed bar arrives: the script does not run again, and it still says why.
        let next = controller.update(&history(3), None, AFTER);
        assert_eq!(
            next.error.as_ref().map(|e| e.message.as_str()),
            Some("boom")
        );
        assert_eq!(next.output.bar_count, 1);
        assert_eq!(next.output.plots[0].values, [Some(10.0)]);
        assert!(!next.rebuilt);
    }

    #[test]
    fn a_script_that_fails_on_a_live_tick_does_not_start_again() {
        let mut controller = controller(
            "//@version=6\nindicator(\"T\")\nif barstate.isrealtime and close > 12\n    runtime.error(\"late\")\nplot(close)",
        );
        let with_live = |close: f64| {
            let mut bars = history(3);
            bars.push(bar(3 * 60_000, close));
            bars
        };
        // Three closed bars, then a live one that is fine.
        let first = controller.update(&with_live(11.0), None, 200_000);
        assert!(first.error.is_none(), "{:?}", first.error);
        // A tick that makes it fail.
        let failed = controller.update(&with_live(13.0), None, 210_000);
        assert_eq!(
            failed.error.as_ref().map(|e| e.message.as_str()),
            Some("late")
        );
        assert_eq!(failed.output.bar_count, 3);
        // The price comes back, so the condition is false again, and then the bar closes: the
        // script stays stopped.
        let back = controller.update(&with_live(11.0), None, 220_000);
        assert_eq!(
            back.error.as_ref().map(|e| e.message.as_str()),
            Some("late")
        );
        let closed = controller.update(&with_live(11.0), None, 240_000);
        assert_eq!(
            closed.error.as_ref().map(|e| e.message.as_str()),
            Some("late")
        );
        assert_eq!(closed.output.bar_count, 3);
    }

    #[test]
    fn an_empty_snapshot_forgets_the_failure() {
        let mut controller = controller(FAILS_ON_THE_LAST_BAR);
        assert!(controller.update(&history(3), None, AFTER).error.is_some());
        assert!(controller.update(&[], None, AFTER).error.is_none());
        assert!(controller.update(&[], None, AFTER).error.is_none());
    }

    #[test]
    fn a_rebuild_replaces_the_standing_failure() {
        let mut controller = controller(
            "//@version=6\nindicator(\"T\")\nif barstate.islast and bar_index > 2\n    runtime.error(\"boom\")\nplot(close)",
        );
        // Three bars: the last has index 2, so the script is fine.
        assert!(controller.update(&history(3), None, AFTER).error.is_none());
        // Five bars is a different snapshot, rebuilt: the last bar fails now.
        assert!(controller.update(&history(5), None, AFTER).error.is_some());
        // Back to three bars: rebuilt, and fine again.
        assert!(controller.update(&history(3), None, AFTER).error.is_none());
    }

    /// Drawings, markers, alerts and (for a strategy) a fill that bar 2 produced; bar 3 does the same and
    /// then fails.
    const EFFECTS_BEFORE_THE_ERROR: &str = "//@version=6\nindicator(\"T\")\n\
        if bar_index >= 2\n    label.new(bar_index, close, \"x\")\n    line.new(bar_index, close, bar_index + 1, close)\n    alert(\"hi\", alert.freq_all)\n\
        plotshape(bar_index >= 2, style = shape.circle)\n\
        bgcolor(bar_index >= 2 ? color.red : na)\n\
        if bar_index == 3\n    runtime.error(\"boom\")\n\
        plot(close)";

    fn assert_only_bars_before_the_failure_are_left(update: &ExecutionUpdate) {
        let output = &update.output;
        assert_eq!(output.bar_count, 3);
        assert_eq!(output.labels.len(), 1, "labels");
        assert_eq!(output.lines.len(), 1, "lines");
        assert_eq!(output.alerts.len(), 1, "alerts");
        assert_eq!(output.markers[0].values, [false, false, true], "markers");
        assert_eq!(output.backgrounds[0].colors.len(), 3, "backgrounds");
        assert_eq!(output.plots[0].values.len(), 3, "plots");
    }

    #[test]
    fn a_failed_history_bar_leaves_none_of_its_effects_in_the_output() {
        let mut controller = controller(EFFECTS_BEFORE_THE_ERROR);
        let failed = controller.update(&history(5), None, AFTER);
        assert!(failed.error.is_some());
        assert_only_bars_before_the_failure_are_left(&failed);
    }

    #[test]
    fn a_closed_bar_that_fails_when_it_arrives_leaves_none_of_its_effects_in_the_output() {
        // The history is fine; the bar that arrives afterwards runs on its own, not in a replay.
        let mut controller = controller(EFFECTS_BEFORE_THE_ERROR);
        assert!(controller.update(&history(3), None, AFTER).error.is_none());
        let failed = controller.update(&history(4), None, AFTER);
        assert!(!failed.rebuilt);
        assert!(failed.error.is_some());
        assert_only_bars_before_the_failure_are_left(&failed);
    }

    #[test]
    fn a_fill_on_the_failing_bar_is_not_in_the_strategy_report() {
        // The order bar 2 places fills at the open of bar 3, before the script runs there and fails.
        let source = "//@version=6\nstrategy(\"S\")\n\
            if bar_index == 2\n    strategy.entry(\"L\", strategy.long)\n\
            if bar_index == 3\n    runtime.error(\"boom\")\n\
            plot(close)";
        let mut controller = controller(source);
        let failed = controller.update(&history(5), None, AFTER);
        assert!(failed.error.is_some());
        let report = failed.output.strategy.expect("a strategy report");
        assert!(report.open_trades.is_empty(), "{:?}", report.open_trades);
        assert_eq!(report.equity.len(), 3);
    }
}
