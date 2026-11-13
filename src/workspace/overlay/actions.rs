//! Adding, editing and removing the markets compared on a chart, as undoable steps.

use crate::model::InstrumentId;
use crate::model::chart::price_scale::PriceScaleMode;
use crate::model::overlay::{AddOutcome, OverlayCommand, OverlayId, OverlayScale};
use crate::ui::chart::settings::overlay_popover::{self, OverlaySettingsPopover};
use crate::workspace::ChartWorkspace;
use crate::workspace::undo::{RESTYLE_WINDOW, Snapshot, UndoEntry};
use eframe::egui;
use std::time::Instant;

/// The axes that only compared markets call for: with none left they go back to Regular.
/// (Logarithmic is a choice of its own and stays.)
pub(in crate::workspace) fn reverts_to_regular(mode: PriceScaleMode) -> bool {
    matches!(mode, PriceScaleMode::Percent | PriceScaleMode::IndexedTo100)
}

impl ChartWorkspace {
    /// Pushes the chart's overlays as they stand, to undo the edit that follows. `scale_mode` is the
    /// axis mode to put back with them, when the edit changes it.
    fn remember_overlays(&mut self, chart_id: u64, scale_mode: Option<PriceScaleMode>) {
        let Some(chart) = self.charts.iter().find(|chart| chart.id == chart_id) else {
            return;
        };
        self.undo.push(UndoEntry {
            chart_id,
            before: Snapshot::Overlays {
                list: chart.overlays.clone(),
                scale_mode,
            },
        });
        self.redo.clear();
        self.last_restyle = None;
    }

    /// Whether `chart_id` is a chart another market can be drawn on.
    pub(in crate::workspace) fn can_compare(&self, chart_id: u64) -> bool {
        self.charts
            .iter()
            .find(|chart| chart.id == chart_id)
            .is_some_and(|chart| {
                chart.is_market()
                    && !chart.source.is_line_chart()
                    && !chart
                        .chart
                        .effective_chart_type(chart.source)
                        .is_synthetic()
            })
    }

    /// Draws `instrument` on `chart_id`, on top of the others. Nothing is added for the chart's
    /// own market or one already compared; the outcome says which. The first line on the chart's
    /// own scale switches a Regular axis to Percent, because that is the only axis on which a
    /// rebased line reads as a comparison (TradingView's "Same % scale" does the same). Only the
    /// first: once the chart compares something, the axis the user picked is theirs. The switch is
    /// part of the add's undo step.
    pub(in crate::workspace) fn add_overlay(
        &mut self,
        chart_id: u64,
        instrument: InstrumentId,
        title: Option<String>,
    ) -> Option<AddOutcome> {
        if !self.can_compare(chart_id) {
            return None;
        }
        let chart = self.charts.iter().find(|chart| chart.id == chart_id)?;
        let main = InstrumentId::new(chart.source, chart.symbol.clone());
        // Check on a copy first: a refused add must leave no undo step.
        let mut probe = chart.overlays.clone();
        let outcome = probe.add(&main, instrument.clone(), title.clone());
        if !matches!(outcome, AddOutcome::Added(_)) {
            return Some(outcome);
        }
        let first_on_main_scale = !chart
            .overlays
            .iter()
            .any(|overlay| overlay.scale == OverlayScale::Main);
        let switches = first_on_main_scale && chart.scale_mode == PriceScaleMode::Regular;
        self.remember_overlays(chart_id, switches.then_some(PriceScaleMode::Regular));
        let chart = self.charts.iter_mut().find(|chart| chart.id == chart_id)?;
        chart.overlays = probe;
        if switches {
            chart.scale_mode = PriceScaleMode::Percent;
        }
        Some(outcome)
    }

