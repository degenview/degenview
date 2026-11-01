//! Settings ▸ MCP Server: the on/off switch, listener status, the credentials issued to MCP
//! clients and what each may do.
//!
//! The page itself stays short: a client is one row, and its permissions (like the maximum every
//! client is capped by) are edited in a sheet opened from that row, so seventeen switches never
//! sit in the page. A pure view over [`McpSettingsInput`]: it returns [`McpAction`]s and never
//! touches the server, the store or the Keychain itself.

use crate::mcp::state::McpStatus;
use crate::model::mcp::{
    MAX_CLIENT_NAME_LEN, McpAction, McpClient, McpSettings, Permission, PermissionSet,
};
use crate::ui::alert_center::{Tone, relative_time};
use crate::ui::icons::{self, Icon};
use crate::ui::settings_window::{card, divider, section, status_badge, status_message};
use crate::ui::{kit, settings_card, sheet, theme};
use eframe::egui::{
    self, Align, Color32, Id, Layout, Margin, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};
use std::time::{Duration, Instant};

/// Height of every button and text field on the page, so a field and the button beside it line up.
const CONTROL_HEIGHT: f32 = 28.0;
const CONTROL_RADIUS: f32 = 7.0;
/// How long a copy button says "Copied".
const COPIED_FOR: Duration = Duration::from_millis(1600);
const SHEET_WIDTH: f32 = 480.0;
/// Width of the Read / Write tag on a permission row.
const ACCESS_TAG_WIDTH: f32 = 46.0;

pub struct McpSettingsInput<'a> {
    pub status: &'a McpStatus,
    pub settings: &'a McpSettings,
    pub clients: &'a [McpClient],
    pub global: &'a PermissionSet,
    /// `(client name, token)` of a credential created this session and not yet dismissed.
    pub fresh_token: Option<(&'a str, &'a str)>,
    pub load_error: Option<&'a str>,
    pub notice: Option<&'a str>,
    pub now_ms: i64,
}

