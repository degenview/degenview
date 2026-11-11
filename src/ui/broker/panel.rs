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
    CancelOutcome, CancelResult, FillFilter, HistoryFilter, Page, PlacedOrder,
};
use degenview_brokers::{
    BrokerId, BrokerSnapshot, ClientOrderId, Decimal, LiveFill, LiveOrder, OrderId, OrderStatus,
    Pending, Side, Status,
};
use eframe::egui::{self, Color32, Id};

use crate::host::broker_host::BrokerHost;
use crate::model::broker::prefs::BrokerPrefs;
use crate::model::broker::ticket::{self as rules, StatusTone, size_text};
use crate::trading::edit::{
    EditEvent, EditForm, EditFrame, EditSession, EditVenue, EditView, ReviewLine, Step,
};
use crate::trading::model::AccountStats;
use crate::ui::icons::Icon;
use crate::ui::paper::grid::{self as grid, Col, col};
use crate::ui::paper::tables::empty;
use crate::ui::trade::panel_header::{self, Chip, Header, HeaderEvent, PanelEvent, VenueBar};
use crate::ui::trade::tables::{self, OrderAction};
use crate::ui::trade::{notice, stats_strip};
use crate::ui::{
    broker::badge,
    kit::{self, dialog, sheet},
    theme,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BrokerTab {
    Orders,
    History,
    Fills,
    Balances,
}

impl BrokerTab {
    fn title(self) -> &'static str {
        match self {
            Self::Orders => "Orders",
            Self::History => "History",
            Self::Fills => "Fills",
            Self::Balances => "Balances",
        }
    }

    fn icon(self) -> crate::ui::icons::Icon {
        use crate::ui::icons::Icon;
        match self {
            Self::Orders => Icon::List,
            Self::History => Icon::Clock,
            Self::Fills => Icon::Exchange,
            Self::Balances => Icon::Banknote,
        }
    }
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

/// An edit that is open: the broker it is on, and the whole life of the edit.
pub struct OpenEdit {
    broker: BrokerId,
    session: EditSession,
}

pub struct BrokerPanelState {
    tab: BrokerTab,
    selected: BTreeSet<OrderId>,
    history: Paged<LiveOrder>,
    history_choice: HistoryChoice,
    fills_more: Paged<LiveFill>,
    edit: Option<OpenEdit>,
    cancel_ask: Option<CancelAsk>,
    flights: Vec<Flight>,
    notice: Option<(String, bool, Instant)>,
    /// Presses a test makes for the edit sheet, applied at its next frame.
    #[cfg(test)]
    edit_events: Vec<EditEvent>,
}

impl Default for BrokerPanelState {
    fn default() -> Self {
        Self {
            tab: BrokerTab::Orders,
            selected: BTreeSet::new(),
            history: Paged::default(),
            history_choice: HistoryChoice::All,
            fills_more: Paged::default(),
            edit: None,
            cancel_ask: None,
            flights: Vec::new(),
            notice: None,
            #[cfg(test)]
            edit_events: Vec::new(),
        }
    }
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

pub use crate::model::broker::ticket::type_label;
use crate::ui::kit::Tone;

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

// ---- The panel -----------------------------------------------------------------------------------

/// The edit sheet and the cancel question, drawn when the panel itself is not (an edit started from
/// a chart's line, with the panel closed).
pub fn dialogs_only(
    ctx: &egui::Context,
    state: &mut BrokerPanelState,
    host: &BrokerHost,
    broker_id: BrokerId,
) {
    let Some(snapshot) = host.snapshot(broker_id) else {
        // The connection is gone: so are the questions about it.
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
        self.edit.is_some() || self.cancel_ask.is_some()
    }

    /// The message the panel last showed, if it is still fresh: what a closed panel cannot show.
    pub fn take_notice(&mut self) -> Option<(String, bool)> {
        self.notice.take().map(|(text, error, _)| (text, error))
    }

    /// Closes the edit sheet: a replay started, and no order may be changed from one.
    pub fn close_order_entry(&mut self) -> bool {
        let open = self.edit.is_some();
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
        self.edit.as_ref().is_some_and(|e| e.session.is_ready())
    }

    #[cfg(test)]
    pub(crate) fn edit_review_enabled(&self) -> bool {
        self.edit.as_ref().is_some_and(|e| e.session.can_review())
    }

    #[cfg(test)]
    pub(crate) fn edit_preview_failed(&self) -> bool {
        self.edit
            .as_ref()
            .is_some_and(|e| e.session.preview_failed())
    }

    #[cfg(test)]
    pub(crate) fn retry_edit_preview(&mut self) {
        self.edit_events.push(EditEvent::TryAgain);
    }

    #[cfg(test)]
    pub(crate) fn edit_confirming(&self) -> bool {
        self.edit
            .as_ref()
            .is_some_and(|e| e.session.is_confirming())
    }

    #[cfg(test)]
    pub(crate) fn edit_open(&self) -> bool {
        self.edit.is_some()
    }

    /// The filled size of the order the sheet shows, as it shows it.
    #[cfg(test)]
    pub(crate) fn edit_filled_size(&self) -> Option<Decimal> {
        self.edit.as_ref().map(|e| e.session.filled_size())
    }

    #[cfg(test)]
    pub(crate) fn edit_error_text(&self) -> Option<&str> {
        self.edit.as_ref().and_then(|e| e.session.error())
    }

    /// Presses "Review change" at the next frame, whatever the button's state.
    #[cfg(test)]
    pub(crate) fn press_edit_review(&mut self) {
        self.edit_events.push(EditEvent::Review);
    }

    fn say(&mut self, text: impl Into<String>, error: bool) {
        self.notice = Some((text.into(), error, Instant::now()));
    }

    /// Starts editing `order` (it must be eligible).
    pub fn begin_edit(&mut self, broker: BrokerId, order: LiveOrder, price: Option<Decimal>) {
        self.edit = Some(OpenEdit {
            broker,
            session: EditSession::new(order, price, Instant::now()),
        });
    }
}

pub fn ui(
    ui: &mut egui::Ui,
    state: &mut BrokerPanelState,
    host: &BrokerHost,
    broker_id: BrokerId,
    bar: &VenueBar,
    stats: Option<&AccountStats>,
) -> Vec<PanelEvent> {
    let mut actions = Vec::new();
    if degenview_brokers::registry::by_id(broker_id).is_none() {
        return actions;
    }
    ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
    badge::rule(ui);
    let snapshot = host.snapshot(broker_id);
    let prefs = host.prefs(broker_id);

    header(ui, state, bar, snapshot.as_deref(), &mut actions);
    divider(ui);

    let Some(snapshot) = snapshot else {
        let message = if host.connection(broker_id).is_saved() {
            "Connecting…"
        } else {
            "Connect this broker in Settings ▸ Brokers to trade here."
        };
        if empty_with_settings(ui, message) {
            actions.push(PanelEvent::OpenSettings);
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
    stats_strip::show(ui, stats);
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
    crate::ui::paper::tables::empty_with_action(
        ui,
        Icon::ArrowLeftRight,
        "Live trading",
        message,
        Some("Open Settings"),
    )
}

fn divider(ui: &mut egui::Ui) {
    notice::divider(ui);
}

fn note(ui: &mut egui::Ui, text: &str, error: bool) {
    notice::note(ui, text, error);
}

/// The shared header: the venue pill, the portfolio, the sections with the working-order count and
/// the panel's buttons.
fn header(
    ui: &mut egui::Ui,
    state: &mut BrokerPanelState,
    bar: &VenueBar,
    snapshot: Option<&BrokerSnapshot>,
    actions: &mut Vec<PanelEvent>,
) {
    let working = snapshot.map_or(0, |snapshot| {
        snapshot
            .orders
            .iter()
            .filter(|v| v.order.status.is_working())
            .count()
            + snapshot.pending.len()
    });
    let tabs: Vec<kit::TabItem<BrokerTab>> = [
        BrokerTab::Orders,
        BrokerTab::History,
        BrokerTab::Fills,
        BrokerTab::Balances,
    ]
    .into_iter()
    .map(|tab| kit::TabItem {
        value: tab,
        title: tab.title(),
        icon: tab.icon(),
        count: if tab == BrokerTab::Orders { working } else { 0 },
    })
    .collect();
    let account = snapshot
        .and_then(|snapshot| snapshot.session.as_ref())
        .and_then(|session| session.accounts.first())
        .map(|account| account.label.clone());
    let chip = Chip {
        label: account.unwrap_or_default(),
        sub: None,
        tip: "The account orders go to".to_owned(),
        rows: Vec::new(),
        add: None,
    };
    let can_order = snapshot.is_some_and(|s| {
        s.can_send_orders() && s.session.as_ref().is_some_and(|x| x.permissions.can_trade)
    });
    let events = panel_header::show(
        ui,
        &Header {
            bar,
            chip: &chip,
            tabs: &tabs,
            overlay: None,
            can_order,
            more: &[],
            settings_tip: "Brokers settings",
        },
        &mut state.tab,
    );
    for event in events {
        if let HeaderEvent::Panel(event) = event {
            actions.push(event);
        }
    }
}

fn banners(ui: &mut egui::Ui, snapshot: &BrokerSnapshot, actions: &mut Vec<PanelEvent>) {
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
            if snapshot.held_elsewhere > 0 {
                note(ui, &held_elsewhere_text(snapshot.held_elsewhere), false);
            }
            if let Some(error) = &snapshot.last_error {
                note(ui, error, true);
            }
        }
    }
}

/// Said when the journal holds unconfirmed orders that are not this account's.
fn held_elsewhere_text(count: usize) -> String {
    if count == 1 {
        "1 unconfirmed order belongs to a different account. It is left untouched here and checked when that account is connected."
            .to_owned()
    } else {
        format!(
            "{count} unconfirmed orders belong to a different account. They are left untouched here and checked when that account is connected."
        )
    }
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
    let rows = crate::trading::broker_backend::order_rows(snapshot);
    let mut selected: BTreeSet<String> = state
        .selected
        .iter()
        .map(|id| id.as_str().to_owned())
        .collect();
    let actions = tables::orders(ui, "broker-orders", &rows, &mut selected, true);
    state.selected = selected.into_iter().map(OrderId::new).collect();

    for action in actions {
        match action {
            OrderAction::Check(id) => {
                // For the account on screen: a record of another account is not this one's to look up.
                if let (Some(runtime), Some(account)) = (host.runtime(broker_id), &snapshot.account)
                {
                    state.flights.push(Flight::Retry(
                        runtime.retry(ClientOrderId(id), account.id.clone()),
                    ));
                    state.say("Looking the order up at the broker…", false);
                }
            }
            OrderAction::Edit(id) => {
                if let Some(view) = snapshot
                    .orders
                    .iter()
                    .find(|view| view.order.id.as_str() == id)
                {
                    state.begin_edit(broker_id, view.order.clone(), None);
                }
            }
            OrderAction::Cancel(id) => request_cancel(
                state,
                host,
                broker_id,
                vec![OrderId::new(id)],
                prefs.confirm_single_cancel,
            ),
        }
    }

    // Selection only ever holds orders that are still working.
    let working_ids: BTreeSet<OrderId> = snapshot
        .orders
        .iter()
        .filter(|v| v.order.status.is_working())
        .map(|v| v.order.id.clone())
        .collect();
    state.selected.retain(|id| working_ids.contains(id));

    let bar = tables::orders_bar(ui, working_ids.len(), state.selected.len());
    if bar.cancel_selected {
        let ids: Vec<OrderId> = state.selected.iter().cloned().collect();
        request_cancel(state, host, broker_id, ids, true);
    }
    if bar.cancel_all {
        // Always asks, and says which orders: what is cancelled is what is shown now.
        let ids: Vec<OrderId> = working_ids.into_iter().collect();
        request_cancel(state, host, broker_id, ids, true);
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

// ---- Dialogs: the edit, the cancel question -----------------------------------------------

fn dialogs(
    ctx: &egui::Context,
    state: &mut BrokerPanelState,
    host: &BrokerHost,
    broker_id: BrokerId,
    snapshot: &BrokerSnapshot,
) {
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
    let Some(open) = &mut state.edit else { return };
    let Some(runtime) = host.runtime(open.broker) else {
        state.edit = None;
        return;
    };
    let venue: &dyn EditVenue = &*runtime;
    let now = Instant::now();
    let session = &mut open.session;
    let order = session.order();
    let spec = snapshot.product(&order.product);
    let live = snapshot
        .orders
        .iter()
        .find(|v| v.order.id == order.id)
        .map(|v| &v.order);
    let frame = EditFrame {
        now,
        live,
        account: snapshot.account.as_ref().map(|account| &account.id),
        generation: snapshot.generation,
        spec,
    };
    let mut step = session.update(&frame, Some(venue));
    if matches!(step, Step::Open) {
        ctx.request_repaint_after(Duration::from_millis(120));
        #[cfg(test)]
        let mut events = std::mem::take(&mut state.edit_events);
        #[cfg(not(test))]
        let mut events = Vec::new();
        events.extend(draw_edit(ctx, &mut open.session));
        for event in events {
            step = open.session.handle(event, now, venue);
            if matches!(step, Step::Closed(_)) {
                break;
            }
        }
    }
    if let Step::Closed(notice) = step {
        state.edit = None;
        if let Some(notice) = notice {
            state.say(notice.text, notice.error);
        }
    }
}

/// Draws the sheet or, once a change is reviewed, the question about it; returns what was pressed.
fn draw_edit(ctx: &egui::Context, session: &mut EditSession) -> Vec<EditEvent> {
    match session.view() {
        EditView::Reviewing(lines) => draw_review(ctx, &lines).into_iter().collect(),
        EditView::Editing(form) => draw_form(ctx, session, &form),
    }
}

/// The approval: what changes, from what to what, with the broker's figures. Nothing is sent until
/// the user accepts.
fn draw_review(ctx: &egui::Context, lines: &[ReviewLine]) -> Option<EditEvent> {
    let message = "This changes the order at the broker. A new price may lose your place in the queue, and the order can fill at its old terms until the change is applied.";
    let mut spec = dialog::Dialog::new(
        Icon::Pencil,
        badge::LIVE,
        "Change live order?",
        message,
        "Change order",
    );
    spec.dismiss = "Back";
    spec.cautious = true;
    let outcome = dialog::show(ctx, Id::new("broker-edit-confirm"), &spec, |ui| {
        ui.add_space(12.0);
        for line in lines {
            ui.horizontal(|ui| {
                ui.label(kit::text(
                    line.label,
                    kit::size::BODY,
                    kit::REGULAR,
                    theme::text_secondary(),
                ));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(kit::text(
                        &line.value,
                        kit::size::BODY,
                        kit::MEDIUM,
                        theme::text_primary(),
                    ));
                });
            });
            ui.add_space(4.0);
        }
    });
    match outcome {
        Some(dialog::Outcome::Accepted) => Some(EditEvent::Confirm),
        Some(_) => Some(EditEvent::Back),
        None => None,
    }
}

fn draw_form(ctx: &egui::Context, session: &mut EditSession, form: &EditForm) -> Vec<EditEvent> {
    let mut events = Vec::new();
    let mut close = false;
    let mut review = false;
    sheet::show(ctx, Id::new("broker-edit"), 440.0, |ui| {
        badge::rule(ui);
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
                &form.heading,
                kit::size::SUBHEADLINE + 1.0,
                kit::REGULAR,
                theme::text_secondary(),
            ));
            ui.add_space(16.0);
            let (price, size) = session.fields_mut();
            crate::ui::settings::api_keys::field(ui, form.price_title, None, "0", price, false);
            ui.add_space(12.0);
            crate::ui::settings::api_keys::field(ui, "Size", None, "0", size, false);
            ui.add_space(8.0);
            let red = |ui: &mut egui::Ui, text: &str, weight| {
                ui.label(kit::text(text, kit::size::CAPTION + 0.5, weight, kit::RED));
            };
            if let Some(text) = &form.typed_error {
                red(ui, text, kit::REGULAR);
            }
            for problem in &form.problems {
                red(ui, problem, kit::REGULAR);
            }
            for problem in &form.broker_problems {
                red(ui, problem, kit::MEDIUM);
            }
            if let Some(total) = &form.broker_total {
                ui.label(kit::text(
                    format!("Broker's figure for the changed order: {total}"),
                    kit::size::CAPTION + 0.5,
                    kit::REGULAR,
                    theme::text_secondary(),
                ));
            }
            if let rules::PreviewGate::Failed(error) = &form.gate {
                red(
                    ui,
                    &format!(
                        "The broker could not preview this change ({error}). It cannot be sent without that."
                    ),
                    kit::REGULAR,
                );
                ui.add_space(4.0);
                if kit::push_button(ui, None, "Try again", true).clicked() {
                    events.push(EditEvent::TryAgain);
                }
            } else if form.asking {
                ui.label(kit::text(
                    "Asking the broker to check this change…",
                    kit::size::CAPTION + 0.5,
                    kit::REGULAR,
                    theme::text_tertiary(),
                ));
            }
            if let Some(error) = &form.error {
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
                    if form.sending {
                        ui.spinner();
                    } else if kit::sheet_button(ui, "Review change", true, form.can_review)
                        .clicked()
                    {
                        review = true;
                    }
                    if kit::sheet_button(ui, "Cancel", false, !form.sending).clicked() {
                        close = true;
                    }
                });
            });
            ui.add_space(20.0);
        });
    });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && !form.sending {
        close = true;
    }
    if close {
        events.push(EditEvent::Cancel);
    } else if review {
        events.push(EditEvent::Review);
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::broker::ticket::edit_blocker;
    use degenview_brokers::{
        Capabilities, OrderSize, OrderStatus, OrderType, ProductId, TifKind, Tradability,
    };
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
        assert_eq!(short_id("abcdefghijkl"), "abcdefgh");
    }
}
