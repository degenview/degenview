//! The market-flow panel of a tab: whether it shows, which mode, its filter, and drawing it at the
//! right of the window. The sockets are the app's (`host::liquidation`, `host::taker_flow`,
//! `host::depth`), shared by every tab; only the mode on show asks for its feed, and a panel that
//! is not drawn stops renewing the lease.

use super::ChartWorkspace;
use crate::host::depth::DepthHost;
use crate::host::liquidation::LiquidationHost;
use crate::host::taker_flow::TakerFlowHost;
use crate::model::depth::{DepthMarket, market_of};
use crate::model::taker_flow::asset_of;
use crate::ui::market_flow::{
    self, Action, Body, BookBody, BookFeed, FlowBody, MarketFlowState, Mode,
};
use eframe::egui;

/// The feeds every window shares, one per mode.
#[derive(Clone)]
struct Hosts {
    liquidations: LiquidationHost,
    taker_flow: TakerFlowHost,
    depth: DepthHost,
}

/// What a tab holds of the panel. A tab always starts with it closed, like the Paper panel.
#[derive(Default)]
pub(super) struct MarketFlowPane {
    visible: bool,
    view: MarketFlowState,
    hosts: Option<Hosts>,
}

impl MarketFlowPane {
    pub(super) fn is_visible(&self) -> bool {
        self.visible
    }

    pub(super) fn toggle(&mut self) {
        self.visible = !self.visible;
    }
}

impl ChartWorkspace {
    /// Gives the tab the feeds every window shares. Cheap to call every frame.
    pub fn set_market_flow(
        &mut self,
        liquidations: &LiquidationHost,
        taker_flow: &TakerFlowHost,
        depth: &DepthHost,
    ) {
        if self.market_flow.hosts.is_none() {
            self.market_flow.hosts = Some(Hosts {
                liquidations: liquidations.clone(),
                taker_flow: taker_flow.clone(),
                depth: depth.clone(),
            });
        }
    }

    /// The asset the focused chart's market is of, when it is a crypto one.
    fn focused_flow_asset(&self) -> Option<String> {
        let chart = self.focused_market_chart()?;
        asset_of(chart.source, &chart.symbol)
    }

    /// The focused chart's market, when its exchange sends an order book.
    fn focused_depth_market(&self) -> Option<DepthMarket> {
        let instrument = self.focused_instrument()?;
        market_of(instrument.source, &instrument.symbol)
    }

    /// The panel at the right while it is shown.
    pub(super) fn show_market_flow_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if !self.market_flow.visible {
            return;
        }
        let mode = self.market_flow.view.mode;
        let asset = (mode == Mode::TakerFlow)
            .then(|| self.focused_flow_asset())
            .flatten();
        let market = (mode == Mode::Book)
            .then(|| self.focused_depth_market())
            .flatten();
        let id = egui::Id::new(("market-flow", self.id));
        let pane = &mut self.market_flow;
        if mode == Mode::Book {
            pane.view.book.follow(market.as_ref());
        }
        let Some(hosts) = pane.hosts.as_ref() else {
            return;
        };
        // Only the mode on show asks for its feed, so only its sockets stay open.
        let closed = match mode {
            Mode::Liquidations => {
                let snapshot = hosts.liquidations.watch(ctx, pane.view.minimum);
                dock(ui, id, &mut pane.view, &Body::Liquidations(&snapshot))
            }
            Mode::TakerFlow => {
                let flow = asset
                    .as_deref()
                    .map(|asset| hosts.taker_flow.watch(ctx, asset));
                let body = Body::TakerFlow(FlowBody {
                    asset: asset.as_deref(),
                    snapshot: flow.as_deref(),
                });
                dock(ui, id, &mut pane.view, &body)
            }
            Mode::Book => {
                let depth = market
                    .as_ref()
                    .map(|market| hosts.depth.watch(ctx, market, pane.view.book.view()));
                let feed = market
                    .as_ref()
                    .zip(depth.as_deref())
                    .map(|(market, snapshot)| BookFeed {
                        venue: market.venue,
                        symbol: &market.symbol,
                        snapshot,
                    });
                dock(ui, id, &mut pane.view, &Body::Book(BookBody { feed }))
            }
        };
        if closed {
            pane.visible = false;
        }
    }
}

/// Draws the panel at the right of the window; whether it was closed.
fn dock(ui: &mut egui::Ui, id: egui::Id, view: &mut MarketFlowState, body: &Body<'_>) -> bool {
    let mut closed = false;
    egui::Panel::right(id)
        .resizable(true)
        .default_size(340.0)
        .min_size(280.0)
        .max_size(560.0)
        .frame(
            egui::Frame::new()
                .fill(crate::ui::theme::chrome())
                .inner_margin(egui::Margin::ZERO),
        )
        .show(ui, |ui| {
            closed = market_flow::show(ui, view, body) == Some(Action::Close);
        });
    closed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panel_starts_closed_and_the_toolbar_toggles_it() {
        let mut pane = MarketFlowPane::default();
        assert!(!pane.is_visible());
        pane.toggle();
        assert!(pane.is_visible());
        pane.toggle();
        assert!(!pane.is_visible());
    }

    #[test]
    fn the_panel_opens_on_the_liquidations() {
        assert_eq!(MarketFlowPane::default().view.mode, Mode::Liquidations);
    }
}
