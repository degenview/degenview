//! The paper account, as a trading venue.
//!
//! Reads the paper host (the engine and its store stay as they are: saves, iCloud Sync and the MCP
//! tools all go through it) and turns its `f64` state into the venue vocabulary, rounded to the
//! market's increments.

use std::sync::{Arc, Mutex, MutexGuard};

use degenview_brokers::{Decimal, MarketRef, Side};

use super::backend::TradingBackend;
use super::model::{
    AccountStrip, Amount, BackendState, BalanceRow, FillRow, MarketAccess, MarketRules, OrderClass,
    OrderRow, PositionRow, QuoteView, RowStatus, TabKind, TicketCaps, TimeInForce, Tone,
    Unavailable, UnavailableKind, VenueInfo, VenueKind,
};
use crate::host::paper_host::PaperHost;
use crate::model::DataSource;
use crate::model::paper::{
    PaperCurrency, PaperFill, PaperInstrument, PaperOrder, PaperOrderSide, PaperOrderStatus,
    PaperOrderType, PaperPosition,
};
use crate::model::trading_venue::TradingVenue;

pub struct PaperBackend {
    host: Arc<Mutex<PaperHost>>,
}

impl PaperBackend {
    pub fn new(host: Arc<Mutex<PaperHost>>) -> Self {
        Self { host }
    }

    fn host(&self) -> MutexGuard<'_, PaperHost> {
        self.host.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// A paper figure as an exact decimal, to the eighth place (the finest increment paper uses).
fn dec(value: f64) -> Decimal {
    if !value.is_finite() {
        return Decimal::ZERO;
    }
    // `f64`'s Display never uses an exponent, and prints the shortest text that reads back as the
    // same number, so `0.1` is `0.1` and not `0.1000000000000000055…`.
    value
        .to_string()
        .parse::<Decimal>()
        .unwrap_or_default()
        .round_dp(8)
}

fn code(currency: PaperCurrency) -> &'static str {
    match currency {
        PaperCurrency::Usd => "USD",
        PaperCurrency::Eur => "EUR",
        PaperCurrency::Gbp => "GBP",
        PaperCurrency::Jpy => "JPY",
        PaperCurrency::Usdt => "USDT",
        PaperCurrency::Usdc => "USDC",
    }
}

fn side(side: PaperOrderSide) -> Side {
    match side {
        PaperOrderSide::Buy => Side::Buy,
        PaperOrderSide::Sell => Side::Sell,
    }
}

fn status(status: PaperOrderStatus) -> RowStatus {
    let (label, tone) = match status {
        PaperOrderStatus::PendingSubmission => ("Pending", Tone::Neutral),
        PaperOrderStatus::Working => ("Open", Tone::Good),
        PaperOrderStatus::PartiallyFilled => ("Partly filled", Tone::Good),
        PaperOrderStatus::Filled => ("Filled", Tone::Good),
        PaperOrderStatus::PendingCancel => ("Cancelling", Tone::Warning),
        PaperOrderStatus::Canceled => ("Cancelled", Tone::Neutral),
        PaperOrderStatus::Rejected => ("Rejected", Tone::Bad),
        PaperOrderStatus::Expired => ("Expired", Tone::Neutral),
    };
    RowStatus { label, tone }
}

fn kind_label(kind: PaperOrderType) -> &'static str {
    match kind {
        PaperOrderType::Market => "Market",
        PaperOrderType::Limit => "Limit",
        PaperOrderType::Stop => "Stop",
        PaperOrderType::StopLimit => "Stop-limit",
    }
}

fn order_row(order: &PaperOrder) -> OrderRow {
    let working = order.status.is_working();
    OrderRow {
        id: order.id.to_string(),
        market: order.instrument.symbol.clone(),
        side: side(order.side),
        kind: kind_label(order.kind).to_owned(),
        price: order.limit_price.or(order.stop_price).map(dec),
        filled: dec(order.filled_quantity),
        size: Some(dec(order.original_quantity)),
        spend: None,
        status: status(order.status),
        created_ms: Some(order.created_at),
        working,
        can_cancel: working,
        can_edit: working,
        note: order.rejection_reason.clone(),
        unconfirmed: false,
    }
}

fn fill_row(fill: &PaperFill) -> FillRow {
    FillRow {
        id: fill.id.to_string(),
        market: fill.instrument.symbol.clone(),
        side: side(fill.side),
        price: dec(fill.price),
        size: dec(fill.quantity),
        fee: dec(fill.commission),
        time_ms: fill.timestamp,
    }
}

fn position_row(host: &PaperHost, position: &PaperPosition) -> PositionRow {
    PositionRow {
        id: format!("{}:{}", position.account_id, position.instrument.key),
        market: position.instrument.symbol.clone(),
        size: dec(position.signed_quantity),
        entry_price: dec(position.average_entry_price),
        mark_price: host.mark(position).map(dec),
        unrealized: Some(dec(host.unrealized_pnl(position))),
        currency: code(position.instrument.quote_currency).to_owned(),
    }
}

/// The market a chart shows as a paper instrument, if paper can trade it.
fn instrument(market: &MarketRef) -> Result<PaperInstrument, Unavailable> {
    let not_here = |text: &str| Unavailable {
        kind: UnavailableKind::NotThisVenue,
        text: text.to_owned(),
    };
    let source = DataSource::from_watchlist_slug(&market.source)
        .ok_or_else(|| not_here("Paper trading has no price for this chart."))?;
    if source == DataSource::CoinMarketCap {
        return Err(not_here(
            "CoinMarketCap charts have no tradable price, so paper trading skips them.",
        ));
    }
    Ok(PaperInstrument::chart(
        market.symbol.clone(),
        market.symbol.clone(),
        source,
    ))
}

