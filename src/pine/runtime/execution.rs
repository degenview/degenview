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
    /// The `log.*()` messages written during this update; empty unless
    /// [`ExecutionController::with_logs`] was asked for.
    pub logs: LogBatch,
}

pub struct ExecutionController {
    session: Session,
    bars: Vec<Kline>,
    visible_range: Option<(i64, i64)>,
    live_open_time: Option<i64>,
    /// How long after its close the newest bar is still live; `None`: until a newer one arrives.
    settle_delay_ms: Option<i64>,
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
            settle_delay_ms: Some(0),
            last_error: None,
        })
    }

    /// Runs the script against a chart with this foreground and background colour.
    pub fn with_theme(mut self, theme: compiler::program::ChartTheme) -> Self {
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

    /// Says how long the feed may still be changing a bar after its close, see
    /// [`DataSource::settle_delay_ms`](crate::model::DataSource::settle_delay_ms). Without it a bar
    /// closes by the clock, on the values it has then.
    pub fn with_settle_delay(mut self, delay_ms: Option<i64>) -> Self {
        self.settle_delay_ms = delay_ms;
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

    /// Tells the script the length of the chart's timeframe (`timeframe.in_seconds()`), so it is
    /// not read off the gap between bars: a bucket cut short by midnight, a session's first bar
    /// after a night or a weekend would give a length the chart does not have. A price-based chart's
    /// [`with_bar_seconds`](Self::with_bar_seconds) takes precedence.
    pub fn with_timeframe_seconds(mut self, seconds: f64) -> Self {
        self.session.configured_bar_seconds = (seconds > 0.0).then_some(seconds);
        self
    }

    /// How the script's last run failed; `None` while it runs, and before its first update.
    pub fn error(&self) -> Option<&Diagnostic> {
        self.last_error.as_ref()
    }

    pub fn update(
        &mut self,
        bars: &[Kline],
        visible_range: Option<(i64, i64)>,
        now_ms: i64,
    ) -> ExecutionUpdate {
        self.session.logs.set_clock(now_ms);
        self.session.now_ms = Some(now_ms);
        let live_bar_has_closed = self.live_open_time.is_some()
            && bars.last().is_some_and(|bar| self.has_settled(bar, now_ms));
        if self.bars == bars && self.visible_range == visible_range && !live_bar_has_closed {
            let standing = self.last_error.clone();
            return self.result(Vec::new(), standing, false);
        }
        if bars.is_empty() {
            self.bars.clear();
            self.live_open_time = None;
            self.visible_range = visible_range;
            self.session.visible_range = visible_range;
            self.session.reset(None);
            self.last_error = None;
            return self.result(Vec::new(), None, true);
        }

        // A stopped script runs nothing, so what a live feed does to its newest bars (finishing
        // one after its close, adding the next) is no reason to replay and start it again.
        let stays_stopped = self.session.is_halted() && has_same_history(&self.bars, bars);
        let must_rebuild = self.bars.is_empty()
            || self.visible_range != visible_range
            || !(stays_stopped || self.can_apply_incrementally(bars, now_ms));
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
            let phase = if self.has_settled(&new_last, now_ms) {
                BarPhase::RealtimeClose { is_new: false }
            } else {
                BarPhase::RealtimeTick { is_new: false }
            };
            ran |= self.execute_event(new_last.clone(), phase, &mut alerts, &mut error);
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
            }
            if error.is_none() {
                if self.has_settled(&new_last, now_ms) {
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
                }
            }
        }
        // Which bar is forming is a fact about the snapshot, whether or not the script ran it: a
        // stopped script must not look like new history on the next tick, which would replay it.
        self.live_open_time = (!self.has_settled(&new_last, now_ms)).then_some(new_last.open_time);
        self.bars = bars.to_vec();
        if ran {
            self.last_error = error.clone();
        } else {
            error = self.last_error.clone();
        }
        self.result(alerts, error, false)
    }

    fn rebuild(&mut self, bars: &[Kline], now_ms: i64) -> ExecutionUpdate {
        let mut alerts = Vec::new();
        let mut error = None;
        let last_is_live = bars
            .last()
            .is_some_and(|bar| !self.has_settled(bar, now_ms));
        let history_len = bars.len().saturating_sub(usize::from(last_is_live));
        // The live bar's own open, not an estimate from the history: a calendar bar has no fixed length.
        let live_open = bars
            .last()
            .filter(|_| last_is_live)
            .map(|bar| bar.open_time);
        let history = &bars[..history_len];
        let seed = bars
            .last()
            .map(|bar| (bar.close_time - bar.open_time).max(0) as f64 / 1000.0);
        let replayed = self.session.load_history(history, live_open, seed);
        // A replay interrupted between bars leaves a partial state that only another replay can
        // continue from; one that finished, or stopped the script, takes ticks.
        let takes_ticks = replayed.is_ok() || self.session.is_halted();
        match replayed {
            Err(run_error) => error = script_error(run_error),
            Ok(_) => {
                if let Some(live) = bars.last().filter(|_| last_is_live) {
                    self.execute_event(
                        live.clone(),
                        BarPhase::RealtimeTick { is_new: true },
                        &mut alerts,
                        &mut error,
                    );
                }
            }
        }
        self.live_open_time = live_open.filter(|_| takes_ticks);
        self.bars = bars.to_vec();
        self.last_error = error.clone();
        self.result(alerts, error, true)
    }

    /// Whether a closed bar is still within the time its feed takes to be done with it.
    fn may_still_change(&self, bar: &Kline, now_ms: i64) -> bool {
        self.settle_delay_ms.is_some() && !self.has_settled(bar, now_ms)
    }

    /// Whether the newest bar, with nothing newer to go by, is finished.
    fn has_settled(&self, bar: &Kline, now_ms: i64) -> bool {
        self.settle_delay_ms
            .is_some_and(|delay| bar.close_time.saturating_add(delay) <= now_ms)
    }

    fn can_apply_incrementally(&self, next: &[Kline], now_ms: i64) -> bool {
        if self.bars.is_empty() || next.is_empty() {
            return false;
        }
        let old_last = self.bars.last().expect("old last");
        let new_last = next.last().expect("new last");
        let before_old_last = self.bars.len() - 1;
        if old_last.open_time == new_last.open_time {
            return self.bars.len() == next.len()
                && self.agrees_with(&next[..before_old_last], now_ms)
                && self.live_open_time == Some(old_last.open_time);
        }
        next.len() == self.bars.len() + 1
            && self.agrees_with(&next[..before_old_last], now_ms)
            && next[next.len() - 2].open_time == old_last.open_time
            && (self.live_open_time == Some(old_last.open_time)
                || next[next.len() - 2] == *old_last)
    }

    /// Whether `next` has the bars this run saw, as far as it goes. A bar the feed may still be
    /// correcting counts whatever its values: the script keeps what it saw of it, as it would on a
    /// chart left open, instead of starting over.
    fn agrees_with(&self, next: &[Kline], now_ms: i64) -> bool {
        self.bars.iter().zip(next).all(|(seen, now)| {
            seen == now || (seen.open_time == now.open_time && self.may_still_change(seen, now_ms))
        })
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
    ) -> ExecutionUpdate {
        ExecutionUpdate {
            output: self.session.output(),
            alerts,
            error,
            rebuilt,
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
        RunError::Rejection => None,
    }
}

