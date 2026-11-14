//! The security section: approvals, remote connections (the interface to listen on and the remote
//! ceiling), the maximum every client is capped by, and the alert before approvals are turned off.

use super::controls::{caption, copy_button, field, overview, row, secondary};
use super::{CONTROL_HEIGHT, Copied, Editing, McpSettingsInput, McpSettingsState};
use crate::model::mcp::{BindMode, McpAction, PermissionSet};
use crate::ui::icons::Icon;
use crate::ui::kit::Tone;
use crate::ui::settings::window::{card, section};
use crate::ui::{kit, settings::card, theme};
use eframe::egui::{self, Align, Id, Layout, Rect, Ui, vec2};
use std::net::IpAddr;

pub(super) fn security_section(
    ui: &mut Ui,
    state: &mut McpSettingsState,
    input: &McpSettingsInput,
    actions: &mut Vec<McpAction>,
) {
    section(ui, "Security", None, |ui| {
        card(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 10.0;
            row(ui, "Authentication", |ui| secondary(ui, "Always required"));
            kit::divider(ui);
            remote_rows(ui, state, input, actions);
            kit::divider(ui);
            maximum_row(ui, state, input);
        });
    });
}

/// Where the parts of the "Maximum permissions" row landed (read by the layout test).
#[cfg_attr(not(test), allow(dead_code))]
pub(super) struct MaximumRow {
    pub(super) title: Rect,
    pub(super) edit: Rect,
    pub(super) summary: Rect,
}

/// The title and its Edit button share one line of fixed height, and the summary takes as many
/// lines as it needs beneath it, so the button stays at the top right however much the maximum holds.
pub(super) fn maximum_row(
    ui: &mut Ui,
    state: &mut McpSettingsState,
    input: &McpSettingsInput,
) -> MaximumRow {
    limit_row(
        ui,
        state,
        "Maximum permissions",
        "The most any credential can do. Each one can only narrow this.",
        input.global,
        Editing::Maximum,
    )
}

/// A title with its Edit button and the groups of a permission limit beneath it.
fn limit_row(
    ui: &mut Ui,
    state: &mut McpSettingsState,
    title_text: &str,
    hover: &str,
    set: &PermissionSet,
    editing: Editing,
) -> MaximumRow {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        let (mut title, mut edit) = (Rect::NOTHING, Rect::NOTHING);
        ui.allocate_ui_with_layout(
            vec2(ui.available_width(), CONTROL_HEIGHT),
            Layout::right_to_left(Align::Center),
            |ui| {
                let button = kit::sheet_button(ui, "Edit…", false, true).on_hover_text(hover);
                edit = button.rect;
                if button.clicked() {
                    state.sheet = Some(editing);
                    state.confirm_revoke = false;
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    title = ui
                        .label(kit::text(
                            title_text,
                            kit::size::SUBHEADLINE + 1.0,
                            kit::REGULAR,
                            theme::text_primary(),
                        ))
                        .rect;
                });
            },
        );
        let summary = overview(ui, set);
        MaximumRow {
            title,
            edit,
            summary,
        }
    })
    .inner
}

/// What a person may type for an interface: an address of this Mac, `192.168.1.20` or `fd00::20`
/// (brackets are allowed). Not this Mac's own loopback address, and not "every interface".
pub fn parse_interface(text: &str) -> Result<BindMode, String> {
    let text = text.trim().trim_start_matches('[').trim_end_matches(']');
    if text.is_empty() {
        return Err("Enter the address of this Mac to listen on".into());
    }
    let address: IpAddr = text
        .parse()
        .map_err(|_| format!("`{text}` is not an IP address"))?;
    match BindMode::interface(address) {
        Ok(BindMode::Loopback) => {
            Err("That is this Mac's own address. Turn the switch off to listen here only".into())
        }
        Ok(bind) => Ok(bind),
        Err(error) => Err(error.to_string()),
    }
}

/// The address this Mac would use to reach the local network, if it has one. Connects a UDP
/// socket to a documentation address, which sends nothing but makes the system pick the route.
pub fn detect_local_address() -> Option<IpAddr> {
    let socket = std::net::UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    socket.connect(("192.0.2.1", 9)).ok()?;
    let address = socket.local_addr().ok()?.ip();
    (!address.is_unspecified() && !address.is_loopback()).then_some(address)
}

