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
