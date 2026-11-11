//! A connected broker, as a trading venue.
//!
//! Reads the `degenview-brokers` crate's snapshots through [`BrokerHost`] and turns them into the
//! venue vocabulary. It names no broker: what a broker is comes from its descriptor.

use std::sync::Arc;

use degenview_brokers::model::orders::CancelOutcome;
use degenview_brokers::model::reducer::{OrderView, OverlayKind};
use degenview_brokers::{
    AccountId, Broker, BrokerSnapshot, Capabilities, Decimal, MarketRef, OrderId, OrderKind,
    OrderSize, OrderType, PendingState, PendingView, ProductResolution, ProductSpec, Quote, Side,
    Status, TifKind,
};

use super::backend::TradingBackend;
use super::draft::{
    CancelResult, DraftCheck, DraftField, DraftIssue, Estimate, PreviewResult, SubmitResult,
    Submitted, TradeDraft, floor_to_increment,
};
use super::model::{
    AccountStats, AccountStrip, Amount, BackendState, Binding, Hero, HeroNote, MarketAccess,
    MarketRules, OrderClass, OrderRow, QuoteView, RowStatus, StatCell, StatTone, TicketCaps,
    TimeInForce, Tone, Trend, Unavailable, UnavailableKind, VenueInfo, VenueKind,
};
use super::task::Task;
use super::valuation::{self, Holding};
use crate::host::broker_host::{BrokerHost, Connection};
use crate::model::broker;
use crate::model::broker::chart::ChartTrading;
use crate::model::broker::ticket::{
    self as rules, StatusTone, TicketContext, TicketDraft, TicketField, TicketKind, TicketTif,
};
use crate::model::broker::venue::TradingVenue;

pub struct BrokerBackend {
    host: Arc<BrokerHost>,
    broker: &'static dyn Broker,
}

impl BrokerBackend {
    pub fn new(host: Arc<BrokerHost>, broker: &'static dyn Broker) -> Self {
        Self { host, broker }
    }

    fn id(&self) -> degenview_brokers::BrokerId {
        self.broker.descriptor().id
    }

    fn snapshot(&self) -> Option<Arc<BrokerSnapshot>> {
        self.host.snapshot(self.id())
    }

    /// The connection `binding` names, if it is still the one running. The crate's worker checks
    /// the account again on its own thread: this refuses early and says why.
    fn runtime_for(
        &self,
        binding: &Binding,
    ) -> Result<Arc<degenview_brokers::BrokerRuntime>, String> {
        let Some(runtime) = self.host.runtime(self.id()) else {
            return Err("The broker is not connected.".to_owned());
        };
        if binding.account.is_empty() || runtime.generation() != binding.generation {
            return Err("The connection changed since the order was set up. Nothing was sent. Check the order again.".to_owned());
        }
        Ok(runtime)
    }

    fn market_of(&self, market: &MarketRef) -> Option<Market> {
        let snapshot = self.snapshot()?;
        let product = self.product(market)?;
        let spec = snapshot.product(&product)?.clone();
        let caps = snapshot.capabilities_of(&product)?.clone();
        let quote = self.host.quote(self.id(), &product);
        Some(Market {
            snapshot,
            spec,
            caps,
            quote,
        })
    }

    fn product(&self, market: &MarketRef) -> Option<degenview_brokers::ProductId> {
        match self.broker.resolve_market(market) {
            ProductResolution::Market(product) => Some(product),
            _ => None,
        }
    }
}

fn tone(tone: StatusTone) -> Tone {
    match tone {
        StatusTone::Good => Tone::Good,
        StatusTone::Warning => Tone::Warning,
        StatusTone::Bad => Tone::Bad,
        StatusTone::Neutral => Tone::Neutral,
    }
}

fn state_of(snapshot: &BrokerSnapshot) -> BackendState {
    match &snapshot.status {
        Status::Connecting | Status::Syncing => BackendState::Connecting,
        Status::Live if snapshot.journal_error.is_some() => BackendState::Paused(
            broker::chart::Unavailable::JournalUnreadable
                .text()
                .to_owned(),
        ),
        Status::Live => BackendState::Ready,
        Status::Reconnecting { reason, .. } => {
            BackendState::Paused(format!("Reconnecting ({reason}). New orders are paused."))
        }
        Status::Failed(error) => BackendState::NeedsAttention(error.to_string()),
        Status::Stopped => BackendState::NotConnected,
    }
}

