//! The order sheet beside the chart: which venue this tab trades on, what the sheet is given, and
//! what its actions do. The sheet itself is `ui::trade::order_sheet`; it knows no venue.

use degenview_brokers::{Decimal, MarketRef, Side};
use eframe::egui;

use super::{ChartWorkspace, WorkspaceAction};
use crate::host::trading_hub::TradingHub;
use crate::model::trading_venue::TradingVenue;
use crate::trading::model::{OrderClass, VenueInfo};
use crate::ui::trade::order_sheet::{self, SheetAction, SheetInput};

impl ChartWorkspace {
    /// The venue this tab trades on: the one it chose while that is still enabled, else the one
    /// used last, else none (the sheet then offers the list to choose from).
    fn resolved_venue(&mut self, hub: &TradingHub) -> Option<TradingVenue> {
        let venue = if hub.is_enabled(self.trading_venue) {
            Some(self.trading_venue)
        } else {
            hub.default_venue()
        };
        if let Some(venue) = venue {
            self.trading_venue = venue;
        }
        venue
    }

    /// Opens the sheet (and the orders under it) with an order started: its side, its type and a
    /// price where something was clicked.
    pub(super) fn start_order(
        &mut self,
        side: Side,
        class: Option<OrderClass>,
        price: Option<Decimal>,
    ) {
        self.trade_sheet_visible = true;
        self.paper_panel_visible = true;
        self.trade_sheet.start(side, class, price);
    }

    pub(super) fn show_trade_sheet(&mut self, ui: &mut egui::Ui) -> Vec<WorkspaceAction> {
        let mut actions = Vec::new();
        let Some(hub) = TradingHub::global() else {
            return actions;
        };
        let all: Vec<VenueInfo> = hub.infos();
        let enabled: Vec<VenueInfo> = all
            .iter()
            .filter(|info| hub.is_enabled(info.venue))
            .cloned()
            .collect();
        let venue = self.resolved_venue(&hub);
        let backend = venue.and_then(|venue| hub.backend(venue));
        let current = venue
            .and_then(|venue| all.iter().find(|info| info.venue == venue))
            .zip(backend.as_deref());
        let market = self.focused_market_chart().map(|chart| MarketRef {
            source: chart.source.watchlist_slug().to_lowercase(),
            symbol: chart.symbol.clone(),
        });
        let confirm_resting = match venue {
            Some(TradingVenue::Broker(id)) => hub.brokers().prefs(id).confirm_resting_orders,
            _ => false,
        };
        let input = SheetInput {
            enabled: &enabled,
            all: &all,
            current,
            market,
            confirm_resting,
            replay: self.replay_blocks_trading(),
        };
        for action in order_sheet::show(ui, &mut self.trade_sheet, &input) {
            self.apply_sheet_action(&hub, &all, action, &mut actions);
        }
        actions
    }

    /// What the sheet's and the panel's venue controls ask for.
    fn apply_sheet_action(
        &mut self,
        hub: &std::sync::Arc<TradingHub>,
        all: &[VenueInfo],
        action: SheetAction,
        actions: &mut Vec<WorkspaceAction>,
    ) {
        match action {
            SheetAction::PickVenue(venue) => {
                self.trading_venue = venue;
                hub.set_last_used(venue);
            }
            SheetAction::AddBroker | SheetAction::OpenSettings => {
                actions.push(WorkspaceAction::OpenBrokerSettings);
            }
            SheetAction::Connect(venue) => {
                let needs_no_key = all
                    .iter()
                    .find(|info| info.venue == venue)
                    .is_some_and(|info| info.needs_no_credentials);
                if needs_no_key {
                    hub.enable(venue);
                    self.trading_venue = venue;
                    hub.set_last_used(venue);
                } else {
                    actions.push(WorkspaceAction::OpenBrokerSettings);
                }
            }
            SheetAction::Close => self.trade_sheet_visible = false,
        }
    }