    /// Opens the settings of one compared market beside `anchor` (its row on the overlay card), or
    /// puts them away when they are already up for it.
    pub(in crate::workspace) fn toggle_overlay_popover(
        &mut self,
        chart_id: u64,
        overlay_id: OverlayId,
        anchor: egui::Rect,
    ) {
        if self
            .overlay_popover
            .as_ref()
            .is_some_and(|popover| popover.is_for(chart_id, overlay_id))
        {
            self.overlay_popover = None;
            return;
        }
        let exists = self
            .charts
            .iter()
            .find(|chart| chart.id == chart_id)
            .is_some_and(|chart| chart.overlays.get(overlay_id).is_some());
        if exists {
            // One popover at a time: a script's would otherwise sit under this one.
            self.script_popover = None;
            self.overlay_popover = Some(OverlaySettingsPopover::open(chart_id, overlay_id, anchor));
        }
    }

    /// Draws the open overlay popover, if any, and applies the edit it returns. It goes when its
    /// market (or chart) does.
    pub(in crate::workspace) fn show_overlay_popover(&mut self, ctx: &egui::Context) {
        let Some(popover) = self.overlay_popover.take() else {
            overlay_popover::set_open(ctx, None);
            return;
        };
        let (chart_id, overlay_id) = (popover.chart_id(), popover.overlay_id());
        let Some(chart) = self.charts.iter().find(|chart| chart.id == chart_id) else {
            overlay_popover::set_open(ctx, None);
            return;
        };
        let Some(overlay) = chart.overlays.get(overlay_id) else {
            overlay_popover::set_open(ctx, None);
            return;
        };
        let own_scales = overlay_popover::own_scales(chart.overlays.iter());
        let outcome = popover.show(ctx, overlay, &own_scales, chart.scale_mode);
        match outcome {
            overlay_popover::Outcome::Close => {
                overlay_popover::set_open(ctx, None);
            }
            overlay_popover::Outcome::Open => {
                overlay_popover::set_open(ctx, Some((chart_id, overlay_id)));
                self.overlay_popover = Some(popover);
            }
            overlay_popover::Outcome::Command(command) => {
                let removed = matches!(command, OverlayCommand::Remove(_));
                self.apply_overlay_command(chart_id, command);
                if removed {
                    overlay_popover::set_open(ctx, None);
                } else {
                    overlay_popover::set_open(ctx, Some((chart_id, overlay_id)));
                    self.overlay_popover = Some(popover);
                }
            }
        }
    }

