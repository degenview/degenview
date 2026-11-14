//! The clients section: one row per credential issued to an MCP client.

use super::controls::{capsule, caption, clickable_card, overview, secondary};
use super::{Editing, McpSettingsInput, McpSettingsState};
use crate::model::mcp::McpClient;
use crate::ui::kit::{Tone, time_text::relative_time};
use crate::ui::settings::window::{card, section};
use crate::ui::{kit, theme};
use eframe::egui::{Id, Ui};

pub(super) fn clients_section(ui: &mut Ui, state: &mut McpSettingsState, input: &McpSettingsInput) {
    section(
        ui,
        "Clients",
        Some(
            "Open a client to choose what it may do, whether it can be used from other machines, or to revoke it. Revoking cuts it off at once.",
        ),
        |ui| {
            if input.clients.is_empty() {
                card(ui, |ui| {
                    ui.add_space(2.0);
                    secondary(
                        ui,
                        "No clients yet. Generate a credential above to connect one.",
                    );
                    ui.add_space(2.0);
                });
                return;
            }
            for (index, client) in input.clients.iter().enumerate() {
                if index > 0 {
                    ui.add_space(8.0);
                }
                if client_row(ui, client, input) {
                    state.sheet = Some(Editing::Client(client.id.clone()));
                    state.confirm_revoke = false;
                }
            }
        },
    );
}

pub(super) fn client_row(ui: &mut Ui, client: &McpClient, input: &McpSettingsInput) -> bool {
    let effective = client.effective(input.global, &input.settings.remote_ceiling);
    let blocked = client
        .permissions
        .iter()
        .filter(|permission| !input.global.contains(*permission))
        .count();
    // Granted and within the maximum, but above what remote access allows.
    let over_ceiling = if client.remote_allowed {
        client
            .permissions
            .iter()
            .filter(|permission| {
                input.global.contains(*permission)
                    && !input.settings.remote_ceiling.contains(*permission)
            })
            .count()
    } else {
        0
    };
    clickable_card(ui, Id::new(("mcp-client", &client.id)), |ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(kit::text(
                    &client.name,
                    12.5,
                    kit::SEMIBOLD,
                    theme::text_primary(),
                ));
                if client.remote_allowed {
                    capsule(ui, "Remote", Tone::Warning, None)
                        .on_hover_text("Can be used from other machines");
                }
            });
            let used = client.last_used_ms.map_or_else(
                || "never used".to_owned(),
                |used| format!("last used {}", relative_time(input.now_ms, used)),
            );
            caption(
                ui,
                format!(
                    "Created {} · {used}",
                    relative_time(input.now_ms, client.created_at_ms)
                ),
            );
            overview(ui, &effective);
            if blocked > 0 {
                ui.label(kit::text(
                    format!(
                        "{blocked} granted permission{} held back by the maximum",
                        if blocked == 1 { "" } else { "s" }
                    ),
                    kit::size::CAPTION,
                    kit::REGULAR,
                    Tone::Warning.text(),
                ));
            }
            if over_ceiling > 0 {
                ui.label(kit::text(
                    format!(
                        "{over_ceiling} granted permission{} held back by the remote ceiling",
                        if over_ceiling == 1 { "" } else { "s" }
                    ),
                    kit::size::CAPTION,
                    kit::REGULAR,
                    Tone::Warning.text(),
                ));
            }
        });
    })
}