/// The "Remote connections" row of the Security card: the switch, and once it is on the interface,
/// the certificate to pin and the ceiling, in rows of the same card.
fn remote_rows(
    ui: &mut Ui,
    state: &mut McpSettingsState,
    input: &McpSettingsInput,
    actions: &mut Vec<McpAction>,
) {
    let locked = input.load_error.is_some();
    let bind = input.settings.bind;
    let on = !bind.is_loopback() || state.remote_editing;
    row(ui, "Remote connections", |ui| {
        let mut value = on;
        if card::switch_enabled(ui, &mut value, !locked) {
            state.bind_error = None;
            if value {
                state.remote_editing = true;
            } else {
                state.remote_editing = false;
                if !bind.is_loopback() {
                    actions.push(McpAction::SetBind(BindMode::Loopback));
                }
            }
        }
    });
    if !on {
        return;
    }
    caption(
        ui,
        "Other computers connect over TLS, with a credential that has remote access (set on that client).",
    );
    kit::divider(ui);
    row(ui, "Interface", |ui| bind_control(ui, state, bind, actions));
    if let Some(error) = &state.bind_error {
        ui.add(
            egui::Label::new(kit::text(
                error,
                kit::size::CAPTION + 0.5,
                kit::REGULAR,
                Tone::Bad.text(),
            ))
            .wrap(),
        );
    }
    kit::divider(ui);
    row(ui, "Certificate", |ui| match input.tls_fingerprint {
        Some(fingerprint) => copy_button(
            ui,
            &mut state.copied,
            Copied::Fingerprint,
            "Copy Fingerprint",
            fingerprint,
            false,
        ),
        None => secondary(ui, "Made when you apply an address"),
    });
    if let Some(fingerprint) = input.tls_fingerprint {
        ui.add(
            egui::Label::new(
                egui::RichText::new(fingerprint)
                    .monospace()
                    .size(11.0)
                    .color(theme::text_primary()),
            )
            .wrap(),
        );
        caption(
            ui,
            "Self-signed for this address. A client must trust exactly this certificate: compare the fingerprint, never click through a warning.",
        );
    }
    kit::divider(ui);
    limit_row(
        ui,
        state,
        "Remote ceiling",
        "The most a credential with remote access can do, even on this Mac.",
        &input.settings.remote_ceiling,
        Editing::RemoteCeiling,
    );
    caption(
        ui,
        "Every change a remote credential makes waits for your approval here. Failed sign-ins are limited and logged by address.",
    );
}

/// The interface field with Detect and Apply, the same height as the port field.
fn bind_control(
    ui: &mut Ui,
    state: &mut McpSettingsState,
    bind: BindMode,
    actions: &mut Vec<McpAction>,
) {
    let id = Id::new("mcp-bind");
    let editing = ui.memory(|memory| memory.has_focus(id));
    if !editing && state.bind_synced != Some(bind) {
        state.bind_text = match bind {
            BindMode::Interface(address) => address.to_string(),
            BindMode::Loopback => String::new(),
        };
        state.bind_synced = Some(bind);
    }
    let parsed = parse_interface(&state.bind_text);
    let changed = parsed.as_ref().is_ok_and(|parsed| *parsed != bind);
    let mut apply = kit::sheet_button(ui, "Apply", changed, changed).clicked();
    if kit::sheet_button(ui, "Detect", false, true)
        .on_hover_text("Fill in this Mac's address on the local network")
        .clicked()
    {
        match detect_local_address() {
            Some(address) => {
                state.bind_text = address.to_string();
                state.bind_error = None;
            }
            None => {
                state.bind_error =
                    Some("No local network address found. Type the one to listen on".into())
            }
        }
    }
    let response = field(ui, id, &mut state.bind_text, "192.168.1.20", 160.0);
    state.bind_text.truncate(45);
    apply |= response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
    if apply {
        match parsed {
            Ok(parsed) if parsed != bind => {
                state.bind_error = None;
                state.remote_editing = false;
                actions.push(McpAction::SetBind(parsed));
            }
            Ok(_) => {}
            Err(error) => state.bind_error = Some(error),
        }
    } else if response.changed() {
        state.bind_error = None;
    }
}

/// "Turn off approvals?" Nothing changes until it is confirmed.
pub(super) fn turn_off_approvals_alert(
    ctx: &egui::Context,
    state: &mut McpSettingsState,
    actions: &mut Vec<McpAction>,
) {
    if !state.confirm_unprotect {
        return;
    }
    match kit::confirm(
        ctx,
        Id::new("mcp-turn-off-approvals"),
        &kit::Confirm {
            icon: Icon::LockFill,
            title: "Turn off approvals?",
            message: "Clients with write access will delete alerts, charts, drawings, watchlists and saved scripts, and place paper orders, without asking you first. The activity log still records every call.",
            destructive: "Turn Off",
        },
    ) {
        Some(kit::ConfirmOutcome::Confirmed) => {
            state.confirm_unprotect = false;
            actions.push(McpAction::SetRequireConfirmation(false));
        }
        Some(kit::ConfirmOutcome::Cancelled) => state.confirm_unprotect = false,
        None => {}
    }
}
