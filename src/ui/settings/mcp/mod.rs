//! Settings ▸ MCP Server: the on/off switch, listener status, the credentials issued to MCP
//! clients and what each may do.
//!
//! The page itself stays short: a client is one row, and its permissions (like the maximum every
//! client is capped by) are edited in a sheet opened from that row, so seventeen switches never
//! sit in the page. A pure view over [`McpSettingsInput`]: it returns [`McpAction`]s and never
//! touches the server, the store or the Keychain itself.

use crate::mcp::state::McpStatus;
use crate::model::mcp::{
    BindMode, McpAction, McpAuditEntry, McpClient, McpSettings, Permission, PermissionSet,
};
use crate::ui::icons::Icon;
use crate::ui::kit::Tone;
use crate::ui::mcp::activity::{self, ActivityInput, ActivityState};
use crate::ui::settings::window::status_message;
use crate::ui::{kit, theme};
use clients::clients_section;
use credentials::credentials_section;
use eframe::egui::Ui;
use permissions_sheet::permissions_sheet;
use security::{security_section, turn_off_approvals_alert};
use server::server_section;
use std::time::{Duration, Instant};

mod clients;
mod controls;
mod credentials;
mod permissions_sheet;
mod security;
mod server;

pub(in crate::ui) use controls::capsule;

/// Height of every button and text field on the page, so a field and the button beside it line up.
const CONTROL_HEIGHT: f32 = 28.0;
const CONTROL_RADIUS: f32 = 7.0;
/// How long a copy button says "Copied".
const COPIED_FOR: Duration = Duration::from_millis(1600);
const SHEET_WIDTH: f32 = 480.0;

pub struct McpSettingsInput<'a> {
    pub status: &'a McpStatus,
    pub settings: &'a McpSettings,
    pub clients: &'a [McpClient],
    pub global: &'a PermissionSet,
    /// `(client name, token)` of a credential created this session and not yet dismissed.
    pub fresh_token: Option<(&'a str, &'a str)>,
    pub load_error: Option<&'a str>,
    /// The error is a failed seal check: offer to reset MCP access.
    pub can_reset: bool,
    pub notice: Option<&'a str>,
    /// The activity log, newest first: the page previews it and its sheet searches it.
    pub activity: &'a [McpAuditEntry],
    /// Rows in the log this version cannot read, so the sheet can say it hides some.
    pub activity_unreadable: usize,
    pub now_ms: i64,
    pub utc_offset_secs: i64,
    /// SHA-256 of the certificate a remote client pins, when the server listens beyond this Mac.
    pub tls_fingerprint: Option<&'a str>,
}

