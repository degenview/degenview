//! The live trading panel: the account, its working orders, its history, its fills and balances,
//! and the actions on them (a new order, cancel, edit).
//!
//! It draws what the broker last said (`BrokerSnapshot`) and nothing else: no order is added or
//! removed because a button was pressed. Asking to cancel marks an order "Cancelling" until the
//! broker confirms; sending an order shows it as "Unconfirmed" until the broker lists it. Every
//! request goes to the broker's runtime on its own thread, and its answer is polled on later frames,
//! so the panel never waits on the network.
//!
//! The panel is shared by every window by construction: it reads the host's snapshot, which all
//! windows read.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use degenview_brokers::model::orders::{
    CancelOutcome, CancelResult, EditOutcome, EditPreview, FillFilter, HistoryFilter, OrderChange,
    Page, PlacedOrder,
};
use degenview_brokers::{
    Broker, BrokerId, BrokerSnapshot, Capabilities, ClientOrderId, Decimal, LiveFill, LiveOrder,
    MarketRef, OrderId, OrderKind, OrderSize, OrderStatus, OrderType, Pending, PendingState,
    ProductResolution, Side, Status,
};
use eframe::egui::{self, Color32, Id};

use crate::host::broker_host::BrokerHost;
use crate::model::broker_prefs::BrokerPrefs;
use crate::model::broker_ticket::{self as rules, StatusTone, TicketDraft};
use crate::model::trading_venue::TradingVenue;
use crate::ui::alert_center::Tone;
use crate::ui::broker_ticket::{self, TicketEvent, TicketState};
use crate::ui::icons::{self, Icon};
use crate::ui::paper_grid::{self as grid, Col, col};
use crate::ui::paper_tables::empty;
use crate::ui::{broker_badge, dialog, kit, sheet, theme};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BrokerTab {
    Orders,
    History,
    Fills,
    Balances,
}

/// Which finished orders the History tab lists.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HistoryChoice {
    All,
    Filled,
    Cancelled,
    Expired,
    Rejected,
}

impl HistoryChoice {
    pub fn statuses(self) -> Vec<OrderStatus> {
        match self {
            Self::All => vec![
                OrderStatus::Filled,
                OrderStatus::Cancelled,
                OrderStatus::Expired,
                OrderStatus::Failed,
            ],
            Self::Filled => vec![OrderStatus::Filled],
            Self::Cancelled => vec![OrderStatus::Cancelled],
            Self::Expired => vec![OrderStatus::Expired],
            Self::Rejected => vec![OrderStatus::Failed],
        }
    }
}

/// One list that is read from the broker a page at a time.
struct Paged<T> {
    items: Vec<T>,
    cursor: Option<String>,
    pending: Option<Pending<Page<T>>>,
    loaded: bool,
    error: Option<String>,
}

impl<T> Default for Paged<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            cursor: None,
            pending: None,
            loaded: false,
            error: None,
        }
    }
}

impl<T> Paged<T> {
    fn reset(&mut self) {
        *self = Self::default();
    }

