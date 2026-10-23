//! Replay: the bar, its timeline, and starting, stepping and stopping it.

use super::ChartWorkspace;
use crate::model::{ReplayInterval, ReplayStatus};
use crate::ui::replay_bar::{self, ReplayBarAction, ReplayBarInput};
use eframe::egui;
use std::time::{Duration, Instant};

impl ChartWorkspace {
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

    pub(super) fn tick_replay(&mut self) {
        if self.replay.status != ReplayStatus::Playing {
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
            is_preparing: false,
            notice: None,
            resolved_interval: None,
            available_intervals: &available,
        };
        let actions = replay_bar::show(ui, &input);
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
