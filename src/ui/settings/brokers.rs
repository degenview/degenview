//! Settings ▸ Brokers: connect a broker to trade for real from DegenView.
//!
//! One card per broker the build knows, drawn from the broker's own descriptor (its fields, its
//! setup steps, its permission wording), so a new broker needs no change here. A card shows:
//!
//! * not connected: how to create a key, the key's fields, and Connect, which *checks the key with
//!   reads only* (nothing is placed or changed), shows what it may do, and only then saves it to the
//!   operating system's secret store and connects;
//! * connected: how it is doing, what the key may do (a key that can move funds gets a warning), and
//!   Disconnect, which removes the key from this Mac but does not revoke it at the broker.
//!
//! A pasted private key is never left on screen: once entered the field shows only its length.
//! Rules are plain functions; the egui code only draws them.

use std::collections::{BTreeMap, HashMap};
use std::sync::mpsc::{Receiver, TryRecvError};

use degenview_brokers::{
    Broker, BrokerError, BrokerSnapshot, Credentials, Descriptor, FieldKind, Permissions, Session,
    Status,
};
use eframe::egui::{self, vec2};

use crate::host::broker_host::{BrokerHost, Connection, Operation};
use crate::host::trading_hub::TradingHub;
use crate::ui::alert::center::Tone;
use crate::ui::icons::Icon;
use crate::ui::settings::api_keys::{StatusMessage, destructive_button, field};
use crate::ui::settings::window::{divider, status_message};
use crate::ui::{kit, theme};

/// What the Brokers page remembers between frames.
#[derive(Default)]
pub struct BrokersState {
    forms: HashMap<&'static str, BrokerForm>,
    /// Which broker cards are unfolded; none until the user folds one either way.
    open: HashMap<&'static str, Option<bool>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum KeyOperation {
    Saving,
    Removing,
}

#[derive(Default)]
struct BrokerForm {
    values: BTreeMap<String, String>,
    status: Option<StatusMessage>,
    /// The key being checked, and the answer when it comes.
    checking: Option<(Credentials, Receiver<Result<Session, BrokerError>>)>,
    confirming_disconnect: bool,
    /// A save or a removal of the key that has not answered yet (the secret store may prompt).
    operation: Option<(KeyOperation, Operation)>,
    /// What the key may do, as last checked, shown until the connection reports its own.
    checked: Option<Permissions>,
}

// ---- Rules ---------------------------------------------------------------------------------------

/// The form's values as credentials: trimmed, only the descriptor's fields.
pub fn credentials_from(descriptor: &Descriptor, values: &BTreeMap<String, String>) -> Credentials {
    let mut credentials = Credentials::new();
    for field in descriptor.credential_fields {
        if let Some(value) = values.get(field.key) {
            credentials.set(field.key, value.trim());
        }
    }
    credentials
}

pub fn can_connect(descriptor: &Descriptor, values: &BTreeMap<String, String>) -> bool {
    descriptor
        .missing_fields(&credentials_from(descriptor, values))
        .is_empty()
}

/// One line of "what this key may do".
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PermissionRow {
    pub label: &'static str,
    pub granted: bool,
    /// Granted, and DegenView would rather it were not.
    pub warn: bool,
}

pub fn permission_rows(permissions: &Permissions) -> Vec<PermissionRow> {
    vec![
        PermissionRow {
            label: "View balances and orders",
            granted: permissions.can_view,
            warn: false,
        },
        PermissionRow {
            label: "Place and cancel orders",
            granted: permissions.can_trade,
            warn: false,
        },
        PermissionRow {
            label: "Move funds",
            granted: permissions.can_transfer,
            warn: permissions.can_transfer,
        },
    ]
}

/// A sentence for what a checked key means for trading, when there is something to say.
pub fn permission_advice(permissions: &Permissions) -> Option<&'static str> {
    if permissions.can_transfer {
        Some(
            "This key can move funds. DegenView never moves funds and does not need that: create a key without the Transfer permission and use that instead.",
        )
    } else if !permissions.can_trade {
        Some(
            "This key can view but not trade. You can watch balances and orders; to place orders, create a key with the Trade permission.",
        )
    } else {
        None
    }
}

/// The badge for a broker's connection.
pub fn connection_badge(connection: &Connection) -> (String, Tone) {
    match connection {
        Connection::NotConfigured => ("Not connected".to_owned(), Tone::Neutral),
        Connection::Saved => ("Connecting…".to_owned(), Tone::Neutral),
        Connection::Running(snapshot) => status_badge_for(&snapshot.status),
    }
}

pub fn status_badge_for(status: &Status) -> (String, Tone) {
    match status {
        Status::Connecting => ("Connecting…".to_owned(), Tone::Neutral),
        Status::Syncing => ("Reading your account…".to_owned(), Tone::Neutral),
        Status::Live => ("Connected".to_owned(), Tone::Good),
        Status::Reconnecting { .. } => ("Reconnecting…".to_owned(), Tone::Warning),
        Status::Failed(_) => ("Needs attention".to_owned(), Tone::Bad),
        Status::Stopped => ("Disconnected".to_owned(), Tone::Neutral),
    }
}

