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
        actions
    }
}