#[derive(Default)]
pub struct McpSettingsState {
    new_client_name: String,
    port_text: String,
    port_synced: Option<u16>,
    /// What the permissions sheet is open on, if it is open.
    sheet: Option<Editing>,
    /// The sheet's footer is asking "Revoke it?".
    confirm_revoke: bool,
    copied: Option<(Copied, Instant)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Editing {
    /// The global cap every client is limited by.
    Maximum,
    Client(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Copied {
    Endpoint,
    Credential,
    Details,
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

/// The groups a set reaches, as `(label, changes DegenView)`: "Charts · read", "Price Alerts ·
/// read & write". Groups with nothing are left out.
fn access_summary(set: &PermissionSet) -> Vec<(String, bool)> {
    Permission::GROUPS
        .into_iter()
        .filter_map(|group| {
            let access = set.group_access(group);
            match (access.read, access.write) {
                (0, 0) => None,
                (_, 0) => Some((format!("{group} · read"), false)),
                (0, _) => Some((format!("{group} · write"), true)),
                _ => Some((format!("{group} · read & write"), true)),
            }
        })
        .collect()
}

fn initial(name: &str) -> String {
    name.chars()
        .find(|c| c.is_alphanumeric())
        .map_or_else(|| "?".to_owned(), |c| c.to_uppercase().to_string())
}

// MARK: Small controls

fn secondary(ui: &mut Ui, text: impl Into<String>) {
    ui.label(kit::text(
        text,
        kit::size::BODY,
        kit::REGULAR,
        theme::text_secondary(),
    ));
}

fn caption(ui: &mut Ui, text: impl Into<String>) {
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

/// A row of a card: its title on the left, the value or control on the right, tall enough for a
/// 28-point control so rows holding one are not taller than the rest.
fn row(ui: &mut Ui, title: &str, value: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.set_min_height(CONTROL_HEIGHT);
        ui.label(kit::text(
            title,
            kit::size::SUBHEADLINE + 1.0,
            kit::REGULAR,
            theme::text_primary(),
        ));
        ui.with_layout(Layout::right_to_left(Align::Center), value);
    });
}

/// A single-line text field exactly as tall as a button: a rounded capsule with a focus ring.
fn field(ui: &mut Ui, id: Id, text: &mut String, hint: &str, width: f32) -> egui::Response {
    field_with_rect(ui, id, text, hint, width).0
}

/// [`field`], also giving the capsule's rectangle (the text edit inside is shorter than it).
fn field_with_rect(
    ui: &mut Ui,
    id: Id,
    text: &mut String,
    hint: &str,
    width: f32,
) -> (egui::Response, Rect) {
    let focused = ui.memory(|memory| memory.has_focus(id));
    let (rect, _) = ui.allocate_exact_size(vec2(width, CONTROL_HEIGHT), Sense::hover());
    ui.painter()
        .rect_filled(rect, CONTROL_RADIUS, theme::quaternary(0.6));
    ui.painter().rect_stroke(
        rect,
        CONTROL_RADIUS,
        Stroke::new(
            1.0,
            if focused {
                theme::with_alpha(theme::text_secondary(), 0.7)
            } else {
                kit::primary(0.12)
            },
        ),
        StrokeKind::Inside,
    );
    let mut inner = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(vec2(10.0, 0.0)))
            .layout(Layout::left_to_right(Align::Center)),
    );
    let response = kit::plain_field(
        &mut inner,
        id,
        text,
        hint,
        kit::size::BODY,
        kit::REGULAR,
        rect.width() - 20.0,
    );
    (response, rect)
}

/// A copy button that says "Copied" for a moment afterwards.
fn copy_button(
    ui: &mut Ui,
    copied: &mut Option<(Copied, Instant)>,
    kind: Copied,
    label: &str,
    text: &str,
    prominent: bool,
) {
    let recent = copied.is_some_and(|(what, at)| what == kind && at.elapsed() < COPIED_FOR);
    let shown = if recent { "Copied" } else { label };
    if kit::sheet_button(ui, shown, prominent && !recent, true).clicked() {
        ui.ctx().copy_text(text.to_owned());
        *copied = Some((kind, Instant::now()));
    }
    if recent {
        ui.ctx().request_repaint_after(COPIED_FOR);
    }
}

/// A small label capsule; `write` tints it orange: it can change DegenView.
fn pill(ui: &mut Ui, text: &str, write: bool) {
    let tint = if write {
        kit::ORANGE
    } else {
        theme::text_secondary()
    };
    capsule(ui, text, tint, None);
}

/// A tinted capsule label, painted at its exact size. With a `width` the text is centred in a
/// capsule that wide, so a column of them lines up whatever the text.
fn capsule(ui: &mut Ui, text: &str, tint: Color32, width: Option<f32>) -> egui::Response {
    let galley = kit::galley(
        ui,
        text,
        kit::size::CAPTION,
        kit::MEDIUM,
        tint,
        f32::INFINITY,
    );
    let size = vec2(
        width.unwrap_or(galley.size().x + 14.0),
        galley.size().y + 4.0,
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter()
        .rect_filled(rect, 5.0, theme::with_alpha(tint, 0.14));
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, tint);
    response
}

/// The "Read" / "Write" tag every permission row carries. Write is orange (it changes
/// DegenView); Read is neutral. Fixed width so the tags form one column beside the switches.
fn access_tag(ui: &mut Ui, permission: Permission) {
    let (text, tint, meaning) = if permission.is_mutating() {
        ("Write", kit::ORANGE, "Write: can change DegenView")
    } else {
        (
            "Read",
            theme::text_secondary(),
            "Read: only looks, changes nothing",
        )
    };
    capsule(ui, text, tint, Some(ACCESS_TAG_WIDTH)).on_hover_text(meaning);
}

/// The groups a set reaches as wrapping capsules, as many lines as they need. Returns the area
/// they fill, which never reaches past the width the caller leaves.
fn pills(ui: &mut Ui, set: &PermissionSet) -> Rect {
    let summary = access_summary(set);
    if summary.is_empty() {
        return ui
            .label(kit::text(
                "No access",
                kit::size::CAPTION + 0.5,
                kit::REGULAR,
                theme::text_tertiary(),
            ))
            .rect;
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(5.0, 5.0);
        for (label, write) in summary {
            pill(ui, &label, write);
        }
    })
    .response
    .rect
}

