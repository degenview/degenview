//! The liquidations panel of a tab: whether it shows, its filter, and drawing it at the right of
//! the window. The sockets are the app's (`host::liquidation`), shared by every tab; a panel that
//! is not drawn stops renewing their lease.

use super::ChartWorkspace;
use crate::host::liquidation::LiquidationHost;
use crate::ui::liquidation::{self, Action, LiquidationsState};
use eframe::egui;

/// What a tab holds of the panel. A tab always starts with it closed, like the Paper panel.
#[derive(Default)]
pub(super) struct LiquidationsPane {
    visible: bool,
    view: LiquidationsState,
    host: Option<LiquidationHost>,
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
    /// Gives the tab the feed every window shares. Cheap to call every frame.
    pub fn set_liquidations(&mut self, host: &LiquidationHost) {
        if self.liquidations.host.is_none() {
            self.liquidations.host = Some(host.clone());
        }
    }

    /// The panel at the right while it is shown.
    pub(super) fn show_liquidations_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let pane = &mut self.liquidations;
        let Some(host) = pane.host.as_ref().filter(|_| pane.visible) else {
            return;
        };
        let snapshot = host.watch(ctx, pane.view.minimum);
        let mut closed = false;
        egui::Panel::right(egui::Id::new(("liquidations", self.id)))
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
                closed = liquidation::show(ui, &mut pane.view, &snapshot) == Some(Action::Close);
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
}