/// The explanation under a badge, when the state has one.
pub fn status_detail(snapshot: &BrokerSnapshot) -> Option<String> {
    match &snapshot.status {
        Status::Reconnecting { reason, .. } => Some(format!(
            "The connection dropped ({reason}) The orders and balances shown may be out of date until it comes back."
        )),
        Status::Failed(error) => Some(format!("{error} Enter a new key below, or disconnect.")),
        _ => None,
    }
}

/// What disconnecting does, said plainly.
pub const DISCONNECT_MESSAGE: &str = "This removes the key from this Mac and stops the connection. Orders already placed stay live at the broker. It does not revoke the key: to do that, delete it in the broker's own settings.";

// ---- View ----------------------------------------------------------------------------------------

/// The height of a card's header row.
const HEADER_HEIGHT: f32 = 36.0;
/// The height of one row of a grouped list.
const ROW_HEIGHT: f32 = 34.0;
/// Under this much room beside the title, the header shows the state as a dot instead of a badge.
const COMPACT_ROOM: f32 = 240.0;

/// The line under a broker's name in its header: the account and what the key may do while it is
/// connected, otherwise what there is to do next.
pub fn card_summary(descriptor: &Descriptor, connection: &Connection) -> String {
    match connection {
        Connection::NotConfigured if !descriptor.real_money => {
            "Test environment, no real money".to_owned()
        }
        Connection::NotConfigured => "Add an API key to trade".to_owned(),
        Connection::Saved => "Connecting with the saved key…".to_owned(),
        Connection::Running(snapshot) => match &snapshot.status {
            Status::Live => {
                let mut parts = Vec::new();
                if let Some(session) = &snapshot.session {
                    if let Some(account) = session.accounts.first() {
                        parts.push(account.label.to_string());
                    }
                    parts.push(
                        if session.permissions.can_trade {
                            "Can place orders"
                        } else {
                            "View only"
                        }
                        .to_owned(),
                    );
                }
                if parts.is_empty() {
                    "Ready".to_owned()
                } else {
                    parts.join(" · ")
                }
            }
            Status::Connecting => "Connecting…".to_owned(),
            Status::Syncing => "Reading your account…".to_owned(),
            Status::Reconnecting { .. } => "The connection dropped, trying again".to_owned(),
            Status::Failed(_) => "Check the key, then try again".to_owned(),
            Status::Stopped => "Disconnected".to_owned(),
        },
    }
}

pub fn body(ui: &mut egui::Ui, state: &mut BrokersState, hub: Option<std::sync::Arc<TradingHub>>) {
    let Some(hub) = hub else {
        kit::notice_card(
            ui,
            Icon::InfoCircle,
            theme::text_secondary(),
            "Live trading is not available",
            Some("The broker connections did not start in this session."),
            None,
        );
        return;
    };
    for broker in hub.brokers().brokers() {
        let descriptor = broker.descriptor();
        let form = state.forms.entry(descriptor.id.as_str()).or_default();
        let open = state.open.entry(descriptor.id.as_str()).or_default();
        broker_card(ui, &hub, broker, form, open);
        ui.add_space(10.0);
    }
    ui.add_space(4.0);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.add_space(2.0);
        crate::ui::icons::inline(ui, Icon::LockFill, 12.0, theme::text_tertiary());
        ui.add(
            egui::Label::new(kit::text(
                "Credentials are stored in your system's secure storage and never leave this Mac. They are not synced, and no script, alert or other program can use them to trade.",
                kit::size::CAPTION + 0.5,
                kit::REGULAR,
                theme::text_secondary(),
            ))
            .wrap(),
        );
    });
}