fn unavailable(reason: broker::chart::Unavailable) -> Unavailable {
    use broker::chart::Unavailable as Why;
    let kind = match reason {
        Why::NotAMarket | Why::OtherSource | Why::UnknownMarket | Why::MarketNotOffered => {
            UnavailableKind::NotThisVenue
        }
        Why::NotConnected | Why::NotLive | Why::RulesNotRead => UnavailableKind::Waiting,
        _ => UnavailableKind::Blocked,
    };
    Unavailable {
        kind,
        text: reason.text().to_owned(),
    }
}

fn caps_of(caps: &Capabilities) -> TicketCaps {
    TicketCaps {
        order_classes: caps
            .order_types
            .iter()
            .filter_map(|kind| match kind {
                OrderType::Market => Some(OrderClass::Market),
                OrderType::Limit => Some(OrderClass::Limit),
                OrderType::StopLimit => Some(OrderClass::StopLimit),
                OrderType::Other => None,
            })
            .collect(),
        time_in_force: caps
            .tif
            .iter()
            .filter_map(|tif| match tif {
                TifKind::GoodTilCancelled => Some(TimeInForce::GoodTilCancelled),
                TifKind::ImmediateOrCancel => Some(TimeInForce::ImmediateOrCancel),
                TifKind::FillOrKill => Some(TimeInForce::FillOrKill),
                TifKind::GoodTilDate => None,
            })
            .collect(),
        post_only: caps.post_only,
        reduce_only: false,
        brackets: caps.brackets,
        quote_sized_market: caps.quote_sized_market,
        shorting: false,
        edit_in_place: caps.native_edit || caps.atomic_replace,
    }
}

/// The data source whose logo the broker's markets wear.
fn source_of(broker: degenview_brokers::BrokerId) -> Option<crate::model::DataSource> {
    let slug = degenview_brokers::registry::by_id(broker)?
        .descriptor()
        .market_source;
    crate::model::DataSource::from_watchlist_slug(slug)
}

fn order_row(view: &OrderView, snapshot: &BrokerSnapshot) -> OrderRow {
    let order = &view.order;
    let working = order.status.is_working();
    let (label, status_tone) = match view.overlay.map(|overlay| overlay.kind) {
        Some(OverlayKind::CancelRequested) if working => ("Cancel requested", StatusTone::Warning),
        Some(OverlayKind::EditRequested { .. }) if working => {
            ("Edit requested", StatusTone::Warning)
        }
        _ => rules::status_label(order.status),
    };
    let caps = snapshot.capabilities_of(&order.product);
    let (size, spend) = match order.size {
        Some(OrderSize::Base(size)) => (Some(size), None),
        Some(OrderSize::Quote(amount)) => (None, Some(amount)),
        None => (None, None),
    };
    OrderRow {
        id: order.id.as_str().to_owned(),
        market: order.product.as_str().to_owned(),
        side: order.side,
        kind: rules::type_label(order),
        price: order.limit_price.or(order.stop_price),
        trigger: order.limit_price.and(order.stop_price),
        filled: order.filled_size,
        size,
        spend,
        status: RowStatus {
            label,
            tone: tone(status_tone),
        },
        created_ms: order.created_ms,
        working,
        can_cancel: working
            && view.overlay.is_none()
            && order.status != degenview_brokers::OrderStatus::CancelQueued,
        can_edit: rules::edit_blocker(order, caps, view.overlay.is_some()).is_none(),
        note: order.reason.clone(),
        edit_hint: if working {
            rules::edit_blocker(order, caps, view.overlay.is_some()).map(str::to_owned)
        } else {
            None
        },
        source: source_of(snapshot.broker),
        tag: None,
        unconfirmed: false,
    }
}

/// Every order the broker knows of: those being sent first, then the rest newest first.
pub(crate) fn order_rows(snapshot: &BrokerSnapshot) -> Vec<OrderRow> {
    let mut rows: Vec<OrderRow> = snapshot
        .pending
        .iter()
        .map(|pending| pending_row(pending, snapshot.broker))
        .collect();
    let mut orders: Vec<OrderRow> = snapshot
        .orders
        .iter()
        .map(|view| order_row(view, snapshot))
        .collect();
    orders.sort_by_key(|row| std::cmp::Reverse(row.created_ms));
    rows.extend(orders);
    rows
}

