//! Autosave: the debounce that writes a named layout once it has sat unchanged.

use super::ChartWorkspace;
use crate::model::LayoutSnapshot;
use eframe::egui;
use std::time::{Duration, Instant};

/// How long the layout must sit unchanged before an autosave-enabled layout is written.
const AUTOSAVE_DELAY: Duration = Duration::from_secs(2);

/// What the debounce wants done this frame.
#[derive(Debug, Eq, PartialEq)]
enum AutosaveStep {
    Nothing,
    /// Check again after this long.
    Wait(Duration),
    Save,
}

/// The debounce behind autosave. `armed` is the last layout seen dirty; `due` is when it gets
/// written, or `None` once it has been tried, so an unchanged layout is never retried in a loop.
#[derive(Default)]
pub(super) struct AutosaveState {
    armed: Option<LayoutSnapshot>,
    due: Option<Instant>,
    /// The last autosave did not reach disk; the layout is still dirty.
    pub(super) failed: bool,
}

impl AutosaveState {
    /// Advances the debounce: an edit restarts the countdown, and a layout that has sat unchanged
    /// for [`AUTOSAVE_DELAY`] is due. Once tried, a layout is not retried until it changes again.
    fn step(
        &mut self,
        dirty: bool,
        enabled: bool,
        snapshot: impl FnOnce() -> LayoutSnapshot,
        now: Instant,
    ) -> AutosaveStep {
        if !dirty || !enabled {
            self.armed = None;
            self.due = None;
            return AutosaveStep::Nothing;
        }
        let snapshot = snapshot();
        if self.armed.as_ref() != Some(&snapshot) {
            self.armed = Some(snapshot);
            self.due = Some(now + AUTOSAVE_DELAY);
        }
        match self.due {
            None => AutosaveStep::Nothing,
            Some(due) if now < due => AutosaveStep::Wait(due - now),
            Some(_) => {
                self.due = None;
                AutosaveStep::Save
            }
        }
    }
}

impl ChartWorkspace {
    pub(super) fn autosave_enabled(&self) -> bool {
        self.active_view()
            .is_some_and(|view| view.autosave == Some(true))
    }

    pub(super) fn set_autosave(&mut self, enabled: bool) {
        match self.update_active_view(|view| view.autosave = enabled.then_some(true)) {
            Ok(()) => self.autosave = AutosaveState::default(),
            Err(error) => {
                self.layout_error = Some(format!("Couldn’t change Autosave: {error}"));
            }
        }
    }

    /// Runs once a frame: arms the debounce while an autosave layout has unsaved edits, and
    /// writes it once it has sat unchanged for [`AUTOSAVE_DELAY`].
    pub(super) fn tick_autosave(&mut self, ctx: &egui::Context, dirty: bool, enabled: bool) {
        let mut state = std::mem::take(&mut self.autosave);
        let step = state.step(dirty, enabled, || self.layout_snapshot(), Instant::now());
        self.autosave = state;
        match step {
            AutosaveStep::Nothing => {}
            AutosaveStep::Wait(remaining) => ctx.request_repaint_after(remaining),
            AutosaveStep::Save => self.autosave.failed = !self.save_active(false),
        }
    }

    /// Writes a pending autosave now, before the layout changes or the app quits. False means
    /// the layout is still dirty on disk.
    pub fn flush_autosave(&mut self) -> bool {
        self.autosave.due = None;
        let enabled = self.autosave_enabled();
        if !(self.is_dirty() || (enabled && self.drawings_dirty())) || !enabled {
            return true;
        }
        if self.save_active(true) {
            true
        } else {
            self.autosave.failed = true;
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::price_scale::PriceScaleMode;
    use crate::model::{DataSource, Timeframe};

    fn snapshot(logarithmic: bool) -> LayoutSnapshot {
        LayoutSnapshot {
            timeframe: Timeframe::Hour,
            configs: vec![crate::model::ChartConfig {
                id: 1,
                symbol: "BTCUSDT".into(),
                source: DataSource::Binance,
                scale_mode: PriceScaleMode::from_log_flag(logarithmic),
                appearance: crate::model::ChartAppearance::default(),
                card: None,
            }],
            columns: vec![vec![1]],
            sync: Default::default(),
        }
    }

    #[test]
    fn autosave_waits_for_the_layout_to_sit_still_then_saves_once() {
        let mut state = AutosaveState::default();
        let start = Instant::now();
        assert_eq!(
            state.step(true, true, || snapshot(false), start),
            AutosaveStep::Wait(AUTOSAVE_DELAY)
        );
        // An edit one second in restarts the countdown.
        let edited = start + Duration::from_secs(1);
        assert_eq!(
            state.step(true, true, || snapshot(true), edited),
            AutosaveStep::Wait(AUTOSAVE_DELAY)
        );
        // Unchanged since, the layout is due once the delay has passed.
        let due = edited + AUTOSAVE_DELAY;
        assert_eq!(
            state.step(
                true,
                true,
                || snapshot(true),
                due - Duration::from_millis(1)
            ),
            AutosaveStep::Wait(Duration::from_millis(1))
        );
        assert_eq!(
            state.step(true, true, || snapshot(true), due),
            AutosaveStep::Save
        );
        // Tried once: a failed write is not retried every frame, only after the next edit.
        assert_eq!(
            state.step(true, true, || snapshot(true), due + Duration::from_secs(5)),
            AutosaveStep::Nothing
        );
        assert_eq!(
            state.step(true, true, || snapshot(false), due + Duration::from_secs(6)),
            AutosaveStep::Wait(AUTOSAVE_DELAY)
        );
    }

    #[test]
    fn autosave_stays_idle_for_clean_layouts_and_layouts_with_it_off() {
        let mut state = AutosaveState::default();
        let now = Instant::now();
        assert_eq!(
            state.step(false, true, || snapshot(true), now),
            AutosaveStep::Nothing
        );
        assert_eq!(
            state.step(true, false, || snapshot(true), now),
            AutosaveStep::Nothing
        );
        assert!(state.armed.is_none() && state.due.is_none());
        // Going clean (saved by hand) drops a pending countdown.
        state.step(true, true, || snapshot(true), now);
        assert!(state.due.is_some());
        state.step(false, true, || snapshot(true), now);
        assert!(state.due.is_none());
    }
}