/// A card that opens something when clicked; a chevron on the right says so. Returns the click.
fn clickable_card(ui: &mut Ui, id: Id, content: impl FnOnce(&mut Ui)) -> bool {
    let width = ui.available_width();
    let inner = kit::card_frame(false).show(ui, |ui| {
        ui.set_width(width - 30.0);
        // Leave room for the chevron.
        ui.scope(|ui| {
            ui.set_max_width(width - 30.0 - 26.0);
            content(ui);
        });
    });
    let rect = inner.response.rect;
    kit::paint_card_highlight(ui, rect);
    let response = ui.interact(rect, id, Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(
            rect,
            kit::CARD_RADIUS,
            kit::primary(if response.is_pointer_button_down_on() {
                0.07
            } else {
                0.04
            }),
        );
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    icons::paint(
        ui.painter(),
        Rect::from_center_size(
            pos2(rect.right() - 22.0, rect.center().y),
            Vec2::splat(12.0),
        ),
        Icon::ChevronRight,
        theme::text_tertiary(),
    );
    response.clicked()
}

/// A settings card: an icon badge with a title and hint, and below them one control row as tall as
/// a button. The row is given a fixed height, so it can never stretch the card to fill the page.
fn action_card(ui: &mut Ui, icon: Icon, title: &str, hint: &str, controls: impl FnOnce(&mut Ui)) {
    let width = ui.available_width();
    let card = kit::card_frame(false).show(ui, |ui| {
        ui.set_width(width - 30.0);
        ui.spacing_mut().item_spacing.y = 12.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 14.0;
            let (badge, _) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::hover());
            kit::paint_card_badge(ui, badge, icon);
            ui.vertical(|ui| {
                ui.set_max_width(width - 30.0 - 44.0);
                ui.spacing_mut().item_spacing.y = 3.0;
                ui.label(kit::text(title, 12.5, kit::SEMIBOLD, theme::text_primary()));
                caption(ui, hint);
            });
        });
        // Right to left: the button goes in first, the field fills what is left of it.
        ui.allocate_ui_with_layout(
            vec2(ui.available_width(), CONTROL_HEIGHT),
            Layout::right_to_left(Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                controls(ui);
            },
        );
    });
    kit::paint_card_highlight(ui, card.response.rect);
}

// MARK: Page

pub fn body(
    ui: &mut Ui,
    state: &mut McpSettingsState,
    input: &McpSettingsInput,
    actions: &mut Vec<McpAction>,
) {
    if let Some(error) = input.load_error {
        status_message(ui, error, true);
        ui.add_space(12.0);
    }
    server_section(ui, state, input, actions);
    ui.add_space(22.0);
    security_section(ui, state, input);
    ui.add_space(22.0);
    credentials_section(ui, state, input, actions);
    ui.add_space(22.0);
    clients_section(ui, state, input);
    permissions_sheet(ui.ctx(), state, input, actions);
}

fn server_section(
    ui: &mut Ui,
    state: &mut McpSettingsState,
    input: &McpSettingsInput,
    actions: &mut Vec<McpAction>,
) {
    section(ui, "MCP Server", None, |ui| {
        let locked = input.load_error.is_some();
        settings_card::card(
            ui,
            Icon::LockFill,
            "Enable MCP Server",
            Some(
                "Lets AI assistants and automation on this Mac inspect DegenView, within the permissions below. Off until you turn it on.",
            ),
            |ui| {
                let mut enabled = input.settings.enabled;
                if settings_card::switch_enabled(ui, &mut enabled, !locked) {
                    actions.push(McpAction::SetEnabled(enabled));
                }
            },
        );
        ui.add_space(10.0);
        card(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 10.0;
            row(ui, "Status", |ui| {
                status_badge(ui, input.status.label(), status_tone(input.status));
            });
            if let McpStatus::Failed(reason) = input.status {
                ui.add(
                    egui::Label::new(kit::text(
                        reason,
                        kit::size::CAPTION + 0.5,
                        kit::REGULAR,
                        kit::RED,
                    ))
                    .wrap(),
                );
            }
            divider(ui);
            row(ui, "Address", |ui| secondary(ui, "127.0.0.1"));
            divider(ui);
            row(ui, "Port", |ui| {
                port_control(ui, state, input.settings.port, actions)
            });
            divider(ui);
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
        if let Some(notice) = input.notice {
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

fn security_section(ui: &mut Ui, state: &mut McpSettingsState, input: &McpSettingsInput) {
    section(ui, "Security", None, |ui| {
        card(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 10.0;
            row(ui, "Authentication", |ui| secondary(ui, "Always required"));
            divider(ui);
            row(ui, "Remote connections", |ui| {
                secondary(ui, "Not supported")
            });
            divider(ui);
            row(ui, "Browser pages", |ui| secondary(ui, "Always refused"));
            divider(ui);
            maximum_row(ui, state, input);
        });
    });
}

/// Where the parts of the "Maximum permissions" row landed (read by the layout test).
#[cfg_attr(not(test), allow(dead_code))]
struct MaximumRow {
    title: Rect,
    edit: Rect,
    pills: Rect,
}

/// The title and its Edit button share one line of fixed height, and the pills take as many lines
/// as they need beneath it. The button therefore stays at the top right however many groups the
/// maximum holds, instead of drifting to the middle of a growing block of capsules.
fn maximum_row(ui: &mut Ui, state: &mut McpSettingsState, input: &McpSettingsInput) -> MaximumRow {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        let (mut title, mut edit) = (Rect::NOTHING, Rect::NOTHING);
        ui.allocate_ui_with_layout(
            vec2(ui.available_width(), CONTROL_HEIGHT),
            Layout::right_to_left(Align::Center),
            |ui| {
                let button = kit::sheet_button(ui, "Edit…", false, true).on_hover_text(
                    "The most any credential can do. Each one can only narrow this.",
                );
                edit = button.rect;
                if button.clicked() {
                    state.sheet = Some(Editing::Maximum);
                    state.confirm_revoke = false;
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    title = ui
                        .label(kit::text(
                            "Maximum permissions",
                            kit::size::SUBHEADLINE + 1.0,
                            kit::REGULAR,
                            theme::text_primary(),
                        ))
                        .rect;
                });
            },
        );
        let pills = pills(ui, input.global);
        MaximumRow { title, edit, pills }
    })
    .inner
}

fn credentials_section(
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
        .fill(theme::with_alpha(kit::GREEN, 0.09))
        .stroke(Stroke::new(1.0, theme::with_alpha(kit::GREEN, 0.35)))
        .corner_radius(12.0)
        .inner_margin(Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 10.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                icons::inline(ui, Icon::CheckCircleFill, 16.0, kit::GREEN);
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
                let details = connection_json(&input.settings.endpoint(), Some(token));
                copy_button(
                    ui,
                    &mut state.copied,
                    Copied::Details,
                    "Copy Connection Details",
                    &details,
                    false,
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if kit::sheet_button(ui, "Done", false, true).clicked() {
                        actions.push(McpAction::DismissToken);
                    }
                });
            });
        });
}

