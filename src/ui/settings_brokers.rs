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
use eframe::egui;

use crate::host::broker_host::{BrokerHost, Connection};
use crate::ui::alert_center::Tone;
use crate::ui::icons::Icon;
use crate::ui::settings_api_keys::{StatusMessage, destructive_button, field};
use crate::ui::settings_window::{card, section, status_badge};
use crate::ui::{kit, theme};

/// What the Brokers page remembers between frames.
#[derive(Default)]
pub struct BrokersState {
    forms: HashMap<&'static str, BrokerForm>,
}

#[derive(Default)]
struct BrokerForm {
    values: BTreeMap<String, String>,
    status: Option<StatusMessage>,
    /// The key being checked, and the answer when it comes.
    checking: Option<(Credentials, Receiver<Result<Session, BrokerError>>)>,
    confirming_disconnect: bool,
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

pub fn body(ui: &mut egui::Ui, state: &mut BrokersState) {
    let Some(host) = BrokerHost::global() else {
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
    for broker in host.brokers() {
        let descriptor = broker.descriptor();
        let form = state.forms.entry(descriptor.id.as_str()).or_default();
        broker_card(ui, &host, broker, form);
        ui.add_space(8.0);
    }
    ui.add_space(6.0);
    ui.label(kit::text(
        "Credentials are stored in your system's secure storage and never leave this Mac. They are not synced, and no script, alert or other program can use them to trade.",
        kit::size::CAPTION + 0.5,
        kit::REGULAR,
        theme::text_secondary(),
    ));
}

fn broker_card(
    ui: &mut egui::Ui,
    host: &BrokerHost,
    broker: &'static dyn Broker,
    form: &mut BrokerForm,
) {
    let descriptor = broker.descriptor();
    let id = descriptor.id;
    poll_check(host, descriptor, form);
    let connection = host.connection(id);
    if form.checking.is_some() {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(150));
    }
    section(ui, descriptor.display_name, None, |ui| {
        card(ui, |ui| {
            ui.vertical(|ui| {
                let (text, tone) = connection_badge(&connection);
                status_badge(ui, &text, tone);
                let snapshot = match &connection {
                    Connection::Running(snapshot) => Some(snapshot.clone()),
                    _ => None,
                };
                if let Some(detail) = snapshot.as_deref().and_then(status_detail) {
                    ui.add_space(8.0);
                    muted(ui, &detail);
                }
                let failed = snapshot
                    .as_ref()
                    .is_some_and(|s| matches!(s.status, Status::Failed(_)));
                let connected = snapshot.is_some() && !failed;
                if connected {
                    if let Some(snapshot) = &snapshot {
                        connected_details(ui, snapshot, form.checked);
                    }
                    ui.add_space(12.0);
                    if destructive_button(ui, "Disconnect").clicked() {
                        form.confirming_disconnect = true;
                    }
                } else {
                    ui.add_space(12.0);
                    connect_form(ui, host, broker, form);
                    if matches!(connection, Connection::Saved | Connection::Running(_)) {
                        ui.add_space(10.0);
                        if destructive_button(ui, "Remove saved key").clicked() {
                            form.confirming_disconnect = true;
                        }
                    }
                }
                if let Some(status) = &form.status {
                    ui.add_space(10.0);
                    let color = if status.is_error {
                        kit::RED
                    } else {
                        kit::GREEN
                    };
                    ui.add(
                        egui::Label::new(kit::text(
                            &status.text,
                            kit::size::CAPTION + 0.5,
                            kit::REGULAR,
                            color,
                        ))
                        .wrap(),
                    );
                }
            });
        });
    });
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
                form.status = Some(match host.disconnect(id) {
                    Ok(()) => StatusMessage::ok("Disconnected. The key was removed from this Mac."),
                    Err(error) => StatusMessage::error(format!(
                        "Disconnected, but the key could not be removed: {error}"
                    )),
                });
            }
            Some(kit::ConfirmOutcome::Cancelled) => form.confirming_disconnect = false,
            None => {}
        }
    }
}

