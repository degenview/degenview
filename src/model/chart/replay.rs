use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum ReplayStatus {
    #[default]
    Inactive,
    Ready,
    SelectingStart,
    Paused,
    Playing,
    Completed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ReplayInterval {
    Automatic,
    OneMinute,
    FiveMinutes,
    FifteenMinutes,
    ThirtyMinutes,
    OneHour,
    OneDay,
    ChartBar,
}

impl ReplayInterval {
    pub fn label(self) -> &'static str {
        match self {
            Self::Automatic => "Auto",
            Self::OneMinute => "1m",
            Self::FiveMinutes => "5m",
            Self::FifteenMinutes => "15m",
            Self::ThirtyMinutes => "30m",
            Self::OneHour => "1h",
            Self::OneDay => "1D",
            Self::ChartBar => "1 bar",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ReplaySpeed {
    Half,
    Normal,
    Double,
    Five,
    Ten,
    Twenty,
    Fifty,
    Max,
}

impl ReplaySpeed {
    pub const ALL: [Self; 8] = [
        Self::Half,
        Self::Normal,
        Self::Double,
        Self::Five,
        Self::Ten,
        Self::Twenty,
        Self::Fifty,
        Self::Max,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Half => "0.5×",
            Self::Normal => "1×",
            Self::Double => "2×",
            Self::Five => "5×",
            Self::Ten => "10×",
            Self::Twenty => "20×",
            Self::Fifty => "50×",
            Self::Max => "Max",
        }
    }

    pub fn delay_ms(self) -> u64 {
        match self {
            Self::Half => 2_000,
            Self::Normal => 1_000,
            Self::Double => 500,
            Self::Five => 200,
            Self::Ten => 100,
            Self::Twenty => 50,
            Self::Fifty => 20,
            Self::Max => 1,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ReplaySession {
    pub status: ReplayStatus,
    pub symbol: String,
    pub start_ms: i64,
    pub current_ms: i64,
    pub bar_index: usize,
    pub interval: ReplayInterval,
    pub speed: ReplaySpeed,
}

/// What an agent may ask of a running replay clock. Starting one needs a chart's bars, so that is
/// the workspace's business.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockCommand {
    Play,
    Pause,
    StepForward(usize),
    StepBack(usize),
    /// Back to the chosen start.
    Restart,
    /// To the bar at or before this time (epoch milliseconds).
    Seek(i64),
    SetSpeed(ReplaySpeed),
    /// Back to live.
    Stop,
}

/// The replay clock as an agent may read it.
#[derive(Clone, Debug, PartialEq)]
pub struct ReplayView {
    pub status: ReplayStatus,
    pub symbol: Option<String>,
    pub start_ms: Option<i64>,
    pub current_ms: Option<i64>,
    pub bar_index: Option<usize>,
    pub bars_total: usize,
    pub speed: Option<ReplaySpeed>,
    /// The timeframe changed under the replay and its bars are being reloaded.
    pub preparing: bool,
}

/// One deterministic replay clock for a tab. Charts are gated by this single
/// timestamp rather than independent indices.
#[derive(Default)]
pub struct ReplayEngine {
    pub status: ReplayStatus,
    pub session: Option<ReplaySession>,
    pub timeline: Vec<i64>,
}

impl ReplayEngine {
    pub fn view(&self, preparing: bool) -> ReplayView {
        let session = self.session.as_ref();
        ReplayView {
            status: self.status,
            symbol: session.map(|session| session.symbol.clone()),
            start_ms: session.map(|session| session.start_ms),
            current_ms: session.map(|session| session.current_ms),
            bar_index: session.map(|session| session.bar_index),
            bars_total: self.timeline.len(),
            speed: session.map(|session| session.speed),
            preparing,
        }
    }

    /// Runs a clock command. `false` when there is no replay to act on.
    pub fn apply(&mut self, command: ClockCommand) -> bool {
        if self.session.is_none() {
            return false;
        }
        match command {
            ClockCommand::Play => self.play(),
            ClockCommand::Pause => self.pause(),
            ClockCommand::StepForward(steps) => {
                for _ in 0..steps {
                    if !self.step_forward() {
                        break;
                    }
                }
            }
            ClockCommand::StepBack(steps) => {
                for _ in 0..steps {
                    if !self.step_backward() {
                        break;
                    }
                }
            }
            ClockCommand::Restart => self.restart(),
            ClockCommand::Seek(time) => {
                let last = self.timeline.len().saturating_sub(1);
                if last == 0 {
                    self.seek(0.0);
                } else {
                    let index = self
                        .timeline
                        .partition_point(|candidate| *candidate <= time)
                        .saturating_sub(1);
                    self.seek(index as f64 / last as f64);
                }
            }
            ClockCommand::SetSpeed(speed) => self.set_speed(speed),
            ClockCommand::Stop => self.stop(),
        }
        true
    }

    pub fn is_bar_visible(&self) -> bool {
        self.status != ReplayStatus::Inactive
    }

    pub fn is_active(&self) -> bool {
        self.session.is_some()
            && !matches!(self.status, ReplayStatus::Inactive | ReplayStatus::Ready)
    }

    pub fn current_ms(&self) -> Option<i64> {
        self.session.as_ref().map(|session| session.current_ms)
    }

    pub fn open(&mut self) {
        if self.status == ReplayStatus::Inactive {
            self.status = ReplayStatus::Ready;
        }
    }

    pub fn begin_selecting(&mut self) {
        self.status = ReplayStatus::SelectingStart;
        if let Some(session) = &mut self.session {
            session.status = ReplayStatus::SelectingStart;
        }
    }

    pub fn start(&mut self, requested_ms: i64, symbol: &str, timeline: Vec<i64>) {
        let mut valid = timeline;
        valid.sort_unstable();
        valid.dedup();
        if valid.is_empty() {
            return;
        }
        let index = Self::index_at_or_before(requested_ms, &valid).unwrap_or(0);
        let (interval, speed) = self
            .session
            .as_ref()
            .map_or((ReplayInterval::Automatic, ReplaySpeed::Normal), |s| {
                (s.interval, s.speed)
            });
        self.timeline = valid;
        self.session = Some(ReplaySession {
            status: ReplayStatus::Paused,
            symbol: symbol.to_owned(),
            start_ms: requested_ms,
            current_ms: self.timeline[index],
            bar_index: index,
            interval,
            speed,
        });
        self.status = ReplayStatus::Paused;
    }

    pub fn play(&mut self) {
        if self.status == ReplayStatus::Paused {
            self.status = ReplayStatus::Playing;
            if let Some(session) = &mut self.session {
                session.status = ReplayStatus::Playing;
            }
        }
    }

    pub fn pause(&mut self) {
        if self.status == ReplayStatus::Playing {
            self.status = ReplayStatus::Paused;
            if let Some(session) = &mut self.session {
                session.status = ReplayStatus::Paused;
            }
        }
    }

    pub fn step_forward(&mut self) -> bool {
        let Some(session) = &mut self.session else {
            return false;
        };
        if self.timeline.is_empty() {
            return false;
        }
        let next = session.bar_index + 1;
        if next >= self.timeline.len() {
            session.status = ReplayStatus::Completed;
            self.status = ReplayStatus::Completed;
            return false;
        }
        session.bar_index = next;
        session.current_ms = self.timeline[next];
        true
    }

    pub fn step_backward(&mut self) -> bool {
        let Some(session) = &mut self.session else {
            return false;
        };
        if session.bar_index == 0 {
            return false;
        }
        session.bar_index -= 1;
        session.current_ms = self.timeline[session.bar_index];
        session.status = ReplayStatus::Paused;
        self.status = ReplayStatus::Paused;
        true
    }

    pub fn seek(&mut self, fraction: f64) {
        if self.session.is_none() || self.timeline.is_empty() {
            return;
        }
        let index =
            ((fraction.clamp(0.0, 1.0)) * (self.timeline.len() - 1) as f64).round() as usize;
        self.seek_to_index(index);
    }

    pub fn set_speed(&mut self, speed: ReplaySpeed) {
        if let Some(session) = &mut self.session {
            session.speed = speed;
        }
    }

    pub fn set_interval(&mut self, interval: ReplayInterval) {
        if let Some(session) = &mut self.session {
            session.interval = interval;
        }
    }

    pub fn stop(&mut self) {
        self.session = None;
        self.timeline.clear();
        self.status = ReplayStatus::Inactive;
    }

    /// Carries the session over to `timeline`, the bars of another timeframe: the cursor keeps
    /// its time (snapped to a bar that exists now), the start keeps its own, and playing or
    /// picking a start carries on. Returns false when the new bars begin after the cursor, which
    /// then sits on their first bar instead.
    pub fn rebase(&mut self, timeline: Vec<i64>) -> bool {
        let Some(session) = &self.session else {
            return true;
        };
        let (cursor, before) = (session.current_ms, self.status);
        let mut valid = timeline;
        valid.sort_unstable();
        valid.dedup();
        if valid.is_empty() {
            return true;
        }
        let covered = valid[0] <= cursor;
        let index = Self::index_at_or_before(cursor, &valid).unwrap_or(0);
        self.timeline = valid;
        self.seek_to_index(index);
        if self.status == ReplayStatus::Paused
            && matches!(before, ReplayStatus::Playing | ReplayStatus::SelectingStart)
        {
            self.status = before;
            if let Some(session) = &mut self.session {
                session.status = before;
            }
        }
        covered
    }

    /// Cancel a start selection: back to Paused if a session exists, else Ready.
    pub fn cancel_selection(&mut self) {
        if self.session.is_some() {
            self.status = ReplayStatus::Paused;
            if let Some(session) = &mut self.session {
                session.status = ReplayStatus::Paused;
            }
        } else {
            self.status = ReplayStatus::Ready;
        }
    }

    /// Seek back to the chosen start point (not the first bar).
    pub fn restart(&mut self) {
        let start = self.session.as_ref().map(|session| session.start_ms);
        let Some(start) = start else {
            return;
        };
        let Some(index) = Self::index_at_or_before(start, &self.timeline) else {
            return;
        };
        self.seek_to_index(index);
    }

    fn seek_to_index(&mut self, index: usize) {
        let Some(session) = &mut self.session else {
            return;
        };
        if self.timeline.is_empty() {
            return;
        }
        let index = index.min(self.timeline.len() - 1);
        session.bar_index = index;
        session.current_ms = self.timeline[index];
        session.status = if index == self.timeline.len() - 1 {
            ReplayStatus::Completed
        } else {
            ReplayStatus::Paused
        };
        self.status = session.status;
    }

    fn index_at_or_before(date: i64, dates: &[i64]) -> Option<usize> {
        dates
            .partition_point(|candidate| *candidate <= date)
            .checked_sub(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timeline() -> Vec<i64> {
        (0..5).map(|i| 1_000 + i * 100).collect()
    }

    #[test]
    fn choosing_an_initial_start_does_not_mark_the_chart_as_replaying() {
        let mut engine = ReplayEngine::default();
        engine.open();
        engine.begin_selecting();
        assert!(!engine.is_active());
        engine.start(1_000, "BTCUSDT", timeline());
        assert!(engine.is_active());
    }

    #[test]
    fn start_seeks_to_first_bar_at_or_before_request() {
        let mut engine = ReplayEngine::default();
        engine.start(1_150, "BTCUSDT", timeline());
        assert_eq!(engine.status, ReplayStatus::Paused);
        assert_eq!(engine.session.as_ref().unwrap().bar_index, 1);
        assert_eq!(engine.current_ms(), Some(1_100));
    }

    #[test]
    fn steps_forward_and_completes_at_end() {
        let mut engine = ReplayEngine::default();
        engine.start(1_000, "BTCUSDT", timeline());
        for _ in 0..4 {
            assert!(engine.step_forward());
        }
        assert!(!engine.step_forward());
        assert_eq!(engine.status, ReplayStatus::Completed);
    }

    #[test]
    fn steps_backward_and_seek() {
        let mut engine = ReplayEngine::default();
        engine.start(1_000, "BTCUSDT", timeline());
        engine.seek(1.0);
        assert_eq!(engine.session.as_ref().unwrap().bar_index, 4);
        assert!(engine.step_backward());
        assert_eq!(engine.session.as_ref().unwrap().bar_index, 3);
    }

    #[test]
    fn rebase_keeps_the_cursor_time_on_the_new_bars() {
        let mut engine = ReplayEngine::default();
        engine.start(1_100, "X", timeline());
        engine.seek(0.5); // 1_200
        engine.play();
        // Finer bars: the cursor snaps to the last one at or before 1_200.
        let covered = engine.rebase((0..20).map(|i| 1_000 + i * 50).collect());
        assert!(covered);
        assert_eq!(engine.current_ms(), Some(1_200));
        assert_eq!(engine.status, ReplayStatus::Playing);
        assert_eq!(engine.session.as_ref().map(|s| s.start_ms), Some(1_100));
    }

    #[test]
    fn rebase_onto_bars_that_start_later_lands_on_the_first_one() {
        let mut engine = ReplayEngine::default();
        engine.start(1_000, "X", timeline());
        assert!(!engine.rebase(vec![5_000, 5_100, 5_200]));
        assert_eq!(engine.current_ms(), Some(5_000));
        assert_eq!(engine.status, ReplayStatus::Paused);
    }

    #[test]
    fn rebase_leaves_a_completed_replay_playable_when_there_are_more_bars() {
        let mut engine = ReplayEngine::default();
        engine.start(1_000, "X", timeline());
        engine.seek(1.0);
        assert_eq!(engine.status, ReplayStatus::Completed);
        assert!(engine.rebase((0..20).map(|i| 1_000 + i * 50).collect()));
        assert_eq!(engine.status, ReplayStatus::Paused);
        assert!(engine.rebase(vec![]));
    }
}
