//! The market-flow panel of a tab: whether it shows, which mode, its filter, and drawing it at the
//! right of the window. The sockets are the app's (`host::liquidation`, `host::taker_flow`),
//! shared by every tab; only the mode on show asks for its feed, and a panel that is not drawn
//! stops renewing the lease.

use super::ChartWorkspace;
use crate::host::liquidation::LiquidationHost;
use crate::host::taker_flow::TakerFlowHost;
use crate::model::taker_flow::asset_of;
use crate::ui::liquidation::{self, Action, Body, FlowBody, LiquidationsState, Mode};
use eframe::egui;

/// What a tab holds of the panel. A tab always starts with it closed, like the Paper panel.
#[derive(Default)]
pub(super) struct LiquidationsPane {
    visible: bool,
    view: LiquidationsState,
    hosts: Option<(LiquidationHost, TakerFlowHost)>,
}

impl LiquidationsPane {
    pub(super) fn is_visible(&self) -> bool {
        self.visible
    }

    pub(super) fn toggle(&mut self) {
        self.visible = !self.visible;
    }
}

impl ChartWorkspace {
    /// Gives the tab the feeds every window shares. Cheap to call every frame.
    pub fn set_liquidations(&mut self, liquidations: &LiquidationHost, taker_flow: &TakerFlowHost) {
        if self.liquidations.hosts.is_none() {
            self.liquidations.hosts = Some((liquidations.clone(), taker_flow.clone()));
        }
    }

    /// The asset the focused chart's market is of, when it is a crypto one.
    fn focused_flow_asset(&self) -> Option<String> {
        let chart = self.focused_market_chart()?;
        asset_of(chart.source, &chart.symbol)
    }

    /// The panel at the right while it is shown.
    pub(super) fn show_liquidations_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if !self.liquidations.visible {
            return;
        }
        let asset = (self.liquidations.view.mode == Mode::TakerFlow)
            .then(|| self.focused_flow_asset())
            .flatten();
        let id = egui::Id::new(("liquidations", self.id));
        let pane = &mut self.liquidations;
        let Some((liquidations, taker_flow)) = pane.hosts.as_ref() else {
            return;
        };
        // Only the mode on show keeps its sockets open.
        let (liquidated, flow) = match pane.view.mode {
            Mode::Liquidations => (Some(liquidations.watch(ctx, pane.view.minimum)), None),
            Mode::TakerFlow => (
                None,
                asset.as_deref().map(|asset| taker_flow.watch(ctx, asset)),
            ),
        };
        let body = match (&liquidated, pane.view.mode) {
            (Some(snapshot), _) => Body::Liquidations(snapshot),
            (None, _) => Body::TakerFlow(FlowBody {
                asset: asset.as_deref(),
                snapshot: flow.as_deref(),
            }),
        };
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
                closed = liquidation::show(ui, &mut pane.view, &body) == Some(Action::Close);
            });
        if closed {
            pane.visible = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panel_starts_closed_and_the_toolbar_toggles_it() {
        let mut pane = LiquidationsPane::default();
        assert!(!pane.is_visible());
        pane.toggle();
        assert!(pane.is_visible());
        pane.toggle();
        assert!(!pane.is_visible());
    }

    #[test]
    fn the_panel_opens_on_the_liquidations() {
        assert_eq!(LiquidationsPane::default().view.mode, Mode::Liquidations);
    }
}