    /// Applies an edit to `chart_id`'s overlays as one undo step. Returns whether it changed
    /// anything; an edit that changes nothing leaves no step.
    pub(in crate::workspace) fn apply_overlay_command(
        &mut self,
        chart_id: u64,
        command: OverlayCommand,
    ) -> bool {
        let Some(chart) = self.charts.iter().find(|chart| chart.id == chart_id) else {
            return false;
        };
        let mut probe = chart.overlays.clone();
        if !probe.apply(command) {
            return false;
        }
        let style_run = match command {
            OverlayCommand::SetColor(id, _) => Some(id),
            _ => None,
        };
        // A colour dragged in the picker reports every frame: the run is one step, as a drawing's
        // restyle is. The run breaks on another overlay, another command, or any other undo step.
        let now = Instant::now();
        let merges = style_run.is_some_and(|id| {
            self.last_overlay_style
                .is_some_and(|(chart, overlay, len, at)| {
                    chart == chart_id
                        && overlay == id
                        && len == self.undo.len()
                        && now.saturating_duration_since(at) <= RESTYLE_WINDOW
                })
        });
        // Taking the last compared market off puts the axis back to Regular, as adding the first
        // switched it to Percent; the step carries the old mode so undo restores both.
        let reverts = if probe.is_empty() {
            self.charts
                .iter()
                .find(|chart| chart.id == chart_id)
                .map(|chart| chart.scale_mode)
                .filter(|mode| reverts_to_regular(*mode))
        } else {
            None
        };
        if !merges {
            self.remember_overlays(chart_id, reverts);
        }
        if let Some(chart) = self.charts.iter_mut().find(|chart| chart.id == chart_id) {
            chart.overlays = probe;
            if reverts.is_some() {
                chart.scale_mode = PriceScaleMode::Regular;
            }
        }
        self.last_overlay_style = style_run.map(|id| (chart_id, id, self.undo.len(), now));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::super::{OverlayBackend, OverlayHub};
    use super::*;
    use crate::model::overlay::feed::OverlayStatus;
    use crate::model::overlay::{OverlayScale, ScaleChoice};
    use crate::model::{DataSource, Kline, Timeframe};
    use crate::workspace::test_support::{candle, chart_with_drawings, empty_workspace};
    use eframe::egui;
    use std::sync::Arc;

    fn market(symbol: &str) -> InstrumentId {
        InstrumentId::new(DataSource::Binance, symbol)
    }

    fn overlays_of(workspace: &ChartWorkspace, chart_id: u64) -> Vec<String> {
        workspace
            .charts
            .iter()
            .find(|chart| chart.id == chart_id)
            .unwrap()
            .overlays
            .iter()
            .map(|overlay| overlay.instrument.symbol.clone())
            .collect()
    }

    fn chart(workspace: &ChartWorkspace, chart_id: u64) -> &crate::workspace::chart::Chart {
        workspace
            .charts
            .iter()
            .find(|chart| chart.id == chart_id)
            .unwrap()
    }

    #[test]
    fn the_toolbars_plus_opens_the_compare_picker_on_a_chart_that_can_take_one() {
        use crate::ui::chart::symbol_picker::PickMode;
        let mut workspace = empty_workspace(1);
        let a = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
        workspace.focus_chart(a);
        assert!(workspace.can_compare(a));
        workspace.open_compare_picker();
        assert!(workspace.symbol_picker.is_open());
        assert_eq!(workspace.symbol_picker.mode(), PickMode::Compare);
    }

    #[test]
    fn adding_draws_on_the_chart_alone_and_switches_regular_to_percent() {
        let mut workspace = empty_workspace(1);
        let a = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
        let b = chart_with_drawings(&mut workspace, "AAAUSDT", &[]);
        assert_eq!(chart(&workspace, a).scale_mode, PriceScaleMode::Regular);
        let outcome = workspace.add_overlay(a, market("ETHUSDT"), None);
        assert!(matches!(outcome, Some(AddOutcome::Added(_))));
        assert_eq!(overlays_of(&workspace, a), ["ETHUSDT"]);
        assert!(
            overlays_of(&workspace, b).is_empty(),
            "another chart is untouched"
        );
        assert_eq!(chart(&workspace, a).scale_mode, PriceScaleMode::Percent);
        assert_eq!(chart(&workspace, b).scale_mode, PriceScaleMode::Regular);
        assert_eq!(workspace.chart_count(), 2, "no chart card was added");
    }

    #[test]
    fn a_chart_already_on_log_or_indexed_keeps_its_mode() {
        let mut workspace = empty_workspace(1);
        let a = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
        workspace.charts[0].scale_mode = PriceScaleMode::IndexedTo100;
        workspace.add_overlay(a, market("ETHUSDT"), None);
        assert_eq!(
            chart(&workspace, a).scale_mode,
            PriceScaleMode::IndexedTo100
        );
    }

    #[test]
    fn the_charts_own_market_and_a_repeat_add_nothing_and_leave_no_undo_step() {
        let mut workspace = empty_workspace(1);
        let a = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
        workspace.add_overlay(a, market("ETHUSDT"), None);
        let steps = workspace.undo.len();
        assert_eq!(
            workspace.add_overlay(a, market("btcusdt"), None),
            Some(AddOutcome::MainMarket)
        );
        assert!(matches!(
            workspace.add_overlay(a, market("ETHUSDT"), None),
            Some(AddOutcome::Duplicate(_))
        ));
        assert_eq!(overlays_of(&workspace, a), ["ETHUSDT"]);
        assert_eq!(workspace.undo.len(), steps);
    }

    #[test]
    fn a_line_only_source_takes_no_overlay() {
        let mut workspace = empty_workspace(1);
        let ctx = egui::Context::default();
        let chart_id = workspace
            .add_chart_for(&InstrumentId::new(DataSource::Polymarket, "token"), &ctx)
            .chart_id;
        assert_eq!(
            workspace.add_overlay(chart_id, market("ETHUSDT"), None),
            None
        );
        assert!(!workspace.can_compare(chart_id));
        assert!(!workspace.can_compare(999));
    }

    #[test]
    fn a_price_based_chart_type_takes_no_overlay() {
        let mut workspace = empty_workspace(1);
        let a = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
        workspace.charts[0].chart.appearance_mut().style.chart_type =
            crate::model::ChartType::Renko;
        assert!(!workspace.can_compare(a));
    }

    #[test]
    fn taking_the_last_compared_market_off_puts_the_axis_back_to_regular() {
        let mut workspace = empty_workspace(1);
        let a = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
        workspace.add_overlay(a, market("ETHUSDT"), None);
        workspace.add_overlay(a, market("SOLUSDT"), None);
        assert_eq!(chart(&workspace, a).scale_mode, PriceScaleMode::Percent);
        let (first, second) = {
            let list = chart(&workspace, a).overlays.as_slice();
            (list[0].id, list[1].id)
        };
        // One left: still comparing, the axis stays.
        workspace.apply_overlay_command(a, OverlayCommand::Remove(first));
        assert_eq!(chart(&workspace, a).scale_mode, PriceScaleMode::Percent);
        workspace.apply_overlay_command(a, OverlayCommand::Remove(second));
        assert!(overlays_of(&workspace, a).is_empty());
        assert_eq!(chart(&workspace, a).scale_mode, PriceScaleMode::Regular);
        // Undo brings the market and its axis back in one step; redo takes both off again.
        workspace.undo(&eframe::egui::Context::default());
        assert_eq!(overlays_of(&workspace, a), ["SOLUSDT"]);
        assert_eq!(chart(&workspace, a).scale_mode, PriceScaleMode::Percent);
        workspace.redo(&eframe::egui::Context::default());
        assert_eq!(chart(&workspace, a).scale_mode, PriceScaleMode::Regular);
    }

    #[test]
    fn remove_all_reverts_the_axis_too_but_a_logarithmic_axis_stays() {
        let mut workspace = empty_workspace(1);
        let a = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
        workspace.add_overlay(a, market("ETHUSDT"), None);
        workspace.apply_overlay_command(a, OverlayCommand::RemoveAll);
        assert_eq!(chart(&workspace, a).scale_mode, PriceScaleMode::Regular);

        workspace.add_overlay(a, market("ETHUSDT"), None);
        workspace.set_scale_mode(a, PriceScaleMode::Logarithmic);
        workspace.apply_overlay_command(a, OverlayCommand::RemoveAll);
        assert_eq!(chart(&workspace, a).scale_mode, PriceScaleMode::Logarithmic);
    }

    #[test]
    fn undo_and_redo_restore_the_overlays_as_one_step_each() {
        let mut workspace = empty_workspace(1);
        let a = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
        workspace.add_overlay(a, market("ETHUSDT"), None);
        workspace.add_overlay(a, market("SOLUSDT"), None);
        let id = chart(&workspace, a).overlays.as_slice()[0].id;
        assert!(workspace.apply_overlay_command(a, OverlayCommand::SetVisible(id, false)));
        assert!(!workspace.apply_overlay_command(a, OverlayCommand::SetVisible(id, false)));
        workspace.undo(&eframe::egui::Context::default());
        assert!(chart(&workspace, a).overlays.get(id).unwrap().visible);
        workspace.undo(&eframe::egui::Context::default());
        assert_eq!(overlays_of(&workspace, a), ["ETHUSDT"]);
        workspace.redo(&eframe::egui::Context::default());
        assert_eq!(overlays_of(&workspace, a), ["ETHUSDT", "SOLUSDT"]);
        workspace.apply_overlay_command(a, OverlayCommand::RemoveAll);
        assert!(overlays_of(&workspace, a).is_empty());
        workspace.undo(&eframe::egui::Context::default());
        assert_eq!(overlays_of(&workspace, a), ["ETHUSDT", "SOLUSDT"]);
    }

    #[test]
    fn overlays_are_saved_with_the_layout_and_restored_exactly() {
        let mut workspace = empty_workspace(1);
        let a = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
        workspace.add_overlay(a, market("ETHUSDT"), None);
        workspace.add_overlay(a, market("SOLUSDT"), Some("Solana".into()));
        let first = chart(&workspace, a).overlays.as_slice()[0].id;
        workspace.apply_overlay_command(a, OverlayCommand::SetScale(first, ScaleChoice::Own));
        let saved = workspace.persisted();
        let json = serde_json::to_string(&saved).unwrap();
        let restored: crate::workspace::PersistedWorkspace = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.charts[0].overlays, chart(&workspace, a).overlays);
        let ctx = egui::Context::default();
        let reopened = crate::workspace::chart::Chart::restored(
            restored.charts[0].clone(),
            Timeframe::Day,
            &ctx,
        );
        assert_eq!(reopened.overlays, chart(&workspace, a).overlays);
        assert_eq!(
            reopened.overlays.get(first).unwrap().scale,
            OverlayScale::Own(first)
        );
    }