    /// Takes a page that arrived.
    fn poll(&mut self) {
        let Some(pending) = &self.pending else { return };
        let Some(answer) = pending.poll() else { return };
        self.pending = None;
        self.loaded = true;
        match answer {
            Ok(page) => {
                self.items.extend(page.items);
                self.cursor = page.next_cursor;
                self.error = None;
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }
}

enum Flight {
    Cancel(Pending<Vec<CancelResult>>),
    Retry(Pending<PlacedOrder>),
}

struct CancelAsk {
    ids: Vec<OrderId>,
    title: String,
    message: String,
}

enum EditStage {
    Editing,
    /// The change, and the broker's preview of exactly it.
    Confirming(OrderChange, Box<EditPreview>),
    Sending(Pending<EditOutcome>),
}

pub struct EditState {
    broker: BrokerId,
    order: LiveOrder,
    price_text: String,
    size_text: String,
    preview: Option<EditPreview>,
    preview_for: Option<String>,
    preview_pending: Option<(String, Pending<EditPreview>)>,
    preview_error: Option<String>,
    /// When the broker's answer arrived: an old answer is asked for again.
    preview_at: Option<Instant>,
    seen: String,
    changed_at: Instant,
    stage: EditStage,
    error: Option<String>,
    /// Whether the Review button was enabled in the last frame.
    #[cfg(test)]
    review_enabled: bool,
}

impl EditState {
    /// Asks the broker again, at once. Reached by the "Try again" button.
    fn retry_preview(&mut self) {
        self.preview_error = None;
        self.preview_for = None;
        self.preview_at = None;
        self.changed_at = Instant::now()
            .checked_sub(Duration::from_millis(450))
            .unwrap_or_else(Instant::now);
    }
}

pub struct BrokerPanelState {
    tab: BrokerTab,
    selected: BTreeSet<OrderId>,
    history: Paged<LiveOrder>,
    history_choice: HistoryChoice,
    fills_more: Paged<LiveFill>,
    ticket: Option<TicketState>,
    edit: Option<EditState>,
    cancel_ask: Option<CancelAsk>,
    flights: Vec<Flight>,
    notice: Option<(String, bool, Instant)>,
    #[cfg(test)]
    force_edit_review: bool,
}

impl Default for BrokerPanelState {
    fn default() -> Self {
        Self {
            tab: BrokerTab::Orders,
            selected: BTreeSet::new(),
            history: Paged::default(),
            history_choice: HistoryChoice::All,
            fills_more: Paged::default(),
            ticket: None,
            edit: None,
            cancel_ask: None,
            flights: Vec::new(),
            notice: None,
            #[cfg(test)]
            force_edit_review: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PanelAction {
    Close,
    OpenSettings,
}

// ---- Rules (pure) --------------------------------------------------------------------------------

/// What a set of cancel answers says, in one sentence, and whether it is a problem.
pub fn cancel_summary(results: &[CancelResult]) -> (String, bool) {
    let requested = results
        .iter()
        .filter(|r| r.outcome == CancelOutcome::Requested)
        .count();
    let mut problems: Vec<String> = Vec::new();
    for result in results {
        match &result.outcome {
            CancelOutcome::Requested => {}
            CancelOutcome::AlreadyFilled => problems.push("one had already filled".to_owned()),
            CancelOutcome::NotFound => {
                problems.push("one could not be found (it may already be finished)".to_owned());
            }
            CancelOutcome::BeingReplaced => {
                problems.push("one is being edited; try again in a moment".to_owned());
            }
            CancelOutcome::Rejected(text) => problems.push(text.clone()),
            CancelOutcome::Unknown(_) => problems.push(
                "one did not get an answer: it may or may not be cancelled, and is being read again"
                    .to_owned(),
            ),
        }
    }
    let asked = match requested {
        0 => String::new(),
        1 => "Cancel requested. The order stays until the broker confirms.".to_owned(),
        n => format!("{n} cancels requested. The orders stay until the broker confirms."),
    };
    if problems.is_empty() {
        (asked, false)
    } else if asked.is_empty() {
        let text = problems.join("; ");
        (capitalise(&text), true)
    } else {
        (format!("{asked} But {}.", problems.join("; ")), true)
    }
}

fn capitalise(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |c| {
        c.to_uppercase().collect::<String>() + chars.as_str()
    })
}

/// Why an order cannot be edited, or `None` when it can.
pub fn edit_blocker(
    order: &LiveOrder,
    caps: Option<&Capabilities>,
    cancelling: bool,
) -> Option<&'static str> {
    if order.status != OrderStatus::Open {
        return Some("Only open orders can be edited.");
    }
    if cancelling {
        return Some("A cancel is in progress for this order.");
    }
    if !matches!(order.order_type, OrderType::Limit | OrderType::StopLimit) {
        return Some("This type of order cannot be edited.");
    }
    if matches!(order.size, Some(OrderSize::Quote(_))) || order.size.is_none() {
        return Some("Orders sized by an amount to spend cannot be edited.");
    }
    match caps {
        Some(caps) if caps.native_edit => None,
        Some(_) => Some("This broker does not support editing this market's orders."),
        None => Some("The market's rules have not been read yet."),
    }
}

/// The type column.
pub fn type_label(order: &LiveOrder) -> String {
    match order.order_type {
        OrderType::Market => "Market".to_owned(),
        OrderType::Limit if order.post_only => "Limit, post-only".to_owned(),
        OrderType::Limit => "Limit".to_owned(),
        OrderType::StopLimit => "Stop-limit".to_owned(),
        OrderType::Other => "Other".to_owned(),
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

fn side_color(side: Side) -> Color32 {
    match side {
        Side::Buy => theme::UP,
        Side::Sell => theme::DOWN,
    }
}

fn time_text(ms: Option<i64>) -> String {
    let Some(ms) = ms else {
        return "—".to_owned();
    };
    let secs = ms.div_euclid(1000) + crate::service::timezone::local_utc_offset_secs(ms);
    let day = secs.rem_euclid(86_400);
    format!("{:02}:{:02}:{:02}", day / 3600, day % 3600 / 60, day % 60)
}

fn price_of(order: &LiveOrder, snapshot: &BrokerSnapshot) -> String {
    let Some(price) = order.marker_price() else {
        return "Market".to_owned();
    };
    snapshot.product(&order.product).map_or_else(
        || rules::display(price),
        |spec| rules::price_display(price, spec),
    )
}

fn size_text(order: &LiveOrder) -> String {
    match order.size {
        Some(OrderSize::Base(size)) => rules::display(size),
        Some(OrderSize::Quote(amount)) => format!("{} (spend)", rules::display(amount)),
        None => "—".to_owned(),
    }
}

// ---- The panel -----------------------------------------------------------------------------------

/// Chooses the venue of a tab. Shown only when a broker is saved or connected, so a user with
/// paper trading alone sees no change. Returns true when the venue changed.
pub fn venue_switch(ui: &mut egui::Ui, venue: &mut TradingVenue, host: &BrokerHost) -> bool {
    let mut options: Vec<(TradingVenue, String)> = vec![(TradingVenue::Paper, "Paper".to_owned())];
    for broker in host.brokers() {
        let id = broker.descriptor().id;
        let present = !matches!(
            host.connection(id),
            crate::host::broker_host::Connection::NotConfigured
        );
        if present || *venue == TradingVenue::Broker(id) {
            options.push((
                TradingVenue::Broker(id),
                broker.descriptor().display_name.to_owned(),
            ));
        }
    }
    if options.len() == 1 {
        return false;
    }
    let before = *venue;
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        ui.spacing_mut().item_spacing.x = 10.0;
        ui.label(kit::text(
            "Trade on",
            kit::size::CAPTION + 0.5,
            kit::MEDIUM,
            theme::text_secondary(),
        ));
        let labelled: Vec<(TradingVenue, &str)> = options
            .iter()
            .map(|(v, name)| (*v, name.as_str()))
            .collect();
        kit::segmented(ui, Id::new("broker-venue"), &labelled, venue);
    });
    ui.add_space(6.0);
    *venue != before
}

pub struct PanelInput {
    /// The focused chart's market, if it is a market chart.
    pub market: Option<MarketRef>,
    /// A replay is running: no live order may be started.
    pub replay: bool,
}

/// The ticket, the edit sheet and the cancel question, drawn when the panel itself is not (an order
/// started from a chart, with the panel closed).
pub fn dialogs_only(
    ctx: &egui::Context,
    state: &mut BrokerPanelState,
    host: &BrokerHost,
    broker_id: BrokerId,
) {
    let Some(snapshot) = host.snapshot(broker_id) else {
        // The connection is gone: so are the questions about it.
        state.ticket = None;
        state.edit = None;
        state.cancel_ask = None;
        return;
    };
    poll_flights(state);
    dialogs(ctx, state, host, broker_id, &snapshot);
}

impl BrokerPanelState {
    /// Whether any question is open (the workspace keeps drawing them while the panel is closed).
    pub fn has_dialog(&self) -> bool {
        self.ticket.is_some() || self.edit.is_some() || self.cancel_ask.is_some()
    }

    /// The message the panel last showed, if it is still fresh: what a closed panel cannot show.
    pub fn take_notice(&mut self) -> Option<(String, bool)> {
        self.notice.take().map(|(text, error, _)| (text, error))
    }

    /// Closes the ticket and the edit sheet: a replay started, and no order may be made from one.
    pub fn close_order_entry(&mut self) -> bool {
        let open = self.ticket.is_some() || self.edit.is_some();
        self.ticket = None;
        self.edit = None;
        open
    }

    /// Asks to cancel one order (a chart's ✕), following the user's confirmation choice.
    pub fn cancel_one(&mut self, host: &BrokerHost, broker: BrokerId, id: OrderId) {
        let confirm = host.prefs(broker).confirm_single_cancel;
        request_cancel(self, host, broker, vec![id], confirm);
    }

    /// Shows `tab`. The pictures and tests use it; the user does through the tab bar.
    #[cfg(test)]
    pub(crate) fn set_tab(&mut self, tab: BrokerTab) {
        self.tab = tab;
    }

    #[cfg(test)]
    pub(crate) fn cancel_ask_open(&self) -> bool {
        self.cancel_ask.is_some()
    }

    #[cfg(test)]
    pub(crate) fn request_cancel_for_test(
        &mut self,
        host: &BrokerHost,
        broker: BrokerId,
        ids: Vec<OrderId>,
        confirm: bool,
    ) {
        request_cancel(self, host, broker, ids, confirm);
    }

    #[cfg(test)]
    pub(crate) fn edit_ready(&self) -> bool {
        self.edit.as_ref().is_some_and(|e| {
            e.preview_for.is_some()
                && e.preview_for.as_deref() == Some(e.seen.as_str())
                && e.preview.as_ref().is_some_and(EditPreview::is_acceptable)
        })
    }

    #[cfg(test)]
    pub(crate) fn edit_review_enabled(&self) -> bool {
        self.edit.as_ref().is_some_and(|e| e.review_enabled)
    }

    #[cfg(test)]
    pub(crate) fn edit_preview_failed(&self) -> bool {
        self.edit
            .as_ref()
            .is_some_and(|e| e.preview_error.is_some())
    }

    #[cfg(test)]
    pub(crate) fn retry_edit_preview(&mut self) {
        if let Some(edit) = self.edit.as_mut() {
            edit.retry_preview();
        }
    }

    #[cfg(test)]
    pub(crate) fn edit_confirming(&self) -> bool {
        self.edit
            .as_ref()
            .is_some_and(|e| matches!(e.stage, EditStage::Confirming(..)))
    }

    #[cfg(test)]
    pub(crate) fn edit_open(&self) -> bool {
        self.edit.is_some()
    }

    #[cfg(test)]
    pub(crate) fn press_edit_review(&mut self) {
        self.force_edit_review = true;
    }

    /// Opens the ticket for `product` with `draft` already filled (a click on the chart, say).
    #[allow(dead_code)] // The chart's menu and order buttons open it next.
    pub fn open_ticket(
        &mut self,
        broker: BrokerId,
        product: degenview_brokers::ProductId,
        draft: TicketDraft,
    ) {
        self.ticket = Some(TicketState::new(broker, product, draft));
    }

    fn say(&mut self, text: impl Into<String>, error: bool) {
        self.notice = Some((text.into(), error, Instant::now()));
    }

    /// Starts editing `order` (it must be eligible).
    pub fn begin_edit(&mut self, broker: BrokerId, order: LiveOrder, price: Option<Decimal>) {
        let price_text = price
            .or(order.limit_price)
            .map(|p| p.to_string())
            .unwrap_or_default();
        let size_text = match order.size {
            Some(OrderSize::Base(size)) => size.to_string(),
            _ => String::new(),
        };
        self.edit = Some(EditState {
            broker,
            order,
            price_text,
            size_text,
            preview: None,
            preview_for: None,
            preview_pending: None,
            preview_error: None,
            preview_at: None,
            seen: String::new(),
            changed_at: Instant::now(),
            stage: EditStage::Editing,
            error: None,
            #[cfg(test)]
            review_enabled: false,
        });
    }
}

pub fn ui(
    ui: &mut egui::Ui,
    state: &mut BrokerPanelState,
    host: &BrokerHost,
    broker_id: BrokerId,
    input: &PanelInput,
) -> Vec<PanelAction> {
    let mut actions = Vec::new();
    let Some(broker) = degenview_brokers::registry::by_id(broker_id) else {
        return actions;
    };
    ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
    broker_badge::rule(ui);
    let snapshot = host.snapshot(broker_id);
    let prefs = host.prefs(broker_id);

    header(
        ui,
        state,
        host,
        broker,
        snapshot.as_deref(),
        input,
        &mut actions,
    );
    divider(ui);

    let Some(snapshot) = snapshot else {
        let message = if host.connection(broker_id).is_saved() {
            "Connecting…"
        } else {
            "Connect this broker in Settings ▸ Brokers to trade here."
        };
        if empty_with_settings(ui, message) {
            actions.push(PanelAction::OpenSettings);
        }
        return actions;
    };

    banners(ui, &snapshot, &mut actions);
    if let Some((text, error, at)) = &state.notice {
        if at.elapsed() < Duration::from_secs(12) {
            note(ui, text, *error);
        } else {
            state.notice = None;
        }
    }
    poll_flights(state);
    tab_row(ui, state, &snapshot);
    divider(ui);

    match state.tab {
        BrokerTab::Orders => orders_tab(ui, state, host, broker_id, &snapshot, &prefs),
        BrokerTab::History => history_tab(ui, state, host, broker_id),
        BrokerTab::Fills => fills_tab(ui, state, host, broker_id, &snapshot),
        BrokerTab::Balances => balances_tab(ui, &snapshot),
    }
    // The tables leave room for the orders footer, which only the Orders tab draws. Whatever is
    // left is taken, so the panel's content is as tall as the room it was given: a resizable panel
    // sizes itself to its content, and one that is short by a footer shrinks every frame, so a
    // drag of its edge never sticks.
    let rest = ui.available_height();
    if rest > 0.0 {
        ui.allocate_exact_size(egui::vec2(ui.available_width(), rest), egui::Sense::hover());
    }

    dialogs(ui.ctx(), state, host, broker_id, &snapshot);
    actions
}

fn empty_with_settings(ui: &mut egui::Ui, message: &str) -> bool {
    crate::ui::paper_tables::empty_with_action(
        ui,
        Icon::ArrowLeftRight,
        "Live trading",
        message,
        Some("Open Settings"),
    )
}

fn divider(ui: &mut egui::Ui) {
    let (line, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().rect_filled(line, 0.0, theme::separator());
}

fn note(ui: &mut egui::Ui, text: &str, error: bool) {
    let tint = if error {
        kit::RED
    } else {
        theme::text_secondary()
    };
    egui::Frame::new()
        .fill(theme::with_alpha(tint, 0.1))
        .inner_margin(egui::Margin::symmetric(12, 7))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(
                egui::Label::new(kit::text(
                    text,
                    kit::size::CAPTION + 0.5,
                    kit::REGULAR,
                    tint,
                ))
                .wrap(),
            );
        });
}

fn header(
    ui: &mut egui::Ui,
    state: &mut BrokerPanelState,
    host: &BrokerHost,
    broker: &'static dyn Broker,
    snapshot: Option<&BrokerSnapshot>,
    input: &PanelInput,
    actions: &mut Vec<PanelAction>,
) {
    let descriptor = broker.descriptor();
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(12, 9))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                broker_badge::badge(ui, descriptor.display_name);
                if let Some(snapshot) = snapshot {
                    let (text, tone) =
                        crate::ui::settings_brokers::status_badge_for(&snapshot.status);
                    kit::status_badge(ui, &text, tone);
                    if let Some(account) =
                        snapshot.session.as_ref().and_then(|s| s.accounts.first())
                    {
                        ui.label(kit::text(
                            &account.label,
                            kit::size::BODY,
                            kit::REGULAR,
                            theme::text_secondary(),
                        ));
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if icons::button(
                        ui,
                        Icon::Gear,
                        22.0,
                        theme::text_secondary(),
                        "Brokers settings",
                    )
                    .clicked()
                    {
                        actions.push(PanelAction::OpenSettings);
                    }
                    if icons::button(
                        ui,
                        Icon::Xmark,
                        22.0,
                        theme::text_secondary(),
                        "Close panel",
                    )
                    .clicked()
                    {
                        actions.push(PanelAction::Close);
                    }
                    let can_trade = snapshot.is_some_and(|s| {
                        s.can_send_orders()
                            && s.session.as_ref().is_some_and(|x| x.permissions.can_trade)
                    });
                    if kit::sheet_button(ui, "New order", true, can_trade).clicked() {
                        new_order(state, host, broker, input);
                    }
                });
            });
        });
}

