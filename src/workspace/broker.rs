//! Live trading in a workspace: which broker a tab trades at, what each chart may do there, and what
//! the charts' live lines and pills ask for.
//!
//! The rules (which chart is a broker's market, when orders are refused and why) are pure
//! (`model::broker::chart`); this file reads the broker's snapshot and fills each chart card with what
//! it should draw, then acts on what the card reports. Nothing here places or changes an order: it
//! opens the ticket, the edit sheet or the cancel question, each of which shows what will be sent.

use std::sync::Arc;

use degenview_brokers::{
    Broker, BrokerId, BrokerSnapshot, Decimal, MarketRef, OrderStatus, OrderType, Side,
};
use eframe::egui;

use super::ChartWorkspace;
use super::chart::Chart;
use crate::host::broker_host::BrokerHost;
use crate::model::broker::chart::{ChartTrading, Unavailable, chart_trading};
use crate::model::broker::ticket::{self as rules, TicketKind};
use crate::model::chart::context::{LiveFacts, LiveOrderRequest};
use crate::model::chart::kind::ChartType;
use crate::trading::model::OrderClass;
use crate::ui::broker_chart_trading::{BrokerOverlayAction, OrderLine};
use crate::ui::broker_panel;
use crate::ui::theme;

impl ChartWorkspace {
    /// The broker this tab trades at, if the venue is a broker and the host knows it.
    fn active_broker(&self) -> Option<(BrokerId, &'static dyn Broker, Arc<BrokerHost>)> {
        let id = self.trading_venue.broker()?;
        let broker = degenview_brokers::registry::by_id(id)?;
        Some((id, broker, BrokerHost::global()?))
    }

    /// A replay's prices are history: no live order may be started, changed or sent from one.
    pub(super) fn replay_blocks_trading(&self) -> bool {
        self.replay.is_active() || self.replay.status == super::ReplayStatus::SelectingStart
    }

    fn broker_market(chart: &Chart) -> Option<MarketRef> {
        chart.is_market().then(|| MarketRef {
            source: chart.source.watchlist_slug().to_lowercase(),
            symbol: chart.symbol.clone(),
        })
    }

    /// Whether a click on this chart's prices would be a market price: not on a chart type whose
    /// bars are constructed.
    fn prices_are_constructed(chart: &Chart) -> bool {
        let chart_type = chart.chart.effective_chart_type(chart.source);
        chart_type.is_synthetic() || chart_type == ChartType::HeikinAshi
    }