    #[test]
    fn changing_the_overlays_is_a_change_to_the_layout() {
        let mut workspace = empty_workspace(1);
        let a = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
        workspace.set_baseline();
        assert!(!workspace.is_dirty());
        workspace.add_overlay(a, market("ETHUSDT"), None);
        workspace.charts[0].scale_mode = PriceScaleMode::Regular;
        assert!(workspace.is_dirty());
        workspace.set_baseline();
        let id = chart(&workspace, a).overlays.as_slice()[0].id;
        workspace.apply_overlay_command(
            a,
            OverlayCommand::SetDash(id, crate::model::chart::kind::LineDash::Dotted),
        );
        assert!(workspace.is_dirty(), "a restyle of an overlay counts");
    }

    #[test]
    fn switching_the_charts_market_keeps_its_overlays() {
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(1);
        let a = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
        workspace.add_overlay(a, market("ETHUSDT"), None);
        let item = crate::model::watchlist::WatchlistInstrument::from_market(
            DataSource::Binance,
            "SOLUSDT",
            "SOLUSDT",
        );
        workspace.switch_chart(a, &item, &ctx);
        assert_eq!(chart(&workspace, a).symbol, "SOLUSDT");
        assert_eq!(overlays_of(&workspace, a), ["ETHUSDT"]);
        // And an overlay of the market the chart switches to is kept, not deleted.
        let to_eth = crate::model::watchlist::WatchlistInstrument::from_market(
            DataSource::Binance,
            "ETHUSDT",
            "ETHUSDT",
        );
        workspace.switch_chart(a, &to_eth, &ctx);
        assert_eq!(overlays_of(&workspace, a), ["ETHUSDT"]);
    }

