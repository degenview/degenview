//! Whether a chart can place live orders, and why not when it cannot. Pure.
//!
//! A chart is tradable at a broker only when all of this holds: it is a market chart whose data
//! comes from the broker's own market-data source (so its symbol *is* the broker's market; nothing
//! is substituted), the connection is live, the credential may trade, the broker lists the market
//! and says it can be traded, and the chart is not in replay (a replay's prices are history; a click
//! on one must never become a live order).

use degenview_brokers::{
    Broker, BrokerSnapshot, Decimal, MarketRef, ProductId, ProductResolution, Status,
};

/// Said when the broker marks a market untradable without saying why.
const NOT_TRADABLE: &str = "Trading is not available in this market right now.";

/// Why a chart cannot place live orders. The sentences are shown to the user.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Unavailable {
    NotAMarket,
    OtherSource,
    UnknownMarket,
    NotConnected,
    NotLive,
    NeedsAttention,
    /// The record of orders waiting for confirmation cannot be read, so an order that may be live
    /// could be missing from it.
    JournalUnreadable,
    ViewOnly,
    RulesNotRead,
    /// The broker's market list is loaded and this market is not on it.
    MarketNotOffered,
    /// The broker's own words for why this market cannot be traded with this account.
    MarketNotTradable(&'static str),
    InReplay,
}