    /// Fills each chart card's live inputs: the header pills, the broker's bid and ask, and a line for
    /// each working order. Runs every frame; costs nothing when the tab is on paper.
    pub(super) fn configure_broker_cards(&mut self) {
        let Some((id, broker, host)) = self.active_broker() else {
            for chart in &mut self.charts {
                let live = &mut chart.chart.live;
                if live.show_lines || live.order_entry.is_some() {
                    live.show_lines = false;
                    live.order_entry = None;
                    live.lines.clear();
                    live.bid = None;
                    live.ask = None;
                }
            }
            return;
        };
        let snapshot = host.snapshot(id);
        let prefs = host.prefs(id);
        let replay = self.replay_blocks_trading();
        let review_closed = replay && self.trade_sheet.close_review();
        if (replay && self.broker_panel.close_order_entry()) || review_closed {
            self.say(
                "A replay started, so the live order was closed unsent.",
                true,
            );
        }
        for chart in &mut self.charts {
            let market = Self::broker_market(chart);
            let trading = chart_trading(broker, market.as_ref(), snapshot.as_deref(), replay);
            let resolved = market
                .as_ref()
                .and_then(|market| match broker.resolve_market(market) {
                    degenview_brokers::ProductResolution::Market(product) => Some(product),
                    _ => None,
                });
            let live = &mut chart.chart.live;
            live.broker_name = broker.descriptor().display_name.to_owned();
            // Pills appear on the broker's own markets, greyed with the reason when orders are not
            // possible right now; a chart of anything else is left alone.
            live.order_entry = match trading.reason() {
                None => Some(Ok(())),
                Some(
                    Unavailable::NotAMarket | Unavailable::OtherSource | Unavailable::UnknownMarket,
                ) => None,
                Some(reason) => Some(Err(reason)),
            };
            let (Some(product), Some(snapshot)) = (resolved, snapshot.as_deref()) else {
                live.show_lines = false;
                live.lines.clear();
                live.bid = None;
                live.ask = None;
                continue;
            };
            let spec = snapshot.product(&product).cloned();
            live.price_increment = spec.as_ref().map(|spec| spec.price_increment);
            let format = |price: Decimal| {
                spec.as_ref().map_or_else(
                    || rules::display(price),
                    |spec| rules::price_display(price, spec),
                )
            };
            if live.order_entry.is_some() {
                host.want_quote(id, &product);
                let quote = host.quote(id, &product);
                live.bid = quote.as_ref().and_then(|q| q.bid).map(format);
                live.ask = quote.as_ref().and_then(|q| q.ask).map(format);
            } else {
                live.bid = None;
                live.ask = None;
            }
            live.show_lines = prefs.show_on_charts;
            live.lines = snapshot
                .orders
                .iter()
                .filter(|view| view.order.product == product && view.order.status.is_working())
                .filter_map(|view| {
                    let order = &view.order;
                    let price = order.marker_price()?;
                    let caps = snapshot.capabilities_of(&order.product);
                    let editable =
                        rules::edit_blocker(order, caps, view.overlay.is_some()).is_none();
                    let note = match (view.overlay.map(|o| o.kind), order.status) {
                        (
                            Some(degenview_brokers::model::reducer::OverlayKind::CancelRequested),
                            _,
                        )
                        | (_, OrderStatus::CancelQueued) => Some("cancel requested"),
                        (Some(_), _) | (_, OrderStatus::EditQueued) => Some("edit requested"),
                        _ => None,
                    };
                    Some(OrderLine {
                        id: order.id.clone(),
                        side: order.side,
                        tag: match order.order_type {
                            OrderType::StopLimit => "STP-LMT".to_owned(),
                            OrderType::Market => "MKT".to_owned(),
                            _ => "LMT".to_owned(),
                        },
                        price: price.to_string().parse().ok()?,
                        price_text: format(price),
                        remaining_text: order
                            .remaining_base()
                            .map_or_else(|| "—".to_owned(), rules::display),
                        tint: match order.side {
                            Side::Buy => theme::UP,
                            Side::Sell => theme::DOWN,
                        },
                        note,
                        can_edit: editable,
                        can_cancel: view.overlay.is_none()
                            && order.status != OrderStatus::CancelQueued,
                    })
                })
                .collect();
        }
    }

    /// What the context menu needs to offer live orders on this chart, when a broker is active.
    pub(super) fn live_facts(&self, chart: &Chart) -> Option<LiveFacts> {
        let (id, broker, host) = self.active_broker()?;
        let snapshot = host.snapshot(id);
        let market = Self::broker_market(chart);
        let trading = chart_trading(
            broker,
            market.as_ref(),
            snapshot.as_deref(),
            self.replay_blocks_trading(),
        );
        let caps = trading.product().and_then(|product| {
            snapshot
                .as_deref()
                .and_then(|s| s.capabilities_of(product))
                .cloned()
        });
        let spec = trading.product().and_then(|product| {
            snapshot
                .as_deref()
                .and_then(|s| s.product(product))
                .cloned()
        });
        Some(LiveFacts {
            broker_name: broker.descriptor().display_name.to_owned(),
            unavailable: trading.reason(),
            price_increment: spec.map(|spec| spec.price_increment),
            market: caps
                .as_ref()
                .is_none_or(|caps| caps.allows(OrderType::Market)),
            limit: caps
                .as_ref()
                .is_none_or(|caps| caps.allows(OrderType::Limit)),
            stop_limit: caps
                .as_ref()
                .is_none_or(|caps| caps.allows(OrderType::StopLimit)),
            constructed: Self::prices_are_constructed(chart),
        })
    }