    #[test]
    fn overlays_change_nothing_a_script_an_alert_a_drawing_or_an_order_reads() {
        let mut workspace = empty_workspace(1);
        let a = chart_with_drawings(&mut workspace, "BTCUSDT", &[1, 2]);
        workspace.charts[0].candles = (0..5)
            .map(|n| candle(n * 60_000, 10.0 + n as f64))
            .collect();
        let (candles, symbol, source) = {
            let c = chart(&workspace, a);
            (c.candles.clone(), c.symbol.clone(), c.source)
        };
        let alert = format!("{:?}", ChartWorkspace::alert_asset(chart(&workspace, a)));
        let paper = format!(
            "{:?}",
            ChartWorkspace::paper_instrument(chart(&workspace, a))
        );
        let drawings = chart(&workspace, a).drawings.clone();
        for symbol in ["ETHUSDT", "SOLUSDT", "BNBUSDT"] {
            workspace.add_overlay(a, market(symbol), None);
        }
        let after = chart(&workspace, a);
        assert_eq!(
            (after.candles.clone(), after.symbol.clone(), after.source),
            (candles, symbol, source)
        );
        assert_eq!(format!("{:?}", ChartWorkspace::alert_asset(after)), alert);
        assert_eq!(
            format!("{:?}", ChartWorkspace::paper_instrument(after)),
            paper
        );
        assert_eq!(after.drawings, drawings);
        assert_eq!(workspace.focused_instrument(), Some(market("BTCUSDT")));
    }