fn pending_row(pending: &PendingView, broker: degenview_brokers::BrokerId) -> OrderRow {
    let draft = &pending.draft;
    let (kind, price, trigger, size, spend) = match &draft.kind {
        OrderKind::Market {
            size: OrderSize::Base(size),
        } => ("Market", None, None, Some(*size), None),
        OrderKind::Market {
            size: OrderSize::Quote(amount),
        } => ("Market", None, None, None, Some(*amount)),
        OrderKind::Limit {
            base_size, price, ..
        } => ("Limit", Some(*price), None, Some(*base_size), None),
        OrderKind::StopLimit {
            base_size,
            limit_price,
            stop_price,
            ..
        } => (
            "Stop-limit",
            Some(*limit_price),
            Some(*stop_price),
            Some(*base_size),
            None,
        ),
    };
    let unconfirmed = pending.state == PendingState::Unknown;
    OrderRow {
        id: pending.client_id.0.clone(),
        market: draft.product.as_str().to_owned(),
        side: draft.side,
        kind: kind.to_owned(),
        price,
        trigger,
        filled: Decimal::ZERO,
        size,
        spend,
        status: if unconfirmed {
            RowStatus {
                label: "Unconfirmed",
                tone: Tone::Warning,
            }
        } else {
            RowStatus {
                label: "Sending…",
                tone: Tone::Neutral,
            }
        },
        created_ms: Some(pending.created_ms),
        working: true,
        can_cancel: false,
        can_edit: false,
        note: None,
        edit_hint: None,
        source: source_of(broker),
        tag: None,
        unconfirmed,
    }
}

/// What a draft needs to be read against one market at this broker.
struct Market {
    snapshot: Arc<BrokerSnapshot>,
    spec: ProductSpec,
    caps: Capabilities,
    quote: Option<Quote>,
}

impl Market {
    fn context(&self) -> TicketContext<'_> {
        TicketContext {
            spec: &self.spec,
            caps: &self.caps,
            quote: self.quote.as_ref(),
            balances: &self.snapshot.balances,
            fees: self.snapshot.fees.as_ref(),
            now_ms: crate::model::watchlist::now_ms(),
        }
    }
}

fn ticket_of(draft: &TradeDraft) -> TicketDraft {
    TicketDraft {
        side: draft.side,
        kind: match draft.class {
            OrderClass::Market => TicketKind::Market,
            OrderClass::Limit | OrderClass::Stop => TicketKind::Limit,
            OrderClass::StopLimit => TicketKind::StopLimit,
        },
        size_text: draft.size_text.clone(),
        spend_in_quote: draft.spend_in_quote,
        price_text: draft.price_text.clone(),
        stop_text: draft.stop_text.clone(),
        tif: match draft.tif {
            TimeInForce::FillOrKill => TicketTif::FillOrKill,
            _ => TicketTif::GoodTilCancelled,
        },
        post_only: draft.post_only,
    }
}

fn field_of(field: TicketField) -> DraftField {
    match field {
        TicketField::Size => DraftField::Size,
        TicketField::Price | TicketField::LimitPrice => DraftField::Price,
        TicketField::StopPrice => DraftField::StopPrice,
        TicketField::TimeInForce => DraftField::TimeInForce,
        TicketField::Type => DraftField::Order,
    }
}

const NO_STOP: &str = "This broker has no plain stop order. Use a stop limit.";

fn cancel_text(outcome: CancelOutcome) -> CancelResult {
    match outcome {
        CancelOutcome::Requested => Ok(()),
        CancelOutcome::AlreadyFilled => Err("It filled before the cancel arrived.".to_owned()),
        CancelOutcome::NotFound => Err("The broker does not know this order any more.".to_owned()),
        CancelOutcome::BeingReplaced => {
            Err("The order is being changed. Try again in a moment.".to_owned())
        }
        CancelOutcome::Rejected(text) => Err(text),
        CancelOutcome::Unknown(_) => Err(
            "The cancel was sent but the answer was lost. The order is being checked.".to_owned(),
        ),
    }
}

