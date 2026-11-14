//! Price alerts from a chart: the request a bell or menu makes and the editor sheet it opens.

use super::chart::Chart;
use super::{ChartWorkspace, WorkspaceAction};
use crate::model::DataSource;
use crate::model::alert::draft::Direction;
use crate::model::alert::{AlertAsset, AlertCurrency, PriceAlert};
use crate::model::watchlist::now_ms;
use crate::ui::alert::editor::{self, PriceAlertEditorOutcome, PriceAlertEditorState};
use eframe::egui;
use std::collections::HashMap;

/// A new price alert to start: the market, and the price a chart click suggests for it.
pub(super) struct AlertRequest {
    pub(super) asset: AlertAsset,
    /// The clicked price to prefill, when the request came from a chart point.
    pub(super) price: Option<f64>,
    /// The chart's last price, to choose the way the alert crosses.
    pub(super) last: Option<f64>,
}

impl AlertRequest {
    pub(super) fn market(asset: AlertAsset) -> Self {
        Self {
            asset,
            price: None,
            last: None,
        }
    }
}

/// The tab's side of price alerts: the bell's count, the sheet a bell or menu asked for, the sheet
/// that is open, and the FX rates it prices other currencies with.
#[derive(Default)]
pub(super) struct AlertUi {
    /// Alerts fired since the Alerts window was last looked at.
    unseen: usize,
    /// A chart's bell (or a watchlist row's menu) asked for a price alert this frame; the next
    /// `show_alert_editor` opens the sheet on it.
    request: Option<AlertRequest>,
    /// The "Create Price Alert" sheet, over this tab's window.
    editor: Option<PriceAlertEditorState>,
    /// USD → currency rates for the sheet's currency menu, pushed in by the app.
    fx_rates: HashMap<AlertCurrency, f64>,
}

impl AlertUi {
    /// Asks for the sheet on a new alert; it opens with the next frame.
    pub(super) fn request(&mut self, request: AlertRequest) {
        self.request = Some(request);
    }

    /// Opens the sheet on an existing alert, to edit it.
    pub(super) fn edit(&mut self, alert: &PriceAlert) {
        self.editor = Some(PriceAlertEditorState::new(
            alert.asset.clone(),
            Some(alert.clone()),
            Some(alert.currency),
        ));
    }

    /// How many alerts fired unseen, for the bell's badge.
    pub(super) fn unseen(&self) -> usize {
        self.unseen
    }

    /// USD → currency rates, to draw alert levels in their own currency.
    pub(super) fn fx_rates(&self) -> &HashMap<AlertCurrency, f64> {
        &self.fx_rates
    }

    /// How many units of `currency` one USD buys, when known (USD is always 1).
    pub(super) fn usd_rate(&self, currency: AlertCurrency) -> Option<f64> {
        if currency == AlertCurrency::Usd {
            Some(1.0)
        } else {
            self.fx_rates
                .get(&currency)
                .copied()
                .filter(|rate| *rate > 0.0)
        }
    }

    #[cfg(test)]
    pub(super) fn take_request(&mut self) -> Option<AlertRequest> {
        self.request.take()
    }

    #[cfg(test)]
    pub(super) fn editor(&self) -> Option<&PriceAlertEditorState> {
        self.editor.as_ref()
    }
}

impl ChartWorkspace {
    /// How many alerts have fired since the Alerts window was last looked at, for the bell.
    pub fn set_unseen_alerts(&mut self, count: usize) {
        self.alerts.unseen = count;
    }

    /// The app's latest USD → currency rates, for the price-alert sheet.
    pub fn set_alert_fx_rates(&mut self, rates: &HashMap<AlertCurrency, f64>) {
        if self.alerts.fx_rates != *rates {
            self.alerts.fx_rates.clone_from(rates);
        }
    }

    /// The open price-alert sheet is in a currency that needs FX rates.
    pub fn alert_editor_needs_fx_rates(&self) -> bool {
        self.alerts
            .editor
            .as_ref()
            .is_some_and(|editor| editor.draft.needs_fx_rates())
    }

    /// Keeps the open sheet's market priced: it asks for the quote for as long as it is open.
    pub(super) fn show_alert_editor(&mut self, ctx: &egui::Context) -> Vec<WorkspaceAction> {
        let owner = editor::quote_owner(self.id);
        if let Some(request) = self.alerts.request.take() {
            let mut editor = PriceAlertEditorState::new(request.asset, None, None);
            // A chart point prefills the level and the way to cross it; the user still reviews
            // and saves, so nothing is created here.
            if let Some(price) = request.price {
                editor.draft.target = AlertCurrency::Usd.plain_price_text(price);
                editor.draft.direction =
                    match crate::model::chart::context::alert_side(price, request.last) {
                        Some(crate::model::chart::context::AlertSide::Below) => Direction::Below,
                        _ => Direction::Above,
                    };
            }
            self.alerts.editor = Some(editor);
        }
        let Some(state) = self.alerts.editor.as_mut() else {
            return Vec::new();
        };
        self.quote_coordinator.set_demand(
            owner,
            vec![state.draft.asset().instrument.clone()],
            true,
        );
        state.feed_from_book(&self.quote_book, &self.alerts.fx_rates);
        let endpoints = self
            .store
            .lock()
            .map(|store| store.webhook_endpoints_cached())
            .unwrap_or_default();
        let mut actions = Vec::new();
        let mut finished = false;
        match editor::show(ctx, state, &endpoints, now_ms()) {
            Some(PriceAlertEditorOutcome::Save(alert)) => {
                actions.push(WorkspaceAction::SavePriceAlert {
                    alert,
                    baseline: state.draft.current_price(),
                });
                finished = true;
            }
            Some(PriceAlertEditorOutcome::Cancel) => finished = true,
            Some(PriceAlertEditorOutcome::OpenWebhookSettings) => {
                actions.push(WorkspaceAction::OpenWebhookSettings);
            }
            None => {}
        }
        if finished {
            self.alerts.editor = None;
            self.quote_coordinator.release(owner);
        }
        actions
    }

    /// The market a chart's bell starts an alert on: its identity, the ticker and a name.
    pub(super) fn alert_asset(chart: &Chart) -> AlertAsset {
        Self::alert_asset_for(&chart.symbol, chart.source)
    }

    pub fn alert_asset_for(symbol: &str, source: DataSource) -> AlertAsset {
        let ticker = source.base_asset(symbol);
        AlertAsset::new(
            crate::model::InstrumentId::new(source, symbol),
            ticker.clone(),
            ticker,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui;

    #[test]
    fn the_bell_starts_an_alert_on_the_charts_market() {
        let btc = ChartWorkspace::alert_asset_for("BTCUSDT", DataSource::Binance);
        assert_eq!(btc.symbol, "BTC");
        assert_eq!(btc.instrument.symbol, "BTCUSDT");
        assert_eq!(btc.key(), "BINANCE:BTCUSDT");
        let coinbase = ChartWorkspace::alert_asset_for("ETH-USD", DataSource::Coinbase);
        assert_eq!(coinbase.symbol, "ETH");
        assert!(ui::supports_alerts(DataSource::Binance));
        assert!(ui::supports_alerts(DataSource::Alpaca));
        assert!(!ui::supports_alerts(DataSource::CoinMarketCap));
        assert!(!ui::supports_alerts(DataSource::Polymarket));
        assert!(!ui::supports_alerts(DataSource::Kalshi));
    }
}