/// One broker: a header that folds the rest. A broker with a problem starts unfolded, the others
/// folded, until the user says otherwise.
fn broker_card(
    ui: &mut egui::Ui,
    hub: &std::sync::Arc<TradingHub>,
    broker: &'static dyn Broker,
    form: &mut BrokerForm,
    open: &mut Option<bool>,
) {
    let descriptor = broker.descriptor();
    let id = descriptor.id;
    let host = hub.brokers();
    poll_check(hub, descriptor, form);
    poll_operation(descriptor, form);
    let connection = host.connection(id);
    if form.checking.is_some() || form.operation.is_some() {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(150));
    }
    let snapshot = match &connection {
        Connection::Running(snapshot) => Some(snapshot.clone()),
        _ => None,
    };
    let failed = snapshot
        .as_ref()
        .is_some_and(|s| matches!(s.status, Status::Failed(_)));
    let is_open = open.unwrap_or(failed);

    let width = ui.available_width();
    let mut prepared = kit::card_frame(false).begin(ui);
    let inner = &mut prepared.content_ui;
    inner.set_width(width - 28.0 - 2.0);
    let head = header(inner, descriptor, &connection, is_open);
    if head.clicked() {
        *open = Some(!is_open);
    }
    if is_open {
        inner.add_space(12.0);
        divider(inner);
        inner.add_space(14.0);
        inner.spacing_mut().item_spacing.y = 10.0;
        if let Some(detail) = snapshot.as_deref().and_then(status_detail) {
            let (title, _) = connection_badge(&connection);
            let tint = if failed { kit::RED } else { kit::ORANGE };
            kit::notice_card(inner, Icon::TriangleFill, tint, &title, Some(&detail), None);
        }
        match (&snapshot, failed) {
            (Some(snapshot), false) => connected_body(inner, host, id, snapshot, form),
            _ => setup_body(inner, host, broker, &connection, form),
        }
        if let Some(status) = &form.status {
            status_message(inner, &status.text, status.is_error);
        }
    }
    prepared.frame = kit::card_frame(head.hovered());
    prepared.end(ui);

    if form.confirming_disconnect {
        let dialog_id = egui::Id::new(("broker-disconnect", id.as_str()));
        match kit::confirm(
            ui.ctx(),
            dialog_id,
            &kit::Confirm {
                icon: Icon::Trash,
                title: &format!("Disconnect {}?", descriptor.display_name),
                message: DISCONNECT_MESSAGE,
                destructive: "Disconnect",
            },
        ) {
            Some(kit::ConfirmOutcome::Confirmed) => {
                form.confirming_disconnect = false;
                form.checked = None;
                form.values.clear();
                form.status = None;
                // Through the hub, which owns the venue's lifecycle: the venue also stops being
                // enabled, so it is not connected again at the next start.
                form.operation = Some((
                    KeyOperation::Removing,
                    hub.disable(crate::model::broker::venue::TradingVenue::Broker(id)),
                ));
            }
            Some(kit::ConfirmOutcome::Cancelled) => form.confirming_disconnect = false,
            None => {}
        }
    }
}

/// The row that stays when the card is folded: the broker, how it is doing, and the chevron.
fn header(
    ui: &mut egui::Ui,
    descriptor: &Descriptor,
    connection: &Connection,
    open: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width(), HEADER_HEIGHT),
        egui::Sense::click(),
    );
    let response = crate::ui::accessibility::label(
        response,
        format!(
            "{}, {}",
            descriptor.display_name,
            if open { "expanded" } else { "collapsed" }
        ),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand);

    let tile = egui::Rect::from_min_size(
        egui::pos2(rect.left(), rect.center().y - 15.0),
        vec2(30.0, 30.0),
    );
    match crate::model::DataSource::from_watchlist_slug(descriptor.market_source) {
        // The broker's own mark, the logo its markets already wear on charts.
        Some(source) => {
            crate::ui::source_logo::paint(ui, source, tile);
            ui.painter().rect_stroke(
                tile,
                tile.width() * 0.22,
                egui::Stroke::new(1.0, theme::ink(14)),
                egui::StrokeKind::Inside,
            );
        }
        None => kit::paint_card_badge(ui, tile, Icon::ArrowLeftRight),
    }
    let chevron = egui::Rect::from_center_size(
        egui::pos2(rect.right() - 8.0, rect.center().y),
        vec2(16.0, 16.0),
    );
    crate::ui::icons::paint(
        ui.painter(),
        chevron,
        if open {
            Icon::ChevronDown
        } else {
            Icon::ChevronRight
        },
        theme::text_secondary(),
    );

    let left = tile.right() + 12.0;
    let (label, tone) = connection_badge(connection);
    let right = chevron.left() - 10.0;
    // A narrow page gives the words the room and leaves the state to a dot.
    let compact = right - left < COMPACT_ROOM;
    let state_left = if compact {
        let dot = egui::pos2(right - 4.0, rect.center().y);
        ui.painter().circle_filled(dot, 4.0, tone.dot());
        right - 8.0
    } else {
        kit::paint_status_badge(ui, right, rect.center().y, &label, tone).left()
    };
    // A test environment wears a chip, so it is not taken for the real account.
    let chip = (!descriptor.real_money).then(|| {
        kit::galley(
            ui,
            "Test",
            kit::size::CAPTION,
            kit::SEMIBOLD,
            theme::text_secondary(),
            f32::INFINITY,
        )
    });
    let chip_room = chip.as_ref().map_or(0.0, |chip| chip.size().x + 14.0 + 8.0);
    let room = state_left - 12.0 - left;
    let name = fitted(
        ui,
        descriptor.display_name,
        kit::size::HEADLINE,
        kit::SEMIBOLD,
        theme::text_primary(),
        room - chip_room,
    );
    let summary = fitted(
        ui,
        &card_summary(descriptor, connection),
        kit::size::SUBHEADLINE,
        kit::REGULAR,
        theme::text_secondary(),
        room,
    );
    let top = rect.center().y - (name.size().y + 2.0 + summary.size().y) / 2.0;
    let name_height = name.size().y;
    let name_width = name.size().x;
    ui.painter()
        .galley(egui::pos2(left, top), name, theme::text_primary());
    if let Some(chip) = chip {
        let pill = egui::Rect::from_min_size(
            egui::pos2(left + name_width + 8.0, top + name_height / 2.0 - 8.0),
            vec2(chip.size().x + 14.0, 16.0),
        );
        ui.painter().rect_filled(pill, 8.0, theme::ink(10));
        ui.painter().galley(
            egui::pos2(pill.left() + 7.0, pill.center().y - chip.size().y / 2.0),
            chip,
            theme::text_secondary(),
        );
    }
    ui.painter().galley(
        egui::pos2(left, top + name_height + 2.0),
        summary,
        theme::text_secondary(),
    );
    response
}

