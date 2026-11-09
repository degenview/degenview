//! Paper trading on charts: quotes handed to the paper host, portfolio cards and chart orders.

use super::ChartWorkspace;
use super::chart::{Chart, REFRESH_INTERVAL};
use crate::host::paper_host::PaperHost;
use crate::model::DataSource;
use crate::model::paper::{PaperCurrency, PaperInstrument, PaperQuote};
use crate::model::watchlist_quote::{Freshness, evaluate_freshness};
use crate::ui::paper_chart_trading::PaperOverlayAction;
use std::time::{Duration, Instant};

const PAPER_QUOTE_FLUSH_INTERVAL: Duration = Duration::from_millis(500);

impl ChartWorkspace {
    /// Hands each market chart's latest price to the paper host. Runs whether or not the Paper
    /// Trading panel is open, so a working order keeps filling while any chart for its instrument
    /// is on screen. With the panel closed only instruments the host holds an order or a position
    /// on are sent. Nothing runs while the app is quit, asleep or showing no such chart.
    pub(super) fn sync_paper_quotes(&mut self, host: &mut PaperHost) {
        // The simulated account is only made when the user is trading on paper.
        if self.paper_panel_visible && !self.trading_venue.is_live() {
            let _ = host.ensure_default_account();
        }
        let now = Instant::now();
        let wall_clock = crate::model::watchlist::now_ms();
        for chart in &mut self.charts {
            if !chart.is_market() || !Self::supports_paper_trading(chart.source) {
                continue;
            }
            let Some(last) = chart.candles.last().map(|candle| candle.close) else {
                continue;
            };
            let instrument = Self::paper_instrument(chart);
            if !self.paper_panel_visible && !host.tracks(&instrument.key) {
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
            let Some(quote) = paper_quote(
                instrument.key,
                chart.source,
                last,
                chart.last_data_ms,
                wall_clock,
            ) else {
                continue;
            };
            let _ = host.process_quote(quote);
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
            && !self.trading_venue.is_live()
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

    /// Runs what the chart overlay asked for and returns why any of it failed, for the panel to
    /// show: the host keeps only its last error, which a following success would clear.
    pub(super) fn apply_paper_overlay_actions(
        host: &mut PaperHost,
        actions: Vec<PaperOverlayAction>,
    ) -> Vec<String> {
        let mut failures = Vec::new();
        for action in actions {
            let result = match action {
                PaperOverlayAction::ClosePosition {
                    account_id,
                    instrument_key,
                } => host.close_position(account_id, &instrument_key).map(drop),
                PaperOverlayAction::ModifyOrder { order_id, price } => {
                    host.move_order(order_id, price)
                }
                PaperOverlayAction::CancelOrder(id) => host.cancel(id),
            };
            if let Err(error) = result {
                failures.push(error.message());
            }
        }
        if !failures.is_empty() {
            host.clear_error();
        }
        failures
    }
}

/// The quote a chart's last price is worth to the paper engine, or `None` when it must not be
/// traded on. The timestamp is when the price arrived, not when it is read; a price older than
/// its source's window is withheld, and a stock outside US hours is sent as closed so no order
/// fills at yesterday's close.
fn paper_quote(
    instrument_key: String,
    source: DataSource,
    last: f64,
    data_ms: Option<i64>,
    now_ms: i64,
) -> Option<PaperQuote> {
    if !last.is_finite() || last <= 0.0 {
        return None;
    }
    let received_at = data_ms?;
    let is_market_open = match evaluate_freshness(source, received_at, now_ms) {
        Freshness::Live => true,
        Freshness::MarketClosed => false,
        _ => return None,
    };
    Some(PaperQuote {
        instrument_key,
        bid: None,
        ask: None,
        last: Some(last),
        timestamp: received_at,
        is_market_open,
    })
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
    fn quote_carries_the_arrival_time_not_the_read_time() {
        let quote = paper_quote(
            "binance:BTCUSDT".into(),
            DataSource::Binance,
            10.0,
            Some(9_000),
            10_000,
        )
        .expect("fresh price");
        assert_eq!(quote.timestamp, 9_000);
        assert!(quote.is_market_open);
    }

    #[test]
    fn a_price_older_than_its_window_is_withheld() {
        // Binance allows 45 s.
        assert!(
            paper_quote(
                "binance:BTCUSDT".into(),
                DataSource::Binance,
                10.0,
                Some(0),
                46_000
            )
            .is_none()
        );
        assert!(
            paper_quote(
                "binance:BTCUSDT".into(),
                DataSource::Binance,
                10.0,
                None,
                1_000
            )
            .is_none()
        );
    }

    #[test]
    fn a_stock_outside_us_hours_is_sent_as_closed() {
        // 2026-11-01 is a Sunday.
        let sunday = 1_793_491_200_000_i64;
        let quote = paper_quote(
            "alpaca:AAPL".into(),
            DataSource::Alpaca,
            200.0,
            Some(sunday),
            sunday + 1_000,
        )
        .expect("quote");
        assert!(!quote.is_market_open);
        // The same instant on a 24/7 venue is open.
        let crypto = paper_quote(
            "binance:BTCUSDT".into(),
            DataSource::Binance,
            1.0,
            Some(sunday),
            sunday + 1_000,
        )
        .expect("quote");
        assert!(crypto.is_market_open);
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