    /// A provider that answers at once from a table.
    struct Immediate;

    impl OverlayBackend for Immediate {
        fn submit(&self, _: DataSource, job: Box<dyn FnOnce() + Send>) {
            job();
        }
        fn klines(
            &self,
            _: DataSource,
            symbol: &str,
            _: &str,
            _: u32,
        ) -> Result<Vec<Kline>, String> {
            if symbol == "BADUSDT" {
                return Err("no such market".into());
            }
            Ok((0..5).map(|n| candle(n * 60_000, 2.0 + n as f64)).collect())
        }
        fn page(&self, _: DataSource, _: &str, _: &str, _: i64) -> Result<Vec<Kline>, String> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn the_card_is_handed_each_overlays_candles_and_a_failing_market_is_only_its_own_problem() {
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(1);
        workspace.overlay_hub = OverlayHub::with_backend(Arc::new(Immediate));
        let a = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
        workspace.add_overlay(a, market("ETHUSDT"), None);
        workspace.add_overlay(a, market("BADUSDT"), None);
        workspace.add_overlay(a, market("btc"), None);
        for _ in 0..3 {
            workspace.tick_overlays(&ctx);
        }
        let card = &chart(&workspace, a).chart;
        let by_name = |symbol: &str| {
            let id = card
                .overlay_list()
                .iter()
                .find(|o| o.instrument.symbol == symbol)
                .unwrap()
                .id;
            card.overlay_feed(id).unwrap().clone()
        };
        assert_eq!(by_name("ETHUSDT").status, OverlayStatus::Ready);
        assert_eq!(by_name("ETHUSDT").candles.len(), 5);
        assert!(matches!(by_name("BADUSDT").status, OverlayStatus::Error(_)));
        assert_eq!(
            card.overlay_list().len(),
            2,
            "the chart's own market was refused"
        );

        // Removing the chart releases every market it held.
        workspace.remove_chart(a);
        assert_eq!(workspace.overlay_hub.series_count(), 0);
    }

    #[test]
    fn two_charts_comparing_one_market_share_one_fetch_and_each_keeps_its_own_list() {
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(1);
        workspace.overlay_hub = OverlayHub::with_backend(Arc::new(Immediate));
        let a = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
        let b = chart_with_drawings(&mut workspace, "XRPUSDT", &[]);
        workspace.add_overlay(a, market("ETHUSDT"), None);
        workspace.add_overlay(b, market("ETHUSDT"), None);
        workspace.add_overlay(b, market("SOLUSDT"), None);
        workspace.tick_overlays(&ctx);
        assert_eq!(workspace.overlay_hub.series_count(), 2);
        workspace.apply_overlay_command(a, OverlayCommand::RemoveAll);
        workspace.tick_overlays(&ctx);
        assert_eq!(
            workspace.overlay_hub.series_count(),
            2,
            "chart b still holds both"
        );
        assert_eq!(overlays_of(&workspace, b), ["ETHUSDT", "SOLUSDT"]);
    }

    fn mode(workspace: &ChartWorkspace, chart_id: u64) -> PriceScaleMode {
        chart(workspace, chart_id).scale_mode
    }

    #[test]
    fn undoing_the_first_add_puts_the_axis_back_and_redo_switches_it_again() {
        let mut workspace = empty_workspace(1);
        let a = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
        workspace.add_overlay(a, market("ETHUSDT"), None);
        assert_eq!(mode(&workspace, a), PriceScaleMode::Percent);
        workspace.undo(&eframe::egui::Context::default());
        assert!(overlays_of(&workspace, a).is_empty());
        assert_eq!(
            mode(&workspace, a),
            PriceScaleMode::Regular,
            "the axis came back with it"
        );
        workspace.redo(&eframe::egui::Context::default());
        assert_eq!(overlays_of(&workspace, a), ["ETHUSDT"]);
        assert_eq!(mode(&workspace, a), PriceScaleMode::Percent);
        workspace.undo(&eframe::egui::Context::default());
        assert_eq!(mode(&workspace, a), PriceScaleMode::Regular);
    }

