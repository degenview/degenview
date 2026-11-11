//! Replay: the bar, its timeline, and starting, stepping and stopping it.

use super::ChartWorkspace;
use crate::model::{ClockCommand, ReplayInterval, ReplaySpeed, ReplayStatus, ReplayView};
use crate::ui::replay::bar::{self, ReplayBarAction, ReplayBarInput};
use eframe::egui;
use std::time::{Duration, Instant};

/// Where an agent starts a replay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplayStart {
    /// The chart's first bar.
    First,
    /// The bar at or before this time (epoch milliseconds); earlier than the first bar is the
    /// first bar.
    At(i64),
}

/// What an agent may ask of the replay clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplayCommand {
    /// Starts (or restarts) the replay on a chart, the focused one when none is named. Paused.
    Start {
        chart_id: Option<u64>,
        at: ReplayStart,
    },
    Play,
    Pause,
    StepForward(usize),
    StepBack(usize),
    /// Back to the chosen start.
    Restart,
    /// To the bar at or before this time.
    Seek(i64),
    SetSpeed(ReplaySpeed),
    /// Back to live.
    Stop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplayError {
    NoSuchChart,
    /// The chart has no candles yet (it may still be loading).
    NoCandles,
    /// There is no replay to act on.
    NotRunning,
}

impl ChartWorkspace {
    pub fn replay_view(&self) -> ReplayView {
        self.replay.view(self.replay_rebase)
    }

    /// Runs a command of the replay clock for an agent, through the engine the replay bar drives.
    pub fn replay_command(&mut self, command: ReplayCommand) -> Result<ReplayView, ReplayError> {
        if let ReplayCommand::Start { chart_id, at } = command {
            let mut previous_focus = None;
            if let Some(chart_id) = chart_id {
                if !self
                    .charts
                    .iter()
                    .any(|chart| chart.id == chart_id && chart.is_market())
                {
                    return Err(ReplayError::NoSuchChart);
                }
                previous_focus = Some(self.focused);
                self.focus_chart(chart_id);
            }
            let (symbol, timeline) = self.focused_chart_data();
            let Some(first) = timeline.first().copied() else {
                // A refused start leaves the user's chart where it was.
                if let Some(previous) = previous_focus {
                    self.focused = previous;
                }
                return Err(ReplayError::NoCandles);
            };
            let requested = match at {
                ReplayStart::First => first,
                ReplayStart::At(time) => time,
            };
            self.replay.start(requested, &symbol, timeline);
            self.replay_rebase = false;
            self.replay_last_step = Instant::now();
            return Ok(self.replay_view());
        }
        let clock = match command {
            ReplayCommand::Start { .. } => unreachable!("handled above"),
            ReplayCommand::Play => ClockCommand::Play,
            ReplayCommand::Pause => ClockCommand::Pause,
            ReplayCommand::StepForward(steps) => ClockCommand::StepForward(steps),
            ReplayCommand::StepBack(steps) => ClockCommand::StepBack(steps),
            ReplayCommand::Restart => ClockCommand::Restart,
            ReplayCommand::Seek(time) => ClockCommand::Seek(time),
            ReplayCommand::SetSpeed(speed) => ClockCommand::SetSpeed(speed),
            ReplayCommand::Stop => ClockCommand::Stop,
        };
        if !self.replay.apply(clock) {
            return Err(ReplayError::NotRunning);
        }
        if clock == ClockCommand::Play {
            self.replay_last_step = Instant::now();
        }
        Ok(self.replay_view())
    }

    /// Timeline the replay bar builds from the focused chart's candle opens.
    #[allow(dead_code)] // consumed by the replay bar UI (second agent)
    pub(super) fn replay_timeline(&self) -> Vec<i64> {
        self.resolved_focused_id()
            .and_then(|id| self.charts.iter().find(|chart| chart.id == id))
            .map(|chart| chart.candles.iter().map(|kline| kline.open_time).collect())
            .unwrap_or_default()
    }

    pub(super) fn toggle_replay(&mut self) {
        if self.replay.is_bar_visible() {
            self.replay.stop();
        } else {
            self.replay.open();
        }
    }

    /// After a timeframe change: once the focused chart has its new bars, the replay moves onto
    /// them at the same time (candles cut at the old timeframe's cursor would otherwise hide a
    /// chart whose bars begin later).
    fn finish_replay_rebase(&mut self) {
        let Some(chart) = self
            .resolved_focused_id()
            .and_then(|id| self.charts.iter().find(|chart| chart.id == id))
        else {
            self.replay_rebase = false;
            return;
        };
        if chart.loading {
            return;
        }
        let timeline: Vec<i64> = chart.candles.iter().map(|kline| kline.open_time).collect();
        self.replay_rebase = false;
        if timeline.is_empty() {
            self.replay.stop();
            self.say(
                format!(
                    "Replay stopped: no {} history to replay.",
                    self.timeframe.label()
                ),
                true,
            );
        } else if !self.replay.rebase(timeline) {
            self.say(
                format!(
                    "{} history starts after the replay position, so the replay moved to its first bar.",
                    self.timeframe.label()
                ),
                false,
            );
        }
        self.replay_last_step = Instant::now();
    }