fn clients_section(ui: &mut Ui, state: &mut McpSettingsState, input: &McpSettingsInput) {
    section(
        ui,
        "Clients",
        Some(
            "Open a client to choose what it may do, or to revoke it. Revoking cuts it off at once.",
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

fn client_row(ui: &mut Ui, client: &McpClient, input: &McpSettingsInput) -> bool {
    let effective = client.effective(input.global);
    let blocked = client
        .permissions
        .iter()
        .filter(|permission| !input.global.contains(*permission))
        .count();
    clickable_card(ui, Id::new(("mcp-client", &client.id)), |ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.label(kit::text(
                &client.name,
                12.5,
                kit::SEMIBOLD,
                theme::text_primary(),
            ));
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
            pills(ui, &effective);
            if blocked > 0 {
                ui.label(kit::text(
                    format!(
                        "{blocked} granted permission{} held back by the maximum",
                        if blocked == 1 { "" } else { "s" }
                    ),
                    kit::size::CAPTION,
                    kit::REGULAR,
                    kit::ORANGE,
                ));
            }
        });
    })
}

// MARK: Permissions sheet

fn permissions_sheet(
    ctx: &egui::Context,
    state: &mut McpSettingsState,
    input: &McpSettingsInput,
    actions: &mut Vec<McpAction>,
) {
    let Some(editing) = state.sheet.clone() else {
        return;
    };
    // The client was revoked (here or elsewhere): nothing left to edit.
    let client = match &editing {
        Editing::Client(id) => match input.clients.iter().find(|client| client.id == *id) {
            Some(client) => Some(client),
            None => {
                state.sheet = None;
                state.confirm_revoke = false;
                return;
            }
        },
        Editing::Maximum => None,
    };
    let set = client.map_or(input.global, |client| &client.permissions);
    let mut close = false;

    sheet::show(ctx, Id::new("mcp-permissions"), SHEET_WIDTH, |ui| {
        ui.add_space(24.0);
        sheet::inset(ui, |ui| match client {
            Some(client) => {
                let used = client.last_used_ms.map_or_else(
                    || "never used".to_owned(),
                    |used| format!("last used {}", relative_time(input.now_ms, used)),
                );
                sheet::header_with(
                    ui,
                    &client.name,
                    &format!(
                        "Created {} · {used}",
                        relative_time(input.now_ms, client.created_at_ms)
                    ),
                    |ui| {
                        let (badge, _) = ui.allocate_exact_size(Vec2::splat(46.0), Sense::hover());
                        ui.painter()
                            .rect_filled(badge, 12.0, theme::with_alpha(kit::ACCENT, 0.14));
                        ui.painter().text(
                            badge.center(),
                            egui::Align2::CENTER_CENTER,
                            initial(&client.name),
                            egui::FontId::proportional(20.0),
                            kit::ACCENT,
                        );
                    },
                );
            }
            None => sheet::header(
                ui,
                Icon::Sliders,
                "Maximum permissions",
                "The most any credential can do. Each one can only narrow this.",
                kit::ACCENT,
            ),
        });
        ui.add_space(16.0);
        sheet::inset(ui, |ui| presets(ui, &editing, set, actions));
        ui.add_space(14.0);

        let list_height = (ctx.content_rect().height() - 330.0).clamp(160.0, 420.0);
        egui::ScrollArea::vertical()
            .max_height(list_height)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                sheet::inset(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for (index, group) in Permission::GROUPS.into_iter().enumerate() {
                        if index > 0 {
                            ui.add_space(16.0);
                        }
                        group_card(ui, group, &editing, set, input.global, actions);
                    }
                    ui.add_space(12.0);
                    caption(ui, "Changes apply immediately.");
                });
            });

        ui.add_space(14.0);
        ui.separator();
        sheet::inset(ui, |ui| {
            ui.add_space(14.0);
            sheet_footer(ui, state, client, actions, &mut close);
            ui.add_space(14.0);
        });
    });

    // Escape backs out of the revoke question first, then closes.
    if !egui::Popup::is_any_open(ctx) && ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        if state.confirm_revoke {
            state.confirm_revoke = false;
        } else {
            close = true;
        }
    }
    if close {
        state.sheet = None;
        state.confirm_revoke = false;
    }
}

