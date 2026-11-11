//! The paper account, as a trading venue.
//!
//! Reads the paper host (the engine and its store stay as they are: saves, iCloud Sync and the MCP
//! tools all go through it) and turns its `f64` state into the venue vocabulary, rounded to the
//! market's increments.

use std::sync::{Arc, Mutex, MutexGuard};

use degenview_brokers::{Decimal, MarketRef, Side};

use super::backend::TradingBackend;
use super::draft::{
    CancelResult, DraftCheck, DraftField, DraftIssue, Estimate, PreviewResult, SubmitResult,
    Submitted, TradeDraft,
};
use super::model::{
    AccountStats, AccountStrip, Amount, BackendState, Binding, MarketAccess, MarketRules,
    OrderClass, OrderRow, QuoteView, RowStatus, TicketCaps, TimeInForce, Tone, Unavailable,
    UnavailableKind, VenueInfo, VenueKind,
};
use super::task::Task;
use crate::host::paper_host::PaperHost;
use crate::model::DataSource;
use crate::model::broker_ticket::display;
use crate::model::paper::{
    PaperCurrency, PaperInstrument, PaperOrder, PaperOrderRole, PaperOrderSide, PaperOrderStatus,
    PaperOrderType, PaperTimeInForce,
};
use crate::model::paper_ticket::{TicketDraft as PaperTicket, TicketField};
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