/// Opens a ticket for the focused chart's market, if that chart is a market this broker trades.
fn new_order(
    state: &mut BrokerPanelState,
    host: &BrokerHost,
    broker: &'static dyn Broker,
    input: &PanelInput,
) {
    let descriptor = broker.descriptor();
    if input.replay {
        state.say("Not available during replay.", true);
        return;
    }
    let Some(market) = &input.market else {
        state.say(
            "Focus a market chart first: the order is for the chart's market.",
            true,
        );
        return;
    };
    match broker.resolve_market(market) {
        ProductResolution::Market(product) => {
            let prefs = host.prefs(descriptor.id);
            let mut draft = TicketDraft::new(Side::Buy);
            draft.spend_in_quote = prefs.spend_in_quote_by_default;
            state.ticket = Some(TicketState::new(descriptor.id, product, draft));
        }
        ProductResolution::OtherSource { .. } => state.say(
            format!(
                "This chart's data does not come from {name}, so it is not a {name} market. Open the market's {name} chart to trade it.",
                name = descriptor.display_name
            ),
            true,
        ),
        ProductResolution::Unrecognised => state.say(
            format!("{} is not a market {} can trade.", market.symbol, descriptor.display_name),
            true,
        ),
    }
}

fn banners(ui: &mut egui::Ui, snapshot: &BrokerSnapshot, actions: &mut Vec<PanelAction>) {
    match &snapshot.status {
        Status::Failed(error) => {
            note(
                ui,
                &format!("{error} Open Settings to enter a new key."),
                true,
            );
            let _ = actions;
        }
        Status::Reconnecting { reason, .. } => note(
            ui,
            &format!(
                "Reconnecting ({reason}) What is shown may be out of date, and new orders are paused."
            ),
            false,
        ),
        _ => {
            if let Some(session) = &snapshot.session
                && !session.permissions.can_trade
            {
                note(
                    ui,
                    "This key can view but not trade. Create a key with the Trade permission to place orders.",
                    false,
                );
            }
            if let Some(error) = &snapshot.journal_error {
                note(ui, &error.to_string(), true);
            }
            if let Some(error) = &snapshot.last_error {
                note(ui, error, true);
            }
        }
    }
}

fn tab_row(ui: &mut egui::Ui, state: &mut BrokerPanelState, snapshot: &BrokerSnapshot) {
    let working = snapshot
        .orders
        .iter()
        .filter(|v| v.order.status.is_working())
        .count()
        + snapshot.pending.len();
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            kit::tab_bar_slim(
                ui,
                Id::new("broker-tabs"),
                &[
                    kit::TabItem {
                        value: BrokerTab::Orders,
                        title: "Orders",
                        icon: Icon::List,
                        count: working,
                    },
                    kit::TabItem {
                        value: BrokerTab::History,
                        title: "History",
                        icon: Icon::List,
                        count: 0,
                    },
                    kit::TabItem {
                        value: BrokerTab::Fills,
                        title: "Fills",
                        icon: Icon::List,
                        count: 0,
                    },
                    kit::TabItem {
                        value: BrokerTab::Balances,
                        title: "Balances",
                        icon: Icon::List,
                        count: 0,
                    },
                ],
                &mut state.tab,
            );
        });
}

