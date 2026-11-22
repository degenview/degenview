//! Paper trading on charts: quotes handed to the paper host, portfolio cards and chart orders.

use crate::host::paper::PaperHost;
use crate::model::paper::{PaperCurrency, PaperInstrument, PaperQuote};
use crate::model::watchlist::quote::{Freshness, WatchlistQuote, evaluate_freshness};
use crate::model::{DataSource, InstrumentId};
use crate::ui::paper::chart_trading::PaperOverlayAction;
use crate::workspace::ChartWorkspace;
use crate::workspace::chart::{Chart, REFRESH_INTERVAL};
use std::time::{Duration, Instant};

const PAPER_QUOTE_FLUSH_INTERVAL: Duration = Duration::from_millis(500);

impl ChartWorkspace {
    /// Hands each market chart's latest price to the paper host. Runs whether or not the Paper
    /// Trading panel is open, so a working order keeps filling while any chart for its instrument
    /// is on screen. With the panel closed only instruments the host holds an order or a position
    /// on are sent. Nothing runs while the app is quit, asleep or showing no such chart.
    pub(in crate::workspace) fn sync_paper_quotes(&mut self, host: &mut PaperHost) {
        let now = Instant::now();
        let wall_clock = crate::clock::now_ms();
        let timeframe = self.timeframe;
        let mut snapshot_demand = Vec::new();
        for chart in &mut self.charts {
            if !chart.is_market() || !Self::supports_paper_trading(chart.source) {
                continue;
            }
            let instrument = Self::paper_instrument(chart);
            if !self.trade.panel_visible && !host.tracks(&instrument.key) {
                continue;
            }
            // Bars that may be minutes old cannot price an order: those charts are priced from
            // the market's real-time snapshot instead.
            let quote = if chart.source.bars_are_real_time(timeframe) {
                chart.candles.last().and_then(|candle| {
                    paper_quote(
                        instrument.key,
                        chart.source,
                        candle.close,
                        chart.last_data_ms,
                        wall_clock,
                    )
                })
            } else {
                let market = InstrumentId::new(chart.source, chart.symbol.as_str());
                let snapshot = self.quote_book.get(&market);
                snapshot_demand.push(market);
                snapshot_quote(instrument.key, snapshot.as_ref(), wall_clock)
            };
            let Some(quote) = quote else {
                continue;
            };
            let Some(last) = quote.last else {
                continue;
            };
            let should_send = chart.last_paper_quote.is_none_or(|(price, sent_at)| {
                let elapsed = sent_at.elapsed();
                (price != last && elapsed >= PAPER_QUOTE_FLUSH_INTERVAL)
                    || elapsed >= REFRESH_INTERVAL
            });
            if !should_send {
                continue;
            }
            let _ = host.process_quote(quote);
            chart.last_paper_quote = Some((last, now));
        }
        // Renewed every frame, so it lapses with the workspace (a background tab stops asking).
        self.quote_coordinator.set_sidebar_demand(
            paper_quote_owner(self.id),
            snapshot_demand,
            true,
        );
    }