    /// Starts an order in the sheet for this chart's market, filled in as the menu or a pill asks.
    /// Refused, with the reason, whenever an order could not be placed from this chart.
    pub(super) fn open_live_ticket(&mut self, chart_index: usize, request: LiveOrderRequest) {
        let Some((id, broker, host)) = self.active_broker() else {
            self.say("Choose a broker in the trading panel first.", true);
            return;
        };
        let chart = &self.charts[chart_index];
        let market = Self::broker_market(chart);
        let snapshot = host.snapshot(id);
        let trading = chart_trading(
            broker,
            market.as_ref(),
            snapshot.as_deref(),
            self.replay_blocks_trading(),
        );
        if let ChartTrading::Unavailable(reason) = trading {
            self.say(reason.text(), true);
            return;
        }
        let class = match request.kind {
            TicketKind::Market => OrderClass::Market,
            TicketKind::Limit => OrderClass::Limit,
            TicketKind::StopLimit => OrderClass::StopLimit,
        };
        self.start_order(request.side, Some(class), request.price);
        if request.kind == TicketKind::Market && request.side == Side::Buy {
            self.trade_sheet.draft.spend_in_quote = host.prefs(id).spend_in_quote_by_default;
        }
    }

    /// Acts on what the charts' live lines reported: ask to cancel, or open the edit sheet. Neither
    /// changes anything until the user has confirmed it there.
    pub(super) fn apply_broker_overlay_actions(&mut self, actions: Vec<BrokerOverlayAction>) {
        let Some((id, _, host)) = self.active_broker() else {
            return;
        };
        let Some(snapshot) = host.snapshot(id) else {
            return;
        };
        for action in actions {
            match action {
                BrokerOverlayAction::Cancel(order) => {
                    self.broker_panel.cancel_one(&host, id, order)
                }
                BrokerOverlayAction::ProposeEdit { id: order, price } => {
                    self.begin_broker_edit(id, &snapshot, &order, Some(price));
                }
                BrokerOverlayAction::Open(order) => {
                    self.begin_broker_edit(id, &snapshot, &order, None)
                }
            }
        }
    }

    fn begin_broker_edit(
        &mut self,
        broker: BrokerId,
        snapshot: &BrokerSnapshot,
        order: &degenview_brokers::OrderId,
        price: Option<Decimal>,
    ) {
        if self.replay_blocks_trading() {
            self.say(Unavailable::InReplay.text(), true);
            return;
        }
        match snapshot.orders.iter().find(|view| &view.order.id == order) {
            Some(view) => self
                .broker_panel
                .begin_edit(broker, view.order.clone(), price),
            None => self.say("That order is no longer working.", true),
        }
    }

