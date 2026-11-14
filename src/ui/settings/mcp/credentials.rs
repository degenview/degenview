//! The credentials section: issuing a credential and the card showing a new token.

use super::controls::{action_card, caption, copy_button, copy_button_with, field};
use super::{Copied, McpSettingsInput, McpSettingsState, connection_json};
use crate::model::mcp::{MAX_CLIENT_NAME_LEN, McpAction};
use crate::ui::icons::{self, Icon};
use crate::ui::kit::Tone;
use crate::ui::settings::window::section;
use crate::ui::{kit, theme};
use eframe::egui::{self, Align, Id, Layout, Margin, Stroke, Ui};

pub(super) fn credentials_section(
    ui: &mut Ui,
    state: &mut McpSettingsState,
    input: &McpSettingsInput,
    actions: &mut Vec<McpAction>,
) {
    section(ui, "Credentials", None, |ui| {
        let ready = !state.new_client_name.trim().is_empty() && input.load_error.is_none();
        action_card(
            ui,
            Icon::Key,
            "Generate Credential",
            "A token for one MCP client. It is shown once; only a hash is kept.",
            |ui| {
                let mut submit = kit::sheet_button(ui, "Generate", true, ready).clicked();
                let width = ui.available_width();
                let response = field(
                    ui,
                    Id::new("mcp-client-name"),
                    &mut state.new_client_name,
                    "Client name, e.g. Claude Desktop",
                    width,
                );
                if state.new_client_name.chars().count() > MAX_CLIENT_NAME_LEN {
                    state.new_client_name = state
                        .new_client_name
                        .chars()
                        .take(MAX_CLIENT_NAME_LEN)
                        .collect();
                }
                submit |= response.lost_focus()
                    && ui.input(|input| input.key_pressed(egui::Key::Enter))
                    && ready;
                if submit && ready {
                    actions.push(McpAction::CreateClient(std::mem::take(
                        &mut state.new_client_name,
                    )));
                }
            },
        );
        if let Some((name, token)) = input.fresh_token {
            ui.add_space(10.0);
            token_card(ui, state, name, token, input, actions);
        }
    });
}

/// The one-time view of a new credential, with everything needed to connect a client.
fn token_card(
    ui: &mut Ui,
    state: &mut McpSettingsState,
    name: &str,
    token: &str,
    input: &McpSettingsInput,
    actions: &mut Vec<McpAction>,
) {
    egui::Frame::new()
        .fill(Tone::Good.fill())
        .stroke(Stroke::new(1.0, Tone::Good.edge()))
        .corner_radius(12.0)
        .inner_margin(Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 10.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                icons::inline(ui, Icon::CheckCircleFill, 16.0, Tone::Good.text());
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.label(kit::text(
                        format!("Credential created for {name}"),
                        12.5,
                        kit::SEMIBOLD,
                        theme::text_primary(),
                    ));
                    caption(
                        ui,
                        "Copy it now. It is not stored and cannot be shown again.",
                    );
                });
            });
            egui::Frame::new()
                .fill(kit::primary(0.06))
                .corner_radius(8.0)
                .inner_margin(Margin::symmetric(10, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(token)
                                .monospace()
                                .size(12.0)
                                .color(theme::text_primary()),
                        )
                        .wrap(),
                    );
                });
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                copy_button(
                    ui,
                    &mut state.copied,
                    Copied::Credential,
                    "Copy Credential",
                    token,
                    true,
                );
                copy_button_with(
                    ui,
                    &mut state.copied,
                    Copied::Details,
                    "Copy Connection Details",
                    false,
                    || connection_json(&input.settings.endpoint(), Some(token)),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if kit::sheet_button(ui, "Done", false, true).clicked() {
                        actions.push(McpAction::DismissToken);
                    }
                });
            });
        });
}