#[derive(Default)]
pub struct McpSettingsState {
    new_client_name: String,
    port_text: String,
    port_synced: Option<u16>,
    /// The "other machines" switch is on and waiting for an address; nothing is saved until one
    /// is applied.
    remote_editing: bool,
    bind_text: String,
    bind_synced: Option<BindMode>,
    bind_error: Option<String>,
    /// The address field of the open client sheet's "only from these addresses" list.
    range_text: String,
    range_error: Option<String>,
    /// What the permissions sheet is open on, if it is open.
    sheet: Option<Editing>,
    /// The sheet's footer is asking "Revoke it?".
    confirm_revoke: bool,
    /// The load-error banner is asking "Revoke everything?".
    confirm_reset: bool,
    /// The activity log's sheet, with the filters set in it.
    activity: ActivityState,
    /// The "Turn off approvals?" alert is open.
    confirm_unprotect: bool,
    copied: Option<(Copied, Instant)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Editing {
    /// The global cap every client is limited by.
    Maximum,
    /// The most a credential with remote access may do, whatever the cap above allows.
    RemoteCeiling,
    Client(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Copied {
    Endpoint,
    Credential,
    Details,
    Fingerprint,
}

/// The `mcpServers` entry a client's config file takes. The token is shown only while it is
/// still known; afterwards a placeholder stands in for it.
pub fn connection_json(endpoint: &str, token: Option<&str>) -> String {
    let value = serde_json::json!({
        "mcpServers": {
            "degenview": {
                "type": "http",
                "url": endpoint,
                "headers": {
                    "Authorization": format!("Bearer {}", token.unwrap_or("<YOUR_TOKEN>")),
                },
            }
        }
    });
    serde_json::to_string_pretty(&value).unwrap_or_default()
}

fn status_tone(status: &McpStatus) -> Tone {
    match status {
        McpStatus::Running { .. } => Tone::Good,
        McpStatus::Starting | McpStatus::Stopping => Tone::Warning,
        McpStatus::Failed(_) => Tone::Bad,
        McpStatus::Disabled => Tone::Neutral,
    }
}

/// What a set of permissions amounts to, in two short pieces: how much it can do ("Read only",
/// "Read & write") and where. Replaces a capsule per area, which made a long set read as noise.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AccessOverview {
    headline: &'static str,
    /// The areas, a few by name and then a count; empty when there is nothing to name.
    detail: String,
    /// It can change DegenView, not only look.
    writes: bool,
}

/// At most this many areas are named before "+ N more".
const NAMED_AREAS: usize = 3;

fn named(areas: &[&str]) -> String {
    let shown = areas
        .iter()
        .take(NAMED_AREAS)
        .copied()
        .collect::<Vec<_>>()
        .join(", ");
    match areas.len().saturating_sub(NAMED_AREAS) {
        0 => shown,
        more => format!("{shown} +{more} more"),
    }
}

fn access_overview(set: &PermissionSet) -> AccessOverview {
    let total = Permission::GROUPS.len();
    let (mut reads, mut writes) = (Vec::new(), Vec::new());
    for group in Permission::GROUPS {
        let access = set.group_access(group);
        if access.write > 0 {
            writes.push(group);
        } else if access.read > 0 {
            reads.push(group);
        }
    }
    if reads.is_empty() && writes.is_empty() {
        return AccessOverview {
            headline: "No access",
            detail: String::new(),
            writes: false,
        };
    }
    if writes.is_empty() {
        let detail = if reads.len() == total {
            format!("All {total} areas")
        } else {
            named(&reads)
        };
        return AccessOverview {
            headline: "Read only",
            detail,
            writes: false,
        };
    }
    let mut detail = format!("Changes {}", named(&writes));
    if !reads.is_empty() {
        detail.push_str(&format!(
            " · reads {} more area{}",
            reads.len(),
            if reads.len() == 1 { "" } else { "s" }
        ));
    }
    AccessOverview {
        headline: "Read & write",
        detail,
        writes: true,
    }
}

fn initial(name: &str) -> String {
    name.chars()
        .find(|c| c.is_alphanumeric())
        .map_or_else(|| "?".to_owned(), |c| c.to_uppercase().to_string())
}

pub fn body(
    ui: &mut Ui,
    state: &mut McpSettingsState,
    input: &McpSettingsInput,
    actions: &mut Vec<McpAction>,
) {
    if let Some(error) = input.load_error {
        status_message(ui, error, true);
        ui.add_space(8.0);
        if input.can_reset {
            reset_row(ui, state, actions);
        }
        ui.add_space(12.0);
    }
    server_section(ui, state, input, actions);
    ui.add_space(22.0);
    security_section(ui, state, input, actions);
    ui.add_space(22.0);
    credentials_section(ui, state, input, actions);
    ui.add_space(22.0);
    clients_section(ui, state, input);
    ui.add_space(22.0);
    let activity = ActivityInput {
        entries: input.activity,
        unreadable: input.activity_unreadable,
        now_ms: input.now_ms,
        utc_offset_secs: input.utc_offset_secs,
    };
    activity::preview(ui, &mut state.activity, &activity);
    permissions_sheet(ui.ctx(), state, input, actions);
    activity::sheet(ui.ctx(), &mut state.activity, &activity, actions);
    turn_off_approvals_alert(ui.ctx(), state, actions);
}

/// "Reset MCP Access…", then a one-line confirmation: it revokes every credential.
fn reset_row(ui: &mut Ui, state: &mut McpSettingsState, actions: &mut Vec<McpAction>) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        if state.confirm_reset {
            ui.label(kit::text(
                "Revoke every credential and turn the server off?",
                kit::size::BODY,
                kit::MEDIUM,
                theme::text_primary(),
            ));
            if kit::destructive_sheet_button(ui, Icon::Xmark, "Reset").clicked() {
                actions.push(McpAction::ResetAccess);
                state.confirm_reset = false;
            }
            if kit::sheet_button(ui, "Cancel", false, true).clicked() {
                state.confirm_reset = false;
            }
        } else if kit::destructive_sheet_button(ui, Icon::Xmark, "Reset MCP Access…").clicked() {
            state.confirm_reset = true;
        }
    });
}

#[cfg(test)]
mod tests;
