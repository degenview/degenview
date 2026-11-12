//! Live trading in a workspace: which broker a tab trades at, what each chart may do there, and what
//! the charts' live lines and pills ask for.
//!
//! The rules (which chart is a broker's market, when orders are refused and why) are pure
//! (`model::broker::chart`); this file reads the broker's snapshot and fills each chart card with what
//! it should draw, then acts on what the card reports. Nothing here places or changes an order: it
//! opens the ticket, the edit sheet or the cancel question, each of which shows what will be sent.

use std::sync::Arc;

use degenview_brokers::{Decimal, MarketRef, Side};
use eframe::egui;

use super::ChartWorkspace;
use super::chart::Chart;
use crate::model::broker::chart::Unavailable;
use crate::model::broker::prefs::BrokerPrefs;
use crate::model::broker::ticket::{self as rules};
use crate::model::chart::context::{LiveFacts, LiveOrderRequest};
use crate::model::chart::kind::ChartType;
use crate::trading::backend::TradingBackend;
use crate::trading::model::{MarketAccess, OrderClass};
use crate::ui::broker::chart_trading::{BrokerOverlayAction, OrderLine};
use crate::ui::broker::panel::{self, PanelContext};
use crate::ui::theme;
use degenview_brokers::model::ticket::TicketKind;

impl ChartWorkspace {
    /// The venue of the broker this tab trades at, and how the user trades there.
    fn active_broker(&self) -> Option<(Arc<dyn TradingBackend>, BrokerPrefs)> {
        let hub = self.trading_hub.as_ref()?;
        let prefs = hub.prefs(self.trading_venue)?;
        Some((hub.backend(self.trading_venue)?, prefs))
    }

    /// What the orders panel is told about the tab's broker besides the venue itself.
    pub(super) fn broker_panel_context(&self) -> Option<PanelContext> {
        let (_, prefs) = self.active_broker()?;
        Some(PanelContext {
            confirm_single_cancel: prefs.confirm_single_cancel,
        })
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
        let Some((backend, prefs)) = self.active_broker() else {
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
        let show_lines = prefs.show_on_charts;
        let broker_name = backend.info().name;
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
            let reason = backend.chart_unavailable(market.as_ref(), replay);
            let lines = market
                .as_ref()
                .and_then(|market| backend.chart_lines(market));
            let live = &mut chart.chart.live;
            live.broker_name.clone_from(&broker_name);
            // Pills appear on the broker's own markets, greyed with the reason when orders are not
            // possible right now; a chart of anything else is left alone.
            live.order_entry = match reason {
                None => Some(Ok(())),
                Some(
                    Unavailable::NotAMarket | Unavailable::OtherSource | Unavailable::UnknownMarket,
                ) => None,
                Some(reason) => Some(Err(reason)),
            };
            let (Some(market), Some(lines)) = (market.as_ref(), lines) else {
                live.show_lines = false;
                live.lines.clear();
                live.bid = None;
                live.ask = None;
                continue;
            };
            live.price_increment = lines.price_increment;
            let format = |price: Decimal| {
                lines.price_increment.map_or_else(
                    || rules::display(price),
                    |increment| rules::price_at(price, increment),
                )
            };
            if live.order_entry.is_some() {
                let quote = backend.quote(market);
                live.bid = quote.as_ref().and_then(|q| q.bid).map(format);
                live.ask = quote.as_ref().and_then(|q| q.ask).map(format);
            } else {
                live.bid = None;
                live.ask = None;
            }
            live.show_lines = show_lines;
            live.lines = lines
                .orders
                .into_iter()
                .filter_map(|order| {
                    Some(OrderLine {
                        price: order.price.to_string().parse().ok()?,
                        id: order.id,
                        side: order.side,
                        tag: order.tag.to_owned(),
                        price_text: order.price_text,
                        remaining_text: order.remaining_text,
                        tint: match order.side {
                            Side::Buy => theme::UP,
                            Side::Sell => theme::DOWN,
                        },
                        note: order.note,
                        can_edit: order.can_edit,
                        can_cancel: order.can_cancel,
                    })
                })
                .collect();
        }
    }

    /// What the context menu needs to offer live orders on this chart, when a broker is active.
    pub(super) fn live_facts(&self, chart: &Chart) -> Option<LiveFacts> {
        let (backend, _) = self.active_broker()?;
        let market = Self::broker_market(chart);
        let unavailable = backend.chart_unavailable(market.as_ref(), self.replay_blocks_trading());
        let rules = market
            .as_ref()
            .and_then(|market| match backend.market(market) {
                MarketAccess::Ready(rules) => Some(*rules),
                MarketAccess::Unavailable(_) => None,
            });
        let allows = |class| rules.as_ref().is_none_or(|rules| rules.caps.allows(class));
        Some(LiveFacts {
            broker_name: backend.info().name,
            unavailable,
            price_increment: rules.as_ref().map(|rules| rules.price_increment),
            market: allows(OrderClass::Market),
            limit: allows(OrderClass::Limit),
            stop_limit: allows(OrderClass::StopLimit),
            constructed: Self::prices_are_constructed(chart),
        })
    }

