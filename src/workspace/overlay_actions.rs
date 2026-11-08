//! Adding, editing and removing the markets compared on a chart, as undoable steps.

use super::ChartWorkspace;
use super::undo::{Snapshot, UndoEntry};
use crate::model::InstrumentId;
use crate::model::overlay::{AddOutcome, OverlayCommand};
use crate::model::price_scale::PriceScaleMode;

impl ChartWorkspace {
    /// Pushes the chart's overlays as they stand, to undo the edit that follows.
    fn remember_overlays(&mut self, chart_id: u64) {
        let Some(chart) = self.charts.iter().find(|chart| chart.id == chart_id) else {
            return;
        };
        self.undo.push(UndoEntry {
            chart_id,
            before: Snapshot::Overlays(chart.overlays.clone()),
        });
        self.redo.clear();
        self.last_restyle = None;
    }

    /// Whether `chart_id` is a chart another market can be drawn on.
    pub(super) fn can_compare(&self, chart_id: u64) -> bool {
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
    /// rebased line reads as a comparison (TradingView's "Same % scale" does the same).
    pub(super) fn add_overlay(
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
        self.remember_overlays(chart_id);
        let chart = self.charts.iter_mut().find(|chart| chart.id == chart_id)?;
        chart.overlays = probe;
        if chart.scale_mode == PriceScaleMode::Regular {
            chart.scale_mode = PriceScaleMode::Percent;
        }
        Some(outcome)
    }

    /// Applies an edit to `chart_id`'s overlays as one undo step. Returns whether it changed
    /// anything; an edit that changes nothing leaves no step.
    pub(super) fn apply_overlay_command(&mut self, chart_id: u64, command: OverlayCommand) -> bool {
        let Some(chart) = self.charts.iter().find(|chart| chart.id == chart_id) else {
            return false;
        };
        let mut probe = chart.overlays.clone();
        if !probe.apply(command) {
            return false;
        }
        self.remember_overlays(chart_id);
        if let Some(chart) = self.charts.iter_mut().find(|chart| chart.id == chart_id) {
            chart.overlays = probe;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::super::overlays::{OverlayBackend, OverlayHub};
    use super::super::test_support::{candle, chart_with_drawings, empty_workspace};
    use super::*;
    use crate::model::overlay::{OverlayScale, ScaleChoice};
    use crate::model::overlay_feed::OverlayStatus;
    use crate::model::{DataSource, Kline, Timeframe};
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

    fn chart(workspace: &ChartWorkspace, chart_id: u64) -> &super::super::chart::Chart {
        workspace
            .charts
            .iter()
            .find(|chart| chart.id == chart_id)
            .unwrap()
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
    fn undo_and_redo_restore_the_overlays_as_one_step_each() {
        let mut workspace = empty_workspace(1);
        let a = chart_with_drawings(&mut workspace, "BTCUSDT", &[]);
        workspace.add_overlay(a, market("ETHUSDT"), None);
        workspace.add_overlay(a, market("SOLUSDT"), None);
        let id = chart(&workspace, a).overlays.as_slice()[0].id;
        assert!(workspace.apply_overlay_command(a, OverlayCommand::SetVisible(id, false)));
        assert!(!workspace.apply_overlay_command(a, OverlayCommand::SetVisible(id, false)));
        workspace.undo();
        assert!(chart(&workspace, a).overlays.get(id).unwrap().visible);
        workspace.undo();
        assert_eq!(overlays_of(&workspace, a), ["ETHUSDT"]);
        workspace.redo();
        assert_eq!(overlays_of(&workspace, a), ["ETHUSDT", "SOLUSDT"]);
        workspace.apply_overlay_command(a, OverlayCommand::RemoveAll);
        assert!(overlays_of(&workspace, a).is_empty());
        workspace.undo();
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
        let restored: super::super::PersistedWorkspace = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.charts[0].overlays, chart(&workspace, a).overlays);
        let ctx = egui::Context::default();
        let reopened =
            super::super::chart::Chart::restored(restored.charts[0].clone(), Timeframe::Day, &ctx);
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
            OverlayCommand::SetDash(id, crate::model::chart_type::LineDash::Dotted),
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
    struct Instant;

    impl OverlayBackend for Instant {
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
        workspace.overlay_hub = OverlayHub::with_backend(Arc::new(Instant));
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
        workspace.overlay_hub = OverlayHub::with_backend(Arc::new(Instant));
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
}