    #[test]
    fn undoing_a_later_edit_never_reverts_an_axis_the_user_picked_afterwards() {
        let mut workspace = empty_workspace(1);
        let a = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
        workspace.add_overlay(a, market("ETHUSDT"), None);
        let id = chart(&workspace, a).overlays.as_slice()[0].id;
        workspace.set_scale_mode(a, PriceScaleMode::IndexedTo100);
        workspace.apply_overlay_command(a, OverlayCommand::SetVisible(id, false));
        workspace.undo(&eframe::egui::Context::default());
        assert!(chart(&workspace, a).overlays.get(id).unwrap().visible);
        assert_eq!(mode(&workspace, a), PriceScaleMode::IndexedTo100);
        workspace.redo(&eframe::egui::Context::default());
        assert_eq!(mode(&workspace, a), PriceScaleMode::IndexedTo100);
    }

    #[test]
    fn only_the_first_line_on_the_charts_scale_switches_the_axis() {
        let mut workspace = empty_workspace(1);
        let a = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
        workspace.add_overlay(a, market("ETHUSDT"), None);
        // The user goes back to Regular on purpose: another market must not undo that.
        workspace.set_scale_mode(a, PriceScaleMode::Regular);
        workspace.add_overlay(a, market("SOLUSDT"), None);
        assert_eq!(mode(&workspace, a), PriceScaleMode::Regular);
        workspace.undo(&eframe::egui::Context::default());
        assert_eq!(mode(&workspace, a), PriceScaleMode::Regular);
    }

    #[test]
    fn a_chart_whose_overlays_all_have_their_own_scale_switches_on_its_first_main_one() {
        let mut workspace = empty_workspace(1);
        let a = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
        workspace.add_overlay(a, market("ETHUSDT"), None);
        let id = chart(&workspace, a).overlays.as_slice()[0].id;
        workspace.apply_overlay_command(a, OverlayCommand::SetScale(id, ScaleChoice::Own));
        workspace.set_scale_mode(a, PriceScaleMode::Regular);
        workspace.add_overlay(a, market("SOLUSDT"), None);
        assert_eq!(mode(&workspace, a), PriceScaleMode::Percent);
    }

    #[test]
    fn showing_or_hiding_every_market_is_one_undo_step() {
        let mut workspace = empty_workspace(1);
        let a = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
        for symbol in ["ETHUSDT", "SOLUSDT", "BNBUSDT"] {
            workspace.add_overlay(a, market(symbol), None);
        }
        let steps = workspace.undo.len();
        assert!(workspace.apply_overlay_command(a, OverlayCommand::SetAllVisible(false)));
        assert!(chart(&workspace, a).overlays.iter().all(|o| !o.visible));
        assert_eq!(workspace.undo.len(), steps + 1);
        assert!(!workspace.apply_overlay_command(a, OverlayCommand::SetAllVisible(false)));
        assert_eq!(workspace.undo.len(), steps + 1, "nothing changed, no step");
        workspace.undo(&eframe::egui::Context::default());
        assert!(chart(&workspace, a).overlays.iter().all(|o| o.visible));
    }