    /// Draws the ticket, the edit sheet and the cancel question when the panel (which normally does)
    /// is closed, and passes on what it has to say.
    pub(super) fn show_broker_dialogs(&mut self, ctx: &egui::Context) {
        let Some(id) = self.trading_venue.broker() else {
            return;
        };
        let Some(host) = BrokerHost::global() else {
            return;
        };
        if !self.paper_panel_visible && self.broker_panel.has_dialog() {
            broker_panel::dialogs_only(ctx, &mut self.broker_panel, &host, id);
        }
        // The panel shows its own message; with it closed the tab's notice does.
        if !self.paper_panel_visible
            && let Some((text, error)) = self.broker_panel.take_notice()
        {
            self.say(text, error);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};

    use degenview_brokers::coinbase::fake::{CoinbaseExchange, credentials, generate_key_pem};
    use degenview_brokers::{
        BrokerId, Env, MemorySecretStore, OrderDraft, OrderKind, ProductId, Status, TimeInForce,
    };
    use eframe::egui;

    use super::*;
    use crate::host::broker_host::StoreJournal;
    use crate::model::broker::venue::TradingVenue;
    use crate::model::{DataSource, InstrumentId};
    use crate::workspace::test_support::empty_workspace;

    const ID: BrokerId = BrokerId::new("coinbase");

    /// One mock exchange and one host for every test here (the host is process-wide by design),
    /// so the tests take turns.
    fn shared() -> (
        &'static (Arc<BrokerHost>, CoinbaseExchange),
        std::sync::MutexGuard<'static, ()>,
    ) {
        static HOST: OnceLock<(Arc<BrokerHost>, CoinbaseExchange)> = OnceLock::new();
        static TURN: Mutex<()> = Mutex::new(());
        let turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
        let pair = HOST.get_or_init(|| {
            let exchange = CoinbaseExchange::start();
            let store = Arc::new(Mutex::new(crate::store::Store::open_in_memory().unwrap()));
            let env = Env::default()
                .with_loopback_base_url(exchange.rest.url())
                .with_loopback_ws_url(exchange.stream.url.clone())
                .with_stream_timing(degenview_brokers::StreamTiming::fast())
                .with_request_gap(Duration::from_millis(2));
            let host = BrokerHost::new(
                Arc::new(MemorySecretStore::new()),
                Arc::new(StoreJournal::new(store.clone())),
                Some(store),
                env,
                |config| config.fast(),
            );
            host.adopt(ID, credentials(&generate_key_pem()))
                .wait()
                .unwrap();
            let host = BrokerHost::set_global_for_tests(host);
            (host, exchange)
        });
        wait("live", || {
            pair.0
                .snapshot(ID)
                .is_some_and(|s| s.status == Status::Live)
        });
        (pair, turn)
    }

    fn wait(what: &str, mut ready: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(8);
        while Instant::now() < deadline {
            if ready() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("timed out waiting for {what}");
    }

    fn limit(side: Side, price: &str) -> OrderDraft {
        OrderDraft {
            product: ProductId::new("BTC-USD"),
            side,
            kind: OrderKind::Limit {
                base_size: "0.001".parse().unwrap(),
                price: price.parse().unwrap(),
                tif: TimeInForce::GoodTilCancelled,
                post_only: false,
            },
        }
    }

    /// A workspace with a broker's chart and another source's, trading at the broker.
    fn workspace() -> ChartWorkspace {
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(1);
        workspace.add_chart_for(&InstrumentId::new(DataSource::Coinbase, "BTC-USD"), &ctx);
        workspace.add_chart_for(&InstrumentId::new(DataSource::Binance, "BTCUSDT"), &ctx);
        workspace.trading_venue = TradingVenue::Broker(ID);
        workspace
    }

    #[test]
    fn a_tab_on_paper_draws_no_live_trading_and_a_live_tab_draws_only_its_brokers_markets() {
        let ((host, _), _turn) = shared();
        let mut workspace = workspace();
        workspace.trading_venue = TradingVenue::Paper;
        workspace.configure_broker_cards();
        assert!(
            workspace
                .charts
                .iter()
                .all(|c| !c.chart.live.show_lines && c.chart.live.order_entry.is_none())
        );

        workspace.trading_venue = TradingVenue::Broker(ID);
        let runtime = host.runtime(ID).unwrap();
        let placed = runtime
            .submit(
                limit(Side::Buy, "87654.32"),
                crate::host::broker_host::account_of(&runtime),
            )
            .wait()
            .unwrap();
        wait("the order in the snapshot", || {
            host.snapshot(ID)
                .unwrap()
                .orders
                .iter()
                .any(|v| v.order.id == placed.order_id)
        });
        workspace.configure_broker_cards();
        let coinbase = &workspace.charts[0].chart.live;
        assert_eq!(
            coinbase.order_entry,
            Some(Ok(())),
            "orders can be placed from the broker's own chart"
        );
        assert!(coinbase.show_lines);
        let line = coinbase
            .lines
            .iter()
            .find(|l| l.id == placed.order_id)
            .expect("the order is a line");
        assert_eq!(line.price_text, "87,654.32");
        assert_eq!(line.remaining_text, "0.001");
        assert_eq!(line.side, Side::Buy);
        assert!(line.can_edit && line.can_cancel);
        // A chart of another source is left alone: no pills, no lines.
        let other = &workspace.charts[1].chart.live;
        assert_eq!(other.order_entry, None);
        assert!(other.lines.is_empty());
    }

    #[test]
    fn a_replay_greys_the_pills_refuses_a_ticket_and_closes_one_that_is_open() {
        let ((_, _), _turn) = shared();
        let mut workspace = workspace();
        workspace.configure_broker_cards();
        assert_eq!(workspace.charts[0].chart.live.order_entry, Some(Ok(())));
        let request = || LiveOrderRequest {
            side: Side::Buy,
            kind: TicketKind::Market,
            price: None,
        };
        workspace.open_live_ticket(0, request());
        assert!(
            workspace.trade_sheet_visible,
            "outside replay the order starts in the sheet"
        );
        assert_eq!(workspace.trade_sheet.draft.side, Side::Buy);
        assert_eq!(workspace.trade_sheet.draft.class, OrderClass::Market);

        workspace.replay.start(1, "BTC-USD", vec![1, 2, 3]);
        assert!(workspace.replay.is_active());
        workspace.configure_broker_cards();
        assert_eq!(
            workspace.charts[0].chart.live.order_entry,
            Some(Err(Unavailable::InReplay)),
            "the pills stay, greyed, with the reason"
        );
        workspace.trade_sheet_visible = false;
        workspace.open_live_ticket(0, request());
        assert!(
            !workspace.trade_sheet_visible,
            "and no order can be started during one"
        );
        // Editing from a line is refused too.
        let order = OrderId::new("anything");
        workspace.apply_broker_overlay_actions(vec![BrokerOverlayAction::Open(order)]);
        assert!(!workspace.broker_panel.has_dialog());
    }

    use degenview_brokers::OrderId;

    #[test]
    fn a_line_cancelled_or_dragged_asks_first_and_the_line_stays_until_the_broker_answers() {
        let ((host, exchange), _turn) = shared();
        let mut workspace = workspace();
        let runtime = host.runtime(ID).unwrap();
        let placed = runtime
            .submit(
                limit(Side::Sell, "99999.00"),
                crate::host::broker_host::account_of(&runtime),
            )
            .wait()
            .unwrap();
        wait("the order", || {
            host.snapshot(ID)
                .unwrap()
                .orders
                .iter()
                .any(|v| v.order.id == placed.order_id)
        });
        let cancels = || {
            exchange.rest.with_state(|s| {
                s.requests_seen
                    .iter()
                    .filter(|r| r.contains("batch_cancel"))
                    .count()
            })
        };
        let edits = || {
            exchange.rest.with_state(|s| {
                s.requests_seen
                    .iter()
                    .filter(|r| r.ends_with("/orders/edit"))
                    .count()
            })
        };
        let (before_cancels, before_edits) = (cancels(), edits());

        // Dragging a line proposes: the edit sheet opens and nothing is sent.
        workspace.apply_broker_overlay_actions(vec![BrokerOverlayAction::ProposeEdit {
            id: placed.order_id.clone(),
            price: "99000.00".parse().unwrap(),
        }]);
        assert!(
            workspace.broker_panel.has_dialog(),
            "the proposed change is shown for approval"
        );
        assert_eq!(
            edits(),
            before_edits,
            "nothing is changed by dropping a line"
        );
        workspace.broker_panel.close_order_entry();

        // ✕ cancels (single cancels do not ask by default): the request goes out, the line stays.
        workspace.apply_broker_overlay_actions(vec![BrokerOverlayAction::Cancel(
            placed.order_id.clone(),
        )]);
        wait("the cancel request", || cancels() == before_cancels + 1);
        wait("the broker's answer", || {
            host.snapshot(ID).unwrap().orders.iter().any(|v| {
                v.order.id == placed.order_id
                    && v.order.status == degenview_brokers::OrderStatus::CancelQueued
            })
        });
        workspace.configure_broker_cards();
        let line = workspace.charts[0]
            .chart
            .live
            .lines
            .iter()
            .find(|l| l.id == placed.order_id)
            .expect("the line stays until the broker confirms");
        assert_eq!(line.note, Some("cancel requested"));
        assert!(
            !line.can_cancel && !line.can_edit,
            "nothing more can be asked of it"
        );
    }

    #[test]
    fn the_menu_sees_live_facts_only_on_a_live_tab() {
        let ((_, _), _turn) = shared();
        let mut workspace = workspace();
        workspace.trading_venue = TradingVenue::Paper;
        assert!(workspace.live_facts(&workspace.charts[0]).is_none());
        workspace.trading_venue = TradingVenue::Broker(ID);
        let facts = workspace.live_facts(&workspace.charts[0]).unwrap();
        assert_eq!(facts.broker_name, "Coinbase");
        assert_eq!(facts.unavailable, None);
        assert_eq!(facts.price_increment, Some("0.01".parse().unwrap()));
        let other = workspace.live_facts(&workspace.charts[1]).unwrap();
        assert_eq!(
            other.unavailable,
            Some(Unavailable::OtherSource),
            "never traded on a similar market"
        );
    }
}