fn poll_flights(state: &mut BrokerPanelState) {
    let mut done = Vec::new();
    for (index, flight) in state.flights.iter().enumerate() {
        match flight {
            Flight::Cancel(pending) => {
                if let Some(answer) = pending.poll() {
                    done.push((index, Some(answer.map(|r| cancel_summary(&r)))));
                }
            }
            Flight::Retry(pending) => {
                if let Some(answer) = pending.poll() {
                    done.push((
                        index,
                        Some(
                            answer
                                .map(|_| ("The order was found and is placed.".to_owned(), false)),
                        ),
                    ));
                }
            }
        }
    }
    for (index, answer) in done.into_iter().rev() {
        state.flights.remove(index);
        match answer {
            Some(Ok((text, error))) if !text.is_empty() => state.say(text, error),
            Some(Err(error)) => state.say(error.to_string(), true),
            _ => {}
        }
    }
}

// ---- Orders --------------------------------------------------------------------------------------

fn orders_tab(
    ui: &mut egui::Ui,
    state: &mut BrokerPanelState,
    host: &BrokerHost,
    broker_id: BrokerId,
    snapshot: &BrokerSnapshot,
    prefs: &BrokerPrefs,
) {
    enum Row<'a> {
        Pending(&'a degenview_brokers::PendingView),
        Order(&'a degenview_brokers::model::reducer::OrderView),
    }
    let rows: Vec<Row> = snapshot
        .pending
        .iter()
        .map(Row::Pending)
        .chain(snapshot.orders.iter().map(Row::Order))
        .collect();
    if rows.is_empty() {
        empty(
            ui,
            Icon::List,
            "No orders",
            "Working orders rest here until they fill or are cancelled.",
        );
        return;
    }
    const COLS: [Col; 9] = [
        col("", 30.0, 0.0, false),
        col("Market", 110.0, 1.2, false),
        col("Side", 64.0, 0.0, false),
        col("Type", 120.0, 1.0, false),
        col("Price", 110.0, 1.0, true),
        col("Filled / Size", 130.0, 1.0, true),
        col("Status", 120.0, 0.0, false),
        col("Time", 80.0, 0.0, true),
        col("", 76.0, 0.0, true),
    ];
    let mut cancel_one: Option<OrderId> = None;
    let mut edit_one: Option<LiveOrder> = None;
    let mut retry: Option<ClientOrderId> = None;
    let mut toggled: Option<OrderId> = None;
    grid::show(ui, "broker-orders", &COLS, rows.len(), |ui, row, column| {
        let weak = theme::text_secondary();
        let primary = theme::text_primary();
        match &rows[row] {
            Row::Pending(pending) => match column {
                1 => grid::text_cell(ui, pending.draft.product.as_str(), primary),
                2 => grid::tag(
                    ui,
                    if pending.draft.side == Side::Buy {
                        "BUY"
                    } else {
                        "SELL"
                    },
                    side_color(pending.draft.side),
                ),
                3 => grid::text_cell(ui, pending_type(&pending.draft.kind), weak),
                4 => grid::figure(ui, &pending_price(&pending.draft.kind), primary),
                5 => grid::figure(ui, &pending_size(&pending.draft.kind), primary),
                6 => {
                    let (label, tone) = match pending.state {
                        PendingState::Sending => ("Sending…", Tone::Neutral),
                        PendingState::Unknown => ("Unconfirmed", Tone::Warning),
                    };
                    kit::status_badge(ui, label, tone);
                }
                7 => grid::figure(ui, &time_text(Some(pending.created_ms)), weak),
                8 if pending.state == PendingState::Unknown => {
                    let clicked = kit::sheet_button(ui, "Check", false, true)
                        .on_hover_text(
                            "Look the order up at the broker; sends it again only if it is not there",
                        )
                        .clicked();
                    if clicked {
                        retry = Some(pending.client_id.clone());
                    }
                }
                _ => {}
            },
            Row::Order(view) => {
                let order = &view.order;
                let working = order.status.is_working();
                let dim = if working { primary } else { weak };
                match column {
                    0 if working => {
                        let mut on = state.selected.contains(&order.id);
                        if ui
                            .add(egui::Checkbox::without_text(&mut on))
                            .on_hover_text("Select to cancel several orders at once")
                            .changed()
                        {
                            toggled = Some(order.id.clone());
                        }
                    }
                    1 => grid::text_cell(ui, order.product.as_str(), dim),
                    2 => grid::tag(
                        ui,
                        if order.side == Side::Buy {
                            "BUY"
                        } else {
                            "SELL"
                        },
                        side_color(order.side),
                    ),
                    3 => grid::text_cell(ui, &type_label(order), weak),
                    4 => grid::figure(ui, &price_of(order, snapshot), dim),
                    5 => grid::figure(
                        ui,
                        &format!(
                            "{} / {}",
                            rules::display(order.filled_size),
                            size_text(order)
                        ),
                        dim,
                    ),
                    6 => {
                        let (label, status_tone) = if view.overlay.is_some() && working {
                            match view.overlay.map(|o| o.kind) {
                                Some(
                                    degenview_brokers::model::reducer::OverlayKind::CancelRequested,
                                ) => ("Cancel requested", StatusTone::Warning),
                                _ => ("Edit requested", StatusTone::Warning),
                            }
                        } else {
                            rules::status_label(order.status)
                        };
                        kit::status_badge(ui, label, tone(status_tone));
                    }
                    7 => grid::figure(ui, &time_text(order.created_ms), weak),
                    8 if working => {
                        let caps = snapshot.capabilities_of(&order.product);
                        let blocker = edit_blocker(order, caps, view.overlay.is_some());
                        let pencil = ui
                            .add_enabled_ui(blocker.is_none(), |ui| {
                                icons::button(
                                    ui,
                                    Icon::Pencil,
                                    20.0,
                                    theme::text_secondary(),
                                    blocker.unwrap_or("Edit price or size"),
                                )
                            })
                            .inner;
                        if pencil.clicked() {
                            edit_one = Some(order.clone());
                        }
                        let can_cancel = view.overlay.is_none();
                        let cross = ui
                            .add_enabled_ui(can_cancel, |ui| {
                                icons::button(ui, Icon::Xmark, 20.0, kit::RED, "Cancel order")
                            })
                            .inner;
                        if cross.clicked() {
                            cancel_one = Some(order.id.clone());
                        }
                    }
                    _ => {}
                }
            }
        }
    });

    if let Some(id) = toggled
        && !state.selected.remove(&id)
    {
        state.selected.insert(id);
    }
    // Selection only ever holds orders that are still working.
    let working_ids: BTreeSet<OrderId> = snapshot
        .orders
        .iter()
        .filter(|v| v.order.status.is_working())
        .map(|v| v.order.id.clone())
        .collect();
    state.selected.retain(|id| working_ids.contains(id));

    if let Some(client_id) = retry
        && let Some(runtime) = host.runtime(broker_id)
    {
        state.flights.push(Flight::Retry(runtime.retry(client_id)));
        state.say("Looking the order up at the broker…", false);
    }
    if let Some(order) = edit_one {
        state.begin_edit(broker_id, order, None);
    }
    if let Some(id) = cancel_one {
        request_cancel(
            state,
            host,
            broker_id,
            vec![id],
            prefs.confirm_single_cancel,
        );
    }

    // The bar under the table.
    let selected = state.selected.len();
    let open = working_ids.len();
    let mut cancel_selected = false;
    let mut cancel_all = false;
    let (bar, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), grid::FOOTER_HEIGHT),
        egui::Sense::hover(),
    );
    ui.painter().rect_filled(bar, 0.0, kit::window());
    ui.painter().hline(
        bar.x_range(),
        bar.top(),
        egui::Stroke::new(1.0, theme::separator()),
    );
    let mut row = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(bar.shrink2(egui::vec2(20.0, 0.0)))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    row.spacing_mut().item_spacing.x = 10.0;
    if kit::sheet_button(&mut row, &format!("Cancel all ({open})"), false, open > 0).clicked() {
        cancel_all = true;
    }
    if kit::sheet_button(
        &mut row,
        &format!("Cancel selected ({selected})"),
        false,
        selected > 0,
    )
    .clicked()
    {
        cancel_selected = true;
    }
    row.label(kit::text(
        format!("{open} working"),
        kit::size::CAPTION + 0.5,
        kit::REGULAR,
        theme::text_secondary(),
    ));
    if cancel_selected {
        let ids: Vec<OrderId> = state.selected.iter().cloned().collect();
        request_cancel(state, host, broker_id, ids, true);
    }
    if cancel_all {
        // Always asks, and says which orders: what is cancelled is what is shown now.
        let ids: Vec<OrderId> = working_ids.into_iter().collect();
        request_cancel(state, host, broker_id, ids, true);
    }
}