impl Unavailable {
    pub fn text(self) -> &'static str {
        match self {
            Self::NotAMarket => "This chart has no market to trade.",
            Self::OtherSource => {
                "This chart's data is not from this broker, so it is not its market. Open the broker's own chart of the market."
            }
            Self::UnknownMarket => "This is not a market the broker can trade.",
            Self::NotConnected => "The broker is not connected.",
            Self::NotLive => {
                "Waiting for the broker's connection. Orders are paused until it is back."
            }
            Self::NeedsAttention => "The broker needs attention. Open Settings ▸ Brokers.",
            Self::JournalUnreadable => {
                "New orders are paused: the record of orders waiting for confirmation cannot be read. Cancelling still works."
            }
            Self::ViewOnly => "This key can view but not trade.",
            Self::RulesNotRead => "Reading the market's rules…",
            Self::MarketNotOffered => {
                "This broker does not offer this market, so it cannot be traded here."
            }
            Self::MarketNotTradable(reason) => reason,
            Self::InReplay => "Not available during replay.",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ChartTrading {
    Ready { product: ProductId },
    Unavailable(Unavailable),
}

impl ChartTrading {
    pub fn product(&self) -> Option<&ProductId> {
        match self {
            Self::Ready { product } => Some(product),
            Self::Unavailable(_) => None,
        }
    }

    pub fn reason(&self) -> Option<Unavailable> {
        match self {
            Self::Ready { .. } => None,
            Self::Unavailable(reason) => Some(*reason),
        }
    }
}

/// The market a chart shows, as far as a broker can tell, and whether it may be traded now.
pub fn chart_trading(
    broker: &dyn Broker,
    market: Option<&MarketRef>,
    snapshot: Option<&BrokerSnapshot>,
    replay: bool,
) -> ChartTrading {
    let Some(market) = market else {
        return ChartTrading::Unavailable(Unavailable::NotAMarket);
    };
    let product = match broker.resolve_market(market) {
        ProductResolution::Market(product) => product,
        ProductResolution::OtherSource { .. } => {
            return ChartTrading::Unavailable(Unavailable::OtherSource);
        }
        ProductResolution::Unrecognised => {
            return ChartTrading::Unavailable(Unavailable::UnknownMarket);
        }
    };
    let Some(snapshot) = snapshot else {
        return ChartTrading::Unavailable(Unavailable::NotConnected);
    };
    match &snapshot.status {
        Status::Live => {}
        Status::Failed(_) => return ChartTrading::Unavailable(Unavailable::NeedsAttention),
        _ => return ChartTrading::Unavailable(Unavailable::NotLive),
    }
    if snapshot.journal_error.is_some() {
        return ChartTrading::Unavailable(Unavailable::JournalUnreadable);
    }
    if !snapshot
        .session
        .as_ref()
        .is_some_and(|session| session.permissions.can_trade)
    {
        return ChartTrading::Unavailable(Unavailable::ViewOnly);
    }
    match snapshot.capabilities_of(&product) {
        // Not read yet is different from not on the list: once the market list is loaded, a market
        // missing from it never will be, so it must not read as "reading" for ever.
        None if snapshot.capabilities.is_empty() => {
            return ChartTrading::Unavailable(Unavailable::RulesNotRead);
        }
        None => return ChartTrading::Unavailable(Unavailable::MarketNotOffered),
        Some(caps) if !caps.is_tradable() => {
            let reason = snapshot
                .product(&product)
                .and_then(|spec| spec.restrictions.blocked_reason())
                .unwrap_or(NOT_TRADABLE);
            return ChartTrading::Unavailable(Unavailable::MarketNotTradable(reason));
        }
        Some(_) => {}
    }
    if replay {
        return ChartTrading::Unavailable(Unavailable::InReplay);
    }
    ChartTrading::Ready { product }
}

/// A clicked price as an exact decimal on the market's increment: the *nearest* valid price (a click
/// has no precision to preserve). `None` for a price that is not positive and finite.
pub fn snap_to_increment(price: f64, increment: Decimal) -> Option<Decimal> {
    if !price.is_finite() || price <= 0.0 {
        return None;
    }
    // Through text, so 0.1 stays 0.1 and not 0.1000000000000000055.
    let exact: Decimal = format!("{price}").parse().ok()?;
    let snapped = if increment.is_zero() {
        exact
    } else {
        (exact / increment).round() * increment
    };
    (snapped > Decimal::ZERO).then_some(snapped.normalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use degenview_brokers::registry;
    use std::str::FromStr;

    fn d(text: &str) -> Decimal {
        Decimal::from_str(text).unwrap()
    }

    #[test]
    fn a_click_snaps_to_the_nearest_valid_price_and_stays_exact() {
        assert_eq!(snap_to_increment(90_000.004, d("0.01")), Some(d("90000")));
        assert_eq!(
            snap_to_increment(90_000.006, d("0.01")),
            Some(d("90000.01"))
        );
        assert_eq!(snap_to_increment(0.1, d("0.1")), Some(d("0.1")));
        assert_eq!(
            snap_to_increment(123.456, Decimal::ZERO),
            Some(d("123.456"))
        );
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(snap_to_increment(bad, d("0.01")), None, "{bad}");
        }
        // Rounds to nothing: not a price.
        assert_eq!(snap_to_increment(0.001, d("1")), None);
    }

    fn market(source: &str, symbol: &str) -> MarketRef {
        MarketRef {
            source: source.to_owned(),
            symbol: symbol.to_owned(),
        }
    }

    #[test]
    fn the_reasons_are_checked_in_order_and_each_has_a_sentence() {
        let broker = registry::all()[0];
        let source = broker.descriptor().market_source;
        let reason =
            |market: Option<&MarketRef>| chart_trading(broker, market, None, false).reason();
        assert_eq!(reason(None), Some(Unavailable::NotAMarket));
        assert_eq!(
            reason(Some(&market("elsewhere", "BTC-USD"))),
            Some(Unavailable::OtherSource)
        );
        assert_eq!(
            reason(Some(&market(source, "not a symbol"))),
            Some(Unavailable::UnknownMarket)
        );
        // A good market, nothing connected.
        assert_eq!(
            reason(Some(&market(source, "BTC-USD"))),
            Some(Unavailable::NotConnected)
        );
        for reason in [
            Unavailable::NotAMarket,
            Unavailable::OtherSource,
            Unavailable::UnknownMarket,
            Unavailable::NotConnected,
            Unavailable::NotLive,
            Unavailable::NeedsAttention,
            Unavailable::ViewOnly,
            Unavailable::RulesNotRead,
            Unavailable::MarketNotOffered,
            Unavailable::MarketNotTradable(NOT_TRADABLE),
            Unavailable::InReplay,
        ] {
            assert!(
                reason.text().ends_with('.') || reason.text().ends_with('…'),
                "{reason:?}"
            );
        }
    }

    fn snapshot(
        status: Status,
        can_trade: bool,
        product: Option<&str>,
        tradable: bool,
    ) -> BrokerSnapshot {
        use degenview_brokers::{
            AccountId, AccountKind, AccountRef, Capabilities, OrderType, Permissions, Session,
            TifKind, Tradability,
        };
        let mut capabilities = std::collections::HashMap::new();
        if let Some(product) = product {
            capabilities.insert(
                ProductId::new(product),
                Capabilities {
                    tradable: if tradable {
                        Tradability::Yes
                    } else {
                        Tradability::No("no".to_owned())
                    },
                    order_types: vec![OrderType::Limit],
                    tif: vec![TifKind::GoodTilCancelled],
                    stop_limit_tif: vec![],
                    post_only: false,
                    quote_sized_market: false,
                    native_edit: true,
                    atomic_replace: false,
                    brackets: false,
                },
            );
        }
        BrokerSnapshot {
            revision: 1,
            broker: registry::all()[0].descriptor().id,
            status,
            session: Some(Session {
                permissions: Permissions {
                    can_view: true,
                    can_trade,
                    can_transfer: false,
                },
                accounts: vec![AccountRef {
                    id: AccountId::new("a"),
                    label: "Main".to_owned(),
                    kind: AccountKind::Spot,
                }],
            }),
            balances: Vec::new(),
            balances_ms: None,
            orders: Vec::new(),
            fills: Vec::new(),
            pending: Vec::new(),
            products: std::sync::Arc::new(Vec::new()),
            capabilities: std::sync::Arc::new(capabilities),
            fees: None,
            last_sync_ms: None,
            last_error: None,
            journal_error: None,
        }
    }

    #[test]
    fn a_chart_is_tradable_only_when_every_condition_holds_and_the_first_failure_is_the_reason() {
        let broker = registry::all()[0];
        let source = broker.descriptor().market_source;
        let btc = market(source, "BTC-USD");
        let check = |snapshot: Option<&BrokerSnapshot>, replay: bool| {
            chart_trading(broker, Some(&btc), snapshot, replay)
        };

        let good = snapshot(Status::Live, true, Some("BTC-USD"), true);
        assert_eq!(
            check(Some(&good), false),
            ChartTrading::Ready {
                product: ProductId::new("BTC-USD")
            }
        );

        assert_eq!(check(None, false).reason(), Some(Unavailable::NotConnected));
        for status in [
            Status::Connecting,
            Status::Syncing,
            Status::Reconnecting {
                since_ms: 0,
                reason: "x".to_owned(),
            },
        ] {
            assert_eq!(
                check(Some(&snapshot(status, true, Some("BTC-USD"), true)), false).reason(),
                Some(Unavailable::NotLive)
            );
        }
        let failed = snapshot(
            Status::Failed(degenview_brokers::BrokerError::invalid("no")),
            true,
            Some("BTC-USD"),
            true,
        );
        assert_eq!(
            check(Some(&failed), false).reason(),
            Some(Unavailable::NeedsAttention)
        );
        // An unreadable record of unconfirmed orders pauses new orders, whatever else is true.
        let mut unreadable = snapshot(Status::Live, true, Some("BTC-USD"), true);
        unreadable.journal_error = Some(degenview_brokers::BrokerError::invalid("cannot be read"));
        assert_eq!(
            check(Some(&unreadable), false).reason(),
            Some(Unavailable::JournalUnreadable)
        );
        assert!(!unreadable.can_send_orders());
        assert_eq!(
            check(
                Some(&snapshot(Status::Live, false, Some("BTC-USD"), true)),
                false
            )
            .reason(),
            Some(Unavailable::ViewOnly)
        );
        assert_eq!(
            check(Some(&snapshot(Status::Live, true, None, true)), false).reason(),
            Some(Unavailable::RulesNotRead)
        );
        // A loaded market list that lacks this market says so, rather than "reading" for ever.
        assert_eq!(
            check(
                Some(&snapshot(Status::Live, true, Some("ETH-USD"), true)),
                false
            )
            .reason(),
            Some(Unavailable::MarketNotOffered)
        );
        // The broker's own words are what the user reads, not a generic sentence.
        let blocked = check(
            Some(&snapshot(Status::Live, true, Some("BTC-USD"), false)),
            false,
        )
        .reason();
        assert!(
            matches!(blocked, Some(Unavailable::MarketNotTradable(text)) if !text.is_empty()),
            "{blocked:?}"
        );
        // A replay refuses even a fully tradable market.
        assert_eq!(
            check(Some(&good), true).reason(),
            Some(Unavailable::InReplay)
        );
    }
}
