//! Stateful bridge between the app-wide portfolio host and the portfolio cards on chart grids.
//! A card paints a `CardData`; this gathers it the way the Portfolio tab does (prices, holdings,
//! history) for the card's own scope, and keeps each card's price demand and feeds.
//!
//! The ledger and quote feed are app-wide; only the per-card selection and demand live here, one
//! `CardRuntime` per card. A runtime releases its demand when the card goes away.

use crate::host::portfolio_host::{PortfolioHost, local_offset, now_ms};
use crate::model::portfolio::{Portfolio, PortfolioAsset, PortfolioLedgerSnapshot};
use crate::model::portfolio_accounting::holdings;
use crate::model::portfolio_card::{CardData, CardKind, Performance, PortfolioCard, performance};
use crate::model::portfolio_format::PortfolioValueFormatter;
use crate::model::portfolio_statistics::{
    PortfolioHistoryRange, PortfolioPeriodChange, PortfolioStatistics,
};
use crate::service::portfolio_history::{IntradayFeed, IntradayRequest};
use crate::service::portfolio_performers::{MoveRequest, PerformerFeed};
use crate::service::portfolio_quotes::PortfolioQuoteFeed;
use eframe::egui;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Quote-demand owners for cards, clear of tab ids, `alert_editor::quote_owner` and the alert
/// runtime's own.
static NEXT_OWNER: AtomicU64 = AtomicU64::new(1 << 61);

/// What every workspace needs to show portfolio cards; cheap to clone.
#[derive(Clone)]
pub struct PortfolioCardEnv {
    host: Arc<Mutex<PortfolioHost>>,
    quotes: PortfolioQuoteFeed,
    performers: PerformerFeed,
}

/// One card's own feeds and price demand.
pub struct CardRuntime {
    owner: u64,
    feed: PortfolioQuoteFeed,
    intraday: IntradayFeed,
}

impl Drop for CardRuntime {
    fn drop(&mut self) {
        self.feed.release(self.owner);
    }
}

impl PortfolioCardEnv {
    pub fn new(host: Arc<Mutex<PortfolioHost>>, quotes: PortfolioQuoteFeed) -> Self {
        Self {
            host,
            quotes,
            performers: PerformerFeed::default(),
        }
    }

    pub fn runtime(&self) -> CardRuntime {
        CardRuntime {
            owner: NEXT_OWNER.fetch_add(1, Ordering::Relaxed),
            feed: self.quotes.sibling(),
            intraday: IntradayFeed::default(),
        }
    }

    /// The portfolios a card can be scoped to, in the Portfolio tab's order.
    pub fn portfolios(&self) -> Vec<Portfolio> {
        self.host.lock().map_or_else(
            |_| Vec::new(),
            |host| {
                host.snapshot()
                    .portfolios
                    .iter()
                    .filter(|portfolio| !portfolio.is_archived)
                    .cloned()
                    .collect()
            },
        )
    }