    pub(in crate::workspace) fn configure_paper_cards(&mut self, host: &PaperHost) {
        let account_id = host.snapshot().selected_account_id;
        let account = account_id.and_then(|id| {
            host.snapshot()
                .accounts
                .iter()
                .find(|account| account.id == id)
        });
        let enabled = self.trade.panel_visible
            && !self.trade.venue.is_live()
            && self.trade.paper_panel.show_trading_on_charts()
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

    pub(in crate::workspace) fn paper_instrument(chart: &Chart) -> PaperInstrument {
        PaperInstrument::chart(
            &chart.symbol,
            chart.source.display_symbol(&chart.symbol),
            chart.source,
        )
    }

    pub(in crate::workspace) fn supports_paper_trading(source: DataSource) -> bool {
        source != DataSource::CoinMarketCap
    }

    /// Runs what the chart overlay asked for and returns why any of it failed, for the panel to
    /// show: the host keeps only its last error, which a following success would clear.
    pub(in crate::workspace) fn apply_paper_overlay_actions(
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
                failures.push(error.to_string());
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
/// The quote-demand owner of workspace `workspace_id`'s paper pricing: clear of the sidebar's (the
/// tab id), the alert sheet's (`1 << 62`) and the symbol picker's (`1 << 61`).
pub(in crate::workspace) fn paper_quote_owner(workspace_id: u64) -> u64 {
    (1 << 60) + workspace_id
}

/// A paper quote from a real-time snapshot of the market (the watchlist quote book), for charts
/// whose own bars may be late. Withheld unless the snapshot is current; the book ages an
/// unrefreshed one into `Stale` itself.
fn snapshot_quote(
    instrument_key: String,
    snapshot: Option<&WatchlistQuote>,
    now_ms: i64,
) -> Option<PaperQuote> {
    let snapshot = snapshot?;
    let last = snapshot
        .last
        .filter(|price| price.is_finite() && *price > 0.0)?;
    let is_market_open = match snapshot.freshness {
        Freshness::Live => true,
        Freshness::MarketClosed => false,
        _ => return None,
    };
    Some(PaperQuote {
        instrument_key,
        bid: None,
        ask: None,
        last: Some(last),
        timestamp: now_ms,
        is_market_open,
    })
}

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
    use crate::model::Timeframe;
    use crate::workspace::chart::displayed_candles;
    use crate::workspace::test_support::{candle, empty_workspace};
    use eframe::egui;
    use std::collections::HashMap;

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

    fn snapshot(last: Option<f64>, freshness: Freshness) -> WatchlistQuote {
        WatchlistQuote {
            last,
            freshness,
            ..WatchlistQuote::default()
        }
    }

    #[test]
    fn a_current_snapshot_prices_a_late_chart_stamped_with_the_read_time() {
        let live = snapshot(Some(200.0), Freshness::Live);
        let quote = snapshot_quote("alpaca:AAPL".into(), Some(&live), 5_000).expect("quote");
        assert_eq!(quote.last, Some(200.0));
        assert_eq!(quote.timestamp, 5_000);
        assert!(quote.is_market_open);
        let closed = snapshot(Some(200.0), Freshness::MarketClosed);
        let quote = snapshot_quote("alpaca:AAPL".into(), Some(&closed), 5_000).expect("quote");
        assert!(!quote.is_market_open);
    }

    #[test]
    fn a_snapshot_that_is_stale_missing_or_unpriced_is_withheld() {
        for quote in [
            Some(snapshot(Some(200.0), Freshness::Stale)),
            Some(snapshot(Some(200.0), Freshness::Unavailable)),
            Some(snapshot(None, Freshness::Live)),
            Some(snapshot(Some(0.0), Freshness::Live)),
            Some(snapshot(Some(f64::NAN), Freshness::Live)),
            None,
        ] {
            assert!(snapshot_quote("alpaca:AAPL".into(), quote.as_ref(), 5_000).is_none());
        }
    }

    /// What the paper host holds for the instrument after the workspace synced its charts.
    fn synced_last(workspace: &mut ChartWorkspace, symbol: &str) -> Option<f64> {
        let mut host = PaperHost::load(workspace.store.clone());
        workspace.trade.panel_visible = true;
        workspace.sync_paper_quotes(&mut host);
        let key = PaperInstrument::chart(symbol, symbol, DataSource::Alpaca).key;
        host.snapshot()
            .quotes
            .get(&key)
            .and_then(|quote| quote.last)
    }

    fn workspace_with_alpaca_chart(timeframe: Timeframe, close: f64) -> ChartWorkspace {
        let mut workspace = empty_workspace(7);
        workspace.timeframe = timeframe;
        let ctx = egui::Context::default();
        workspace.add_chart_for(&InstrumentId::new(DataSource::Alpaca, "AAPL"), &ctx);
        let chart = workspace.charts.last_mut().expect("chart");
        chart.candles = vec![candle(1_000, close)];
        chart.last_data_ms = Some(crate::clock::now_ms());
        workspace
    }

    fn book_quote(workspace: &ChartWorkspace, quote: WatchlistQuote) {
        let key = InstrumentId::new(DataSource::Alpaca, "AAPL").key();
        workspace.quote_book.apply(HashMap::from([(key, quote)]));
    }

    #[test]
    fn a_daily_alpaca_chart_never_prices_paper_with_its_late_close() {
        let mut workspace = workspace_with_alpaca_chart(Timeframe::DAY_1, 150.0);
        assert_eq!(synced_last(&mut workspace, "AAPL"), None);
    }

    #[test]
    fn a_daily_alpaca_chart_is_priced_from_the_real_time_snapshot() {
        let mut workspace = workspace_with_alpaca_chart(Timeframe::DAY_1, 150.0);
        book_quote(&workspace, snapshot(Some(151.5), Freshness::Live));
        assert_eq!(synced_last(&mut workspace, "AAPL"), Some(151.5));
    }

    #[test]
    fn a_stale_snapshot_does_not_price_a_daily_alpaca_chart() {
        let mut workspace = workspace_with_alpaca_chart(Timeframe::DAY_1, 150.0);
        book_quote(&workspace, snapshot(Some(151.5), Freshness::Stale));
        assert_eq!(synced_last(&mut workspace, "AAPL"), None);
    }

    #[test]
    fn an_intraday_alpaca_chart_still_prices_paper_with_its_own_close() {
        let mut workspace = workspace_with_alpaca_chart(Timeframe::HOUR_1, 150.0);
        book_quote(&workspace, snapshot(Some(151.5), Freshness::Live));
        // Weekends and nights read as closed but current: the close still prices the order.
        assert_eq!(synced_last(&mut workspace, "AAPL"), Some(150.0));
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