pub(crate) fn order_row(order: &PaperOrder) -> OrderRow {
    let working = order.status.is_working();
    OrderRow {
        id: order.id.to_string(),
        market: order.instrument.symbol.clone(),
        side: side(order.side),
        kind: kind_label(order.kind).to_owned(),
        price: order.limit_price.or(order.stop_price).map(dec),
        trigger: order.limit_price.and(order.stop_price).map(dec),
        filled: dec(order.filled_quantity),
        size: Some(dec(order.original_quantity)),
        spend: None,
        status: status(order.status),
        created_ms: Some(order.created_at),
        working,
        can_cancel: working,
        can_edit: working,
        note: order.rejection_reason.clone(),
        edit_hint: None,
        source: Some(order.instrument.source),
        tag: match order.role {
            PaperOrderRole::Entry => None,
            PaperOrderRole::TakeProfit => Some(RowStatus {
                label: order.role.badge_text(),
                tone: Tone::Good,
            }),
            PaperOrderRole::StopLoss => Some(RowStatus {
                label: order.role.badge_text(),
                tone: Tone::Bad,
            }),
        },
        unconfirmed: false,
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

/// The sheet's draft as the paper ticket reads it, with the account's figures filled in: the free
/// funds, the leverage and commission, the open position, and the market's prices.
fn paper_ticket(
    host: &PaperHost,
    market: &MarketRef,
    draft: &TradeDraft,
) -> Option<(PaperTicket, u64)> {
    let instrument = instrument(market).ok()?;
    let snapshot = host.snapshot();
    let account = snapshot
        .selected_account_id
        .and_then(|id| snapshot.accounts.iter().find(|a| a.id == id))
        .or_else(|| snapshot.accounts.first())?;
    let quote = snapshot.quotes.get(&instrument.key);
    let existing = snapshot
        .positions
        .iter()
        .find(|p| p.account_id == account.id && p.instrument.key == instrument.key)
        .map_or(0.0, |p| p.signed_quantity);
    let mut ticket = PaperTicket::new(instrument);
    ticket.leverage = account
        .settings
        .leverage
        .leverage(ticket.instrument.asset_class);
    ticket.commission = account.settings.commission;
    ticket.bid = quote.and_then(|q| q.bid);
    ticket.ask = quote.and_then(|q| q.ask);
    ticket.last = quote.and_then(|q| q.last);
    ticket.available_funds = Some(host.metrics(account.id).available_funds);
    ticket.existing_signed_quantity = existing;
    ticket.side = match draft.side {
        Side::Buy => PaperOrderSide::Buy,
        Side::Sell => PaperOrderSide::Sell,
    };
    ticket.kind = match draft.class {
        OrderClass::Market => PaperOrderType::Market,
        OrderClass::Limit => PaperOrderType::Limit,
        OrderClass::Stop => PaperOrderType::Stop,
        OrderClass::StopLimit => PaperOrderType::StopLimit,
    };
    ticket.time_in_force = match draft.tif {
        TimeInForce::Day => PaperTimeInForce::Day,
        _ => PaperTimeInForce::GoodTilCanceled,
    };
    ticket.quantity_text = draft.size_text.clone();
    ticket.limit_price_text = draft.price_text.clone();
    ticket.stop_price_text = draft.stop_text.clone();
    if draft.bracket {
        ticket.take_profit_text = draft.take_profit_text.clone();
        ticket.stop_loss_text = draft.stop_loss_text.clone();
    }
    Some((ticket, account.id))
}

fn field_of(field: TicketField) -> DraftField {
    match field {
        TicketField::Quantity => DraftField::Size,
        TicketField::LimitPrice => DraftField::Price,
        TicketField::StopPrice => DraftField::StopPrice,
        TicketField::TakeProfit => DraftField::TakeProfit,
        TicketField::StopLoss => DraftField::StopLoss,
    }
}

const NO_ACCOUNT: &str = "There is no paper account yet.";
const ACCOUNT_CHANGED: &str =
    "The account changed since the order was set up. Nothing was sent. Check the order again.";

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

    fn stats(&self) -> Option<AccountStats> {
        let host = self.host();
        let snapshot = host.snapshot();
        let account = snapshot
            .selected_account_id
            .and_then(|id| snapshot.accounts.iter().find(|a| a.id == id))
            .or_else(|| snapshot.accounts.first())?;
        let metrics = host.metrics(account.id);
        Some(super::paper_stats::stats(
            Some(&metrics),
            account.base_currency,
        ))
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
            base_asset: crate::model::paper_ticket::base_symbol(&instrument),
            // What the chart's pair says, so a euro market reads in euros; the engine itself
            // still settles every chart in the account's currency.
            quote_asset: instrument
                .source
                .quote_asset(&instrument.symbol)
                .unwrap_or_else(|| code(instrument.quote_currency).to_owned()),
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

    fn check(&self, market: &MarketRef, draft: &TradeDraft) -> DraftCheck {
        let missing = draft.missing();
        let host = self.host();
        let Some((ticket, _)) = paper_ticket(&host, market, draft) else {
            return DraftCheck {
                issues: vec![DraftIssue::new(DraftField::Order, NO_ACCOUNT)],
                missing,
                estimate: Estimate::default(),
            };
        };
        let mut issues: Vec<DraftIssue> = ticket
            .issues()
            .into_iter()
            .map(|(field, text)| DraftIssue::new(field_of(field), text))
            .collect();
        // The same order of fields every frame.
        issues.sort_by_key(|issue| format!("{:?}", issue.field));
        if let (Some(short), true) = (ticket.shortfall(), ticket.quantity().is_some()) {
            issues.push(DraftIssue::new(
                DraftField::Size,
                format!(
                    "This needs {} more than the {} free.",
                    display(dec(short)),
                    display(dec(ticket.available_funds.unwrap_or_default()))
                ),
            ));
        }
        DraftCheck {
            issues,
            missing,
            estimate: Estimate {
                reference: ticket
                    .valuation_price()
                    .or_else(|| ticket.market_price())
                    .map(dec),
                notional: ticket.notional().map(dec),
                fee: ticket.estimated_fee().map(dec),
                margin: ticket.required_margin().map(dec),
            },
        }
    }

    fn max_size(&self, market: &MarketRef, side: Side, price: Option<Decimal>) -> Option<Decimal> {
        let host = self.host();
        let mut draft = TradeDraft::new(side);
        draft.class = OrderClass::Limit;
        if let Some(price) = price {
            draft.price_text = price.to_string();
        }
        let (mut ticket, _) = paper_ticket(&host, market, &draft)?;
        if price.is_none() {
            ticket.kind = PaperOrderType::Market;
        }
        ticket.quantity_for_funds_fraction(1.0).map(dec)
    }

    fn binding(&self) -> Binding {
        let host = self.host();
        let snapshot = host.snapshot();
        let account = snapshot
            .selected_account_id
            .and_then(|id| snapshot.accounts.iter().find(|a| a.id == id))
            .or_else(|| snapshot.accounts.first());
        Binding {
            account: account.map(|a| a.id.to_string()).unwrap_or_default(),
            generation: 0,
        }
    }

    fn preview(
        &self,
        market: &MarketRef,
        draft: &TradeDraft,
        binding: &Binding,
    ) -> Task<Result<PreviewResult, String>> {
        let host = self.host();
        let Some((ticket, account)) = paper_ticket(&host, market, draft) else {
            return Task::ready(Err(NO_ACCOUNT.to_owned()));
        };
        if binding.account != account.to_string() {
            return Task::ready(Err(ACCOUNT_CHANGED.to_owned()));
        }
        let Some(request) = ticket.request(account) else {
            return Task::ready(Err("The order is not complete.".to_owned()));
        };
        // The engine decides on a throwaway copy: nothing is saved or recorded.
        let answer = match host.check_order(request) {
            Ok(()) => PreviewResult {
                accepted: true,
                total: ticket.notional().map(dec),
                fee: ticket.estimated_fee().map(dec),
                average_price: ticket.market_price().map(dec),
                problems: Vec::new(),
                warnings: Vec::new(),
            },
            Err(error) => PreviewResult {
                accepted: false,
                problems: vec![error.message()],
                ..PreviewResult::default()
            },
        };
        Task::ready(Ok(answer))
    }

    fn submit(
        &self,
        market: &MarketRef,
        draft: &TradeDraft,
        binding: &Binding,
    ) -> Task<SubmitResult> {
        let mut host = self.host();
        let Some((ticket, account)) = paper_ticket(&host, market, draft) else {
            return Task::ready(Err(NO_ACCOUNT.to_owned()));
        };
        // Another paper account was chosen since the order was reviewed: not sent to that one.
        if binding.account != account.to_string() {
            return Task::ready(Err(ACCOUNT_CHANGED.to_owned()));
        }
        let Some(request) = ticket.request(account) else {
            return Task::ready(Err("The order is not complete.".to_owned()));
        };
        Task::ready(
            host.submit(request)
                .map(|id| Submitted { id: id.to_string() })
                .map_err(|error| error.message()),
        )
    }

    fn cancel(&self, order_id: &str) -> Task<CancelResult> {
        let Ok(id) = order_id.parse::<u64>() else {
            return Task::ready(Err("That is not a paper order.".to_owned()));
        };
        Task::ready(self.host().cancel(id).map_err(|error| error.message()))
    }
}