impl TradingBackend for BrokerBackend {
    fn info(&self) -> VenueInfo {
        let descriptor = self.broker.descriptor();
        let kind = if descriptor.real_money {
            VenueKind::Live
        } else {
            VenueKind::Test
        };
        VenueInfo {
            venue: TradingVenue::Broker(descriptor.id),
            name: descriptor.display_name.to_owned(),
            kind,
            summary: if descriptor.real_money {
                format!("Real orders on your {} account.", descriptor.display_name)
            } else {
                format!(
                    "Practice orders on {}'s test environment. No real money.",
                    descriptor.display_name
                )
            },
            needs_no_credentials: descriptor.credential_fields.is_empty(),
        }
    }

    fn state(&self) -> BackendState {
        match self.host.connection(self.id()) {
            Connection::NotConfigured => BackendState::NotConnected,
            Connection::Saved => BackendState::Connecting,
            Connection::Running(snapshot) => state_of(&snapshot),
        }
    }

    fn account(&self) -> Option<AccountStrip> {
        let snapshot = self.snapshot()?;
        let account = snapshot
            .session
            .as_ref()
            .and_then(|session| session.accounts.first())?;
        Some(AccountStrip {
            label: account_label(&account.label, account.id.as_str()),
            equity: None,
        })
    }

    fn stats(&self) -> Option<AccountStats> {
        let snapshot = self.snapshot()?;
        let holdings: Vec<Holding> = snapshot
            .balances
            .iter()
            .map(|balance| Holding {
                asset: balance.asset.clone(),
                available: balance.available,
                held: balance.held,
            })
            .collect();
        let currency = valuation::valuation_currency(&holdings);
        // Prices come from the market that pairs the asset with the valuation currency. Asking is
        // cheap and only done while the strip is drawn, and only for assets actually held.
        let valuation = valuation::value(&holdings, &currency, |asset| {
            let spec = snapshot
                .products
                .iter()
                .find(|spec| spec.base == asset && spec.quote == currency)?;
            self.host.want_quote(self.id(), &spec.id);
            let quote = self.host.quote(self.id(), &spec.id)?;
            match (quote.bid, quote.ask) {
                (Some(bid), Some(ask)) => Some((bid + ask) / Decimal::TWO),
                _ => quote.last,
            }
        });
        let money = |value| valuation::money(value, &currency);
        let working = snapshot
            .orders
            .iter()
            .filter(|view| view.order.status.is_working())
            .count()
            + snapshot.pending.len();
        let note = if valuation.assets == 0 {
            None
        } else {
            let assets = format!(
                "{} {}",
                valuation.assets,
                if valuation.assets == 1 {
                    "asset"
                } else {
                    "assets"
                }
            );
            Some(HeroNote {
                trend: Trend::Flat,
                text: if valuation.unpriced.is_empty() {
                    assets
                } else {
                    format!("{assets} · {} unpriced", valuation.unpriced.len())
                },
                tone: if valuation.unpriced.is_empty() {
                    StatTone::Neutral
                } else {
                    StatTone::Thin
                },
                label: String::new(),
            })
        };
        let mut cells = vec![
            StatCell {
                title: "Cash".to_owned(),
                value: money(valuation.cash),
                tone: StatTone::Neutral,
                help: format!("{currency} free to spend"),
            },
            StatCell {
                title: "On hold".to_owned(),
                value: money(valuation.on_hold),
                tone: StatTone::Neutral,
                help: "Committed to open orders".to_owned(),
            },
            StatCell {
                title: "Holdings".to_owned(),
                value: money(valuation.holdings),
                tone: StatTone::Neutral,
                help: format!("Everything held except {currency}, at the latest prices"),
            },
            StatCell {
                title: "Open orders".to_owned(),
                value: working.to_string(),
                tone: StatTone::Neutral,
                help: "Orders resting at the broker or being sent".to_owned(),
            },
        ];
        if let Some(fees) = &snapshot.fees
            && let (Some(maker), Some(taker)) = (fees.maker_rate, fees.taker_rate)
        {
            cells.push(StatCell {
                title: "Fees".to_owned(),
                value: format!(
                    "{} / {}",
                    valuation::percent(maker),
                    valuation::percent(taker)
                ),
                tone: StatTone::Neutral,
                help: format!("Maker / taker, {}", fees.tier),
            });
        }
        let help = (!valuation.unpriced.is_empty()).then(|| {
            format!(
                "An estimate: {} could not be priced and is left out.",
                valuation.unpriced.join(", ")
            )
        });
        Some(AccountStats {
            hero: Hero {
                title: "Equity".to_owned(),
                value: if valuation.assets == 0 {
                    "—".to_owned()
                } else {
                    format!("≈ {}", money(valuation.equity))
                },
                help,
                note,
            },
            cells,
        })
    }