/// "Read only" / "Everything" / "None": the whole set in one step.
fn presets(ui: &mut Ui, editing: &Editing, current: &PermissionSet, actions: &mut Vec<McpAction>) {
    let choose = |set: PermissionSet, actions: &mut Vec<McpAction>| {
        if &set != current {
            actions.push(match editing {
                Editing::Maximum => McpAction::SetGlobalPermissions(set),
                Editing::Client(id) => McpAction::SetClientPermissions {
                    client_id: id.clone(),
                    permissions: set,
                },
            });
        }
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(kit::text(
            "Preset",
            kit::size::CAPTION + 0.5,
            kit::SEMIBOLD,
            theme::text_secondary(),
        ));
        if kit::sheet_button(ui, "Read only", false, true).clicked() {
            choose(PermissionSet::read_only(), actions);
        }
        if kit::sheet_button(ui, "Everything", false, true).clicked() {
            choose(PermissionSet::everything(), actions);
        }
        if kit::sheet_button(ui, "None", false, true).clicked() {
            choose(PermissionSet::none(), actions);
        }
    });
}

/// One group's switches in a card, each with what it allows.
fn group_card(
    ui: &mut Ui,
    group: &'static str,
    editing: &Editing,
    set: &PermissionSet,
    cap: &PermissionSet,
    actions: &mut Vec<McpAction>,
) {
    let permissions: Vec<Permission> = Permission::ALL
        .into_iter()
        .filter(|permission| permission.group() == group)
        .collect();
    let allowed = permissions.iter().filter(|p| set.contains(**p)).count();
    ui.horizontal(|ui| {
        ui.label(kit::text(
            group,
            kit::size::CAPTION + 0.5,
            kit::SEMIBOLD,
            theme::text_secondary(),
        ));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(kit::text(
                format!("{allowed} of {}", permissions.len()),
                kit::size::CAPTION,
                kit::REGULAR,
                theme::text_tertiary(),
            ));
        });
    });
    ui.add_space(6.0);
    kit::card_frame(false)
        .inner_margin(Margin::symmetric(12, 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            for (index, permission) in permissions.iter().copied().enumerate() {
                if index > 0 {
                    divider(ui);
                }
                permission_row(ui, permission, editing, set, cap, actions);
            }
        });
}

