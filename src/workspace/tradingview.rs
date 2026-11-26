//! The TradingView import over a chart tab: opened from the layout menu, the new-tab page and the
//! File menu, drawn over the tab it was opened on, and able to open what it added there.

use super::{ChartWorkspace, WorkspaceAction};
use crate::host::tradingview::{Outcome, Targets, TradingViewImport};
use crate::store::scripts::ScriptStore;
use eframe::egui;
use std::sync::Arc;

impl ChartWorkspace {
    /// Opens the import, unless this tab already has it open.
    pub fn open_tradingview_import(&mut self, ctx: &egui::Context) {
        if self.tradingview.is_some() {
            return;
        }
        let targets = Targets {
            store: self.store.clone(),
            watchlist: Arc::clone(&self.watchlist),
            scripts: Arc::new(ScriptStore::default_location),
        };
        self.tradingview = Some(TradingViewImport::open(targets, ctx));
        ctx.request_repaint();
    }

    /// Whether a sheet that takes the whole keyboard is over the charts.
    pub(super) fn sheet_over_charts(&self) -> bool {
        self.studio_open() || self.tradingview.is_some()
    }

    pub(super) fn show_tradingview_import(
        &mut self,
        ctx: &egui::Context,
        actions: &mut Vec<WorkspaceAction>,
    ) {
        let Some(import) = self.tradingview.as_mut() else {
            return;
        };
        let outcome = import.show(ctx);
        if import.take_wrote() {
            // The new-tab page keeps what it read: make it read the layouts again.
            self.empty_page.data = None;
        }
        if import.take_added_scripts() {
            actions.push(WorkspaceAction::ScriptsImported);
        }
        match outcome {
            Outcome::Open => {}
            Outcome::Closed => self.tradingview = None,
            Outcome::OpenLayout(id) => {
                self.tradingview = None;
                self.request_open_saved_view(id, ctx);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::model::SavedView;
    use crate::model::tradingview::plan::{self, Mappings, Market, Match, Selection, Taken};
    use crate::model::tradingview::{fixtures, parse};
    use crate::model::{DataSource, PersistedChart};
    use crate::workspace::test_support::empty_workspace;
    use eframe::egui;

    /// What the import writes is what opening it gives back: nothing is dropped or rewritten on
    /// the way into a workspace, which would lose it at the next save.
    #[test]
    fn an_imported_layout_opens_as_it_was_written() {
        let mut library = parse::library(fixtures::documents());
        let line = fixtures::drawing(
            "elsewhere",
            "LineToolTrendLine",
            "COINBASE:BTCUSD",
            serde_json::json!({"showPriceRange": true, "vertLabelsAlign": "bottom"}),
            &[(0, 1.0), (1, 2.0)],
        );
        library.layouts[0].drawings.push(
            crate::model::tradingview::drawing::RawDrawing::read("elsewhere", &line).unwrap(),
        );
        let selection = Selection::all(&library);
        let mut mappings = Mappings::default();
        for (symbol, _) in selection.symbols(&library) {
            let name = symbol.ticker().to_owned();
            let market = Market {
                source: DataSource::Binance,
                symbol: name.clone(),
                name,
                image_url: None,
            };
            mappings.found(symbol, Match::Exact(market));
        }
        let plan = plan::plan(
            &library,
            &selection,
            &mappings,
            &Default::default(),
            &Taken::default(),
            &Default::default(),
        );
        let planned = &plan.layouts[0];
        assert!(planned.charts.len() > 1 && planned.drawings > 2);
        let view = SavedView {
            id: 77,
            name: planned.name.clone(),
            timeframe: planned.timeframe,
            created_at_ms: 1,
            charts: planned.charts.clone(),
            columns: Some(planned.columns.clone()),
            candle_count: 0,
            last_opened_at_ms: None,
            autosave: None,
            sync: Default::default(),
        };

        let mut workspace = empty_workspace(1);
        crate::lock::lock(&workspace.store)
            .replace_saved_views(std::slice::from_ref(&view))
            .unwrap();
        workspace.request_open_saved_view(view.id, &egui::Context::default());

        assert!(!workspace.is_dirty());
        let opened = workspace.persisted();
        assert_eq!(opened.timeframe, view.timeframe);
        assert_eq!(opened.columns, planned.columns);
        // What a chart shows of itself is its own to settle.
        let comparable = |charts: &[PersistedChart]| {
            charts
                .iter()
                .map(|chart| {
                    let mut value = serde_json::to_value(chart).unwrap();
                    let fields = value.as_object_mut().unwrap();
                    for key in ["viewport", "candle_count", "y_zoom"] {
                        fields.remove(key);
                    }
                    value
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(comparable(&opened.charts), comparable(&view.charts));
    }
}