    fn available(&self, market: &MarketRef, side: Side) -> Option<Amount> {
        let snapshot = self.snapshot()?;
        let product = self.product(market)?;
        let spec = snapshot.product(&product)?;
        let asset = match side {
            Side::Buy => &spec.quote,
            Side::Sell => &spec.base,
        };
        let available = snapshot
            .balances
            .iter()
            .filter(|balance| &balance.asset == asset)
            .map(|balance| balance.available)
            .sum::<Decimal>();
        Some(Amount::new(available, asset.clone()))
    }

    fn market(&self, market: &MarketRef) -> MarketAccess {
        let snapshot = self.snapshot();
        match broker::chart::chart_trading(self.broker, Some(market), snapshot.as_deref(), false) {
            ChartTrading::Unavailable(reason) => MarketAccess::Unavailable(unavailable(reason)),
            ChartTrading::Ready { product } => {
                let ready = snapshot.as_deref().and_then(|snapshot| {
                    let spec = snapshot.product(&product)?;
                    let caps = snapshot.capabilities_of(&product)?;
                    Some(MarketRules {
                        name: spec.display_name.clone(),
                        base_asset: spec.base.clone(),
                        quote_asset: spec.quote.clone(),
                        price_increment: spec.price_increment,
                        size_increment: spec.base_increment,
                        min_size: spec.base_min,
                        min_notional: spec.quote_min,
                        caps: caps_of(caps),
                    })
                });
                match ready {
                    Some(rules) => MarketAccess::Ready(Box::new(rules)),
                    None => MarketAccess::Unavailable(unavailable(
                        broker::chart::Unavailable::RulesNotRead,
                    )),
                }
            }
        }
    }

    fn quote(&self, market: &MarketRef) -> Option<QuoteView> {
        let product = self.product(market)?;
        self.host.want_quote(self.id(), &product);
        let quote = self.host.quote(self.id(), &product)?;
        Some(QuoteView {
            bid: quote.bid,
            ask: quote.ask,
            last: quote.last,
        })
    }

    fn orders(&self) -> Vec<OrderRow> {
        self.snapshot()
            .map_or_else(Vec::new, |snapshot| order_rows(&snapshot))
    }

    fn check(&self, market: &MarketRef, draft: &TradeDraft) -> DraftCheck {
        let missing = draft.missing();
        let Some(found) = self.market_of(market) else {
            return DraftCheck {
                issues: vec![DraftIssue::new(
                    DraftField::Order,
                    "The market's rules have not been read yet.",
                )],
                missing,
                estimate: Estimate::default(),
            };
        };
        if draft.class == OrderClass::Stop {
            return DraftCheck {
                issues: vec![DraftIssue::new(DraftField::Order, NO_STOP)],
                missing,
                estimate: Estimate::default(),
            };
        }
        let ticket = ticket_of(draft);
        let context = found.context();
        // A field left empty is not yet a fault; the ticket reports it as one, so it is dropped.
        let issues = ticket
            .issues(&context)
            .into_iter()
            .map(|issue| DraftIssue::new(field_of(issue.field), issue.text))
            .filter(|issue| !missing.contains(&issue.field))
            .collect();
        let estimate = ticket.estimate(&context);
        DraftCheck {
            issues,
            missing,
            estimate: Estimate {
                reference: estimate.reference,
                notional: estimate.notional,
                fee: estimate.fee,
                margin: None,
            },
        }
    }