    /// Everything `card` paints this frame. Starts whatever fetches are missing and asks for a
    /// repaint while prices settle.
    pub fn view(
        &self,
        runtime: &CardRuntime,
        card: &PortfolioCard,
        ctx: &egui::Context,
    ) -> CardData {
        let now = now_ms();
        let offset = local_offset();
        let Ok(mut host) = self.host.lock() else {
            return unavailable(card, now, offset);
        };
        let snapshot = host.snapshot().clone();
        let selected = card.portfolio_id.filter(|id| {
            snapshot
                .portfolios
                .iter()
                .any(|portfolio| portfolio.id == *id)
        });
        let scope = scope_of(&snapshot, selected);
        if scope.is_empty() {
            return CardData {
                has_portfolios: false,
                error: None,
                ..unavailable(card, now, offset)
            };
        }
        let currency = host.reporting_currency_for(selected);
        let privacy = host.privacy_mode();
        let formatter = PortfolioValueFormatter::new(currency, privacy);

        runtime
            .feed
            .update(runtime.owner, &snapshot, &scope, currency, true, now);
        if card.kind == CardKind::ValueChart {
            host.drive_history(ctx, &scope, now, offset);
        }
        let loading_history = host.is_rebuilding_history(&scope);
        drop(host);

        let state = runtime.feed.state();
        let projected = state.projection.as_ref();
        if projected.is_none() || state.is_updating {
            // Nothing else wakes the frame while a worker is still settling the prices.
            ctx.request_repaint_after(Duration::from_millis(400));
        }
        let transactions = projected.map_or_else(
            || {
                snapshot
                    .transactions
                    .iter()
                    .filter(|transaction| scope.contains(&transaction.portfolio_id))
                    .cloned()
                    .collect()
            },
            |projection| projection.transactions.clone(),
        );
        let quotes = projected.map_or_else(HashMap::new, |projection| projection.quotes.clone());
        let holding_values =
            holdings(&transactions, &scope, None, &quotes, now).unwrap_or_default();
        let total_value: f64 = holding_values
            .iter()
            .filter_map(|holding| holding.current_value)
            .sum();
        let daily: Vec<_> =
            projected.map_or_else(Vec::new, |projection| projection.history.clone());

        let history = match card.kind {
            CardKind::ValueChart => {
                let intraday = if card.range == PortfolioHistoryRange::OneDay && projected.is_some()
                {
                    let request = IntradayRequest {
                        portfolios: snapshot
                            .portfolios
                            .iter()
                            .filter(|portfolio| scope.contains(&portfolio.id))
                            .map(|portfolio| (portfolio.id, portfolio.base_currency))
                            .collect(),
                        reporting_currency: currency,
                        transactions: transactions.clone(),
                    };
                    runtime
                        .intraday
                        .points(ctx, &request, &runtime.feed.fx_rates(), now)
                } else {
                    Vec::new()
                };
                if intraday.is_empty() {
                    card.range.filter(&daily, now)
                } else {
                    intraday
                }
            }
            // The big number draws a month of context behind it.
            CardKind::Value => PortfolioHistoryRange::OneMonth.filter(&daily, now),
            CardKind::Allocation | CardKind::TopPerformers => Vec::new(),
        };
        let day_change = (card.kind == CardKind::Value)
            .then(|| PortfolioStatistics::new(&holding_values, &daily, &transactions).day_change)
            .flatten();
        let period_change = (card.kind == CardKind::ValueChart)
            .then(|| PortfolioPeriodChange::new(&history, total_value))
            .flatten();

        let (performance, loading_performance) = if card.kind == CardKind::TopPerformers {
            let requests = move_requests(&holding_values, &transactions, card.range, now);
            let moves = self.performers.moves(ctx, &requests, now);
            if moves.loading {
                ctx.request_repaint_after(Duration::from_millis(400));
            }
            (performance(&holding_values, &moves.moves), moves.loading)
        } else {
            (Performance::default(), false)
        };

        CardData {
            has_portfolios: true,
            scope_name: card.scope_name(&snapshot.portfolios),
            unpriced: projected.map_or(0, |projection| projection.unpriced_asset_count),
            holdings: holding_values,
            history,
            total_value,
            day_change,
            period_change,
            formatter,
            loading: projected.is_none(),
            loading_history,
            error: state.error,
            performance,
            loading_performance,
            now_ms: now,
            utc_offset_secs: offset,
        }
    }
}

/// The portfolios a card covers: its own if it still exists, else every unarchived one.
fn scope_of(snapshot: &PortfolioLedgerSnapshot, selected: Option<u64>) -> HashSet<u64> {
    match selected {
        Some(id) => [id].into_iter().collect(),
        None => snapshot
            .portfolios
            .iter()
            .filter(|portfolio| !portfolio.is_archived)
            .map(|portfolio| portfolio.id)
            .collect(),
    }
}

/// Where each held asset's comparison starts: the range's start, or the day it was first
/// acquired if that is later, so a recent buy is judged from its purchase.
fn move_requests(
    holdings: &[crate::model::portfolio::PortfolioHolding],
    transactions: &[crate::model::portfolio::PortfolioTransaction],
    range: PortfolioHistoryRange,
    now_ms: i64,
) -> Vec<MoveRequest> {
    let mut first: HashMap<&str, i64> = HashMap::new();
    for transaction in transactions
        .iter()
        .filter(|transaction| transaction.kind.adds_quantity())
    {
        first
            .entry(transaction.asset.key.as_str())
            .and_modify(|earliest| *earliest = (*earliest).min(transaction.timestamp))
            .or_insert(transaction.timestamp);
    }
    let range_start = range.duration_ms().map(|duration| now_ms - duration);
    holdings
        .iter()
        .filter(|holding| holding.quantity > 0.0)
        .filter_map(|holding| {
            let acquired = first.get(holding.asset.key.as_str()).copied();
            let since_ms = match (range_start, acquired) {
                (Some(start), Some(acquired)) => start.max(acquired),
                (Some(start), None) => start,
                (None, Some(acquired)) => acquired,
                (None, None) => return None,
            };
            Some(MoveRequest {
                asset: holding.asset.clone() as PortfolioAsset,
                since_ms,
            })
        })
        .collect()
}