    /// The top of the trading panel: the venue pill, shared with the sheet. Returns the venue this
    /// tab trades on, or none when nothing is enabled (the list to choose from is drawn instead,
    /// and the panel has nothing else to show).
    pub(super) fn show_panel_venue(
        &mut self,
        ui: &mut egui::Ui,
        actions: &mut Vec<WorkspaceAction>,
    ) -> Option<TradingVenue> {
        let Some(hub) = TradingHub::global() else {
            return Some(TradingVenue::Paper);
        };
        let all: Vec<VenueInfo> = hub.infos();
        let enabled: Vec<VenueInfo> = all
            .iter()
            .filter(|info| hub.is_enabled(info.venue))
            .cloned()
            .collect();
        let venue = self.resolved_venue(&hub);
        let Some(venue) = venue else {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let width = ui.available_width();
                    egui::Frame::new()
                        .inner_margin(egui::Margin::symmetric(24, 0))
                        .show(ui, |ui| {
                            ui.set_width(width - 48.0);
                            if let Some(venue) = crate::ui::trade::connect_list::show(ui, &all) {
                                self.apply_sheet_action(
                                    &hub,
                                    &all,
                                    SheetAction::Connect(venue),
                                    actions,
                                );
                            }
                        });
                });
            return None;
        };
        if let Some((info, backend)) = all
            .iter()
            .find(|info| info.venue == venue)
            .zip(hub.backend(venue))
        {
            let mut picked = None;
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        picked = crate::ui::trade::venue_pill::show(
                            ui,
                            info,
                            &backend.state(),
                            &enabled,
                        );
                    });
                });
            match picked {
                Some(crate::ui::trade::venue_pill::PillAction::Pick(venue)) => {
                    self.apply_sheet_action(&hub, &all, SheetAction::PickVenue(venue), actions);
                }
                Some(crate::ui::trade::venue_pill::PillAction::AddBroker) => {
                    self.apply_sheet_action(&hub, &all, SheetAction::AddBroker, actions);
                }
                None => {}
            }
        }
        Some(venue)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use degenview_brokers::{Env, MemorySecretStore};
    use eframe::egui;

    use crate::host::broker_host::{BrokerHost, StoreJournal};
    use crate::host::paper_host::PaperHost;
    use crate::host::trading_hub::TradingHub;
    use crate::model::trading_venue::TradingVenue;
    use crate::store::{SharedStore, Store};
    use crate::workspace::test_support::empty_workspace;

    /// The app used to hold the paper host's lock for the whole frame, and the paper venue reads
    /// the same host through the trading hub: opening the trading panel locked a mutex its own
    /// thread already held, and the window froze. The frame must run to the end with the sheet and
    /// the orders panel open on paper.
    #[test]
    fn opening_the_trading_panel_on_paper_does_not_wait_on_a_lock_the_frame_holds() {
        let store: SharedStore = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        let brokers = BrokerHost::new(
            Arc::new(MemorySecretStore::new()),
            Arc::new(StoreJournal::new(store.clone())),
            Some(store.clone()),
            Env::default(),
            |config| config.fast(),
        );
        let paper = Arc::new(Mutex::new(PaperHost::load(store.clone())));
        let hub = TradingHub::set_global_for_tests(TradingHub::new(
            brokers,
            paper.clone(),
            Some(store.clone()),
        ));
        // The process-wide hub is the first one a test installed; the frame must lock its paper host.
        let paper = hub.paper_for_tests();
        paper
            .lock()
            .unwrap()
            .ensure_default_account()
            .expect("default account");
        hub.enable(TradingVenue::Paper);

        let (done, finished) = mpsc::channel();
        std::thread::spawn(move || {
            let mut workspace = empty_workspace(1);
            workspace.trading_venue = TradingVenue::Paper;
            workspace.trade_sheet_visible = true;
            workspace.paper_panel_visible = true;
            let ctx = egui::Context::default();
            for _ in 0..3 {
                let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                    workspace.ui(ui, &paper);
                });
                output.textures_delta.clear();
            }
            let _ = done.send(());
        });
        finished
            .recv_timeout(Duration::from_secs(20))
            .expect("the frame never finished: the trading panel waits on a lock the frame holds");
    }
}