    /// Starts an order in the sheet for this chart's market, filled in as the menu or a pill asks.
    /// Refused, with the reason, whenever an order could not be placed from this chart.
    pub(super) fn open_live_ticket(&mut self, chart_index: usize, request: LiveOrderRequest) {
        let Some((backend, prefs)) = self.active_broker() else {
            self.say("Choose a broker in the trading panel first.", true);
            return;
        };
        let market = Self::broker_market(&self.charts[chart_index]);
        if let Some(reason) =
            backend.chart_unavailable(market.as_ref(), self.replay_blocks_trading())
        {
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
            self.trade_sheet.draft.spend_in_quote = prefs.spend_in_quote_by_default;
        }
    }

    /// Acts on what the charts' live lines reported: ask to cancel, or open the edit sheet. Neither
    /// changes anything until the user has confirmed it there.
    pub(super) fn apply_broker_overlay_actions(&mut self, actions: Vec<BrokerOverlayAction>) {
        let Some((backend, prefs)) = self.active_broker() else {
            return;
        };
        for action in actions {
            match action {
                BrokerOverlayAction::Cancel(order) => {
                    self.broker_panel.cancel_one(
                        backend.broker_commands(),
                        prefs.confirm_single_cancel,
                        order,
                    );
                }
                BrokerOverlayAction::ProposeEdit { id: order, price } => {
                    self.begin_broker_edit(&*backend, &order, Some(price));
                }
                BrokerOverlayAction::Open(order) => self.begin_broker_edit(&*backend, &order, None),
            }
        }
    }

    fn begin_broker_edit(
        &mut self,
        backend: &dyn TradingBackend,
        order: &str,
        price: Option<Decimal>,
    ) {
        if self.replay_blocks_trading() {
            self.say(Unavailable::InReplay.text(), true);
            return;
        }
        if !self
            .broker_panel
            .begin_edit(backend.broker_commands(), order, price)
        {
            self.say("That order is no longer working.", true);
        }
    }

    /// Draws the ticket, the edit sheet and the cancel question when the panel (which normally does)
    /// is closed, and passes on what it has to say.
    pub(super) fn show_broker_dialogs(&mut self, ctx: &egui::Context) {
        let Some((backend, _)) = self.active_broker() else {
            return;
        };
        if !self.paper_panel_visible && self.broker_panel.has_dialog() {
            panel::dialogs_only(ctx, &mut self.broker_panel, &*backend);
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
    use crate::host::broker_host::{BrokerHost, store_journal};
    use crate::host::paper_host::PaperHost;
    use crate::host::trading_hub::TradingHub;
    use crate::model::broker::venue::TradingVenue;
    use crate::model::{DataSource, InstrumentId};
    use crate::workspace::test_support::empty_workspace;

    const ID: BrokerId = BrokerId::new("coinbase");

    /// One mock exchange and one host for every test here (the host is process-wide by design),
    /// so the tests take turns.
    fn shared() -> (
        &'static (Arc<BrokerHost>, CoinbaseExchange, Arc<TradingHub>),
        std::sync::MutexGuard<'static, ()>,
    ) {
        static HOST: OnceLock<(Arc<BrokerHost>, CoinbaseExchange, Arc<TradingHub>)> =
            OnceLock::new();
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
                Arc::new(store_journal(store.clone())),
                Some(store.clone()),
                env,
                |config| config.fast(),
            );
            host.adopt(ID, credentials(&generate_key_pem()))
                .wait()
                .unwrap();
            let paper = Arc::new(Mutex::new(PaperHost::load(store.clone())));
            let hub = TradingHub::new(host.clone(), paper, Some(store));
            (host, exchange, hub)
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
    fn workspace(hub: &Arc<TradingHub>) -> ChartWorkspace {
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(1);
        workspace.add_chart_for(&InstrumentId::new(DataSource::Coinbase, "BTC-USD"), &ctx);
        workspace.add_chart_for(&InstrumentId::new(DataSource::Binance, "BTCUSDT"), &ctx);
        workspace.trading_venue = TradingVenue::Broker(ID);
        workspace.set_trading_hub(hub);
        workspace
    }

    #[test]
    fn a_tab_on_paper_draws_no_live_trading_and_a_live_tab_draws_only_its_brokers_markets() {
        let ((host, _, hub), _turn) = shared();
        let mut workspace = workspace(hub);
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
            .find(|l| l.id == placed.order_id.as_str())
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
        let ((_, _, hub), _turn) = shared();
        let mut workspace = workspace(hub);
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
        workspace.apply_broker_overlay_actions(vec![BrokerOverlayAction::Open(
            order.as_str().to_owned(),
        )]);
        assert!(!workspace.broker_panel.has_dialog());
    }

    use degenview_brokers::OrderId;

    #[test]
    fn a_line_cancelled_or_dragged_asks_first_and_the_line_stays_until_the_broker_answers() {
        let ((host, exchange, hub), _turn) = shared();
        let mut workspace = workspace(hub);
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
            id: placed.order_id.as_str().to_owned(),
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
            placed.order_id.as_str().to_owned(),
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
            .find(|l| l.id == placed.order_id.as_str())
            .expect("the line stays until the broker confirms");
        assert_eq!(line.note, Some("cancel requested"));
        assert!(
            !line.can_cancel && !line.can_edit,
            "nothing more can be asked of it"
        );
    }

    #[test]
    fn the_menu_sees_live_facts_only_on_a_live_tab() {
        let ((_, _, hub), _turn) = shared();
        let mut workspace = workspace(hub);
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
