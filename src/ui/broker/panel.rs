//! The live trading panel: the account, its working orders, its history, its fills and balances,
//! and the actions on them (a new order, cancel, edit).
//!
//! It draws what the venue last said and nothing else: no order is added or removed because a
//! button was pressed. Asking to cancel marks an order "Cancel requested" until the broker
//! confirms; sending an order shows it as "Unconfirmed" until the broker lists it. It knows the
//! broker only through [`TradingBackend`] (what to read) and [`BrokerCommands`] (what to ask); every
//! request is a task polled on later frames, so the panel never waits on the network.
//!
//! The panel is shared by every window by construction: the venue's reads are the same in all of them.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use degenview_brokers::{Decimal, OrderId, Side};
use eframe::egui::{self, Color32, Id};

use crate::model::broker::ticket::{self as rules};
use crate::trading::backend::TradingBackend;
use crate::trading::broker_commands::{Answer, BrokerCommands, CancelReport, HistoryChoice};
use crate::trading::edit::{
    EditEvent, EditForm, EditFrame, EditSession, EditVenue, EditView, ReviewLine, Step,
};
use crate::trading::model::{AccountStats, BackendState, FillRow, OrderRow, Page};
use crate::ui::icons::Icon;
use crate::ui::paper::grid::{self as grid, Col, col};
use crate::ui::paper::tables::empty;
use crate::ui::trade::panel_header::{self, Chip, Header, HeaderEvent, PanelEvent, VenueBar};
use crate::ui::trade::tables::{self, OrderAction, badge_tone};
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

/// One list that is read from the broker a page at a time.
struct Paged<T> {
    items: Vec<T>,
    cursor: Option<String>,
    pending: Option<Answer<Page<T>>>,
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

impl<T: Clone> Paged<T> {
    fn reset(&mut self) {
        *self = Self::default();
    }