/// `text` as one line no wider than `max`, cut with an ellipsis where it does not fit.
pub(in crate::ui) fn fitted(
    ui: &egui::Ui,
    text: &str,
    size: f32,
    weight: f32,
    color: egui::Color32,
    max: f32,
) -> std::sync::Arc<egui::Galley> {
    let whole = kit::galley(ui, text, size, weight, color, f32::INFINITY);
    if whole.size().x <= max {
        return whole;
    }
    let chars: Vec<char> = text.chars().collect();
    let cut = |count: usize| {
        let mut shown: String = chars[..count].iter().collect();
        shown = shown.trim_end().to_owned();
        shown.push('…');
        shown
    };
    let (mut low, mut high) = (0, chars.len());
    while low < high {
        let middle = (low + high).div_ceil(2);
        if kit::galley(ui, &cut(middle), size, weight, color, f32::INFINITY)
            .size()
            .x
            <= max
        {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    kit::galley(ui, &cut(low), size, weight, color, f32::INFINITY)
}

// ---- A connected broker --------------------------------------------------------------------------

fn connected_body(
    ui: &mut egui::Ui,
    host: &std::sync::Arc<BrokerHost>,
    id: degenview_brokers::BrokerId,
    snapshot: &BrokerSnapshot,
    form: &mut BrokerForm,
) {
    let permissions = snapshot
        .session
        .as_ref()
        .map(|session| session.permissions)
        .or(form.checked);
    titled_group(ui, "Access", |ui| {
        let mut first = true;
        if let Some(session) = &snapshot.session {
            for account in &session.accounts {
                first = false;
                value_row(ui, "Account", &account.label, theme::text_secondary(), None);
            }
        }
        if let Some(permissions) = permissions {
            for row in permission_rows(&permissions) {
                if !first {
                    divider(ui);
                }
                first = false;
                let (icon, color, word) = match (row.granted, row.warn) {
                    (true, true) => (Some(Icon::TriangleFill), kit::ORANGE, "Allowed"),
                    (true, false) => (Some(Icon::CheckCircleFill), kit::GREEN, "Allowed"),
                    (false, _) => (None, theme::text_tertiary(), "Not allowed"),
                };
                value_row(ui, row.label, word, color, icon);
            }
        }
    });
    if let Some(advice) = permissions.as_ref().and_then(permission_advice) {
        kit::notice_card(
            ui,
            Icon::TriangleFill,
            kit::ORANGE,
            "About this key",
            Some(advice),
            None,
        );
    }
    trading_choices(ui, host, id);
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.set_min_height(28.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if form.operation.is_some() {
                muted(ui, "Removing the key…");
            } else if destructive_button(ui, "Disconnect").clicked() {
                form.confirming_disconnect = true;
            }
        });
    });
}

/// How the user trades at this broker. Saved to this Mac only; the questions that guard real money
/// (a market order, an edit, Cancel all) are not on this list because they are not optional.
fn trading_choices(
    ui: &mut egui::Ui,
    host: &std::sync::Arc<BrokerHost>,
    id: degenview_brokers::BrokerId,
) {
    let mut prefs = host.prefs(id);
    let before = prefs.clone();
    titled_group(ui, "Trading", |ui| {
        let rows: [(&str, &mut bool); 5] = [
            (
                "Ask before sending a limit or stop-limit order",
                &mut prefs.confirm_resting_orders,
            ),
            (
                "Ask before cancelling a single order",
                &mut prefs.confirm_single_cancel,
            ),
            (
                "Tell me when orders are placed, fill, cancel or are rejected",
                &mut prefs.notify,
            ),
            ("Show working orders on charts", &mut prefs.show_on_charts),
            (
                "Start a market buy as an amount to spend",
                &mut prefs.spend_in_quote_by_default,
            ),
        ];
        for (index, (label, value)) in rows.into_iter().enumerate() {
            if index > 0 {
                divider(ui);
            }
            switch_row(ui, label, value);
        }
    });
    muted(
        ui,
        "Market orders, changing an order and Cancel all always ask.",
    );
    if prefs != before {
        host.set_prefs(id, prefs);
    }
}

// ---- A broker that is not connected --------------------------------------------------------------