    #[test]
    fn dragging_a_colour_is_one_undo_step() {
        use crate::model::editor_theme::Rgba;
        let mut workspace = empty_workspace(1);
        let a = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
        workspace.add_overlay(a, market("ETHUSDT"), None);
        workspace.add_overlay(a, market("SOLUSDT"), None);
        let (eth, sol) = {
            let list = &chart(&workspace, a).overlays;
            (list.as_slice()[0].id, list.as_slice()[1].id)
        };
        let original = chart(&workspace, a).overlays.get(eth).unwrap().color;
        let before = workspace.undo.len();
        // A drag reports a new colour on every frame.
        for n in 0..40u8 {
            let changed = workspace
                .apply_overlay_command(a, OverlayCommand::SetColor(eth, Rgba::new(n, 20, 30, 255)));
            assert!(changed);
        }
        assert_eq!(
            workspace.undo.len(),
            before + 1,
            "one step for the whole drag"
        );
        assert_eq!(
            chart(&workspace, a).overlays.get(eth).unwrap().color,
            Rgba::new(39, 20, 30, 255)
        );
        workspace.undo(&eframe::egui::Context::default());
        assert_eq!(
            chart(&workspace, a).overlays.get(eth).unwrap().color,
            original
        );

        // Another overlay's colour, or another command in between, is a step of its own.
        let start = workspace.undo.len();
        workspace.apply_overlay_command(a, OverlayCommand::SetColor(eth, Rgba::new(1, 1, 1, 255)));
        workspace.apply_overlay_command(a, OverlayCommand::SetColor(sol, Rgba::new(2, 2, 2, 255)));
        workspace.apply_overlay_command(a, OverlayCommand::SetColor(sol, Rgba::new(3, 3, 3, 255)));
        workspace.apply_overlay_command(a, OverlayCommand::SetWidth(sol, 3.0));
        workspace.apply_overlay_command(a, OverlayCommand::SetColor(sol, Rgba::new(4, 4, 4, 255)));
        assert_eq!(workspace.undo.len(), start + 4);
    }

    #[test]
    fn a_colour_edit_after_the_window_or_another_undo_step_starts_a_new_step() {
        use crate::model::editor_theme::Rgba;
        let mut workspace = empty_workspace(1);
        let a = chart_with_drawings(&mut workspace, "BTCUSDT", &[1]);
        workspace.add_overlay(a, market("ETHUSDT"), None);
        let id = chart(&workspace, a).overlays.as_slice()[0].id;
        workspace.apply_overlay_command(a, OverlayCommand::SetColor(id, Rgba::new(9, 9, 9, 255)));
        let steps = workspace.undo.len();
        // A drawing step in between ends the run.
        workspace.apply_drawing_commands(a, vec![crate::model::DrawingCommand::Remove(1)]);
        workspace.apply_overlay_command(a, OverlayCommand::SetColor(id, Rgba::new(8, 8, 8, 255)));
        assert_eq!(workspace.undo.len(), steps + 2);
        // The window lapsing ends it too.
        let (chart_id, overlay, len, _) = workspace.last_overlay_style.unwrap();
        workspace.last_overlay_style =
            Some((chart_id, overlay, len, Instant::now() - RESTYLE_WINDOW * 2));
        workspace.apply_overlay_command(a, OverlayCommand::SetColor(id, Rgba::new(7, 7, 7, 255)));
        assert_eq!(workspace.undo.len(), steps + 3);
    }

    /// Prints a saved session tab holding a chart that compares several markets, for loading into a
    /// throwaway data directory when looking at the real window:
    /// `cargo test --lib print_a_session_that_compares -- --ignored --nocapture`.
    #[test]
    #[ignore = "writes a seed for looking at the app"]
    fn print_a_session_that_compares() {
        let mut workspace = empty_workspace(2);
        let a = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
        for symbol in ["ETHUSDT", "SOLUSDT", "BNBUSDT", "DOGEUSDT", "XRPUSDT"] {
            workspace.add_overlay(a, market(symbol), None);
        }
        let ids: Vec<_> = chart(&workspace, a).overlays.iter().map(|o| o.id).collect();
        workspace.apply_overlay_command(a, OverlayCommand::SetScale(ids[3], ScaleChoice::Own));
        workspace.apply_overlay_command(a, OverlayCommand::SetScale(ids[4], ScaleChoice::Own));
        println!(
            "SEED {}",
            serde_json::to_string(&workspace.persisted()).unwrap()
        );
    }
}