    fn max_size(&self, market: &MarketRef, side: Side, price: Option<Decimal>) -> Option<Decimal> {
        let found = self.market_of(market)?;
        let available = |asset: &str| -> Decimal {
            found
                .snapshot
                .balances
                .iter()
                .filter(|balance| balance.asset == asset)
                .map(|balance| balance.available)
                .sum()
        };
        let size = match side {
            Side::Sell => available(&found.spec.base),
            Side::Buy => {
                let price = price
                    .or_else(|| {
                        found
                            .quote
                            .as_ref()
                            .and_then(|quote| quote.ask.or(quote.last))
                    })
                    .filter(|price| *price > Decimal::ZERO)?;
                available(&found.spec.quote) / price
            }
        };
        let size = floor_to_increment(size, found.spec.base_increment);
        (size >= found.spec.base_min && size > Decimal::ZERO).then_some(size)
    }

    fn binding(&self) -> Binding {
        self.snapshot()
            .and_then(|snapshot| {
                snapshot.account.as_ref().map(|account| Binding {
                    account: account.id.as_str().to_owned(),
                    generation: snapshot.generation,
                })
            })
            .unwrap_or_default()
    }

    fn preview(
        &self,
        market: &MarketRef,
        draft: &TradeDraft,
        binding: &Binding,
    ) -> Task<Result<PreviewResult, String>> {
        let Some(found) = self.market_of(market) else {
            return Task::ready(Err("The market's rules have not been read yet.".to_owned()));
        };
        let runtime = match self.runtime_for(binding) {
            Ok(runtime) => runtime,
            Err(text) => return Task::ready(Err(text)),
        };
        let order = match ticket_of(draft).build(&found.context()) {
            Ok(order) => order,
            Err(issues) => {
                let text = issues.first().map_or_else(
                    || "The order is not complete.".to_owned(),
                    |i| i.text.clone(),
                );
                return Task::ready(Err(text));
            }
        };
        let pending = runtime.preview(order, AccountId::new(binding.account.as_str()));
        Task::new(move || {
            pending.poll().map(|answer| {
                answer
                    .map(|preview| PreviewResult {
                        accepted: preview.is_acceptable(),
                        total: preview.order_total,
                        fee: preview.fees,
                        average_price: preview.estimated_average_price,
                        problems: preview.problems.iter().map(|p| p.text.clone()).collect(),
                        warnings: preview.warnings.clone(),
                    })
                    .map_err(|error| error.to_string())
            })
        })
    }

    fn submit(
        &self,
        market: &MarketRef,
        draft: &TradeDraft,
        binding: &Binding,
    ) -> Task<SubmitResult> {
        let Some(found) = self.market_of(market) else {
            return Task::ready(Err("The market's rules have not been read yet.".to_owned()));
        };
        // The runtime the order was reviewed against, or none: never "whatever is connected now".
        let runtime = match self.runtime_for(binding) {
            Ok(runtime) => runtime,
            Err(text) => return Task::ready(Err(text)),
        };
        let order = match ticket_of(draft).build(&found.context()) {
            Ok(order) => order,
            Err(issues) => {
                let text = issues.first().map_or_else(
                    || "The order is not complete.".to_owned(),
                    |i| i.text.clone(),
                );
                return Task::ready(Err(text));
            }
        };
        let pending = runtime.submit(order, AccountId::new(binding.account.as_str()));
        Task::new(move || {
            pending.poll().map(|answer| {
                answer
                    .map(|placed| Submitted {
                        id: placed.order_id.as_str().to_owned(),
                    })
                    .map_err(|error| error.to_string())
            })
        })
    }

    fn cancel(&self, order_id: &str) -> Task<CancelResult> {
        let Some(runtime) = self.host.runtime(self.id()) else {
            return Task::ready(Err("The broker is not connected.".to_owned()));
        };
        let pending = runtime.cancel(vec![OrderId::new(order_id)]);
        Task::new(move || {
            pending.poll().map(|answer| match answer {
                Ok(results) => results.into_iter().next().map_or(
                    Err("The broker gave no answer for this order.".to_owned()),
                    |result| cancel_text(result.outcome),
                ),
                Err(error) => Err(error.to_string()),
            })
        })
    }
}

/// How an account is named: the venue's label, and for an id long enough to be an opaque uuid its
/// last characters, so two accounts with the same label (two default portfolios) read differently.
fn account_label(label: &str, id: &str) -> String {
    const SHOWN: usize = 6;
    if id.chars().count() > 12 {
        let tail: String = id
            .chars()
            .rev()
            .take(SHOWN)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        format!("{label} · …{tail}")
    } else {
        label.to_owned()
    }
}