fn pending_type(kind: &OrderKind) -> &'static str {
    match kind {
        OrderKind::Market { .. } => "Market",
        OrderKind::Limit { .. } => "Limit",
        OrderKind::StopLimit { .. } => "Stop-limit",
    }
}

fn pending_price(kind: &OrderKind) -> String {
    match kind {
        OrderKind::Market { .. } => "Market".to_owned(),
        OrderKind::Limit { price, .. } => rules::display(*price),
        OrderKind::StopLimit { stop_price, .. } => rules::display(*stop_price),
    }
}

fn pending_size(kind: &OrderKind) -> String {
    match kind {
        OrderKind::Market {
            size: OrderSize::Base(s),
        } => rules::display(*s),
        OrderKind::Market {
            size: OrderSize::Quote(a),
        } => format!("{} (spend)", rules::display(*a)),
        OrderKind::Limit { base_size, .. } | OrderKind::StopLimit { base_size, .. } => {
            rules::display(*base_size)
        }
    }
}

/// Cancels `ids`, asking first when `confirm` (always for more than one).
fn request_cancel(
    state: &mut BrokerPanelState,
    host: &BrokerHost,
    broker: BrokerId,
    ids: Vec<OrderId>,
    confirm: bool,
) {
    if ids.is_empty() {
        return;
    }
    if !confirm && ids.len() == 1 {
        send_cancel(state, host, broker, ids);
        return;
    }
    let many = ids.len();
    state.cancel_ask = Some(CancelAsk {
        title: if many == 1 {
            "Cancel this order?".to_owned()
        } else {
            format!("Cancel {many} orders?")
        },
        message: "The broker is asked to cancel. An order can still fill before the cancel reaches it, and it stays listed until the broker confirms."
            .to_owned(),
        ids,
    });
}

fn send_cancel(
    state: &mut BrokerPanelState,
    host: &BrokerHost,
    broker: BrokerId,
    ids: Vec<OrderId>,
) {
    let Some(runtime) = host.runtime(broker) else {
        return;
    };
    for id in &ids {
        state.selected.remove(id);
    }
    state.flights.push(Flight::Cancel(runtime.cancel(ids)));
    state.say("Asking the broker to cancel…", false);
}

// ---- History, fills, balances ---------------------------------------------------------------------

fn history_tab(
    ui: &mut egui::Ui,
    state: &mut BrokerPanelState,
    host: &BrokerHost,
    broker_id: BrokerId,
) {
    state.history.poll();
    let before = state.history_choice;
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(12, 4))
        .show(ui, |ui| {
            kit::segmented(
                ui,
                Id::new("broker-history-filter"),
                &[
                    (HistoryChoice::All, "All"),
                    (HistoryChoice::Filled, "Filled"),
                    (HistoryChoice::Cancelled, "Cancelled"),
                    (HistoryChoice::Expired, "Expired"),
                    (HistoryChoice::Rejected, "Rejected"),
                ],
                &mut state.history_choice,
            );
        });
    if state.history_choice != before {
        state.history.reset();
    }
    if !state.history.loaded
        && state.history.pending.is_none()
        && state.history.error.is_none()
        && let Some(runtime) = host.runtime(broker_id)
    {
        state.history.pending = Some(runtime.history(
            HistoryFilter {
                statuses: state.history_choice.statuses(),
                limit: 100,
                ..HistoryFilter::default()
            },
            None,
        ));
    }
    if state.history.pending.is_some() {
        ui.ctx().request_repaint_after(Duration::from_millis(120));
    }
    if let Some(error) = &state.history.error {
        note(ui, error, true);
    }
    if state.history.items.is_empty() {
        if state.history.pending.is_some() {
            empty(
                ui,
                Icon::List,
                "Loading history…",
                "Reading finished orders from the broker.",
            );
        } else {
            empty(
                ui,
                Icon::List,
                "No finished orders",
                "Filled, cancelled, expired and rejected orders are listed here.",
            );
        }
        return;
    }
    const COLS: [Col; 8] = [
        col("Time", 90.0, 0.0, false),
        col("Market", 110.0, 1.2, false),
        col("Side", 64.0, 0.0, false),
        col("Type", 120.0, 1.0, false),
        col("Price", 110.0, 1.0, true),
        col("Filled / Size", 130.0, 1.0, true),
        col("Status", 110.0, 0.0, false),
        col("Fees", 90.0, 0.0, true),
    ];
    let items = &state.history.items;
    grid::show(
        ui,
        "broker-history",
        &COLS,
        items.len(),
        |ui, row, column| {
            let order = &items[row];
            let primary = theme::text_primary();
            let weak = theme::text_secondary();
            match column {
                0 => grid::figure(ui, &time_text(order.created_ms), weak),
                1 => grid::text_cell(ui, order.product.as_str(), primary),
                2 => grid::tag(
                    ui,
                    if order.side == Side::Buy {
                        "BUY"
                    } else {
                        "SELL"
                    },
                    side_color(order.side),
                ),
                3 => grid::text_cell(ui, &type_label(order), weak),
                4 => grid::figure(
                    ui,
                    &order
                        .average_price
                        .or(order.limit_price)
                        .map_or("—".to_owned(), rules::display),
                    primary,
                ),
                5 => grid::figure(
                    ui,
                    &format!(
                        "{} / {}",
                        rules::display(order.filled_size),
                        size_text(order)
                    ),
                    primary,
                ),
                6 => {
                    let (label, status_tone) = rules::status_label(order.status);
                    kit::status_badge(ui, label, tone(status_tone));
                }
                _ => grid::figure(ui, &rules::display(order.fees), weak),
            }
        },
    );
    let more = state.history.cursor.is_some();
    if more
        && kit::sheet_button(ui, "Load more", false, state.history.pending.is_none()).clicked()
        && let Some(runtime) = host.runtime(broker_id)
    {
        let cursor = state.history.cursor.clone();
        state.history.pending = Some(runtime.history(
            HistoryFilter {
                statuses: state.history_choice.statuses(),
                limit: 100,
                ..HistoryFilter::default()
            },
            cursor,
        ));
    }
}