fn connected_details(ui: &mut egui::Ui, snapshot: &BrokerSnapshot, checked: Option<Permissions>) {
    let permissions = snapshot
        .session
        .as_ref()
        .map(|session| session.permissions)
        .or(checked);
    if let Some(session) = &snapshot.session {
        for account in &session.accounts {
            ui.add_space(8.0);
            muted(ui, &format!("Account: {}", account.label));
        }
    }
    if let Some(permissions) = permissions {
        ui.add_space(10.0);
        for row in permission_rows(&permissions) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let (icon, color) = match (row.granted, row.warn) {
                    (true, true) => (Icon::TriangleFill, kit::ORANGE),
                    (true, false) => (Icon::CheckCircleFill, kit::GREEN),
                    (false, _) => (Icon::XmarkCircleFill, theme::text_tertiary()),
                };
                crate::ui::icons::inline(ui, icon, 13.0, color);
                ui.label(kit::text(
                    row.label,
                    kit::size::BODY,
                    kit::REGULAR,
                    if row.granted {
                        theme::text_primary()
                    } else {
                        theme::text_tertiary()
                    },
                ));
            });
        }
        if let Some(advice) = permission_advice(&permissions) {
            ui.add_space(10.0);
            kit::notice_card(
                ui,
                Icon::TriangleFill,
                kit::ORANGE,
                "About this key",
                Some(advice),
                None,
            );
        }
    }
}

fn connect_form(
    ui: &mut egui::Ui,
    host: &BrokerHost,
    broker: &'static dyn Broker,
    form: &mut BrokerForm,
) {
    let descriptor = broker.descriptor();
    ui.label(kit::text(
        "Create a key",
        kit::size::SUBHEADLINE + 1.0,
        kit::SEMIBOLD,
        theme::text_primary(),
    ));
    ui.add_space(6.0);
    for (index, step) in descriptor.setup_steps.iter().enumerate() {
        numbered_step(ui, index + 1, step);
    }
    ui.add_space(4.0);
    muted(ui, descriptor.permission_note);
    gray_link(
        ui,
        &format!("Read {}'s instructions", descriptor.display_name),
        descriptor.docs_url,
    );
    ui.add_space(12.0);
    for credential in descriptor.credential_fields {
        let value = form.values.entry(credential.key.to_owned()).or_default();
        match credential.kind {
            FieldKind::Pem => secret_block(ui, credential.label, credential.hint, value),
            _ => field(
                ui,
                credential.label,
                None,
                credential.hint,
                value,
                credential.secret,
            ),
        }
        ui.add_space(12.0);
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let checking = form.checking.is_some();
        let enabled = !checking && can_connect(descriptor, &form.values);
        if kit::sheet_button(ui, "Connect", true, enabled).clicked() {
            let credentials = credentials_from(descriptor, &form.values);
            form.status = None;
            form.checking = Some((
                credentials.clone(),
                host.validate(descriptor.id, credentials),
            ));
        }
        if checking {
            ui.add(
                egui::Spinner::new()
                    .size(14.0)
                    .color(theme::text_secondary()),
            );
            muted(ui, "Checking the key…");
        }
    });
}

/// Takes the check's answer. A key that works is saved and connected; the form is emptied so the
/// private key is no longer held by the page.
fn poll_check(host: &BrokerHost, descriptor: &'static Descriptor, form: &mut BrokerForm) {
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
            form.status = Some(match host.adopt(descriptor.id, credentials) {
                Ok(()) => StatusMessage::ok(crate::platform::saved_securely()),
                Err(error) => StatusMessage::error(format!(
                    "Connected, but the key could not be saved, so it will be forgotten when you quit: {error}"
                )),
            });
            form.values.clear();
        }
        Err(error) => {
            // The error text is the broker's refusal in plain words; it never contains the key.
            form.status = Some(StatusMessage::error(error.to_string()));
        }
    }
}

// ---- Small pieces --------------------------------------------------------------------------------

fn muted(ui: &mut egui::Ui, text: &str) {
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

fn numbered_step(ui: &mut egui::Ui, number: usize, text: &str) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(kit::text(
            format!("{number}."),
            12.0,
            kit::SEMIBOLD,
            theme::text_tertiary(),
        ));
        ui.add(
            egui::Label::new(kit::text(text, 12.0, kit::REGULAR, theme::text_secondary())).wrap(),
        );
    });
}

/// A link, always gray and underlined.
fn gray_link(ui: &mut egui::Ui, label: &str, url: &str) {
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
            if text.is_empty() || revealed {
                ui.add(
                    egui::TextEdit::multiline(text)
                        .id(egui::Id::new(("broker-secret", title)))
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
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        if !text.is_empty() {
            if kit::sheet_button(ui, if revealed { "Hide" } else { "Show" }, false, true).clicked()
            {
                revealed = !revealed;
                ui.data_mut(|data| data.insert_temp(revealed_id, revealed));
            }
            if kit::sheet_button(ui, "Clear", false, true).clicked() {
                text.clear();
                ui.data_mut(|data| data.insert_temp(revealed_id, false));
            }
        }
    });
    muted(ui, crate::platform::credential_stored_hint());
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
