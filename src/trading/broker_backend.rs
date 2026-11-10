//! A connected broker, as a trading venue.
//!
//! Reads the `degenview-brokers` crate's snapshots through [`BrokerHost`] and turns them into the
//! venue vocabulary. It names no broker: what a broker is comes from its descriptor.

use std::sync::Arc;

use degenview_brokers::model::reducer::{OrderView, OverlayKind};
use degenview_brokers::{
    Broker, BrokerSnapshot, Capabilities, Decimal, LiveFill, MarketRef, OrderKind, OrderSize,
    OrderType, PendingState, PendingView, ProductResolution, Side, Status, TifKind,
};

use super::backend::TradingBackend;
use super::model::{
    AccountStrip, Amount, BackendState, BalanceRow, FillRow, MarketAccess, MarketRules, OrderClass,
    OrderRow, PositionRow, QuoteView, RowStatus, TabKind, TicketCaps, TimeInForce, Tone,
    Unavailable, UnavailableKind, VenueInfo, VenueKind,
};
use crate::host::broker_host::{BrokerHost, Connection};
use crate::model::broker_chart::{self, ChartTrading};
use crate::model::broker_ticket::{self as rules, StatusTone};
use crate::model::trading_venue::TradingVenue;

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
            broker_chart::Unavailable::JournalUnreadable
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

fn unavailable(reason: broker_chart::Unavailable) -> Unavailable {
    use broker_chart::Unavailable as Why;
    let kind = match reason {
        Why::NotAMarket | Why::OtherSource | Why::UnknownMarket => UnavailableKind::NotThisVenue,
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
        unconfirmed: false,
    }
}

fn pending_row(pending: &PendingView) -> OrderRow {
    let draft = &pending.draft;
    let (kind, price, size, spend) = match &draft.kind {
        OrderKind::Market {
            size: OrderSize::Base(size),
        } => ("Market", None, Some(*size), None),
        OrderKind::Market {
            size: OrderSize::Quote(amount),
        } => ("Market", None, None, Some(*amount)),
        OrderKind::Limit {
            base_size, price, ..
        } => ("Limit", Some(*price), Some(*base_size), None),
        OrderKind::StopLimit {
            base_size,
            limit_price,
            ..
        } => ("Stop-limit", Some(*limit_price), Some(*base_size), None),
    };
    let unconfirmed = pending.state == PendingState::Unknown;
    OrderRow {
        id: pending.client_id.0.clone(),
        market: draft.product.as_str().to_owned(),
        side: draft.side,
        kind: kind.to_owned(),
        price,
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
        unconfirmed,
    }
}

fn fill_row(fill: &LiveFill) -> FillRow {
    FillRow {
        id: fill.id.clone(),
        market: fill.product.as_str().to_owned(),
        side: fill.side,
        price: fill.price,
        size: fill.size,
        fee: fill.fee,
        time_ms: fill.time_ms,
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
        let label = snapshot
            .session
            .as_ref()
            .and_then(|session| session.accounts.first())
            .map(|account| account.label.clone())?;
        Some(AccountStrip {
            label,
            equity: None,
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
        match broker_chart::chart_trading(self.broker, Some(market), snapshot.as_deref(), false) {
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
                        broker_chart::Unavailable::RulesNotRead,
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

    fn tabs(&self) -> Vec<TabKind> {
        vec![
            TabKind::Orders,
            TabKind::History,
            TabKind::Fills,
            TabKind::Balances,
        ]
    }

    fn orders(&self) -> Vec<OrderRow> {
        let Some(snapshot) = self.snapshot() else {
            return Vec::new();
        };
        let mut rows: Vec<OrderRow> = snapshot.pending.iter().map(pending_row).collect();
        let mut orders: Vec<OrderRow> = snapshot
            .orders
            .iter()
            .map(|view| order_row(view, &snapshot))
            .collect();
        orders.sort_by_key(|row| std::cmp::Reverse(row.created_ms));
        rows.extend(orders);
        rows
    }

    fn positions(&self) -> Vec<PositionRow> {
        Vec::new()
    }

    fn fills(&self) -> Vec<FillRow> {
        let Some(snapshot) = self.snapshot() else {
            return Vec::new();
        };
        let mut fills: Vec<FillRow> = snapshot.fills.iter().map(fill_row).collect();
        fills.sort_by_key(|fill| std::cmp::Reverse(fill.time_ms));
        fills
    }

    fn balances(&self) -> Vec<BalanceRow> {
        let Some(snapshot) = self.snapshot() else {
            return Vec::new();
        };
        let mut rows: Vec<BalanceRow> = snapshot
            .balances
            .iter()
            .filter(|balance| !balance.total().is_zero())
            .map(|balance| BalanceRow {
                asset: balance.asset.clone(),
                available: balance.available,
                held: balance.held,
            })
            .collect();
        rows.sort_by(|a, b| a.asset.cmp(&b.asset));
        rows
    }

    fn revision(&self) -> u64 {
        self.host.revision()
    }
}