fn permission_row(
    ui: &mut Ui,
    permission: Permission,
    editing: &Editing,
    set: &PermissionSet,
    cap: &PermissionSet,
    actions: &mut Vec<McpAction>,
) {
    let held_back = matches!(editing, Editing::Client(_))
        && set.contains(permission)
        && !cap.contains(permission);
    ui.horizontal(|ui| {
        ui.set_min_height(44.0);
        ui.spacing_mut().item_spacing.x = 12.0;
        ui.vertical(|ui| {
            ui.set_max_width((ui.available_width() - 56.0 - ACCESS_TAG_WIDTH - 12.0).max(120.0));
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 7.0;
                ui.label(kit::text(
                    permission.title(),
                    12.5,
                    kit::MEDIUM,
                    theme::text_primary(),
                ))
                .on_hover_text(permission.id());
            });
            caption(ui, permission.detail());
            if held_back {
                ui.label(kit::text(
                    "Held back: the maximum does not allow this",
                    kit::size::CAPTION,
                    kit::REGULAR,
                    kit::ORANGE,
                ));
            }
            ui.add_space(6.0);
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let mut on = set.contains(permission);
            let toggled = settings_card::switch(ui, &mut on);
            access_tag(ui, permission);
            if toggled {
                actions.push(match editing {
                    Editing::Maximum => McpAction::SetGlobalPermission {
                        permission,
                        allowed: on,
                    },
                    Editing::Client(id) => McpAction::SetClientPermission {
                        client_id: id.clone(),
                        permission,
                        allowed: on,
                    },
                });
            }
        });
    });
}