/// Whether `next` is the history `seen` was, at most with its two newest bars changed or bars added
/// after them.
fn has_same_history(seen: &[Kline], next: &[Kline]) -> bool {
    let settled = seen.len().saturating_sub(2);
    next.len() >= seen.len()
        && seen.first().map(|bar| bar.open_time) == next.first().map(|bar| bar.open_time)
        && seen[..settled] == next[..settled]
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

    /// A feed that corrects bars for a while after they closed: the script keeps what it saw,
    /// as it would on a chart left open, and nothing is replayed. Past the feed's settling time a
    /// different bar is a different history.
    #[test]
    fn a_late_correction_to_a_bar_not_yet_settled_is_not_new_history() {
        let mut controller = controller("//@version=6\nindicator(\"T\")\nalert(\"bar\")")
            .with_settle_delay(Some(120_000));
        let mut bars = history(3);
        controller.update(&bars, None, 150_000);
        // The next bar arrives, and with it other values for the two before the last.
        bars.push(bar(180_000, 13.0));
        bars[1].low -= 0.5;
        bars[2].low -= 0.5;
        let update = controller.update(&bars, None, 185_000);
        assert!(!update.rebuilt);
        assert_eq!(update.alerts.len(), 1);
        // A correction between two bars is no replay either.
        bars[2].low -= 0.5;
        assert!(!controller.update(&bars, None, 190_000).rebuilt);
        // The first bar closed over two minutes ago: that one is history.
        bars[0].low -= 0.5;
        assert!(controller.update(&bars, None, 200_000).rebuilt);
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

    /// A new tick of the same forming bar is not new history: nothing is replayed, so nothing can
    /// start the stopped script again.
    fn assert_stays_stopped(controller: &mut ExecutionController, mut bars: Vec<Kline>, now: i64) {
        for tick in 0..2 {
            bars.last_mut().expect("forming bar").close += 0.1;
            let update = controller.update(&bars, None, now + tick);
            assert!(!update.rebuilt, "tick {tick}");
            assert!(update.error.is_some(), "tick {tick}");
            assert!(update.alerts.is_empty(), "tick {tick}");
        }
    }

    #[test]
    fn a_script_that_fails_when_a_live_bar_closes_does_not_start_again() {
        let mut controller = controller(
            "//@version=6\nindicator(\"T\")\nalert(\"hi\", alert.freq_all)\n\
            if barstate.isrealtime and barstate.isconfirmed\n    runtime.error(\"on close\")",
        );
        let mut bars = history(4);
        assert!(controller.update(&bars, None, 200_000).error.is_none());
        // The forming bar closes as the next one arrives.
        bars.push(bar(4 * 60_000, 14.0));
        assert!(controller.update(&bars, None, 250_000).error.is_some());
        assert_stays_stopped(&mut controller, bars, 260_000);
    }

    #[test]
    fn a_script_that_failed_on_history_is_not_replayed_by_a_live_tick() {
        let mut controller = controller(
            "//@version=6\nindicator(\"T\")\nif bar_index == 1\n    runtime.error(\"early\")",
        );
        let bars = history(4);
        assert!(controller.update(&bars, None, 200_000).error.is_some());
        assert_stays_stopped(&mut controller, bars, 210_000);
    }

    #[test]
    fn a_strategy_that_fails_when_a_live_bar_closes_does_not_start_again() {
        let mut controller = controller(
            "//@version=6\nstrategy(\"S\")\n\
            if barstate.isrealtime and barstate.isconfirmed\n    runtime.error(\"on close\")",
        );
        let mut bars = history(4);
        assert!(controller.update(&bars, None, 200_000).error.is_none());
        bars.push(bar(4 * 60_000, 14.0));
        // The close is the first realtime run: the strategy skips the ticks before it, and after
        // it, so only the standing error says it is stopped.
        assert!(controller.update(&bars, None, 250_000).error.is_some());
        assert_stays_stopped(&mut controller, bars, 260_000);
    }

    /// A live feed finishes a bar after the script has already run its close, and may do so once
    /// the next bar is forming. That is the same history.
    #[test]
    fn the_feed_finishing_a_closed_bar_does_not_start_a_stopped_script() {
        let mut controller = controller(
            "//@version=6\nindicator(\"T\")\n\
            if barstate.isrealtime and barstate.isconfirmed\n    runtime.error(\"on close\")",
        );
        let mut bars = history(4);
        assert!(controller.update(&bars, None, 200_000).error.is_none());
        assert!(controller.update(&bars, None, 250_000).error.is_some());
        let stopped = |controller: &mut ExecutionController, bars: &[Kline], now: i64| {
            let update = controller.update(bars, None, now);
            assert!(!update.rebuilt, "at {now}");
            assert!(update.error.is_some(), "at {now}");
        };
        bars[3].close += 0.1;
        stopped(&mut controller, &bars, 251_000);
        bars.push(bar(4 * 60_000, 14.0));
        stopped(&mut controller, &bars, 252_000);
        bars[3].close += 0.1;
        stopped(&mut controller, &bars, 253_000);

        // Older bars replaced: a new history, replayed from the start.
        bars[0].close += 1.0;
        let update = controller.update(&bars, None, 254_000);
        assert!(update.rebuilt);
        assert!(update.error.is_none(), "{:?}", update.error);
    }

    #[test]
    fn an_interrupted_replay_is_replayed_by_the_next_tick() {
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let mut controller = ExecutionController::new(
            "//@version=6\nindicator(\"T\")\nplot(close)",
            HashMap::new(),
            SymbolInfo::default(),
            None,
            Limits::default(),
            &NoLibraries,
            None,
            Some(Arc::clone(&cancel)),
        )
        .expect("valid controller");
        let mut bars = history(4);
        assert!(controller.update(&bars, None, 200_000).error.is_some());

        cancel.store(false, std::sync::atomic::Ordering::Release);
        bars[3].close += 0.1;
        let update = controller.update(&bars, None, 210_000);
        assert!(update.rebuilt, "a partial history takes no tick");
        assert!(update.error.is_none(), "{:?}", update.error);
        assert_eq!(update.output.bar_count, 4);
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
    /// 15-minute candles of the symbol, from midnight to 4:30.
    struct Quarter;
    impl crate::pine::runtime::support::SecurityDataProvider for Quarter {
        fn candles(&self, key: &crate::pine::runtime::support::SecurityKey) -> Option<Vec<Kline>> {
            (key.interval == 900.0).then(|| {
                (0..19)
                    .map(|i| Kline {
                        open_time: i * 900_000,
                        close_time: i * 900_000 + 899_999,
                        open: 1.0,
                        high: 1.0,
                        low: 1.0,
                        close: 1.0,
                        volume: 1.0,
                    })
                    .collect()
            })
        }
    }

    #[test]
    fn a_live_bar_splits_a_finer_request_at_the_clock_the_app_passes() {
        // The app's update path (`now_ms`), not a hand-built session: 4:40 lies inside the live bar
        // 4:00-5:00, so of the 15m candles the 4:15 one is the last confirmed history one, and the
        // 4:30 one forming now the last and the realtime one (TradingView probe S2).
        let hour = 3_600_000;
        let mut controller = ExecutionController::new(
            "//@version=6\nindicator(\"T\")\n\
             plot(array.indexof(request.security_lower_tf(syminfo.tickerid, \"15\", barstate.islastconfirmedhistory ? 1.0 : 0.0), 1.0))\n\
             plot(array.indexof(request.security_lower_tf(syminfo.tickerid, \"15\", barstate.islast ? 1.0 : 0.0), 1.0))\n\
             plot(array.indexof(request.security_lower_tf(syminfo.tickerid, \"15\", barstate.isrealtime ? 1.0 : 0.0), 1.0))",
            HashMap::new(),
            SymbolInfo::default(),
            None,
            Limits::default(),
            &NoLibraries,
            Some(Box::new(Quarter)),
            None,
        )
        .expect("valid controller")
        .with_bar_seconds(3600.0);
        let bars: Vec<Kline> = (0..5)
            .map(|h| Kline {
                open_time: h * hour,
                close_time: h * hour + hour - 1,
                open: 1.0,
                high: 1.0,
                low: 1.0,
                close: 1.0,
                volume: 1.0,
            })
            .collect();
        let update = controller.update(&bars, None, 4 * hour + 40 * 60_000);
        let at = |plot: usize, bar: usize| update.output.plots[plot].values[bar];
        for plot in 0..3 {
            assert_eq!(at(plot, 3), Some(-1.0), "the last history bar, plot {plot}");
        }
        assert_eq!(at(0, 4), Some(1.0), "last confirmed history");
        assert_eq!(at(1, 4), Some(2.0), "last");
        assert_eq!(at(2, 4), Some(2.0), "realtime");
    }

    #[test]
    fn a_candle_still_forming_on_a_live_bar_is_not_confirmed() {
        // TradingView probe S4 (BTCUSDT 1h, 5m request, 13:07 in the bar opened at 13:00): the 13:00
        // candle read `54` (confirmed, last confirmed history), the forming 13:05 one `9` (last,
        // realtime, not confirmed). Here the 15m candles of the bar opened at 4:00, read at 4:40.
        let hour = 3_600_000;
        let mut controller = ExecutionController::new(
            "//@version=6\nindicator(\"T\")\n\
             plot(array.indexof(request.security_lower_tf(syminfo.tickerid, \"15\", barstate.isconfirmed ? 0.0 : 1.0), 1.0))\n\
             plot(array.size(request.security_lower_tf(syminfo.tickerid, \"15\", barstate.isconfirmed ? 0.0 : 1.0)))",
            HashMap::new(),
            SymbolInfo::default(),
            None,
            Limits::default(),
            &NoLibraries,
            Some(Box::new(Quarter)),
            None,
        )
        .expect("valid controller")
        .with_bar_seconds(3600.0);
        let bars: Vec<Kline> = (0..5)
            .map(|h| Kline {
                open_time: h * hour,
                close_time: h * hour + hour - 1,
                open: 1.0,
                high: 1.0,
                low: 1.0,
                close: 1.0,
                volume: 1.0,
            })
            .collect();
        let update = controller.update(&bars, None, 4 * hour + 40 * 60_000);
        let at = |plot: usize, bar: usize| update.output.plots[plot].values[bar];
        assert_eq!(
            at(0, 3),
            Some(-1.0),
            "every candle of a closed bar is confirmed"
        );
        assert_eq!(at(1, 4), Some(3.0), "4:00, 4:15 and the 4:30 one forming");
        assert_eq!(at(0, 4), Some(2.0), "only the forming one is unconfirmed");
    }

    #[test]
    fn a_finer_request_to_another_symbol_reads_the_intrabars_like_the_charts_own() {
        // TradingView probe S5 (a live BTCUSDT 1h bar at 13:18): ETHUSDT `"5"` reads the 13:15 candle
        // (`9`, forming) and, with `lookahead_on`, the 13:00 one, as the chart's own symbol does.
        // Here the 15m candles of "OTHER" under a 1h chart read at 4:40.
        let hour = 3_600_000;
        let mut controller = ExecutionController::new(
            "//@version=6\nindicator(\"T\")\n\
             plot(request.security(\"OTHER\", \"15\", time))\n\
             plot(request.security(\"OTHER\", \"15\", time, lookahead = barmerge.lookahead_on))\n\
             plot(request.security(\"OTHER\", \"15\", barstate.isconfirmed ? 1.0 : 0.0))",
            HashMap::new(),
            SymbolInfo::default(),
            None,
            Limits::default(),
            &NoLibraries,
            Some(Box::new(Quarter)),
            None,
        )
        .expect("valid controller")
        .with_bar_seconds(3600.0);
        let bars: Vec<Kline> = (0..5)
            .map(|h| Kline {
                open_time: h * hour,
                close_time: h * hour + hour - 1,
                open: 1.0,
                high: 1.0,
                low: 1.0,
                close: 1.0,
                volume: 1.0,
            })
            .collect();
        let update = controller.update(&bars, None, 4 * hour + 40 * 60_000);
        assert!(update.error.is_none(), "{:?}", update.error);
        let at = |plot: usize, bar: usize| update.output.plots[plot].values[bar];
        let quarter = 900_000.0;
        assert_eq!(
            at(0, 3),
            Some(3.0 * 3_600_000.0 + 3.0 * quarter),
            "history: the last"
        );
        assert_eq!(
            at(1, 3),
            Some(3.0 * 3_600_000.0),
            "history, lookahead_on: the first"
        );
        assert_eq!(
            at(0, 4),
            Some(4.0 * 3_600_000.0 + 2.0 * quarter),
            "live: the forming one"
        );
        assert_eq!(
            at(1, 4),
            Some(4.0 * 3_600_000.0),
            "live, lookahead_on: the first"
        );
        assert_eq!(at(2, 4), Some(0.0), "the forming candle is not confirmed");
    }

    /// Candles of the requested length from midnight to 12:00, for any symbol.
    struct Uniform;
    impl crate::pine::runtime::support::SecurityDataProvider for Uniform {
        fn candles(&self, key: &crate::pine::runtime::support::SecurityKey) -> Option<Vec<Kline>> {
            let step = (key.interval * 1000.0) as i64;
            Some(
                (0..43_200_000 / step)
                    .map(|i| Kline {
                        open_time: i * step,
                        close_time: i * step + step - 1,
                        open: 1.0,
                        high: 1.0,
                        low: 1.0,
                        close: 1.0,
                        volume: 1.0,
                    })
                    .collect(),
            )
        }
    }

    /// 15-minute candles up to 4:30 for the first ask, up to 4:45 for every later one: the provider
    /// of a live chart, whose newest candle opens between two asks.
    struct Growing(std::sync::atomic::AtomicUsize);
    impl crate::pine::runtime::support::SecurityDataProvider for Growing {
        fn candles(&self, key: &crate::pine::runtime::support::SecurityKey) -> Option<Vec<Kline>> {
            if key.interval != 900.0 {
                return None;
            }
            let asked = self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let count = if asked == 0 { 19 } else { 20 };
            Some(
                (0..count)
                    .map(|i| Kline {
                        open_time: i * 900_000,
                        close_time: i * 900_000 + 899_999,
                        open: 1.0,
                        high: 1.0,
                        low: 1.0,
                        close: 1.0,
                        volume: 1.0,
                    })
                    .collect(),
            )
        }
    }

    #[test]
    fn a_live_bars_intrabars_grow_as_candles_open_after_the_series_was_fetched() {
        // Real window, BTCUSDT 5m with a "1" request at 14:20: a bar that opened after the run began
        // read an empty list, and the live bar's list never grew. TradingView counts the candles as
        // they open.
        let hour = 3_600_000;
        let mut controller = ExecutionController::new(
            "//@version=6\nindicator(\"T\")\n\
             plot(array.size(request.security_lower_tf(syminfo.tickerid, \"15\", close)))",
            HashMap::new(),
            SymbolInfo::default(),
            None,
            Limits::default(),
            &NoLibraries,
            Some(Box::new(Growing(std::sync::atomic::AtomicUsize::new(0)))),
            None,
        )
        .expect("valid controller")
        .with_bar_seconds(3600.0);
        let mut bars: Vec<Kline> = (0..5)
            .map(|h| Kline {
                open_time: h * hour,
                close_time: h * hour + hour - 1,
                open: 1.0,
                high: 1.0,
                low: 1.0,
                close: 1.0,
                volume: 1.0,
            })
            .collect();
        let update = controller.update(&bars, None, 4 * hour + 40 * 60_000);
        assert_eq!(
            update.output.plots[0].values.last(),
            Some(&Some(3.0)),
            "4:00, 4:15 and 4:30 at the load"
        );
        // 4:50: the 4:45 candle has opened, the bar's price has moved, and the series is old enough.
        controller.session.security_live_refresh = std::time::Duration::ZERO;
        bars[4].close = 2.0;
        let update = controller.update(&bars, None, 4 * hour + 50 * 60_000);
        assert_eq!(
            update.output.plots[0].values.last(),
            Some(&Some(4.0)),
            "the candle that opened since is counted"
        );
    }

    #[test]
    fn gaps_on_gives_a_bar_its_value_where_the_candle_closes_or_with_lookahead_where_it_opens() {
        // TradingView probe S6b (BTCUSDT 1h live at 13:31, a 120 request, own symbol and ETHUSDT
        // alike): with `gaps_on` the value sits on the last chart bar of the candle, the live bar's
        // being the developing candle's; with `gaps_on` and `lookahead_on` on the first chart bar.
        let hour = 3_600_000;
        let bars: Vec<Kline> = (0..6)
            .map(|h| Kline {
                open_time: h * hour,
                close_time: h * hour + hour - 1,
                open: 1.0,
                high: 1.0,
                low: 1.0,
                close: 1.0,
                volume: 1.0,
            })
            .collect();
        for symbol in ["syminfo.tickerid", "\"OTHER\""] {
            let source = format!(
                "//@version=6\nindicator(\"T\")\n\
                 plot(request.security({symbol}, \"120\", time, gaps = barmerge.gaps_on))\n\
                 plot(request.security({symbol}, \"120\", time, gaps = barmerge.gaps_on, lookahead = barmerge.lookahead_on))"
            );
            let mut controller = ExecutionController::new(
                &source,
                HashMap::new(),
                SymbolInfo::default(),
                None,
                Limits::default(),
                &NoLibraries,
                Some(Box::new(Uniform)),
                None,
            )
            .expect("valid controller")
            .with_bar_seconds(3600.0);
            // 5:40: the bar opened at 5:00 is live and the last of the candle 4:00-6:00.
            let update = controller.update(&bars, None, 5 * hour + 40 * 60_000);
            assert!(update.error.is_none(), "{symbol}: {:?}", update.error);
            let plotted =
                |plot: usize| -> Vec<Option<f64>> { update.output.plots[plot].values.clone() };
            let two = 2.0 * hour as f64;
            assert_eq!(
                plotted(0),
                [None, Some(0.0), None, Some(two), None, Some(2.0 * two)],
                "{symbol}: the closing bar, the live one included"
            );
            assert_eq!(
                plotted(1),
                [Some(0.0), None, Some(two), None, Some(2.0 * two), None],
                "{symbol}, lookahead_on: the opening bar"
            );
        }
    }
}