/// A card with nothing to show (the ledger is unreachable, or there is no portfolio).
fn unavailable(card: &PortfolioCard, now_ms: i64, utc_offset_secs: i64) -> CardData {
    CardData {
        has_portfolios: true,
        scope_name: card.scope_name(&[]),
        holdings: Vec::new(),
        history: Vec::new(),
        total_value: 0.0,
        day_change: None,
        period_change: None,
        formatter: PortfolioValueFormatter::new(
            crate::model::portfolio::PortfolioCurrency::Usd,
            false,
        ),
        loading: false,
        loading_history: false,
        error: Some("The portfolio database is unavailable.".to_owned()),
        unpriced: 0,
        performance: Performance::default(),
        loading_performance: false,
        now_ms,
        utc_offset_secs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::portfolio::{
        PortfolioCurrency, PortfolioHolding, PortfolioTransaction, PortfolioTransactionType,
    };

    const DAY: i64 = 86_400_000;

    fn asset(symbol: &str) -> PortfolioAsset {
        PortfolioAsset {
            key: format!("binance:{symbol}USDT"),
            symbol: format!("{symbol}/USDT"),
            name: symbol.to_owned(),
            source: DataSource::Binance,
            quote_currency: PortfolioCurrency::Usd,
            metadata: HashMap::new(),
        }
    }

    fn holding(symbol: &str) -> PortfolioHolding {
        PortfolioHolding {
            asset: asset(symbol),
            quantity: 1.0,
            cost_basis: 0.0,
            average_cost: 0.0,
            realized_pnl: 0.0,
            current_price: Some(1.0),
            previous_day_price: None,
            current_value: Some(1.0),
            unrealized_pnl: None,
            allocation: 1.0,
            is_price_stale: false,
        }
    }

    fn buy(symbol: &str, timestamp: i64) -> PortfolioTransaction {
        PortfolioTransaction {
            id: timestamp as u64,
            portfolio_id: 1,
            asset: asset(symbol),
            kind: PortfolioTransactionType::Buy,
            quantity: 1.0,
            price: Some(1.0),
            price_currency: PortfolioCurrency::Usd,
            fee: 0.0,
            fee_currency: PortfolioCurrency::Usd,
            timestamp,
            notes: String::new(),
            source: Default::default(),
            external_transaction_id: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn a_recent_buy_is_judged_from_its_purchase() {
        let now = 100 * DAY;
        let requests = move_requests(
            &[holding("BTC"), holding("ETH")],
            &[buy("BTC", now - 90 * DAY), buy("ETH", now - 2 * DAY)],
            PortfolioHistoryRange::OneMonth,
            now,
        );
        let since = |key: &str| {
            requests
                .iter()
                .find(|request| request.asset.key == key)
                .map(|request| request.since_ms)
        };
        assert_eq!(since("binance:BTCUSDT"), Some(now - 31 * DAY));
        assert_eq!(since("binance:ETHUSDT"), Some(now - 2 * DAY));
    }

    #[test]
    fn all_time_starts_at_the_first_purchase() {
        let now = 100 * DAY;
        let requests = move_requests(
            &[holding("BTC")],
            &[buy("BTC", now - 50 * DAY), buy("BTC", now - 10 * DAY)],
            PortfolioHistoryRange::All,
            now,
        );
        assert_eq!(requests[0].since_ms, now - 50 * DAY);
    }

    #[test]
    fn a_missing_portfolio_falls_back_to_every_active_one() {
        let mut snapshot = PortfolioLedgerSnapshot::default();
        snapshot.portfolios.push(Portfolio::new(1, "A", 0));
        let mut archived = Portfolio::new(2, "B", 0);
        archived.is_archived = true;
        snapshot.portfolios.push(archived);
        assert_eq!(scope_of(&snapshot, None), HashSet::from([1]));
        assert_eq!(scope_of(&snapshot, Some(2)), HashSet::from([2]));
    }
}