fn sheet_footer(
    ui: &mut Ui,
    state: &mut McpSettingsState,
    client: Option<&McpClient>,
    actions: &mut Vec<McpAction>,
    close: &mut bool,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        match client {
            Some(client) if state.confirm_revoke => {
                ui.label(kit::text(
                    format!("Revoke “{}”?", client.name),
                    kit::size::BODY,
                    kit::MEDIUM,
                    theme::text_primary(),
                ));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    if kit::destructive_sheet_button(ui, Icon::Xmark, "Revoke").clicked() {
                        actions.push(McpAction::RevokeClient(client.id.clone()));
                        *close = true;
                    }
                    if kit::sheet_button(ui, "Cancel", false, true).clicked() {
                        state.confirm_revoke = false;
                    }
                });
            }
            Some(_) => {
                if kit::destructive_sheet_button(ui, Icon::Xmark, "Revoke…").clicked() {
                    state.confirm_revoke = true;
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if kit::sheet_button(ui, "Done", true, true).clicked() {
                        *close = true;
                    }
                });
            }
            None => {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if kit::sheet_button(ui, "Done", true, true).clicked() {
                        *close = true;
                    }
                });
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::mcp::McpClient;
    use eframe::egui::Pos2;

    #[test]
    fn connection_json_names_the_endpoint_and_never_invents_a_token() {
        let with_token = connection_json("http://127.0.0.1:8765/mcp", Some("dvmcp_abc"));
        let parsed: serde_json::Value = serde_json::from_str(&with_token).unwrap();
        let server = &parsed["mcpServers"]["degenview"];
        assert_eq!(server["url"], "http://127.0.0.1:8765/mcp");
        assert_eq!(server["headers"]["Authorization"], "Bearer dvmcp_abc");

        let placeholder = connection_json("http://127.0.0.1:8765/mcp", None);
        assert!(placeholder.contains("<YOUR_TOKEN>"));
        assert!(!placeholder.contains("dvmcp_"));
    }

    #[test]
    fn statuses_have_distinct_tones() {
        assert_eq!(status_tone(&McpStatus::Running { port: 1 }), Tone::Good);
        assert_eq!(status_tone(&McpStatus::Failed("x".into())), Tone::Bad);
        assert_eq!(status_tone(&McpStatus::Disabled), Tone::Neutral);
    }

    #[test]
    fn access_summaries_say_read_or_write_per_group() {
        assert!(access_summary(&PermissionSet::none()).is_empty());
        assert_eq!(
            access_summary(&PermissionSet::read_only()),
            [
                ("Charts · read".to_owned(), false),
                ("Drawings · read".to_owned(), false),
                ("Price Alerts · read".to_owned(), false),
                ("Pine Scripts · read".to_owned(), false),
            ]
        );
        let mixed = PermissionSet::of([
            Permission::ChartsRead,
            Permission::ChartsCreate,
            Permission::AlertsDelete,
        ]);
        assert_eq!(
            access_summary(&mixed),
            [
                ("Charts · read & write".to_owned(), true),
                ("Price Alerts · write".to_owned(), true),
            ]
        );
    }

    #[test]
    fn initials_are_one_clean_letter() {
        assert_eq!(initial("Claude Desktop"), "C");
        assert_eq!(initial("  local automation"), "L");
        assert_eq!(initial("🙂 bot"), "B");
        assert_eq!(initial("   "), "?");
    }

    fn client(id: &str, name: &str, permissions: PermissionSet) -> McpClient {
        McpClient {
            id: id.into(),
            name: name.into(),
            verifier: String::new(),
            permissions,
            created_at_ms: 1_000,
            last_used_ms: Some(5_000),
        }
    }

    /// Runs `body` for one headless frame; returns the actions and the response rects it recorded.
    fn frame(state: &mut McpSettingsState, input: &McpSettingsInput, width: f32) -> Vec<McpAction> {
        let ctx = egui::Context::default();
        let mut actions = Vec::new();
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 900.0))),
            ..Default::default()
        };
        // Two frames: the first lays widgets out, the second runs on settled state.
        for _ in 0..2 {
            let mut output = ctx.run_ui(raw.clone(), |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    body(ui, state, input, &mut actions);
                });
            });
            output.textures_delta.clear();
        }
        actions
    }

    fn sample<'a>(
        status: &'a McpStatus,
        settings: &'a McpSettings,
        clients: &'a [McpClient],
        global: &'a PermissionSet,
    ) -> McpSettingsInput<'a> {
        McpSettingsInput {
            status,
            settings,
            clients,
            global,
            fresh_token: None,
            load_error: None,
            notice: None,
            now_ms: 10_000,
        }
    }

    #[test]
    fn the_page_and_both_sheets_lay_out_without_acting_on_their_own() {
        let status = McpStatus::Running { port: 8765 };
        let settings = McpSettings {
            enabled: true,
            port: 8765,
        };
        let clients = [
            client("a", "Claude Desktop", PermissionSet::read_only()),
            client("b", "Automation", PermissionSet::everything()),
        ];
        let global = PermissionSet::read_only();
        let input = sample(&status, &settings, &clients, &global);

        for width in [360.0, 560.0, 900.0] {
            let mut state = McpSettingsState::default();
            assert!(
                frame(&mut state, &input, width).is_empty(),
                "page at {width}"
            );
            for editing in [Editing::Maximum, Editing::Client("b".into())] {
                state.sheet = Some(editing);
                assert!(
                    frame(&mut state, &input, width).is_empty(),
                    "sheet at {width}"
                );
            }
        }
    }

    #[test]
    fn a_sheet_for_a_client_that_no_longer_exists_closes_itself() {
        let status = McpStatus::Disabled;
        let settings = McpSettings::default();
        let global = PermissionSet::read_only();
        let input = sample(&status, &settings, &[], &global);
        let mut state = McpSettingsState {
            sheet: Some(Editing::Client("gone".into())),
            confirm_revoke: true,
            ..Default::default()
        };
        assert!(frame(&mut state, &input, 600.0).is_empty());
        assert!(state.sheet.is_none() && !state.confirm_revoke);
    }

    #[test]
    fn an_action_card_is_only_as_tall_as_its_text_and_one_control_row() {
        for width in [360.0, 560.0, 900.0] {
            let ctx = egui::Context::default();
            let mut height = 0.0;
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 1400.0))),
                ..Default::default()
            };
            let mut output = ctx.run_ui(raw, |ui| {
                height = ui
                    .scope(|ui| {
                        action_card(ui, Icon::Key, "Title", "A hint of some length.", |ui| {
                            let _ = kit::sheet_button(ui, "Go", true, true);
                        })
                    })
                    .response
                    .rect
                    .height();
            });
            output.textures_delta.clear();
            assert!(
                height < 130.0,
                "a card at {width}pt wide is {height}pt tall: it must not stretch to the page"
            );
        }
    }

    /// Runs `add` in a headless frame `width` wide and returns what it returned with the right
    /// edge the frame offered.
    fn laid_out<R>(width: f32, add: impl FnOnce(&mut Ui) -> R) -> (R, f32) {
        let ctx = egui::Context::default();
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 1600.0))),
            ..Default::default()
        };
        let (mut out, mut right) = (None, 0.0);
        let mut add = Some(add);
        let mut output = ctx.run_ui(raw, |ui| {
            right = ui.max_rect().right();
            if let Some(add) = add.take() {
                out = Some(add(ui));
            }
        });
        output.textures_delta.clear();
        (out.expect("ran"), right)
    }

    #[test]
    fn the_edit_button_stays_on_the_title_line_however_many_groups_the_maximum_holds() {
        let status = McpStatus::Running { port: 8765 };
        let settings = McpSettings::default();
        let sets = [
            PermissionSet::none(),
            PermissionSet::of([Permission::ChartsRead]),
            PermissionSet::read_only(),
            PermissionSet::everything(),
        ];
        for width in [300.0, 420.0, 560.0, 900.0] {
            let mut button_tops = Vec::new();
            for set in &sets {
                let input = sample(&status, &settings, &[], set);
                let mut state = McpSettingsState::default();
                let (row, right) = laid_out(width, |ui| maximum_row(ui, &mut state, &input));
                let what = format!("{} groups at {width}pt", access_summary(set).len());
                assert!(
                    (row.edit.center().y - row.title.center().y).abs() < 1.0,
                    "Edit is not level with the title, {what}: {:?} vs {:?}",
                    row.edit,
                    row.title
                );
                assert!(
                    row.pills.top() >= row.edit.bottom() - 0.5,
                    "the capsules start above the button's line, {what}"
                );
                assert!(
                    row.edit.right() > right - 2.0 && row.edit.right() <= right + 0.5,
                    "Edit is not flush right, {what}: {} of {right}",
                    row.edit.right()
                );
                assert!(
                    row.title.right() <= row.edit.left(),
                    "title runs under Edit, {what}"
                );
                assert!(
                    row.pills.right() <= right + 0.5,
                    "the capsules overflow the row, {what}: {} of {right}",
                    row.pills.right()
                );
                assert!(
                    !row.pills.intersects(row.edit),
                    "capsules cover Edit, {what}"
                );
                button_tops.push(row.edit.top());
            }
            assert!(
                button_tops
                    .windows(2)
                    .all(|pair| (pair[0] - pair[1]).abs() < 0.5),
                "the button moves with the number of groups at {width}pt: {button_tops:?}"
            );
        }
    }

    #[test]
    fn a_wrapped_list_of_groups_is_taller_but_never_wider_than_its_line() {
        let status = McpStatus::Disabled;
        let settings = McpSettings::default();
        let narrow = |set: &PermissionSet| {
            let input = sample(&status, &settings, &[], set);
            let mut state = McpSettingsState::default();
            laid_out(300.0, |ui| maximum_row(ui, &mut state, &input))
                .0
                .pills
        };
        let few = narrow(&PermissionSet::of([Permission::ChartsRead]));
        let many = narrow(&PermissionSet::everything());
        assert!(many.height() > few.height(), "{few:?} vs {many:?}");
        assert!(many.width() <= 300.0);
    }

    #[test]
    fn a_client_with_every_group_keeps_its_capsules_clear_of_the_chevron() {
        let status = McpStatus::Running { port: 8765 };
        let settings = McpSettings::default();
        let everything = PermissionSet::everything();
        let clients = [
            client("a", "Claude Desktop", PermissionSet::everything()),
            client("b", "Read only", PermissionSet::read_only()),
            client("c", "Nothing", PermissionSet::none()),
        ];
        let input = sample(&status, &settings, &clients, &everything);
        for width in [300.0, 420.0, 560.0, 900.0] {
            let (rects, right) = laid_out(width, |ui| {
                clients
                    .iter()
                    .map(|c| ui.scope(|ui| client_row(ui, c, &input)).response.rect)
                    .collect::<Vec<_>>()
            });
            for (rect, c) in rects.iter().zip(&clients) {
                assert!(
                    rect.right() <= right + 0.5,
                    "{} overflows at {width}pt",
                    c.name
                );
                assert!(
                    rect.height() < 220.0,
                    "{} is {}pt tall at {width}pt",
                    c.name,
                    rect.height()
                );
            }
            assert!(
                rects[0].height() >= rects[2].height(),
                "more groups should not make a card shorter"
            );
        }
    }

    #[test]
    fn controls_beside_each_other_are_the_same_height() {
        let ctx = egui::Context::default();
        let (mut field_height, mut button_height) = (0.0, 0.0);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let mut text = String::from("8765");
            field_height = field_with_rect(ui, Id::new("t"), &mut text, "", 72.0)
                .1
                .height();
            button_height = kit::sheet_button(ui, "Apply", true, true).rect.height();
        });
        output.textures_delta.clear();
        assert_eq!(CONTROL_HEIGHT, 28.0);
        assert_eq!(button_height, CONTROL_HEIGHT);
        assert!(
            (field_height - CONTROL_HEIGHT).abs() < 0.5,
            "the text field ({field_height}) must match the button ({button_height})"
        );
    }
}
