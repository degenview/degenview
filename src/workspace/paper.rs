//! Paper trading on charts: quotes handed to the paper host, portfolio cards and chart orders.

use super::ChartWorkspace;
use super::chart::{Chart, REFRESH_INTERVAL};
use crate::model::DataSource;
use crate::model::paper::{PaperCurrency, PaperInstrument, PaperQuote};
use crate::paper_host::PaperHost;
use crate::ui::paper_chart_trading::PaperOverlayAction;
use std::time::{Duration, Instant};

const PAPER_QUOTE_FLUSH_INTERVAL: Duration = Duration::from_millis(500);

impl ChartWorkspace {
    pub(super) fn sync_paper_quotes(&mut self, host: &mut PaperHost) {
        if !self.paper_panel_visible {
            return;
        }
        let _ = host.ensure_default_account();
        let now = Instant::now();
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_millis() as i64);
        for chart in &mut self.charts {
            if !chart.is_market() || !Self::supports_paper_trading(chart.source) {
                continue;
            }
            let Some(last) = chart.candles.last().map(|candle| candle.close) else {
                continue;
            };
            if !last.is_finite() || last <= 0.0 {
                continue;
            }
            let should_send = chart.last_paper_quote.is_none_or(|(price, sent_at)| {
                let elapsed = sent_at.elapsed();
                (price != last && elapsed >= PAPER_QUOTE_FLUSH_INTERVAL)
                    || elapsed >= REFRESH_INTERVAL
            });
            if !should_send {
                continue;
            }
            let instrument = Self::paper_instrument(chart);
            let _ = host.process_quote(PaperQuote {
                instrument_key: instrument.key,
                bid: None,
                ask: None,
                last: Some(last),
                timestamp,
                is_market_open: true,
            });
            chart.last_paper_quote = Some((last, now));
        }
    }

    pub(super) fn configure_paper_cards(&mut self, host: &PaperHost) {
        let account_id = host.snapshot().selected_account_id;
        let account = account_id.and_then(|id| {
            host.snapshot()
                .accounts
                .iter()
                .find(|account| account.id == id)
        });
        let enabled = self.paper_panel_visible
            && self.paper_panel.show_trading_on_charts()
            && account.is_some();
        let currency = account.map_or(PaperCurrency::Usd, |account| account.base_currency);
        for chart in &mut self.charts {
            let key = Self::paper_instrument(chart).key;
            let chart_enabled =
                enabled && chart.is_market() && Self::supports_paper_trading(chart.source);
            chart.chart.paper.trading_enabled = chart_enabled;
            chart.chart.paper.show_markers = chart_enabled;
            chart.chart.paper.currency = currency;
            chart.chart.paper.positions = host
                .snapshot()
                .positions
                .iter()
                .filter(|position| {
                    account.is_some_and(|account| account.settings.show_positions_on_chart)
                        && Some(position.account_id) == account_id
                        && position.instrument.key == key
                })
                .map(|position| (position.clone(), host.unrealized_pnl(position)))
                .collect();
            chart.chart.paper.orders = host
                .snapshot()
                .orders
                .iter()
                .filter(|order| {
                    account.is_some_and(|account| account.settings.show_orders_on_chart)
                        && Some(order.account_id) == account_id
                        && order.instrument.key == key
                        && order.status.is_working()
                })
                .cloned()
                .collect();
        }
    }

    pub(super) fn paper_instrument(chart: &Chart) -> PaperInstrument {
        PaperInstrument::chart(
            &chart.symbol,
            chart.source.display_symbol(&chart.symbol),
            chart.source,
        )
    }

    pub(super) fn supports_paper_trading(source: DataSource) -> bool {
        source != DataSource::CoinMarketCap
    }

    pub(super) fn apply_paper_overlay_actions(
        host: &mut PaperHost,
        actions: Vec<PaperOverlayAction>,
    ) {
        for action in actions {
            match action {
                PaperOverlayAction::ClosePosition {
                    account_id,
                    instrument_key,
                } => {
                    let _ = host.close_position(account_id, &instrument_key);
                }
                PaperOverlayAction::ModifyOrder { order_id, price } => {
                    let _ = host.move_order(order_id, price);
                }
                PaperOverlayAction::CancelOrder(id) => {
                    let _ = host.cancel(id);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::chart::displayed_candles;
    use crate::workspace::test_support::candle;

    #[test]
    fn coinmarketcap_index_cards_are_not_paper_markets() {
        assert!(!ChartWorkspace::supports_paper_trading(
            DataSource::CoinMarketCap
        ));
        assert!(ChartWorkspace::supports_paper_trading(DataSource::Binance));
        assert!(ChartWorkspace::supports_paper_trading(DataSource::Alpaca));
        assert!(ChartWorkspace::supports_paper_trading(
            DataSource::Polymarket
        ));
    }

    #[test]
    fn paper_ticket_reference_follows_the_replay_slice() {
        let candles = vec![
            candle(1_000, 10.0),
            candle(2_000, 20.0),
            candle(3_000, 30.0),
        ];
        assert_eq!(
            displayed_candles(&candles, None).last().unwrap().close,
            30.0
        );
        assert_eq!(
            displayed_candles(&candles, Some(2_500))
                .last()
                .unwrap()
                .close,
            20.0
        );
        assert!(displayed_candles(&candles, Some(500)).is_empty());
    }
}
