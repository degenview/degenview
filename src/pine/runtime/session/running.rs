//! Running bars: a history replay, one bar or tick at a time, a script that fails on one, and
//! the output the run leaves.

use super::{Halted, Hooks, RunError, Session};
use crate::model::Kline;
use crate::pine::compiler::program::{BarEvent, BarPhase, ScriptType, VisualOutput};
use crate::pine::runtime::fast_hash::FastMap;
use crate::pine::runtime::state::{AlertEvent, RuntimeState};
#[cfg(test)]
use crate::pine::runtime::support::RuntimeResult;
use crate::pine::runtime::types::{BarFlags, RuntimeContext};
use crate::pine::{Diagnostic, DiagnosticCategory};

impl Session {
    /// Runs `bars` as history and returns the output. Tests read a whole run through this; the
    /// app loads history and reads the output separately.
    #[cfg(test)]
    pub fn evaluate(&mut self, bars: &[Kline]) -> Result<RuntimeResult, RunError> {
        let states = self.load_history(bars, false, None)?;
        Ok(RuntimeResult {
            output: self.output(),
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
        self.bulk_replay = true;
        let mut failed_bar = 0;
        let result = self.replay_history(bars, bars.len(), precedes_live_bar, &mut failed_bar);
        self.bulk_replay = false;
        if matches!(result, Err(RunError::Script(_))) && self.halted.is_some() {
            self.settle_failed_replay(bars, failed_bar, precedes_live_bar);
        }
        self.deadline_at = None;
        result
    }

    /// Runs the first `upto` of `bars` as history. `failed_bar` ends up as the index of the bar that
    /// failed, or of the last one run.
    fn replay_history(
        &mut self,
        bars: &[Kline],
        upto: usize,
        precedes_live_bar: bool,
        failed_bar: &mut usize,
    ) -> Result<Vec<BarFlags>, RunError> {
        let mut states = Vec::with_capacity(upto);
        for (index, bar) in bars[..upto].iter().enumerate() {
            *failed_bar = index;
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
    }

    /// A history replay failed on `failed_bar`, which left `working` half run: its labels, drawings,
    /// alerts and fills are in it, and the state before the bar was never kept (copying it before
    /// every bar would slow every run). Run the bars before it again to get that state back. Nothing
    /// watches this second run: no breakpoint stops in it and the logs, which already hold what the
    /// failing bar wrote, are left alone.
    fn settle_failed_replay(&mut self, bars: &[Kline], failed_bar: usize, precedes_live_bar: bool) {
        let Some(halted) = self.halted.take() else {
            return;
        };
        let hooks = self.hooks.take();
        let logs = std::mem::take(&mut self.logs);
        self.restart_state();
        self.start_deadline();
        self.bulk_replay = true;
        let mut ignored = 0;
        // Should this run fail or be cancelled, the state is what it reached; the error to report
        // is the first run's either way.
        let _ = self.replay_history(bars, failed_bar, precedes_live_bar, &mut ignored);
        self.bulk_replay = false;
        self.hooks = hooks;
        self.logs = logs;
        self.halted = Some(Halted {
            error: halted.error,
            completed_bars: failed_bar,
        });
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
        // A script that failed stays stopped: later bars do not run it (TradingView halts it too).
        if let Some(halted) = &self.halted {
            return Err(RunError::Script(halted.error.clone()));
        }
        let owns_deadline = self.deadline_at.is_none();
        if owns_deadline {
            self.start_deadline();
        }
        let hooked = self.hooks.is_some();
        let realtime = event.phase.is_realtime();
        if hooked {
            // A bar that is cancelled or out of time before it starts is not a measured bar:
            // `execute_bar` would fail on its first line, after the recorder had opened it.
            if let Err(error) = self.check_interruption() {
                if owns_deadline {
                    self.deadline_at = None;
                }
                return Err(error);
            }
            self.hook_begin_bar();
        }
        let result = self.execute_bar(event, is_last, is_last_history);
        if hooked {
            let outcome = match &result {
                Ok(_) => crate::pine::profile::BarOutcome::Completed,
                Err(RunError::Rejection) => crate::pine::profile::BarOutcome::Skipped,
                Err(_) => crate::pine::profile::BarOutcome::Cut,
            };
            self.hook_end_bar(outcome, realtime);
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
            return Err(RunError::Rejection);
        }
        if let Some(previous) = self.last_open_time
            && open_time < previous
        {
            return Err(RunError::Rejection);
        }
        let is_new = Some(open_time) != self.last_open_time;
        if is_new {
            self.intrabar.clear();
            self.intrabar_objects.clear();
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
        // A bar run on its own (not inside `load_history`) that continues from the replayed state would
        // leave that state half run if it failed; keep it to go back to. A replay settles itself.
        let before_bar = (replaying && self.working_is_committed && !self.bulk_replay)
            .then(|| self.working.clone());
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
        if let Some(Hooks::Debug(debug)) = self.hooks.as_deref_mut() {
            debug.begin_bar(chart_bar_index);
        }
        if !is_new {
            for (key, value) in self.intrabar.clone() {
                self.working.variables.insert(key, value);
            }
            self.restore_intrabar_objects();
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
            if error.category == DiagnosticCategory::Cancellation {
                // A run somebody stopped is not a failure of the script. Its state is a partial bar,
                // so a replay starts over; anything else just drops it with the next tick.
                if replaying {
                    self.committed = self.fresh_state();
                    self.working = self.committed.clone();
                    self.working_is_committed = false;
                }
            } else {
                self.halt(&error, before_bar);
            }
            return Err(RunError::Script(error));
        }
        if self.is_strategy() {
            self.finish_strategy_bar(candle.clone());
        }
        if !confirmed {
            self.capture_intrabar_objects();
        }
        if confirmed {
            let builtins = self.read_history_builtins(&mut context);
            self.commit_histories(candle, true, builtins);
            if replaying {
                self.working_is_committed = true;
            } else {
                self.committed = self.working.clone();
                self.working_is_committed = false;
            }
            self.intrabar.clear();
            self.intrabar_objects.clear();
            self.last_committed_open_time = Some(open_time);
        }
        self.last_open_time = Some(open_time);
        Ok(flags)
    }

    /// Stops the script on `error`. The output keeps the bars before the failing one, in every
    /// category: `before_bar` is the state a bar run on its own continued from, `committed` is the
    /// state before any other, and a bar of a history replay (which has neither) is settled by
    /// `settle_failed_replay` once the replay is over.
    fn halt(&mut self, error: &Diagnostic, before_bar: Option<RuntimeState>) {
        let completed_bars = self.working.bar_index.max(0) as usize;
        if let Some(state) = before_bar {
            self.working = state;
        } else if !self.working_is_committed {
            self.working = self.committed.clone();
        }
        self.halted = Some(Halted {
            error: error.clone(),
            completed_bars,
        });
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
        let candle_for_risk = candle.clone();
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
        let bar_index = self.working.bar_index as usize;
        self.working.broker.record_equity(close);
        self.working
            .broker
            .check_risk(&candle_for_risk, bar_index, self.mintick);
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
        // A halted script shows the bars before the one it failed on.
        let bar_count = match &self.halted {
            Some(halted) => halted.completed_bars.min((w.bar_index + 1) as usize),
            None => (w.bar_index + 1) as usize,
        };
        VisualOutput {
            overlay: self.program.declaration.overlay,
            bar_count,
            plots: sorted_by_id(&w.plots)
                .into_iter()
                .map(|mut plot| {
                    if self.halted.is_some() {
                        plot.values.truncate(bar_count);
                        plot.colors.truncate(bar_count);
                    }
                    plot.shifted()
                })
                .collect(),
            hlines: sorted_by_id(&w.hlines),
            markers: sorted_by_id(&w.markers)
                .into_iter()
                .map(crate::pine::outputs::MarkerOutput::finished)
                .collect(),
            arrows: sorted_by_id(&w.arrows)
                .into_iter()
                .map(crate::pine::outputs::ArrowOutput::finished)
                .collect(),
            backgrounds: sorted_by_id(&w.backgrounds)
                .into_iter()
                .map(crate::pine::outputs::ColorOutput::finished)
                .collect(),
            bar_colors: sorted_by_id(&w.bar_colors)
                .into_iter()
                .map(crate::pine::outputs::ColorOutput::finished)
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
                .map(crate::pine::outputs::CandleOutput::finished)
                .collect(),
            alerts: w.alerts.clone(),
            strategy: if self.is_strategy() {
                Some(crate::pine::strategy::StrategyReport {
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
}

fn sorted_by_id<T: Clone>(map: &FastMap<usize, T>) -> Vec<T> {
    let mut entries: Vec<_> = map.iter().collect();
    entries.sort_by_key(|(id, _)| **id);
    entries.into_iter().map(|(_, v)| v.clone()).collect()
}