    /// Takes a page that arrived.
    fn poll(&mut self) {
        let Some(pending) = &mut self.pending else {
            return;
        };
        let Some(answer) = pending.poll().cloned() else {
            return;
        };
        self.pending = None;
        self.loaded = true;
        match answer {
            Ok(page) => {
                self.items.extend(page.items);
                self.cursor = page.next;
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }
}

enum Flight {
    Cancel(Answer<CancelReport>),
    Retry(Answer<()>),
}

struct CancelAsk {
    ids: Vec<String>,
    title: String,
    message: String,
}

/// An edit that is open: the whole life of the edit.
pub struct OpenEdit {
    session: EditSession,
}

/// What the panel is told by the window that draws it, besides the venue.
#[derive(Clone, Copy, Debug)]
pub struct PanelContext {
    /// Whether cancelling a single order asks first (the user's choice, Settings ▸ Brokers).
    pub confirm_single_cancel: bool,
}

pub struct BrokerPanelState {
    tab: BrokerTab,
    selected: BTreeSet<String>,
    history: Paged<OrderRow>,
    history_choice: HistoryChoice,
    fills_more: Paged<FillRow>,
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

fn side_color(side: Side) -> Color32 {
    match side {
        Side::Buy => theme::UP,
        Side::Sell => theme::DOWN,
    }
}

fn side_word(side: Side) -> &'static str {
    if side == Side::Buy { "Buy" } else { "Sell" }
}

fn time_text(ms: Option<i64>) -> String {
    let Some(ms) = ms else {
        return "—".to_owned();
    };
    let secs = ms.div_euclid(1000) + crate::service::timezone::local_utc_offset_secs(ms);
    let day = secs.rem_euclid(86_400);
    format!("{:02}:{:02}:{:02}", day / 3600, day % 3600 / 60, day % 60)
}

/// The size column: the base amount, or "25 (spend)" for a market buy sized by an amount to spend.
fn size_text(row: &OrderRow) -> String {
    match (row.size, row.spend) {
        (Some(size), _) => rules::display(size),
        (None, Some(spend)) => format!("{} (spend)", rules::display(spend)),
        (None, None) => "—".to_owned(),
    }
}

// ---- The panel -----------------------------------------------------------------------------------

/// The edit sheet and the cancel question, drawn when the panel itself is not (an edit started from
/// a chart's line, with the panel closed).
pub fn dialogs_only(
    ctx: &egui::Context,
    state: &mut BrokerPanelState,
    backend: &dyn TradingBackend,
) {
    if backend.state() == BackendState::NotConnected {
        // The connection is gone: so are the questions about it.
        state.edit = None;
        state.cancel_ask = None;
        return;
    }
    poll_flights(state);
    dialogs(ctx, state, backend);
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
    pub fn cancel_one(&mut self, commands: Option<&dyn BrokerCommands>, confirm: bool, id: String) {
        request_cancel(self, commands, vec![id], confirm);
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
        commands: Option<&dyn BrokerCommands>,
        ids: Vec<String>,
        confirm: bool,
    ) {
        request_cancel(self, commands, ids, confirm);
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

    /// Starts editing the order with this id, as the venue has it now; `price` is where a dragged
    /// line was dropped, if it was. `false` when the order is not there (any more).
    pub fn begin_edit(
        &mut self,
        commands: Option<&dyn BrokerCommands>,
        id: &str,
        price: Option<Decimal>,
    ) -> bool {
        let Some(order) = commands.and_then(|commands| commands.world(&OrderId::new(id)).live)
        else {
            return false;
        };
        self.edit = Some(OpenEdit {
            session: EditSession::new(order, price, Instant::now()),
        });
        true
    }
}

pub fn ui(
    ui: &mut egui::Ui,
    state: &mut BrokerPanelState,
    backend: &dyn TradingBackend,
    context: PanelContext,
    bar: &VenueBar,
    stats: Option<&AccountStats>,
) -> Vec<PanelEvent> {
    let mut actions = Vec::new();
    ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
    badge::rule(ui);
    let rows = backend.orders();

    header(ui, state, bar, backend, &rows, &mut actions);
    divider(ui);

    let connection = backend.state();
    if connection == BackendState::NotConnected
        || (connection == BackendState::Connecting && backend.account().is_none())
    {
        let message = if connection == BackendState::Connecting {
            "Connecting…"
        } else {
            "Connect this broker in Settings ▸ Brokers to trade here."
        };
        if empty_with_settings(ui, message) {
            actions.push(PanelEvent::OpenSettings);
        }
        return actions;
    }

    for note_line in backend.notes() {
        note(ui, &note_line.text, note_line.error);
    }
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

    let commands = backend.broker_commands();
    match state.tab {
        BrokerTab::Orders => orders_tab(ui, state, commands, &rows, context),
        BrokerTab::History => history_tab(ui, state, commands),
        BrokerTab::Fills => fills_tab(ui, state, commands, backend.recent_fills()),
        BrokerTab::Balances => balances_tab(ui, backend),
    }
    // The tables leave room for the orders footer, which only the Orders tab draws. Whatever is
    // left is taken, so the panel's content is as tall as the room it was given: a resizable panel
    // sizes itself to its content, and one that is short by a footer shrinks every frame, so a
    // drag of its edge never sticks.
    let rest = ui.available_height();
    if rest > 0.0 {
        ui.allocate_exact_size(egui::vec2(ui.available_width(), rest), egui::Sense::hover());
    }

    dialogs_with(ui.ctx(), state, commands, &rows);
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
    backend: &dyn TradingBackend,
    rows: &[OrderRow],
    actions: &mut Vec<PanelEvent>,
) {
    let working = rows.iter().filter(|row| row.working).count();
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
    let chip = Chip {
        label: backend
            .account()
            .map(|account| account.label)
            .unwrap_or_default(),
        sub: None,
        tip: "The account orders go to".to_owned(),
        rows: Vec::new(),
        add: None,
    };
    let events = panel_header::show(
        ui,
        &Header {
            bar,
            chip: &chip,
            tabs: &tabs,
            overlay: None,
            can_order: backend.can_order(),
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

fn poll_flights(state: &mut BrokerPanelState) {
    let mut done = Vec::new();
    for (index, flight) in state.flights.iter_mut().enumerate() {
        let said = match flight {
            Flight::Cancel(task) => task
                .poll()
                .map(|answer| answer.clone().map(|report| (report.text, report.error))),
            Flight::Retry(task) => task.poll().map(|answer| {
                answer
                    .clone()
                    .map(|()| ("The order was found and is placed.".to_owned(), false))
            }),
        };
        if let Some(said) = said {
            done.push((index, said));
        }
    }
    for (index, said) in done.into_iter().rev() {
        state.flights.remove(index);
        match said {
            Ok((text, error)) if !text.is_empty() => state.say(text, error),
            Err(error) => state.say(error, true),
            Ok(_) => {}
        }
    }
}

// ---- Orders --------------------------------------------------------------------------------------

fn orders_tab(
    ui: &mut egui::Ui,
    state: &mut BrokerPanelState,
    commands: Option<&dyn BrokerCommands>,
    rows: &[OrderRow],
    context: PanelContext,
) {
    let mut selected = state.selected.clone();
    let actions = tables::orders(ui, "broker-orders", rows, &mut selected, true);
    state.selected = selected;

    for action in actions {
        match action {
            OrderAction::Check(id) => {
                if let Some(commands) = commands {
                    state.flights.push(Flight::Retry(commands.retry(&id)));
                    state.say("Looking the order up at the broker…", false);
                }
            }
            OrderAction::Edit(id) => {
                state.begin_edit(commands, &id, None);
            }
            OrderAction::Cancel(id) => {
                request_cancel(state, commands, vec![id], context.confirm_single_cancel);
            }
        }
    }

    // Selection only ever holds orders that are still working.
    let working_ids: BTreeSet<String> = rows
        .iter()
        .filter(|row| row.working && !row.pending)
        .map(|row| row.id.clone())
        .collect();
    state.selected.retain(|id| working_ids.contains(id));

    let bar = tables::orders_bar(ui, working_ids.len(), state.selected.len());
    if bar.cancel_selected {
        let ids: Vec<String> = state.selected.iter().cloned().collect();
        request_cancel(state, commands, ids, true);
    }
    if bar.cancel_all {
        // Always asks, and says which orders: what is cancelled is what is shown now.
        let ids: Vec<String> = working_ids.into_iter().collect();
        request_cancel(state, commands, ids, true);
    }
}

/// Cancels `ids`, asking first when `confirm` (always for more than one).
fn request_cancel(
    state: &mut BrokerPanelState,
    commands: Option<&dyn BrokerCommands>,
    ids: Vec<String>,
    confirm: bool,
) {
    if ids.is_empty() {
        return;
    }
    if !confirm && ids.len() == 1 {
        send_cancel(state, commands, ids);
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
    commands: Option<&dyn BrokerCommands>,
    ids: Vec<String>,
) {
    let Some(commands) = commands else {
        return;
    };
    for id in &ids {
        state.selected.remove(id);
    }
    state.flights.push(Flight::Cancel(commands.cancel(ids)));
    state.say("Asking the broker to cancel…", false);
}

// ---- History, fills, balances ---------------------------------------------------------------------

fn history_tab(
    ui: &mut egui::Ui,
    state: &mut BrokerPanelState,
    commands: Option<&dyn BrokerCommands>,
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
        && let Some(commands) = commands
    {
        state.history.pending = Some(commands.history(state.history_choice, None));
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
                1 => grid::text_cell(ui, &order.market, primary),
                2 => grid::tag(
                    ui,
                    if order.side == Side::Buy {
                        "BUY"
                    } else {
                        "SELL"
                    },
                    side_color(order.side),
                ),
                3 => grid::text_cell(ui, &order.kind, weak),
                4 => grid::figure(
                    ui,
                    &order.price.map_or("—".to_owned(), rules::display),
                    primary,
                ),
                5 => grid::figure(
                    ui,
                    &format!("{} / {}", rules::display(order.filled), size_text(order)),
                    primary,
                ),
                6 => {
                    kit::status_badge(ui, order.status.label, badge_tone(order.status.tone));
                }
                _ => grid::figure(ui, &rules::display(order.fees), weak),
            }
        },
    );
    let more = state.history.cursor.is_some();
    if more
        && kit::sheet_button(ui, "Load more", false, state.history.pending.is_none()).clicked()
        && let Some(commands) = commands
    {
        let cursor = state.history.cursor.clone();
        state.history.pending = Some(commands.history(state.history_choice, cursor));
    }
}

fn fills_tab(
    ui: &mut egui::Ui,
    state: &mut BrokerPanelState,
    commands: Option<&dyn BrokerCommands>,
    recent: Vec<FillRow>,
) {
    state.fills_more.poll();
    let mut all: Vec<&FillRow> = recent.iter().chain(state.fills_more.items.iter()).collect();
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
            1 => grid::text_cell(ui, &fill.market, primary),
            2 => grid::tag(
                ui,
                if fill.side == Side::Buy {
                    "BUY"
                } else {
                    "SELL"
                },
                side_color(fill.side),
            ),
            3 => grid::figure(ui, &fill.price, primary),
            4 => grid::figure(ui, &rules::display(fill.size), primary),
            5 => grid::figure(ui, &rules::display(fill.fee), weak),
            _ => grid::figure(ui, &short_id(&fill.order), weak),
        }
    });
    if kit::sheet_button(
        ui,
        "Load older fills",
        false,
        state.fills_more.pending.is_none(),
    )
    .clicked()
        && let Some(commands) = commands
    {
        let cursor = state.fills_more.cursor.clone();
        state.fills_more.pending = Some(commands.fills(cursor));
    }
}

fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

fn balances_tab(ui: &mut egui::Ui, backend: &dyn TradingBackend) {
    let rows = backend.balances();
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
            let balance = &rows[row];
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

fn dialogs(ctx: &egui::Context, state: &mut BrokerPanelState, backend: &dyn TradingBackend) {
    dialogs_with(ctx, state, backend.broker_commands(), &backend.orders());
}

fn dialogs_with(
    ctx: &egui::Context,
    state: &mut BrokerPanelState,
    commands: Option<&dyn BrokerCommands>,
    rows: &[OrderRow],
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
            .filter_map(|id| rows.iter().find(|row| &row.id == id))
            .map(|row| format!("{} {} {}", side_word(row.side), size_text(row), row.market))
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
                send_cancel(state, commands, ids);
            }
            Some(_) => state.cancel_ask = None,
            None => {}
        }
    }
    edit_sheet(ctx, state, commands);
}

fn edit_sheet(
    ctx: &egui::Context,
    state: &mut BrokerPanelState,
    commands: Option<&dyn BrokerCommands>,
) {
    let Some(open) = &mut state.edit else { return };
    let Some(commands) = commands else {
        state.edit = None;
        return;
    };
    let venue: &dyn EditVenue = commands;
    let now = Instant::now();
    let session = &mut open.session;
    let world = venue.world(&session.order().id);
    let frame = EditFrame {
        now,
        live: world.live.as_ref(),
        account: world.account.as_ref(),
        generation: world.generation,
        spec: world.spec.as_ref(),
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

    #[test]
    fn times_and_ids_are_short() {
        assert_eq!(time_text(None), "—");
        assert_eq!(short_id("abcdefghijkl"), "abcdefgh");
    }
}