fn fills_tab(
    ui: &mut egui::Ui,
    state: &mut BrokerPanelState,
    host: &BrokerHost,
    broker_id: BrokerId,
    snapshot: &BrokerSnapshot,
) {
    state.fills_more.poll();
    let mut all: Vec<&LiveFill> = snapshot
        .fills
        .iter()
        .chain(state.fills_more.items.iter())
        .collect();
    all.sort_by_key(|fill| std::cmp::Reverse(fill.time_ms));
    all.dedup_by(|a, b| a.id == b.id);
    if all.is_empty() {
        empty(
            ui,
            Icon::List,
            "No fills yet",
            "Executions of your orders are listed here.",
        );
        return;
    }
    const COLS: [Col; 7] = [
        col("Time", 90.0, 0.0, false),
        col("Market", 110.0, 1.2, false),
        col("Side", 64.0, 0.0, false),
        col("Price", 110.0, 1.0, true),
        col("Size", 120.0, 1.0, true),
        col("Fee", 100.0, 0.0, true),
        col("Order", 100.0, 1.0, false),
    ];
    grid::show(ui, "broker-fills", &COLS, all.len(), |ui, row, column| {
        let fill = all[row];
        let primary = theme::text_primary();
        let weak = theme::text_secondary();
        match column {
            0 => grid::figure(ui, &time_text(Some(fill.time_ms)), weak),
            1 => grid::text_cell(ui, fill.product.as_str(), primary),
            2 => grid::tag(
                ui,
                if fill.side == Side::Buy {
                    "BUY"
                } else {
                    "SELL"
                },
                side_color(fill.side),
            ),
            3 => grid::figure(
                ui,
                &snapshot.product(&fill.product).map_or_else(
                    || rules::display(fill.price),
                    |spec| rules::price_display(fill.price, spec),
                ),
                primary,
            ),
            4 => grid::figure(ui, &rules::display(fill.size), primary),
            5 => grid::figure(ui, &rules::display(fill.fee), weak),
            _ => grid::figure(ui, &short_id(fill.order_id.as_str()), weak),
        }
    });
    if kit::sheet_button(
        ui,
        "Load older fills",
        false,
        state.fills_more.pending.is_none(),
    )
    .clicked()
        && let Some(runtime) = host.runtime(broker_id)
    {
        let cursor = state.fills_more.cursor.clone();
        state.fills_more.pending = Some(runtime.fills(
            FillFilter {
                limit: 100,
                ..FillFilter::default()
            },
            cursor,
        ));
    }
}

fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

fn balances_tab(ui: &mut egui::Ui, snapshot: &BrokerSnapshot) {
    let mut rows: Vec<&degenview_brokers::Balance> = snapshot
        .balances
        .iter()
        .filter(|b| !b.total().is_zero())
        .collect();
    rows.sort_by(|a, b| a.asset.cmp(&b.asset));
    if rows.is_empty() {
        empty(
            ui,
            Icon::List,
            "No balances",
            "Assets held in this account appear here.",
        );
        return;
    }
    const COLS: [Col; 4] = [
        col("Asset", 100.0, 1.0, false),
        col("Available", 140.0, 1.0, true),
        col("On hold", 140.0, 1.0, true),
        col("Total", 140.0, 1.0, true),
    ];
    grid::show(
        ui,
        "broker-balances",
        &COLS,
        rows.len(),
        |ui, row, column| {
            let balance = rows[row];
            let primary = theme::text_primary();
            match column {
                0 => grid::text_cell(ui, &balance.asset, primary),
                1 => grid::figure(ui, &rules::display(balance.available), primary),
                2 => grid::figure(ui, &rules::display(balance.held), theme::text_secondary()),
                _ => grid::figure_bold(ui, &rules::display(balance.total()), primary),
            }
        },
    );
}

// ---- Dialogs: the ticket, the edit, the cancel question -----------------------------------------------

fn dialogs(
    ctx: &egui::Context,
    state: &mut BrokerPanelState,
    host: &BrokerHost,
    broker_id: BrokerId,
    snapshot: &BrokerSnapshot,
) {
    if let Some(ticket) = &mut state.ticket {
        match broker_ticket::show(ctx, ticket, host) {
            TicketEvent::Open => {}
            TicketEvent::Closed => state.ticket = None,
            TicketEvent::Placed => {
                state.ticket = None;
                state.say(
                    "Order sent. It appears under Orders as the broker confirms it.",
                    false,
                );
            }
            TicketEvent::Unconfirmed(text) => {
                state.ticket = None;
                state.tab = BrokerTab::Orders;
                state.say(
                    format!("{text} It is listed as Unconfirmed until the broker is asked."),
                    true,
                );
            }
        }
    }
    if let Some(ask) = &state.cancel_ask {
        let count = ask.ids.len();
        let mut spec = dialog::Dialog::new(
            Icon::Trash,
            kit::RED,
            &ask.title,
            &ask.message,
            if count == 1 {
                "Cancel order"
            } else {
                "Cancel orders"
            },
        );
        spec.destructive = true;
        spec.dismiss = "Keep";
        let ids = ask.ids.clone();
        let preview: Vec<String> = ids
            .iter()
            .take(8)
            .filter_map(|id| snapshot.orders.iter().find(|v| &v.order.id == id))
            .map(|v| {
                format!(
                    "{} {} {}",
                    if v.order.side == Side::Buy {
                        "Buy"
                    } else {
                        "Sell"
                    },
                    size_text(&v.order),
                    v.order.product
                )
            })
            .collect();
        let outcome = dialog::show(ctx, Id::new("broker-cancel-ask"), &spec, |ui| {
            ui.add_space(10.0);
            for line in &preview {
                ui.label(kit::text(
                    line,
                    kit::size::BODY,
                    kit::REGULAR,
                    theme::text_secondary(),
                ));
            }
            if count > preview.len() {
                ui.label(kit::text(
                    format!("and {} more", count - preview.len()),
                    kit::size::BODY,
                    kit::REGULAR,
                    theme::text_tertiary(),
                ));
            }
        });
        match outcome {
            Some(dialog::Outcome::Accepted) => {
                state.cancel_ask = None;
                send_cancel(state, host, broker_id, ids);
            }
            Some(_) => state.cancel_ask = None,
            None => {}
        }
    }
    edit_sheet(ctx, state, host, snapshot);
}