    pub(super) fn tick_replay(&mut self) {
        if self.replay_rebase {
            if self.replay.session.is_none() {
                self.replay_rebase = false;
            } else {
                self.finish_replay_rebase();
            }
        }
        if self.replay_rebase || self.replay.status != ReplayStatus::Playing {
            return;
        }
        let delay = self
            .replay
            .session
            .as_ref()
            .map_or(1_000, |session| session.speed.delay_ms());
        if self.replay_last_step.elapsed() >= Duration::from_millis(delay) {
            self.replay_last_step = Instant::now();
            self.replay.step_forward();
        }
    }

    pub(super) fn focused_chart_data(&self) -> (String, Vec<i64>) {
        self.resolved_focused_id()
            .and_then(|id| self.charts.iter().find(|chart| chart.id == id))
            .map(|chart| {
                (
                    chart.symbol.clone(),
                    chart.candles.iter().map(|kline| kline.open_time).collect(),
                )
            })
            .unwrap_or_default()
    }

    fn start_replay_first(&mut self) {
        let (symbol, timeline) = self.focused_chart_data();
        if let Some(first) = timeline.first().copied() {
            self.replay.start(first, &symbol, timeline);
        }
    }

    fn start_replay_random(&mut self) {
        let (symbol, timeline) = self.focused_chart_data();
        if timeline.is_empty() {
            return;
        }
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as usize)
            .unwrap_or(0);
        // Leave the final fifth available to play through, as the Swift replay picker does.
        let eligible = ((timeline.len() as f64) * 0.8).ceil().max(1.0) as usize;
        let index = nanos % eligible;
        let picked = timeline[index];
        self.replay.start(picked, &symbol, timeline);
    }

    pub(super) fn show_replay_bar(&mut self, ui: &mut egui::Ui) {
        let available = [ReplayInterval::Automatic, ReplayInterval::ChartBar];
        let input = ReplayBarInput {
            engine: &self.replay,
            is_preparing: self.replay_rebase,
            notice: None,
            resolved_interval: None,
            available_intervals: &available,
        };
        let actions = bar::show(ui, &input);
        self.apply_replay_actions(actions);
    }

    fn apply_replay_actions(&mut self, actions: Vec<ReplayBarAction>) {
        for action in actions {
            match action {
                ReplayBarAction::SelectOnChart => self.replay.begin_selecting(),
                ReplayBarAction::CancelSelection => self.replay.cancel_selection(),
                ReplayBarAction::ReturnToLive => self.replay.stop(),
                ReplayBarAction::ChangeStart => self.replay.begin_selecting(),
                ReplayBarAction::Restart => self.replay.restart(),
                ReplayBarAction::StepBack => {
                    self.replay.step_backward();
                }
                ReplayBarAction::StepForward => {
                    self.replay.step_forward();
                }
                ReplayBarAction::TogglePlayback => {
                    if self.replay.status == ReplayStatus::Playing {
                        self.replay.pause();
                    } else {
                        self.replay.play();
                    }
                }
                ReplayBarAction::Seek(fraction) => self.replay.seek(fraction),
                ReplayBarAction::SetSpeed(speed) => self.replay.set_speed(speed),
                ReplayBarAction::SetInterval(interval) => self.replay.set_interval(interval),
                ReplayBarAction::FirstBar => self.start_replay_first(),
                ReplayBarAction::RandomBar => self.start_replay_random(),
                ReplayBarAction::ChooseDate => {
                    let timeline = self.replay_timeline();
                    self.date_sheet.open(&timeline);
                }
                ReplayBarAction::DismissNotice => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DataSource, InstrumentId};
    use crate::workspace::test_support::{candle, empty_workspace};

    fn rig() -> (ChartWorkspace, u64) {
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(9);
        let chart_id = workspace
            .add_chart_for(&InstrumentId::new(DataSource::Binance, "BTCUSDT"), &ctx)
            .chart_id;
        workspace
            .charts
            .iter_mut()
            .find(|chart| chart.id == chart_id)
            .unwrap()
            .candles = (0..10)
            .map(|n| candle(1_000 + n * 100, 100.0 + n as f64))
            .collect();
        (workspace, chart_id)
    }

    #[test]
    fn an_agent_starts_steps_seeks_and_stops_the_replay() {
        let (mut workspace, chart_id) = rig();
        assert_eq!(workspace.replay_view().status, ReplayStatus::Inactive);
        assert_eq!(
            workspace.replay_command(ReplayCommand::Play),
            Err(ReplayError::NotRunning)
        );

        let view = workspace
            .replay_command(ReplayCommand::Start {
                chart_id: Some(chart_id),
                at: ReplayStart::At(1_250),
            })
            .unwrap();
        assert_eq!(view.status, ReplayStatus::Paused);
        assert_eq!(view.bar_index, Some(2), "the bar at or before the time");
        assert_eq!(view.current_ms, Some(1_200));
        assert_eq!(view.bars_total, 10);
        assert_eq!(view.symbol.as_deref(), Some("BTCUSDT"));

        let view = workspace
            .replay_command(ReplayCommand::StepForward(3))
            .unwrap();
        assert_eq!(view.bar_index, Some(5));
        let view = workspace
            .replay_command(ReplayCommand::StepBack(2))
            .unwrap();
        assert_eq!(view.bar_index, Some(3));
        // Stepping stops at the end rather than running past it.
        let view = workspace
            .replay_command(ReplayCommand::StepForward(500))
            .unwrap();
        assert_eq!(view.bar_index, Some(9));
        assert_eq!(view.status, ReplayStatus::Completed);

        let view = workspace
            .replay_command(ReplayCommand::Seek(1_410))
            .unwrap();
        assert_eq!(view.bar_index, Some(4));
        assert_eq!(view.status, ReplayStatus::Paused);
        let view = workspace.replay_command(ReplayCommand::Restart).unwrap();
        assert_eq!(
            view.bar_index,
            Some(2),
            "back to the chosen start, not the first bar"
        );

        let view = workspace
            .replay_command(ReplayCommand::SetSpeed(ReplaySpeed::Ten))
            .unwrap();
        assert_eq!(view.speed, Some(ReplaySpeed::Ten));
        let view = workspace.replay_command(ReplayCommand::Play).unwrap();
        assert_eq!(view.status, ReplayStatus::Playing);
        let view = workspace.replay_command(ReplayCommand::Pause).unwrap();
        assert_eq!(view.status, ReplayStatus::Paused);

        let view = workspace.replay_command(ReplayCommand::Stop).unwrap();
        assert_eq!(view.status, ReplayStatus::Inactive);
        assert_eq!(view.current_ms, None);
    }

    #[test]
    fn a_replay_needs_a_real_chart_with_candles_and_cuts_what_the_tools_read() {
        let (mut workspace, chart_id) = rig();
        assert_eq!(
            workspace.replay_command(ReplayCommand::Start {
                chart_id: Some(999),
                at: ReplayStart::First
            }),
            Err(ReplayError::NoSuchChart)
        );
        workspace
            .charts
            .iter_mut()
            .find(|chart| chart.id == chart_id)
            .unwrap()
            .candles
            .clear();
        assert_eq!(
            workspace.replay_command(ReplayCommand::Start {
                chart_id: None,
                at: ReplayStart::First
            }),
            Err(ReplayError::NoCandles)
        );
        assert_eq!(workspace.replay_view().status, ReplayStatus::Inactive);

        let (mut workspace, chart_id) = rig();
        workspace
            .replay_command(ReplayCommand::Start {
                chart_id: None,
                at: ReplayStart::At(1_300),
            })
            .unwrap();
        // The candle tools read what the replay shows: nothing after its position.
        let data = workspace.chart_data(chart_id).unwrap();
        assert_eq!(data.replay_ms, Some(1_300));
        assert_eq!(data.candles.last().map(|bar| bar.open_time), Some(1_300));
    }

    #[test]
    fn a_refused_start_leaves_the_focused_chart_alone() {
        let ctx = egui::Context::default();
        let (mut workspace, first) = rig();
        let second = workspace
            .add_chart_for(&InstrumentId::new(DataSource::Binance, "ETHUSDT"), &ctx)
            .chart_id;
        workspace.focus_chart(first);
        assert_eq!(workspace.focused, Some(first));
        // The second chart has no candles yet (it is still loading).
        assert_eq!(
            workspace.replay_command(ReplayCommand::Start {
                chart_id: Some(second),
                at: ReplayStart::First
            }),
            Err(ReplayError::NoCandles)
        );
        assert_eq!(workspace.focused, Some(first), "focus is not moved");
        assert_eq!(workspace.replay_view().status, ReplayStatus::Inactive);
        // A start that works does move it.
        workspace
            .charts
            .iter_mut()
            .find(|chart| chart.id == second)
            .unwrap()
            .candles = (0..5).map(|n| candle(1_000 + n * 100, 10.0)).collect();
        workspace
            .replay_command(ReplayCommand::Start {
                chart_id: Some(second),
                at: ReplayStart::First,
            })
            .unwrap();
        assert_eq!(workspace.focused, Some(second));
    }
}