fn setup_body(
    ui: &mut egui::Ui,
    host: &BrokerHost,
    broker: &'static dyn Broker,
    connection: &Connection,
    form: &mut BrokerForm,
) {
    let descriptor = broker.descriptor();
    titled_group(ui, "Set up", |ui| {
        for (index, step) in descriptor.setup_steps.iter().enumerate() {
            step_row(ui, index + 1, step);
        }
    });
    muted(ui, descriptor.permission_note);
    gray_link(
        ui,
        &format!("Read {}'s instructions", descriptor.display_name),
        descriptor.docs_url,
    );
    if !descriptor.credential_fields.is_empty() {
        ui.add_space(4.0);
    }
    for credential in descriptor.credential_fields {
        let value = form.values.entry(credential.key.to_owned()).or_default();
        // Scoped: a field squeezes the spacing it is drawn in.
        ui.scope(|ui| match credential.kind {
            FieldKind::Pem => secret_block(ui, credential.label, credential.hint, value),
            _ => {
                field(
                    ui,
                    credential.label,
                    None,
                    credential.hint,
                    value,
                    credential.secret,
                );
            }
        });
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.set_min_height(28.0);
        ui.spacing_mut().item_spacing.x = 10.0;
        let checking = form.checking.is_some();
        let busy = form.operation.as_ref().map(|(kind, _)| *kind);
        let enabled = !checking && busy.is_none() && can_connect(descriptor, &form.values);
        if kit::sheet_button(ui, "Connect", true, enabled).clicked() {
            start_check(host, descriptor, form);
        }
        if checking || busy.is_some() {
            ui.add(
                egui::Spinner::new()
                    .size(14.0)
                    .color(theme::text_secondary()),
            );
            muted(
                ui,
                match busy {
                    Some(KeyOperation::Saving) => "Saving the key…",
                    Some(KeyOperation::Removing) => "Removing the key…",
                    None => "Checking the key…",
                },
            );
        }
        if matches!(connection, Connection::Saved | Connection::Running(_)) {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if destructive_button(ui, "Remove saved key").clicked() {
                    form.confirming_disconnect = true;
                }
            });
        }
    });
}

// ---- Rows ----------------------------------------------------------------------------------------

/// A small heading over a recessed list.
pub(in crate::ui) fn titled_group(
    ui: &mut egui::Ui,
    title: &str,
    content: impl FnOnce(&mut egui::Ui),
) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.label(kit::text(
            title,
            kit::size::CAPTION + 0.5,
            kit::SEMIBOLD,
            theme::text_secondary(),
        ));
        egui::Frame::new()
            .fill(theme::ink(6))
            .stroke(egui::Stroke::new(1.0, theme::ink(10)))
            .corner_radius(8.0)
            .inner_margin(egui::Margin::symmetric(12, 0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 0.0;
                content(ui);
            });
    });
}

/// `label` on the left, a value at the right edge (with a mark before it).
fn value_row(
    ui: &mut egui::Ui,
    label: &str,
    value: &str,
    color: egui::Color32,
    icon: Option<Icon>,
) {
    let width = ui.available_width();
    let value = fitted(
        ui,
        value,
        kit::size::BODY,
        kit::REGULAR,
        color,
        width * 0.55,
    );
    let mark = if icon.is_some() { 13.0 + 6.0 } else { 0.0 };
    let label_room = width - value.size().x - mark - 16.0;
    let label = fitted(
        ui,
        label,
        kit::size::BODY,
        kit::REGULAR,
        theme::text_primary(),
        label_room,
    );
    let (rect, _) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), egui::Sense::hover());
    let center = rect.center().y;
    let value_pos = egui::pos2(rect.right() - value.size().x, center - value.size().y / 2.0);
    if let Some(icon) = icon {
        crate::ui::icons::paint(
            ui.painter(),
            egui::Rect::from_center_size(
                egui::pos2(value_pos.x - 6.0 - 6.5, center),
                vec2(13.0, 13.0),
            ),
            icon,
            color,
        );
    }
    ui.painter().galley(value_pos, value, color);
    ui.painter().galley(
        egui::pos2(rect.left(), center - label.size().y / 2.0),
        label,
        theme::text_primary(),
    );
}

/// A setting with its switch at the right edge. A long label wraps and the row grows.
fn switch_row(ui: &mut egui::Ui, label: &str, value: &mut bool) {
    let width = ui.available_width();
    let words = ui.allocate_ui_with_layout(
        vec2(width - 32.0 - 16.0, ROW_HEIGHT),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_height(ROW_HEIGHT);
            ui.add(
                egui::Label::new(kit::text(
                    label,
                    kit::size::BODY,
                    kit::REGULAR,
                    theme::text_primary(),
                ))
                .wrap(),
            );
        },
    );
    let slot = egui::Rect::from_center_size(
        egui::pos2(
            words.response.rect.left() + width - 16.0,
            words.response.rect.center().y,
        ),
        vec2(32.0, 18.0),
    );
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(slot));
    kit::switch(&mut child, value, label);
}

