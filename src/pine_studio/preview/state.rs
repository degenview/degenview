//! Loading the preview's saved settings, the bar and range picks the debugger and the Logs tab
//! ask for, what a run can see (its identity and the debugger's feed), the frame's tick and the
//! status text.

use super::{
    APPLY_DELAY, CANDLE_KEY, MARKET_KEY, PreviewMarket, PreviewState, RECENTS_KEY, RunIdentity,
    TIMEFRAME_KEY, YZOOM_KEY, inputs_fingerprint, inputs_path,
};
use crate::model::pine::script::ScriptType;
use crate::model::{Kline, Timeframe};
use crate::pine::compiler::program::InputSchema;
use crate::pine_studio::inputs::{InputValues, InputsStore};
use crate::store::SharedStore;
use crate::ui::{ChartCard, PREVIEW_INSTANCE};
use eframe::egui;
use serde::Serialize;
use std::time::{Duration, Instant};

#[cfg(test)]
use crate::model::DataSource;
#[cfg(test)]
use crate::pine::compiler::program::InputValue;

impl PreviewState {
    pub fn load(shared: &SharedStore) -> Self {
        let read = |f: &dyn Fn(&crate::store::Store) -> Option<PreviewMarket>| {
            shared.lock().ok().and_then(|store| f(&store))
        };
        let market = read(&|s| s.setting::<PreviewMarket>(MARKET_KEY))
            .filter(|m| PreviewMarket::is_supported(m.source))
            .unwrap_or_else(PreviewMarket::default_market);
        let (recents, timeframe, candles, y_zoom) = shared
            .lock()
            .map(|store| {
                (
                    store
                        .setting::<Vec<PreviewMarket>>(RECENTS_KEY)
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|m| PreviewMarket::is_supported(m.source))
                        .collect::<Vec<_>>(),
                    store
                        .setting::<Timeframe>(TIMEFRAME_KEY)
                        .unwrap_or(Timeframe::Day),
                    store.setting::<usize>(CANDLE_KEY),
                    store.setting::<f64>(YZOOM_KEY),
                )
            })
            .unwrap_or_else(|_| (Vec::new(), Timeframe::Day, None, None));
        let mut card = ChartCard::default();
        // A script preview has no price-scale menu: its scale is always plain.
        card.set_scale_menu(false);
        if let Some(count) = candles {
            card.set_visible_candles(count);
        }
        if let Some(zoom) = y_zoom.filter(|z| z.is_finite()) {
            card.set_y_zoom(zoom.clamp(0.25, 20.0));
        }
        let saved_zoom = (card.visible_candles(), card.y_zoom());
        Self {
            shared: Some(shared.clone()),
            market,
            recents,
            timeframe,
            candles: Vec::new(),
            candles_fingerprint: crate::host::pine::chart::candles_fingerprint(&[]),
            loading: false,
            error: None,
            inbox: None,
            last_fetch: None,
            fetched_for: None,
            card,
            script_id: None,
            script_type: None,
            latest_source: None,
            changed_at: None,
            applied_source: None,
            stale: false,
            compile_failed: false,
            drawn_this_frame: false,
            picker: None,
            saved_zoom,
            script_title: None,
            input_schema: InputSchema::default(),
            input_values: InputValues::new(),
            inputs_store: InputsStore::open(inputs_path()),
            inputs_dirty_at: None,
            picking_bar: false,
            picked_bar: None,
            picking_range: false,
            picked_range: None,
            pending_settings: false,
        }
    }

    /// Waits for a click on a candle of the chart, in the debugger's colours.
    pub fn begin_bar_pick(&mut self) {
        // One "click a candle" at a time.
        self.picking_range = false;
        self.picking_bar = true;
        self.picked_bar = None;
        self.card.set_pick_kind(crate::ui::PickKind::StopAtBar);
    }

    /// What a click on a candle does, without the chart: the same end state.
    #[cfg(test)]
    pub(crate) fn pick_for_test(&mut self, open_time_ms: i64) {
        self.picked_bar = Some(open_time_ms);
        self.cancel_bar_pick();
    }

    pub fn cancel_bar_pick(&mut self) {
        self.picking_bar = false;
        self.card.set_pick_kind(crate::ui::PickKind::default());
    }

    pub fn is_picking_bar(&self) -> bool {
        self.picking_bar
    }

    /// Waits for a drag over the bars of the chart, whose log messages the Logs tab then lists.
    pub fn begin_range_pick(&mut self) {
        self.picking_bar = false;
        self.picking_range = true;
        self.picked_range = None;
        self.card.set_pick_kind(crate::ui::PickKind::LogsRange);
    }

    pub fn cancel_range_pick(&mut self) {
        self.picking_range = false;
        self.card.set_pick_kind(crate::ui::PickKind::default());
    }

    pub fn is_picking_range(&self) -> bool {
        self.picking_range
    }

    /// The bars dragged over since the last call: the open times of the first and the last.
    pub fn take_picked_range(&mut self) -> Option<(i64, i64)> {
        self.picked_range.take()
    }

    /// What the Logs tab lists, to show as a band over those bars.
    pub fn set_logs_range(&mut self, range: Option<(i64, i64)>) {
        self.card.set_logs_range(range);
    }

    /// What a drag over bars does, without the chart: the same end state.
    #[cfg(test)]
    pub(crate) fn pick_range_for_test(&mut self, from_ms: i64, to_ms: i64) {
        self.picked_range = Some((from_ms, to_ms));
        self.cancel_range_pick();
    }

    /// The candle clicked since the last call, as its open time in milliseconds.
    pub fn take_picked_bar(&mut self) -> Option<i64> {
        self.picked_bar.take()
    }

    /// Stands in for the candles the preview would fetch.
    #[cfg(test)]
    pub(crate) fn set_candles_for_test(&mut self, candles: Vec<Kline>) {
        self.set_candles(candles);
    }

    /// Replaces the candles, keeping their fingerprint with them.
    pub(super) fn set_candles(&mut self, candles: Vec<Kline>) {
        self.candles_fingerprint = crate::host::pine::chart::candles_fingerprint(&candles);
        self.candles = candles;
    }

    /// What the chart's bars are, cheaply: how many, and the first and last open times.
    pub fn bar_key(&self) -> Option<(usize, i64, i64)> {
        Some((
            self.candles.len(),
            self.candles.first()?.open_time,
            self.candles.last()?.open_time,
        ))
    }

    /// Sets one input value as if the person had typed it.
    #[cfg(test)]
    pub fn set_input_for_test(&mut self, name: &str, value: InputValue) {
        self.input_values.insert(name.to_owned(), value);
    }

    /// Switches to the same symbol on another data source, leaving the candles as they were.
    #[cfg(test)]
    pub(crate) fn select_source_for_test(&mut self, source: DataSource) {
        let candles = self.candles.clone();
        self.select_market(PreviewMarket {
            ticker: self.market.ticker.clone(),
            source,
        });
        self.set_candles(candles);
    }

    #[cfg(test)]
    pub(crate) fn set_visible_candles_for_test(&mut self, count: usize) {
        self.card.set_visible_candles(count);
    }

    /// What a profile of `source` has to be of, besides the text, to be current now. The visible
    /// range is part of it only for a script that reads it, as for the chart's own runs.
    pub fn run_identity(&self, source: &str, now_ms: i64, light: bool) -> RunIdentity {
        use std::hash::{Hash, Hasher};
        let mut market = std::collections::hash_map::DefaultHasher::new();
        self.market.ticker.hash(&mut market);
        self.market.source.hash(&mut market);
        self.timeframe.label().hash(&mut market);
        RunIdentity {
            candles: self.candles_fingerprint,
            last_is_closed: crate::host::pine::chart::last_candle_is_closed(&self.candles, now_ms),
            market: market.finish(),
            light,
            inputs: inputs_fingerprint(&self.input_values),
            visible_range: crate::host::pine::chart::reads_visible_range(source)
                .then(|| self.card.visible_open_range(&self.candles))
                .flatten(),
        }
    }

    /// The open times of the chart's bars, oldest first.
    pub fn bar_open_times(&self) -> Vec<i64> {
        self.candles.iter().map(|bar| bar.open_time).collect()
    }

    /// The bars the preview chart shows, as a debugging session receives them: frozen, with the
    /// symbol and inputs the script runs on now. `None` until the chart has candles.
    pub fn debug_feed(
        &self,
        now_ms: i64,
        light: bool,
    ) -> Option<crate::pine_studio::debug::DebugFeed> {
        if self.candles.is_empty() {
            return None;
        }
        Some(crate::pine_studio::debug::DebugFeed {
            bars: self.candles.clone(),
            symbol: ChartCard::pine_symbol_info(&self.market.ticker, self.market.source),
            inputs: self.input_values.clone(),
            now_ms,
            visible_range: self.card.visible_open_range(&self.candles),
            theme: if light {
                crate::pine::compiler::program::ChartTheme::LIGHT
            } else {
                crate::pine::compiler::program::ChartTheme::DARK
            },
            label: format!("{} · {}", self.market.ticker, self.timeframe.label()),
        })
    }

    /// Points the preview at another script; its source arrives through `source_changed`.
    pub fn script_changed(&mut self, id: &str) {
        if self.script_id.as_deref() == Some(id) {
            return;
        }
        self.script_id = Some(id.to_owned());
        self.latest_source = None;
        self.changed_at = None;
        self.applied_source = None;
        self.stale = false;
        self.compile_failed = false;
        self.card.set_pine_source(None);
        self.flush_inputs();
        self.input_schema = InputSchema::default();
        self.input_values = self.inputs_store.inputs(id);
        self.card.set_pine_inputs(self.input_values.clone());
    }

    pub fn tick(&mut self, ctx: &egui::Context) {
        self.receive_candles();
        // Persist the scroll zoom and price zoom once they settle.
        let zoom = (self.card.visible_candles(), self.card.y_zoom());
        if zoom != self.saved_zoom {
            self.saved_zoom = zoom;
            self.save(CANDLE_KEY, &zoom.0);
            self.save(YZOOM_KEY, &zoom.1);
        }
        if self.changed_at.is_some() {
            ctx.request_repaint_after(APPLY_DELAY);
        }
        if self
            .inputs_dirty_at
            .is_some_and(|at| at.elapsed() >= Duration::from_millis(500))
        {
            self.flush_inputs();
        } else if self.inputs_dirty_at.is_some() {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
        self.drawn_this_frame = false;
    }

    pub(super) fn save<T: Serialize>(&self, key: &str, value: &T) {
        if let Some(shared) = &self.shared
            && let Ok(store) = shared.lock()
        {
            let _ = store.set_setting(key, value);
        }
    }

    pub fn status_text(&self) -> String {
        if self.script_type == Some(ScriptType::Library) {
            return "Libraries export functions — they don't draw on a chart.".to_owned();
        }
        if self.runtime_error().is_some() {
            return if self.card.has_valid_pine_output() {
                "Runtime failed — last valid output remains active".to_owned()
            } else {
                "Runtime failed".to_owned()
            };
        }
        if self.compile_failed {
            return if self.applied_source.is_some() {
                "Compile failed — last valid output remains active".to_owned()
            } else {
                "Compile failed".to_owned()
            };
        }
        let Some(applied) = &self.applied_source else {
            return "No script applied".to_owned();
        };
        if self.evaluating() {
            return "Evaluating…".to_owned();
        }
        let _ = applied;
        let live = if self.candles.is_empty() {
            ""
        } else {
            " · live"
        };
        format!(
            "Applied {} · {} bars{live}",
            self.title_hint(),
            self.candles.len()
        )
    }

    /// Whether the applied script has no result to show yet: the candles are still coming, or its
    /// run has been going on long enough to say so (`ChartCard::pine_busy`).
    fn evaluating(&self) -> bool {
        self.applied_source.is_some()
            && ((self.loading && self.candles.is_empty())
                || self.card.pine_busy(PREVIEW_INSTANCE, Instant::now()))
    }

    /// Whether the status bar shows its busy indicator: exactly while it reads "Evaluating…".
    pub fn is_busy(&self) -> bool {
        self.script_type != Some(ScriptType::Library)
            && self.runtime_error().is_none()
            && !self.compile_failed
            && self.evaluating()
    }

    fn title_hint(&self) -> String {
        self.script_title.clone().unwrap_or_default()
    }
}