impl TradingBackend for PaperBackend {
    fn info(&self) -> VenueInfo {
        VenueInfo {
            venue: TradingVenue::Paper,
            name: "Paper trading".to_owned(),
            kind: VenueKind::Simulated,
            summary: "Practice with pretend money. Nothing leaves this computer.".to_owned(),
            needs_no_credentials: true,
        }
    }

    fn state(&self) -> BackendState {
        let host = self.host();
        if host.load_failed() {
            return BackendState::NeedsAttention(
                "The paper account could not be read from this computer's database.".to_owned(),
            );
        }
        if host.snapshot().accounts.is_empty() {
            BackendState::NotConnected
        } else {
            BackendState::Ready
        }
    }

    fn account(&self) -> Option<AccountStrip> {
        let host = self.host();
        let snapshot = host.snapshot();
        let account = snapshot
            .selected_account_id
            .and_then(|id| snapshot.accounts.iter().find(|a| a.id == id))
            .or_else(|| snapshot.accounts.first())?;
        let metrics = host.metrics(account.id);
        Some(AccountStrip {
            label: account.name.clone(),
            equity: Some(Amount::new(
                dec(metrics.equity),
                code(account.base_currency),
            )),
        })
    }

    fn available(&self, _market: &MarketRef, _side: Side) -> Option<Amount> {
        let host = self.host();
        let snapshot = host.snapshot();
        let account = snapshot
            .selected_account_id
            .and_then(|id| snapshot.accounts.iter().find(|a| a.id == id))
            .or_else(|| snapshot.accounts.first())?;
        let metrics = host.metrics(account.id);
        Some(Amount::new(
            dec(metrics.available_funds),
            code(account.base_currency),
        ))
    }

    fn market(&self, market: &MarketRef) -> MarketAccess {
        let instrument = match instrument(market) {
            Ok(instrument) => instrument,
            Err(reason) => return MarketAccess::Unavailable(reason),
        };
        if self.state() != BackendState::Ready {
            return MarketAccess::Unavailable(Unavailable {
                kind: UnavailableKind::Waiting,
                text: "There is no paper account yet.".to_owned(),
            });
        }
        MarketAccess::Ready(Box::new(MarketRules {
            name: instrument.display_name.clone(),
            base_asset: instrument.symbol.clone(),
            quote_asset: code(instrument.quote_currency).to_owned(),
            price_increment: dec(instrument.tick_size),
            size_increment: dec(instrument.quantity_increment),
            min_size: dec(instrument.minimum_quantity),
            min_notional: Decimal::ZERO,
            caps: TicketCaps {
                order_classes: vec![
                    OrderClass::Market,
                    OrderClass::Limit,
                    OrderClass::Stop,
                    OrderClass::StopLimit,
                ],
                time_in_force: vec![TimeInForce::GoodTilCancelled, TimeInForce::Day],
                post_only: false,
                reduce_only: false,
                brackets: true,
                quote_sized_market: false,
                shorting: true,
                edit_in_place: true,
            },
        }))
    }

    fn quote(&self, market: &MarketRef) -> Option<QuoteView> {
        let instrument = instrument(market).ok()?;
        let host = self.host();
        let quote = host.snapshot().quotes.get(&instrument.key)?;
        Some(QuoteView {
            bid: quote.bid.map(dec),
            ask: quote.ask.map(dec),
            last: quote.last.map(dec),
        })
    }

    fn tabs(&self) -> Vec<TabKind> {
        vec![
            TabKind::Positions,
            TabKind::Orders,
            TabKind::History,
            TabKind::Fills,
        ]
    }

    fn orders(&self) -> Vec<OrderRow> {
        let host = self.host();
        let snapshot = host.snapshot();
        let Some(account) = snapshot
            .selected_account_id
            .or_else(|| snapshot.accounts.first().map(|a| a.id))
        else {
            return Vec::new();
        };
        let mut rows: Vec<OrderRow> = snapshot
            .orders
            .iter()
            .filter(|order| order.account_id == account)
            .map(order_row)
            .collect();
        rows.sort_by_key(|row| std::cmp::Reverse(row.created_ms));
        rows
    }

    fn positions(&self) -> Vec<PositionRow> {
        let host = self.host();
        let snapshot = host.snapshot();
        let Some(account) = snapshot
            .selected_account_id
            .or_else(|| snapshot.accounts.first().map(|a| a.id))
        else {
            return Vec::new();
        };
        snapshot
            .positions
            .iter()
            .filter(|position| position.account_id == account && position.signed_quantity != 0.0)
            .map(|position| position_row(&host, position))
            .collect()
    }

    fn fills(&self) -> Vec<FillRow> {
        let host = self.host();
        let snapshot = host.snapshot();
        let Some(account) = snapshot
            .selected_account_id
            .or_else(|| snapshot.accounts.first().map(|a| a.id))
        else {
            return Vec::new();
        };
        let mut fills: Vec<FillRow> = snapshot
            .fills
            .iter()
            .filter(|fill| fill.account_id == account)
            .map(fill_row)
            .collect();
        fills.sort_by_key(|fill| std::cmp::Reverse(fill.time_ms));
        fills
    }

    fn balances(&self) -> Vec<BalanceRow> {
        Vec::new()
    }

    fn revision(&self) -> u64 {
        self.host().revision()
    }
}