/// One numbered line of the setup steps; a long one wraps under itself.
pub(in crate::ui) fn step_row(ui: &mut egui::Ui, number: usize, text: &str) {
    ui.add_space(8.0);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let (slot, _) = ui.allocate_exact_size(vec2(18.0, 18.0), egui::Sense::hover());
        let dot = egui::pos2(slot.center().x, slot.top() + 8.0);
        ui.painter().circle_filled(dot, 9.0, theme::ink(10));
        ui.painter().text(
            dot,
            egui::Align2::CENTER_CENTER,
            number.to_string(),
            egui::FontId::proportional(10.0),
            theme::text_secondary(),
        );
        ui.add(
            egui::Label::new(kit::text(
                text,
                kit::size::SUBHEADLINE + 1.0,
                kit::REGULAR,
                theme::text_secondary(),
            ))
            .wrap(),
        );
    });
    ui.add_space(8.0);
}

/// Takes the answer of a save or a removal of the key, and says what it came to.
fn poll_operation(descriptor: &'static Descriptor, form: &mut BrokerForm) {
    let Some((kind, task)) = &mut form.operation else {
        return;
    };
    let kind = *kind;
    let Some(answer) = task.poll().cloned() else {
        return;
    };
    form.operation = None;
    form.status = Some(match (kind, answer) {
        // A broker that takes no key has nothing to keep: say so instead of "saved".
        (KeyOperation::Saving, Ok(())) if descriptor.credential_fields.is_empty() => {
            StatusMessage::ok(
                "Connected. Nothing is saved: connect again after restarting DegenView.",
            )
        }
        (KeyOperation::Saving, Ok(())) => StatusMessage::ok(crate::platform::saved_securely()),
        (KeyOperation::Saving, Err(error)) => StatusMessage::error(format!(
            "Connected, but the key could not be saved, so it will be forgotten when you quit. A key saved earlier may still be stored and connect again at the next start: {error}"
        )),
        (KeyOperation::Removing, Ok(())) if descriptor.credential_fields.is_empty() => {
            StatusMessage::ok("Disconnected.")
        }
        (KeyOperation::Removing, Ok(())) => {
            StatusMessage::ok("Disconnected. The key was removed from this Mac.")
        }
        // The connection is stopped, but the key is still in the keychain: it connects again at
        // the next start unless it is removed there.
        (KeyOperation::Removing, Err(error)) => StatusMessage::error(format!(
            "Disconnected, but the key could NOT be removed from this Mac and will connect again when DegenView starts. Remove it in your keychain, or try again: {error}"
        )),
    });
}

/// Starts checking the typed key, off the interface thread.
fn start_check(host: &BrokerHost, descriptor: &'static Descriptor, form: &mut BrokerForm) {
    let credentials = credentials_from(descriptor, &form.values);
    form.status = None;
    form.checking = Some((
        credentials.clone(),
        host.validate(descriptor.id, credentials),
    ));
}

/// What a click on Connect does, for a test that has no pointer.
#[cfg(test)]
impl BrokersState {
    /// Unfolds a broker's card, as a click on its header would.
    pub(crate) fn open_for_test(&mut self, broker: &'static str) {
        self.open.insert(broker, Some(true));
    }

    pub(crate) fn typed_for_test(&self, broker: &'static str, field: &str) -> Option<String> {
        self.forms.get(broker)?.values.get(field).cloned()
    }

    pub(crate) fn connect_for_test(
        &mut self,
        host: &BrokerHost,
        broker: &'static dyn Broker,
        values: &[(&str, &str)],
    ) {
        let descriptor = broker.descriptor();
        let form = self.forms.entry(descriptor.id.as_str()).or_default();
        for (key, value) in values {
            form.values.insert((*key).to_owned(), (*value).to_owned());
        }
        start_check(host, descriptor, form);
    }
}

/// Takes the check's answer. A key that works is saved and connected; the form is emptied so the
/// private key is no longer held by the page.
fn poll_check(
    hub: &std::sync::Arc<TradingHub>,
    descriptor: &'static Descriptor,
    form: &mut BrokerForm,
) {
    let Some((_, receiver)) = &form.checking else {
        return;
    };
    let answer = match receiver.try_recv() {
        Ok(answer) => answer,
        Err(TryRecvError::Empty) => return,
        Err(TryRecvError::Disconnected) => Err(BrokerError::invalid(
            "The check stopped unexpectedly. Try again.",
        )),
    };
    let Some((credentials, _)) = form.checking.take() else {
        return;
    };
    match answer {
        Ok(session) => {
            form.checked = Some(session.permissions);
            // The key is saved and the connection started through the hub; what the secret store
            // answered arrives later (`poll_operation`).
            form.status = None;
            form.operation = Some((
                KeyOperation::Saving,
                hub.connect(descriptor.id, credentials),
            ));
            form.values.clear();
        }
        Err(error) => {
            // The error text is the broker's refusal in plain words; it never contains the key.
            form.status = Some(StatusMessage::error(error.to_string()));
        }
    }
}

// ---- Small pieces --------------------------------------------------------------------------------

