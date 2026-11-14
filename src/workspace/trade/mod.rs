//! The tab's trading side: the order sheet beside the charts, the orders panel under them, and the
//! venue they trade on. `broker`, `paper` and `sheet` act on it; the rest of the workspace opens and
//! closes it and passes it what a chart asked for.

use crate::host::paper::PaperHost;
use crate::host::trading::TradingHub;
use crate::lock::lock;
use crate::model::broker::venue::TradingVenue;
use crate::ui;
use crate::ui::broker::panel::BrokerPanelState;
use crate::ui::paper::panel::PaperPanel;
use crate::ui::trade::order_sheet::SheetState;
use crate::workspace::{ChartWorkspace, WorkspaceAction};
use eframe::egui;
use std::sync::{Arc, Mutex};

pub(in crate::workspace) mod broker;
pub(in crate::workspace) mod paper;
pub(in crate::workspace) mod sheet;

#[derive(Default)]
pub(in crate::workspace) struct TradeSide {
    /// The venues this tab can trade on, handed in by the window.
    hub: Option<Arc<TradingHub>>,
    /// The orders panel under the charts.
    panel_visible: bool,
    /// The order sheet beside the chart. Open and closed with the orders panel, and by its own ✕.
    sheet_visible: bool,
    sheet: SheetState,
    /// Where this tab's trading happens: paper, or a connected broker with real money. Not
    /// saved: a tab always starts on paper.
    venue: TradingVenue,
    broker_panel: BrokerPanelState,
    paper_panel: PaperPanel,
}

impl TradeSide {
    /// Gives the tab the venues it can trade on; the first hub handed in stays.
    pub(in crate::workspace) fn set_hub(&mut self, hub: &Arc<TradingHub>) {
        if self.hub.is_none() {
            self.hub = Some(Arc::clone(hub));
        }
    }

    pub(in crate::workspace) fn is_panel_open(&self) -> bool {
        self.panel_visible
    }

    #[cfg(test)]
    pub(in crate::workspace) fn is_sheet_open(&self) -> bool {
        self.sheet_visible
    }

    /// Opens the orders panel and the order sheet beside it.
    pub(in crate::workspace) fn open_panel(&mut self) {
        self.panel_visible = true;
        self.sheet_visible = true;
    }

    /// The tool rail's trading button: the orders and the order sheet open and close together.
    pub(in crate::workspace) fn toggle_panel(&mut self) {
        self.panel_visible = !self.panel_visible;
        self.sheet_visible = self.panel_visible;
    }

    /// Shows what paper orders placed from a chart had to say (why they failed) in the panel.
    pub(in crate::workspace) fn report_paper_messages(&mut self, messages: &[String]) {
        self.paper_panel.report_messages(messages);
    }

    #[cfg(test)]
    pub(in crate::workspace) fn sheet(&self) -> &SheetState {
        &self.sheet
    }
}

impl ChartWorkspace {
    /// The order sheet at the right and the orders panel at the bottom, while they are open. The
    /// paper host is locked only while the paper panel draws (see `ChartWorkspace::ui`).
    pub(in crate::workspace) fn show_trade_panels(
        &mut self,
        ui: &mut egui::Ui,
        paper: &Arc<Mutex<PaperHost>>,
    ) -> Vec<WorkspaceAction> {
        let mut actions = Vec::new();
        if self.trade.sheet_visible {
            let sheet_actions = egui::Panel::right(egui::Id::new(("trade_sheet", self.id)))
                .resizable(true)
                .default_size(340.0)
                .min_size(300.0)
                .max_size(460.0)
                .frame(
                    egui::Frame::new()
                        .fill(ui::kit::window())
                        .inner_margin(egui::Margin::ZERO),
                )
                .show(ui, |ui| self.show_trade_sheet(ui))
                .inner;
            actions.extend(sheet_actions);
        }

        if self.trade.panel_visible {
            let mut panel_actions: Vec<WorkspaceAction> = Vec::new();
            let events = egui::Panel::bottom(egui::Id::new(("paper_panel", self.id)))
                .resizable(true)
                .default_size(340.0)
                .min_size(280.0)
                .frame(egui::Frame::new().fill(ui::kit::window()))
                .show(ui, |ui| {
                    let Some((venue, bar)) = self.panel_venue(ui, &mut panel_actions) else {
                        return Vec::new();
                    };
                    let hub = self.trade.hub.clone();
                    let context = self.broker_panel_context();
                    match (venue.broker(), hub, context) {
                        (Some(_), Some(hub), Some(context)) => {
                            let Some(backend) = hub.backend(venue) else {
                                return Vec::new();
                            };
                            let stats = backend.stats();
                            crate::ui::broker::panel::ui(
                                ui,
                                &mut self.trade.broker_panel,
                                &*backend,
                                context,
                                &bar,
                                stats.as_ref(),
                            )
                        }
                        _ => self.trade.paper_panel.ui(ui, &mut lock(paper), &bar),
                    }
                })
                .inner;
            actions.extend(panel_actions);
            for event in events {
                self.apply_panel_event(event, &mut actions);
            }
        }
        actions
    }
}
