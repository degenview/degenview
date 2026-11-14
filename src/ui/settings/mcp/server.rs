//! The server section: the switch, the listener's status and the port.

use super::controls::{copy_button, field, row, secondary};
use super::{Copied, McpSettingsInput, McpSettingsState, status_tone};
use crate::mcp::state::McpStatus;
use crate::model::mcp::McpAction;
use crate::ui::icons::Icon;
use crate::ui::kit::Tone;
use crate::ui::settings::window::{card, section, status_message};
use crate::ui::{kit, settings::card, theme};
use eframe::egui::{self, Id, Ui};

pub(super) fn server_section(
    ui: &mut Ui,
    state: &mut McpSettingsState,
    input: &McpSettingsInput,
    actions: &mut Vec<McpAction>,
) {
    section(ui, "MCP Server", None, |ui| {
        let locked = input.load_error.is_some();
        card::card(
            ui,
            Icon::LockFill,
            "Enable MCP Server",
            Some(
                "Lets AI assistants and automation on this Mac inspect DegenView, within the permissions below. Off until you turn it on.",
            ),
            |ui| {
                let mut enabled = input.settings.enabled;
                if card::switch_enabled(ui, &mut enabled, !locked) {
                    actions.push(McpAction::SetEnabled(enabled));
                }
            },
        );
        ui.add_space(10.0);
        card::card(
            ui,
            Icon::CheckCircle,
            "Ask before high-impact requests",
            Some(
                "Deleting alerts, charts, drawings or watchlists, saving over a script, and every paper order wait for your approval in DegenView. On unless you turn it off. A credential with remote access is asked about every change, whatever this says.",
            ),
            |ui| {
                let mut required = input.settings.require_confirmation;
                if card::switch_enabled(ui, &mut required, !locked) {
                    if required {
                        actions.push(McpAction::SetRequireConfirmation(true));
                    } else {
                        // Turning protection off is asked about; turning it on is not.
                        state.confirm_unprotect = true;
                    }
                }
            },
        );
        ui.add_space(10.0);
        card(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 10.0;
            row(ui, "Status", |ui| {
                kit::status_badge(ui, input.status.label(), status_tone(input.status));
            });
            if let McpStatus::Failed(reason) = input.status {
                ui.add(
                    egui::Label::new(kit::text(
                        reason,
                        kit::size::CAPTION + 0.5,
                        kit::REGULAR,
                        Tone::Bad.text(),
                    ))
                    .wrap(),
                );
            }
            kit::divider(ui);
            row(ui, "Address", |ui| {
                secondary(ui, input.settings.bind.address().to_string())
            });
            kit::divider(ui);
            row(ui, "Port", |ui| {
                port_control(ui, state, input.settings.port, actions)
            });
            kit::divider(ui);
            row(ui, "Endpoint", |ui| {
                let endpoint = input.settings.endpoint();
                copy_button(
                    ui,
                    &mut state.copied,
                    Copied::Endpoint,
                    "Copy",
                    &endpoint,
                    false,
                );
                ui.label(
                    egui::RichText::new(endpoint)
                        .monospace()
                        .size(12.0)
                        .color(theme::text_primary()),
                );
            });
        });
        // A failure the Status row already gives (the port is in use) is said there only.
        let said_in_status = matches!(input.status, McpStatus::Failed(reason) if Some(reason.as_str()) == input.notice);
        if let Some(notice) = input.notice.filter(|_| !said_in_status) {
            ui.add_space(8.0);
            status_message(ui, notice, true);
        }
    });
}

/// The port field and its Apply button, the same height and corner radius, Apply live only while
/// the field holds a different valid port.
fn port_control(
    ui: &mut Ui,
    state: &mut McpSettingsState,
    port: u16,
    actions: &mut Vec<McpAction>,
) {
    let id = Id::new("mcp-port");
    let editing = ui.memory(|memory| memory.has_focus(id));
    if !editing && state.port_synced != Some(port) {
        state.port_text = port.to_string();
        state.port_synced = Some(port);
    }
    let parsed = state.port_text.trim().parse::<u16>().ok();
    let changed = parsed.is_some_and(|value| value != port);
    // Right to left: the button, then the field to its left.
    if kit::sheet_button(ui, "Apply", changed, changed).clicked()
        && let Some(value) = parsed
    {
        actions.push(McpAction::SetPort(value));
    }
    let response = field(ui, id, &mut state.port_text, "8765", 72.0);
    // Digits only: nothing else is a port.
    state.port_text.retain(|c| c.is_ascii_digit());
    state.port_text.truncate(5);
    if response.lost_focus()
        && ui.input(|input| input.key_pressed(egui::Key::Enter))
        && let Some(value) = parsed
        && changed
    {
        actions.push(McpAction::SetPort(value));
    }
}