pub(in crate::ui) fn muted(ui: &mut egui::Ui, text: &str) {
    ui.add(
        egui::Label::new(kit::text(
            text,
            kit::size::CAPTION + 0.5,
            kit::REGULAR,
            theme::text_secondary(),
        ))
        .wrap(),
    );
}

/// A link, always gray and underlined.
pub(in crate::ui) fn gray_link(ui: &mut egui::Ui, label: &str, url: &str) {
    let response = ui
        .add(
            egui::Label::new(
                kit::text(
                    label,
                    kit::size::BODY,
                    kit::REGULAR,
                    theme::text_secondary(),
                )
                .underline(),
            )
            .sense(egui::Sense::click()),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(url);
    if response.clicked() {
        ui.ctx().open_url(egui::OpenUrl::new_tab(url));
    }
}

/// A multi-line secret (a PEM private key). Empty, it is a box to paste into; once something is
/// entered it is hidden behind its length, with Show to check it and Clear to start over.
fn secret_block(ui: &mut egui::Ui, title: &str, hint: &str, text: &mut String) {
    let revealed_id = egui::Id::new(("broker-secret-revealed", title));
    let mut revealed: bool = ui.data(|data| data.get_temp(revealed_id)).unwrap_or(false);
    ui.label(kit::text(
        title,
        kit::size::SUBHEADLINE + 1.0,
        kit::MEDIUM,
        theme::text_primary(),
    ));
    ui.add_space(6.0);
    egui::Frame::new()
        .fill(theme::with_alpha(theme::editor_background(), 0.7))
        .corner_radius(8.0)
        .stroke(egui::Stroke::new(1.0, kit::primary(0.15)))
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let edit_id = egui::Id::new(("broker-secret", title));
            // The box stays while it has focus: swapping it for the hidden summary after the
            // first typed character would drop the rest of what is being typed.
            if text.is_empty() || revealed || ui.memory(|memory| memory.has_focus(edit_id)) {
                ui.add(
                    egui::TextEdit::multiline(text)
                        .id(edit_id)
                        .frame(egui::Frame::NONE)
                        .font(egui::FontId::monospace(kit::size::CAPTION + 0.5))
                        .text_color(theme::text_primary())
                        .desired_rows(4)
                        .desired_width(ui.available_width())
                        .hint_text(kit::text(
                            hint,
                            kit::size::CAPTION + 0.5,
                            kit::REGULAR,
                            theme::text_tertiary(),
                        )),
                );
            } else {
                ui.label(kit::text(
                    format!("Entered ({} characters). Hidden.", text.chars().count()),
                    kit::size::BODY,
                    kit::REGULAR,
                    theme::text_secondary(),
                ));
            }
        });
    if !text.is_empty() {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            if kit::sheet_button(ui, if revealed { "Hide" } else { "Show" }, false, true).clicked()
            {
                revealed = !revealed;
                ui.data_mut(|data| data.insert_temp(revealed_id, revealed));
            }
            if kit::sheet_button(ui, "Clear", false, true).clicked() {
                text.clear();
                ui.data_mut(|data| data.insert_temp(revealed_id, false));
            }
        });
    }
    ui.add_space(6.0);
    muted(ui, crate::platform::credential_stored_hint());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trading::task::Task;
    use degenview_brokers::{BrokerId, ClientIdRules, CredentialField};

    const FIELDS: &[degenview_brokers::CredentialField] = &[
        CredentialField {
            key: "a",
            label: "A",
            hint: "",
            secret: false,
            kind: FieldKind::Text,
        },
        CredentialField {
            key: "b",
            label: "B",
            hint: "",
            secret: true,
            kind: FieldKind::Pem,
        },
    ];

    fn answered(kind: KeyOperation, answer: Result<(), String>) -> BrokerForm {
        BrokerForm {
            operation: Some((kind, Task::ready(answer))),
            ..BrokerForm::default()
        }
    }

    #[test]
    fn a_pending_operation_says_nothing_until_it_answers_and_then_says_what_came_of_it() {
        let descriptor: &'static Descriptor = Box::leak(Box::new(descriptor()));
        let mut waiting = BrokerForm {
            operation: Some((KeyOperation::Removing, Task::new(|| None))),
            ..BrokerForm::default()
        };
        poll_operation(descriptor, &mut waiting);
        assert!(waiting.operation.is_some() && waiting.status.is_none());

        let mut form = answered(KeyOperation::Removing, Ok(()));
        poll_operation(descriptor, &mut form);
        assert!(form.operation.is_none());
        let status = form.status.expect("a message");
        assert!(
            !status.is_error && status.text.contains("removed"),
            "{}",
            status.text
        );

        // The store refused: the connection is stopped, and the message says the key is still there.
        let mut form = answered(
            KeyOperation::Removing,
            Err("the keychain refused".to_owned()),
        );
        poll_operation(descriptor, &mut form);
        let status = form.status.expect("a message");
        assert!(
            status.is_error && status.text.contains("NOT be removed"),
            "{}",
            status.text
        );
        assert!(status.text.contains("the keychain refused"));

        let mut form = answered(KeyOperation::Saving, Err("no space".to_owned()));
        poll_operation(descriptor, &mut form);
        let status = form.status.expect("a message");
        assert!(
            status.is_error && status.text.contains("could not be saved"),
            "{}",
            status.text
        );
    }

    fn descriptor() -> Descriptor {
        Descriptor {
            id: BrokerId::new("acme"),
            display_name: "Acme",
            credential_fields: FIELDS,
            setup_steps: &[],
            permission_note: "",
            docs_url: "",
            client_id: ClientIdRules {
                max_len: 64,
                idempotent: true,
            },
            market_source: "acme",
            account_model: degenview_brokers::AccountModel::Flat,
            real_money: true,
            min_request_gap_ms: 100,
        }
    }

    fn values(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn credentials_are_trimmed_and_limited_to_the_brokers_own_fields() {
        let credentials = credentials_from(
            &descriptor(),
            &values(&[("a", "  key  "), ("b", "\nsecret\n"), ("stray", "x")]),
        );
        assert_eq!(credentials.get("a"), Some("key"));
        assert_eq!(credentials.get("b"), Some("secret"));
        assert_eq!(credentials.get("stray"), None);
    }

    #[test]
    fn connect_needs_every_field() {
        let d = descriptor();
        assert!(!can_connect(&d, &values(&[])));
        assert!(!can_connect(&d, &values(&[("a", "x")])));
        assert!(!can_connect(&d, &values(&[("a", "x"), ("b", "  ")])));
        assert!(can_connect(&d, &values(&[("a", "x"), ("b", "y")])));
    }

    #[test]
    fn a_key_that_can_move_funds_is_warned_about_and_a_view_only_key_is_explained() {
        let moves = Permissions {
            can_view: true,
            can_trade: true,
            can_transfer: true,
        };
        let rows = permission_rows(&moves);
        assert!(rows[2].granted && rows[2].warn);
        assert!(
            permission_advice(&moves)
                .unwrap()
                .contains("never moves funds")
        );

        let view_only = Permissions {
            can_view: true,
            can_trade: false,
            can_transfer: false,
        };
        assert!(!permission_rows(&view_only)[1].granted);
        assert!(
            permission_advice(&view_only)
                .unwrap()
                .contains("view but not trade")
        );

        let trading = Permissions {
            can_view: true,
            can_trade: true,
            can_transfer: false,
        };
        assert_eq!(permission_advice(&trading), None);
        assert!(permission_rows(&trading).iter().all(|row| !row.warn));
    }

    #[test]
    fn badges_say_what_state_the_connection_is_in() {
        assert_eq!(
            connection_badge(&Connection::NotConfigured).1,
            Tone::Neutral
        );
        assert_eq!(
            status_badge_for(&Status::Live),
            ("Connected".to_owned(), Tone::Good)
        );
        assert_eq!(status_badge_for(&Status::Syncing).1, Tone::Neutral);
        let reconnecting = Status::Reconnecting {
            since_ms: 0,
            reason: "x".to_owned(),
        };
        assert_eq!(status_badge_for(&reconnecting).1, Tone::Warning);
        let failed = Status::Failed(BrokerError::invalid("no"));
        assert_eq!(status_badge_for(&failed).1, Tone::Bad);
    }

    #[test]
    fn disconnecting_is_honest_about_what_it_does_not_do() {
        assert!(DISCONNECT_MESSAGE.contains("does not revoke"));
        assert!(DISCONNECT_MESSAGE.contains("stay live"));
    }

    #[test]
    fn a_folded_card_says_what_there_is_to_do() {
        let mut live = descriptor();
        assert_eq!(
            card_summary(&live, &Connection::NotConfigured),
            "Add an API key to trade"
        );
        assert_eq!(
            card_summary(&live, &Connection::Saved),
            "Connecting with the saved key…"
        );
        // A test environment says so, so nobody takes it for the real account.
        live.real_money = false;
        assert_eq!(
            card_summary(&live, &Connection::NotConfigured),
            "Test environment, no real money"
        );
    }

    #[test]
    fn a_long_line_is_cut_to_the_room_it_has() {
        let ctx = egui::Context::default();
        crate::ui::theme::install_offscreen(&ctx);
        let mut widths = Vec::new();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            for max in [400.0, 60.0, 0.0] {
                let galley = fitted(
                    ui,
                    "Default portfolio · Can place orders",
                    11.0,
                    kit::REGULAR,
                    egui::Color32::WHITE,
                    max,
                );
                widths.push((max, galley.size().x, galley.text().to_owned()));
            }
        });
        output.textures_delta.clear();
        assert_eq!(widths[0].2, "Default portfolio · Can place orders");
        assert!(
            widths[1].1 <= 60.0 && widths[1].2.ends_with('…'),
            "{widths:?}"
        );
        assert_eq!(widths[2].2, "…");
    }
}