fn edit_sheet(
    ctx: &egui::Context,
    state: &mut BrokerPanelState,
    host: &BrokerHost,
    snapshot: &BrokerSnapshot,
) {
    let Some(edit) = &mut state.edit else { return };
    let Some(runtime) = host.runtime(edit.broker) else {
        state.edit = None;
        return;
    };
    let spec = snapshot.product(&edit.order.product).cloned();
    // The order as the broker has it now: a fill or a cancel while the sheet is open ends the edit.
    let live = snapshot
        .orders
        .iter()
        .find(|v| v.order.id == edit.order.id)
        .map(|v| v.order.clone());
    if live.as_ref().is_none_or(|o| o.status != OrderStatus::Open)
        && !matches!(edit.stage, EditStage::Sending(_))
    {
        state.edit = None;
        state.say("The order is no longer open, so it was not edited.", true);
        return;
    }
    let change = |edit: &EditState| -> Result<OrderChange, String> {
        let price = rules::parse_decimal(&edit.price_text)
            .map_err(|()| "The price is not a number.".to_owned())?;
        let size = rules::parse_decimal(&edit.size_text)
            .map_err(|()| "The size is not a number.".to_owned())?;
        let change = OrderChange {
            price: price.filter(|p| {
                Some(*p) != edit.order.limit_price && edit.order.order_type == OrderType::Limit
            }),
            stop_price: price.filter(|p| {
                Some(*p) != edit.order.stop_price && edit.order.order_type == OrderType::StopLimit
            }),
            size: size.filter(|s| matches!(edit.order.size, Some(OrderSize::Base(b)) if b != *s)),
        };
        if change.is_empty() {
            Err("Nothing has changed yet.".to_owned())
        } else {
            Ok(change)
        }
    };
    let typed = change(edit);
    let mut problems: Vec<String> = Vec::new();
    if let (Ok(change), Some(spec)) = (&typed, &spec) {
        if let Some(price) = change.price.or(change.stop_price)
            && !degenview_brokers::validate::is_multiple(price, spec.price_increment)
        {
            problems.push(format!(
                "The price must be a multiple of {}. The nearest valid price below is {}.",
                rules::display(spec.price_increment),
                rules::price_display(
                    degenview_brokers::validate::floor_to_increment(price, spec.price_increment),
                    spec
                ),
            ));
        }
        if let Some(size) = change.size {
            if !degenview_brokers::validate::is_multiple(size, spec.base_increment) {
                problems.push(format!(
                    "The size must be a multiple of {}.",
                    rules::display(spec.base_increment)
                ));
            }
            if size < spec.base_min {
                problems.push(format!(
                    "The size is below the minimum of {}.",
                    rules::display(spec.base_min)
                ));
            }
            if size < edit.order.filled_size {
                problems.push("The size cannot be less than what has already filled.".to_owned());
            }
        }
    }
    let valid = typed.is_ok() && problems.is_empty();
    // Preview the exact change once typing pauses.
    let signature = if valid {
        format!("{:?}", typed.as_ref().ok())
    } else {
        String::new()
    };
    if signature != edit.seen {
        edit.seen = signature.clone();
        edit.changed_at = Instant::now();
        edit.preview = None;
        edit.preview_error = None;
        edit.preview_for = None;
        edit.preview_pending = None;
        edit.preview_at = None;
    }
    // An answer older than a minute is a statement about an older market: it is asked for again.
    if edit
        .preview_at
        .is_some_and(|at| at.elapsed() > rules::PREVIEW_FRESH)
        && matches!(edit.stage, EditStage::Editing)
    {
        edit.preview = None;
        edit.preview_for = None;
        edit.preview_at = None;
        edit.changed_at = Instant::now()
            .checked_sub(Duration::from_millis(450))
            .unwrap_or_else(Instant::now);
    }
    if let Some((sig, pending)) = &edit.preview_pending
        && let Some(answer) = pending.poll()
    {
        let sig = sig.clone();
        edit.preview_pending = None;
        if sig == edit.seen {
            match answer {
                Ok(preview) => {
                    edit.preview = Some(preview);
                    edit.preview_at = Some(Instant::now());
                }
                Err(error) => {
                    edit.preview_error = Some(error.to_string());
                    edit.preview_at = None;
                }
            }
            edit.preview_for = Some(sig);
        }
    }
    if valid
        && edit.preview_for.as_deref() != Some(signature.as_str())
        && edit.preview_pending.is_none()
        && edit.changed_at.elapsed() >= Duration::from_millis(450)
        && matches!(edit.stage, EditStage::Editing)
        && let Ok(change) = &typed
    {
        edit.preview_pending = Some((
            signature.clone(),
            runtime.preview_edit(edit.order.id.clone(), *change),
        ));
    }
    ctx.request_repaint_after(Duration::from_millis(120));

    if let EditStage::Sending(pending) = &edit.stage
        && let Some(answer) = pending.poll()
    {
        match answer {
            Ok(_) => {
                state.edit = None;
                state.say(
                    "Edit sent. The order updates when the broker confirms it.",
                    false,
                );
                return;
            }
            Err(error) => {
                if error.kind == degenview_brokers::ErrorKind::OutcomeUnknown {
                    state.edit = None;
                    state.say(
                        format!("{error} The order is being read again to see what happened."),
                        true,
                    );
                    return;
                }
                edit.error = Some(error.to_string());
                edit.stage = EditStage::Editing;
            }
        }
    }

    // The approval.
    if let EditStage::Confirming(change, preview) = &edit.stage {
        let order = &edit.order;
        let message = "This changes the order at the broker. A new price may lose your place in the queue, and the order can fill at its old terms until the change is applied.";
        let mut dialog_spec = dialog::Dialog::new(
            Icon::Pencil,
            broker_badge::LIVE,
            "Change live order?",
            message,
            "Change order",
        );
        dialog_spec.dismiss = "Back";
        dialog_spec.cautious = true;
        let change = *change;
        let outcome = dialog::show(ctx, Id::new("broker-edit-confirm"), &dialog_spec, |ui| {
            ui.add_space(12.0);
            let mut line = |name: &str, value: String| {
                ui.horizontal(|ui| {
                    ui.label(kit::text(
                        name,
                        kit::size::BODY,
                        kit::REGULAR,
                        theme::text_secondary(),
                    ));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(kit::text(
                            value,
                            kit::size::BODY,
                            kit::MEDIUM,
                            theme::text_primary(),
                        ));
                    });
                });
                ui.add_space(4.0);
            };
            line("Market", order.product.to_string());
            line(
                "Action",
                if order.side == Side::Buy {
                    "Buy".to_owned()
                } else {
                    "Sell".to_owned()
                },
            );
            if let Some(new) = change.price.or(change.stop_price) {
                let old = if change.price.is_some() {
                    order.limit_price
                } else {
                    order.stop_price
                };
                line(
                    "Price",
                    format!(
                        "{} → {}",
                        old.map_or("—".to_owned(), rules::display),
                        rules::display(new)
                    ),
                );
            }
            if let Some(new) = change.size {
                line(
                    "Size",
                    format!("{} → {}", size_text(order), rules::display(new)),
                );
            }
            // The broker's figures for the changed order, from a preview of exactly this change.
            if let Some(total) = preview.order_total {
                line("Total (broker's estimate)", rules::display(total));
            }
            if let Some(fee) = preview.fees {
                line("Fee (broker's estimate)", rules::display(fee));
            }
        });
        match outcome {
            Some(dialog::Outcome::Accepted) => {
                edit.error = None;
                let id = edit.order.id.clone();
                edit.stage = EditStage::Sending(runtime.amend(id, change));
            }
            Some(_) => edit.stage = EditStage::Editing,
            None => {}
        }
        return;
    }

    let mut close = false;
    let mut review = false;
    // Review is possible only on the broker's fresh answer that it would accept this exact change.
    let answer = if !valid || edit.preview_for.as_deref() != Some(signature.as_str()) {
        rules::PreviewAnswer::Waiting
    } else {
        match (&edit.preview, &edit.preview_error) {
            (Some(preview), _) if preview.is_acceptable() => rules::PreviewAnswer::Accepted,
            (Some(_), _) => rules::PreviewAnswer::Problems,
            (None, Some(error)) => rules::PreviewAnswer::Failed(error.clone()),
            (None, None) => rules::PreviewAnswer::Waiting,
        }
    };
    let gate = rules::preview_gate(answer, edit.preview_at.map(|at| at.elapsed()));
    let can_review = valid && gate.allows_review();
    #[cfg(test)]
    {
        edit.review_enabled = can_review;
    }
    let mut retry_preview = false;
    let sending = matches!(edit.stage, EditStage::Sending(_));
    sheet::show(ctx, Id::new("broker-edit"), 440.0, |ui| {
        broker_badge::rule(ui);
        sheet::inset(ui, |ui| {
            ui.add_space(20.0);
            ui.label(kit::text(
                "Edit order",
                kit::size::TITLE2 + 1.0,
                kit::SEMIBOLD,
                theme::text_primary(),
            ));
            ui.add_space(2.0);
            ui.label(kit::text(
                format!(
                    "{} {} {} · {}",
                    if edit.order.side == Side::Buy {
                        "Buy"
                    } else {
                        "Sell"
                    },
                    size_text(&edit.order),
                    edit.order.product,
                    type_label(&edit.order)
                ),
                kit::size::SUBHEADLINE + 1.0,
                kit::REGULAR,
                theme::text_secondary(),
            ));
            ui.add_space(16.0);
            let price_title = if edit.order.order_type == OrderType::StopLimit {
                "Stop price"
            } else {
                "Limit price"
            };
            crate::ui::settings_api_keys::field(
                ui,
                price_title,
                None,
                "0",
                &mut edit.price_text,
                false,
            );
            ui.add_space(12.0);
            crate::ui::settings_api_keys::field(ui, "Size", None, "0", &mut edit.size_text, false);
            ui.add_space(8.0);
            if let Err(text) = &typed
                && !text.starts_with("Nothing")
            {
                ui.label(kit::text(
                    text,
                    kit::size::CAPTION + 0.5,
                    kit::REGULAR,
                    kit::RED,
                ));
            }
            for problem in &problems {
                ui.label(kit::text(
                    problem,
                    kit::size::CAPTION + 0.5,
                    kit::REGULAR,
                    kit::RED,
                ));
            }
            if let Some(preview) = &edit.preview {
                for problem in &preview.problems {
                    ui.label(kit::text(
                        &problem.text,
                        kit::size::CAPTION + 0.5,
                        kit::MEDIUM,
                        kit::RED,
                    ));
                }
                if preview.is_acceptable()
                    && let Some(total) = preview.order_total
                {
                    ui.label(kit::text(
                        format!(
                            "Broker's figure for the changed order: {}",
                            rules::display(total)
                        ),
                        kit::size::CAPTION + 0.5,
                        kit::REGULAR,
                        theme::text_secondary(),
                    ));
                }
            }
            if let rules::PreviewGate::Failed(error) = &gate {
                ui.label(kit::text(
                    format!(
                        "The broker could not preview this change ({error}). It cannot be sent without that."
                    ),
                    kit::size::CAPTION + 0.5,
                    kit::REGULAR,
                    kit::RED,
                ));
                ui.add_space(4.0);
                if kit::push_button(ui, None, "Try again", true).clicked() {
                    retry_preview = true;
                }
            } else if gate == rules::PreviewGate::Waiting && valid {
                ui.label(kit::text(
                    "Asking the broker to check this change…",
                    kit::size::CAPTION + 0.5,
                    kit::REGULAR,
                    theme::text_tertiary(),
                ));
            }
            if let Some(error) = &edit.error {
                ui.add_space(8.0);
                kit::notice_card(
                    ui,
                    Icon::TriangleFill,
                    kit::RED,
                    "Order not changed",
                    Some(error),
                    None,
                );
            }
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if sending {
                        ui.spinner();
                    } else if kit::sheet_button(ui, "Review change", true, can_review).clicked() {
                        review = true;
                    }
                    if kit::sheet_button(ui, "Cancel", false, !sending).clicked() {
                        close = true;
                    }
                });
            });
            ui.add_space(20.0);
        });
    });
    #[cfg(test)]
    if std::mem::take(&mut state.force_edit_review) && can_review {
        review = true;
    }
    if retry_preview {
        edit.retry_preview();
    }
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && !sending {
        close = true;
    }
    if close {
        state.edit = None;
    } else if review
        && can_review
        && let Ok(change) = typed
        && let Some(preview) = edit.preview.clone()
    {
        edit.stage = EditStage::Confirming(change, Box::new(preview));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::broker_ticket::TicketKind;
    use degenview_brokers::{OrderStatus, ProductId, TifKind, Tradability};
    use std::str::FromStr;

    fn d(text: &str) -> Decimal {
        Decimal::from_str(text).unwrap()
    }

    fn order(status: OrderStatus, kind: OrderType, size: Option<OrderSize>) -> LiveOrder {
        LiveOrder {
            id: OrderId::new("o"),
            client_id: None,
            product: ProductId::new("BTC-USD"),
            side: Side::Buy,
            order_type: kind,
            status,
            tif: None,
            size,
            limit_price: Some(d("100")),
            stop_price: None,
            stop_direction: None,
            post_only: false,
            filled_size: d("0"),
            filled_value: d("0"),
            average_price: None,
            fees: d("0"),
            created_ms: Some(0),
            updated_ms: None,
            reason: None,
            stop_triggered: false,
        }
    }

    fn caps(native_edit: bool) -> Capabilities {
        Capabilities {
            tradable: Tradability::Yes,
            order_types: vec![OrderType::Limit],
            tif: vec![TifKind::GoodTilCancelled],
            stop_limit_tif: vec![],
            post_only: false,
            quote_sized_market: false,
            native_edit,
            atomic_replace: false,
            brackets: false,
        }
    }

    #[test]
    fn a_cancel_request_is_described_as_a_request_never_as_done() {
        let requested = |id: &str| CancelResult {
            order_id: OrderId::new(id),
            outcome: CancelOutcome::Requested,
        };
        let (text, error) = cancel_summary(&[requested("a")]);
        assert!(text.contains("requested") && text.contains("until the broker confirms"));
        assert!(!error);
        assert!(
            cancel_summary(&[requested("a"), requested("b")])
                .0
                .starts_with("2 cancels requested")
        );
    }

    #[test]
    fn cancels_that_lost_a_race_or_were_refused_are_called_problems() {
        let result = |outcome| CancelResult {
            order_id: OrderId::new("a"),
            outcome,
        };
        let (text, error) = cancel_summary(&[result(CancelOutcome::AlreadyFilled)]);
        assert!(error && text.contains("already filled"), "{text}");
        let (text, error) = cancel_summary(&[
            result(CancelOutcome::Requested),
            result(CancelOutcome::Rejected(
                "This order cannot be cancelled.".to_owned(),
            )),
        ]);
        assert!(
            error && text.contains("Cancel requested") && text.contains("cannot be cancelled"),
            "{text}"
        );
        assert!(
            cancel_summary(&[result(CancelOutcome::NotFound)])
                .0
                .contains("could not be found")
        );
        assert!(
            cancel_summary(&[result(CancelOutcome::BeingReplaced)])
                .0
                .contains("edited")
        );
    }

    #[test]
    fn only_an_open_priced_base_sized_order_with_native_edit_can_be_edited() {
        let open = order(
            OrderStatus::Open,
            OrderType::Limit,
            Some(OrderSize::Base(d("1"))),
        );
        assert_eq!(edit_blocker(&open, Some(&caps(true)), false), None);
        assert!(
            edit_blocker(&open, Some(&caps(false)), false)
                .unwrap()
                .contains("does not support")
        );
        assert!(
            edit_blocker(&open, None, false)
                .unwrap()
                .contains("not been read")
        );
        assert!(
            edit_blocker(&open, Some(&caps(true)), true)
                .unwrap()
                .contains("cancel is in progress")
        );
        let queued = order(
            OrderStatus::CancelQueued,
            OrderType::Limit,
            Some(OrderSize::Base(d("1"))),
        );
        assert!(
            edit_blocker(&queued, Some(&caps(true)), false)
                .unwrap()
                .contains("Only open")
        );
        let market = order(
            OrderStatus::Open,
            OrderType::Other,
            Some(OrderSize::Base(d("1"))),
        );
        assert!(
            edit_blocker(&market, Some(&caps(true)), false)
                .unwrap()
                .contains("type of order")
        );
        let spend = order(
            OrderStatus::Open,
            OrderType::Limit,
            Some(OrderSize::Quote(d("10"))),
        );
        assert!(
            edit_blocker(&spend, Some(&caps(true)), false)
                .unwrap()
                .contains("amount to spend")
        );
    }

    #[test]
    fn history_filters_ask_for_finished_statuses_only() {
        for choice in [
            HistoryChoice::All,
            HistoryChoice::Filled,
            HistoryChoice::Cancelled,
            HistoryChoice::Expired,
            HistoryChoice::Rejected,
        ] {
            assert!(
                choice.statuses().iter().all(|s| s.is_terminal()),
                "{choice:?}"
            );
        }
        assert_eq!(
            HistoryChoice::Rejected.statuses(),
            vec![OrderStatus::Failed]
        );
    }

    #[test]
    fn rows_say_what_they_are() {
        let mut o = order(
            OrderStatus::Open,
            OrderType::Limit,
            Some(OrderSize::Base(d("0.5"))),
        );
        o.post_only = true;
        assert_eq!(type_label(&o), "Limit, post-only");
        assert_eq!(size_text(&o), "0.5");
        assert_eq!(
            size_text(&order(
                OrderStatus::Open,
                OrderType::Market,
                Some(OrderSize::Quote(d("25")))
            )),
            "25 (spend)"
        );
        assert_eq!(time_text(None), "—");
        assert_eq!(
            pending_price(&OrderKind::Market {
                size: OrderSize::Base(d("1"))
            }),
            "Market"
        );
        assert_eq!(TicketKind::Market.label(), "Market");
        assert_eq!(short_id("abcdefghijkl"), "abcdefgh");
    }
}
