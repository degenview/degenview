//! Settings ▸ MCP Server: the on/off switch, listener status, the credentials issued to MCP
//! clients and what each may do.
//!
//! The page itself stays short: a client is one row, and its permissions (like the maximum every
//! client is capped by) are edited in a sheet opened from that row, so seventeen switches never
//! sit in the page. A pure view over [`McpSettingsInput`]: it returns [`McpAction`]s and never
//! touches the server, the store or the Keychain itself.

use crate::mcp::state::McpStatus;
use crate::model::mcp::{
    BindMode, Cidr, MAX_CIDRS, MAX_CLIENT_NAME_LEN, McpAction, McpAuditEntry, McpClient,
    McpSettings, Permission, PermissionSet,
};
use crate::ui::alert::center::{Tone, relative_time};
use crate::ui::icons::{self, Icon};
use crate::ui::mcp::activity::{self, ActivityInput, ActivityState};
use crate::ui::settings::window::{card, divider, section, status_badge, status_message};
use crate::ui::{kit, settings::card, sheet, theme};
use eframe::egui::{
    self, Align, Id, Layout, Margin, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};
use std::net::IpAddr;
use std::time::{Duration, Instant};

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

/// A label capsule in a tone's soft colours (a faint plate, a legible shade, a hairline edge),
/// painted at its exact size. With a `width` the text is centred in a capsule that wide, so a
/// column of them lines up whatever the text.
pub(in crate::ui) fn capsule(
    ui: &mut Ui,
    text: &str,
    tone: Tone,
    width: Option<f32>,
) -> egui::Response {
    let ink = tone.text();
    let galley = kit::galley(
        ui,
        text,
        kit::size::CAPTION,
        kit::MEDIUM,
        ink,
        f32::INFINITY,
    );
    let size = vec2(
        width.unwrap_or(galley.size().x + 14.0),
        galley.size().y + 4.0,
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, 6.0, tone.fill());
    ui.painter()
        .rect_stroke(rect, 6.0, Stroke::new(1.0, tone.edge()), StrokeKind::Inside);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, ink);
    response
}

/// What a set of permissions can do: one capsule ("Read only" is quiet, "Read & write" amber) and
/// the areas beside it, wrapping under it on a narrow line. Returns the area it fills, which never
/// reaches past the width the caller leaves.
fn overview(ui: &mut Ui, set: &PermissionSet) -> Rect {
    let overview = access_overview(set);
    if overview.headline == "No access" {
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
        ui.spacing_mut().item_spacing = vec2(8.0, 4.0);
        let tone = if overview.writes {
            Tone::Warning
        } else {
            Tone::Neutral
        };
        capsule(ui, overview.headline, tone, None);
        ui.label(kit::text(
            overview.detail,
            kit::size::CAPTION + 0.5,
            kit::REGULAR,
            theme::text_secondary(),
        ));
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
    kit::card_frame(false).show(ui, |ui| {
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

fn server_section(
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
                status_badge(ui, input.status.label(), status_tone(input.status));
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
            divider(ui);
            row(ui, "Address", |ui| {
                secondary(ui, input.settings.bind.address().to_string())
            });
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

fn security_section(
    ui: &mut Ui,
    state: &mut McpSettingsState,
    input: &McpSettingsInput,
    actions: &mut Vec<McpAction>,
) {
    section(ui, "Security", None, |ui| {
        card(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 10.0;
            row(ui, "Authentication", |ui| secondary(ui, "Always required"));
            divider(ui);
            remote_rows(ui, state, input, actions);
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
    summary: Rect,
}

/// The title and its Edit button share one line of fixed height, and the summary takes as many
/// lines as it needs beneath it, so the button stays at the top right however much the maximum holds.
fn maximum_row(ui: &mut Ui, state: &mut McpSettingsState, input: &McpSettingsInput) -> MaximumRow {
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
    divider(ui);
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
    divider(ui);
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
    divider(ui);
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

fn client_row(ui: &mut Ui, client: &McpClient, input: &McpSettingsInput) -> bool {
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

/// "Turn off approvals?" Nothing changes until it is confirmed.
fn turn_off_approvals_alert(
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
        Editing::Maximum | Editing::RemoteCeiling => None,
    };
    let set = match (&editing, client) {
        (Editing::RemoteCeiling, _) => &input.settings.remote_ceiling,
        (_, Some(client)) => &client.permissions,
        (_, None) => input.global,
    };
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
            None if editing == Editing::RemoteCeiling => sheet::header(
                ui,
                Icon::Link,
                "Remote ceiling",
                "The most a credential with remote access can do, even on this Mac.",
                theme::text_secondary(),
            ),
            None => sheet::header(
                ui,
                Icon::Sliders,
                "Maximum permissions",
                "The most any credential can do. Each one can only narrow this.",
                kit::ACCENT,
            ),
        });
        // A client's sheet carries more above the groups than the other two, so its presets
        // scroll with them rather than leaving the groups a row or two of the window.
        if client.is_none() {
            ui.add_space(16.0);
            sheet::inset(ui, |ui| presets(ui, &editing, set, actions));
        }
        ui.add_space(14.0);

        let list_height = (ctx.content_rect().height() - 330.0).clamp(160.0, 420.0);
        egui::ScrollArea::vertical()
            .max_height(list_height)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                sheet::inset(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    // With the groups, so a short window scrolls them together instead of leaving
                    // the permissions a row or two to show in.
                    if let Some(client) = client {
                        client_remote_block(ui, state, client, actions);
                        ui.add_space(14.0);
                        presets(ui, &editing, set, actions);
                        ui.add_space(16.0);
                    }
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

/// "Use from other machines" and, once on, the only addresses it may call from.
fn client_remote_block(
    ui: &mut Ui,
    state: &mut McpSettingsState,
    client: &McpClient,
    actions: &mut Vec<McpAction>,
) {
    kit::card_frame(false)
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.horizontal(|ui| {
                ui.set_min_height(CONTROL_HEIGHT);
                ui.vertical(|ui| {
                    ui.set_max_width(ui.available_width() - 56.0);
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.label(kit::text(
                        "Use from other machines",
                        12.5,
                        kit::MEDIUM,
                        theme::text_primary(),
                    ));
                    caption(
                        ui,
                        if client.remote_allowed {
                            "Held to the remote ceiling, and every change waits for your approval."
                        } else {
                            "Off: this credential works from this Mac only."
                        },
                    );
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let mut on = client.remote_allowed;
                    if card::switch(ui, &mut on) {
                        state.range_error = None;
                        actions.push(McpAction::SetClientRemote {
                            client_id: client.id.clone(),
                            allowed: on,
                        });
                    }
                });
            });
            if !client.remote_allowed {
                return;
            }
            divider(ui);
            ui.label(kit::text(
                "Only from these addresses",
                kit::size::CAPTION + 0.5,
                kit::SEMIBOLD,
                theme::text_secondary(),
            ));
            if client.allowed_cidrs.is_empty() {
                caption(ui, "Any address that has the token.");
            } else {
                let mut removed = None;
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                    for (index, cidr) in client.allowed_cidrs.iter().enumerate() {
                        if range_chip(ui, &cidr.to_string()) {
                            removed = Some(index);
                        }
                    }
                });
                if let Some(index) = removed {
                    let mut cidrs = client.allowed_cidrs.clone();
                    cidrs.remove(index);
                    actions.push(McpAction::SetClientCidrs {
                        client_id: client.id.clone(),
                        cidrs,
                    });
                }
            }
            let full = client.allowed_cidrs.len() >= MAX_CIDRS;
            let parsed = Cidr::parse(&state.range_text);
            let ready = parsed.is_ok() && !full;
            ui.allocate_ui_with_layout(
                vec2(ui.available_width(), CONTROL_HEIGHT),
                Layout::right_to_left(Align::Center),
                |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let mut add = kit::sheet_button(ui, "Add", ready, ready).clicked();
                    let width = ui.available_width();
                    let response = field(
                        ui,
                        Id::new("mcp-range"),
                        &mut state.range_text,
                        "192.168.1.0/24",
                        width,
                    );
                    state.range_text.truncate(50);
                    add |= response.lost_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter));
                    if response.changed() {
                        state.range_error = None;
                    }
                    if add {
                        match &parsed {
                            _ if full => {
                                state.range_error =
                                    Some(format!("At most {MAX_CIDRS} address ranges"));
                            }
                            Ok(cidr) => {
                                let mut cidrs = client.allowed_cidrs.clone();
                                if !cidrs.contains(cidr) {
                                    cidrs.push(*cidr);
                                    actions.push(McpAction::SetClientCidrs {
                                        client_id: client.id.clone(),
                                        cidrs,
                                    });
                                }
                                state.range_text.clear();
                                state.range_error = None;
                            }
                            Err(error) => state.range_error = Some(error.clone()),
                        }
                    }
                },
            );
            if let Some(error) = &state.range_error {
                ui.label(kit::text(
                    error,
                    kit::size::CAPTION + 0.5,
                    kit::REGULAR,
                    Tone::Bad.text(),
                ));
            } else {
                caption(
                    ui,
                    "An address or a range, like 192.168.1.20 or 10.0.0.0/8. A request from anywhere else is refused as if the token were wrong.",
                );
            }
        });
}

/// An address range as a capsule with a remove mark. Returns whether the mark was clicked.
fn range_chip(ui: &mut Ui, text: &str) -> bool {
    let galley = kit::galley(
        ui,
        text,
        kit::size::CAPTION + 0.5,
        kit::MEDIUM,
        theme::text_primary(),
        f32::INFINITY,
    );
    let size = vec2(galley.size().x + 14.0 + 20.0, 22.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter()
        .rect_filled(rect, 6.0, theme::with_alpha(theme::text_secondary(), 0.14));
    ui.painter().galley(
        pos2(rect.left() + 8.0, rect.center().y - galley.size().y / 2.0),
        galley,
        theme::text_primary(),
    );
    let mark = Rect::from_center_size(
        pos2(rect.right() - 11.0, rect.center().y),
        Vec2::splat(18.0),
    );
    let response = ui.interact(
        mark,
        ui.id().with(("mcp-range-remove", text)),
        Sense::click(),
    );
    icons::paint(
        ui.painter(),
        Rect::from_center_size(mark.center(), Vec2::splat(8.0)),
        Icon::Xmark,
        if response.hovered() {
            theme::text_primary()
        } else {
            theme::text_tertiary()
        },
    );
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Remove this range")
        .clicked()
}

/// "Read only" / "Everything" / "None": the whole set in one step.
fn presets(ui: &mut Ui, editing: &Editing, current: &PermissionSet, actions: &mut Vec<McpAction>) {
    let choose = |set: PermissionSet, actions: &mut Vec<McpAction>| {
        if &set != current {
            actions.push(match editing {
                Editing::Maximum => McpAction::SetGlobalPermissions(set),
                Editing::RemoteCeiling => McpAction::SetRemoteCeiling(set),
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
            ui.set_max_width((ui.available_width() - 56.0 - 12.0).max(120.0));
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 7.0;
                // Only a permission that can change DegenView is marked, with a small amber dot:
                // the many that only look carry nothing, so the marked ones stand out.
                if permission.is_mutating() {
                    let (dot, response) = ui.allocate_exact_size(Vec2::splat(7.0), Sense::hover());
                    ui.painter()
                        .circle_filled(dot.center(), 3.0, Tone::Warning.dot());
                    response.on_hover_text("Can change DegenView");
                }
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
                    Tone::Warning.text(),
                ));
            }
            ui.add_space(6.0);
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let mut on = set.contains(permission);
            let toggled = card::switch(ui, &mut on);
            if toggled {
                actions.push(match editing {
                    Editing::Maximum => McpAction::SetGlobalPermission {
                        permission,
                        allowed: on,
                    },
                    Editing::RemoteCeiling => {
                        let mut ceiling = set.clone();
                        ceiling.set(permission, on);
                        McpAction::SetRemoteCeiling(ceiling)
                    }
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

    fn overview_of(set: &PermissionSet) -> (&'static str, String, bool) {
        let o = access_overview(set);
        (o.headline, o.detail, o.writes)
    }

    #[test]
    fn a_set_of_permissions_reads_as_how_much_and_where_not_as_a_capsule_per_area() {
        assert_eq!(
            overview_of(&PermissionSet::none()),
            ("No access", String::new(), false)
        );
        let total = Permission::GROUPS.len();
        assert_eq!(
            overview_of(&PermissionSet::read_only()),
            ("Read only", format!("All {total} areas"), false)
        );
        // A few areas are named; a longer list is counted.
        let some = PermissionSet::of([Permission::ChartsRead, Permission::AlertsRead]);
        assert_eq!(
            overview_of(&some),
            ("Read only", "Charts, Price Alerts".to_owned(), false)
        );
        let many = PermissionSet::of([
            Permission::ChartsRead,
            Permission::DrawingsRead,
            Permission::AlertsRead,
            Permission::MarketsRead,
            Permission::PortfolioRead,
        ]);
        assert_eq!(
            overview_of(&many),
            (
                "Read only",
                "Charts, Drawings, Price Alerts +2 more".to_owned(),
                false
            )
        );
        // Anything that changes DegenView says so, names what it changes, and counts the rest.
        let mixed = PermissionSet::of([
            Permission::ChartsRead,
            Permission::ChartsCreate,
            Permission::AlertsDelete,
            Permission::MarketsRead,
        ]);
        assert_eq!(
            overview_of(&mixed),
            (
                "Read & write",
                "Changes Charts, Price Alerts · reads 1 more area".to_owned(),
                true
            )
        );
        let only_writes = PermissionSet::of([Permission::AlertsDelete]);
        assert_eq!(
            overview_of(&only_writes),
            ("Read & write", "Changes Price Alerts".to_owned(), true)
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
            remote_allowed: false,
            allowed_cidrs: Vec::new(),
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
            can_reset: false,
            notice: None,
            activity: &[],
            activity_unreadable: 0,
            now_ms: 10_000,
            utc_offset_secs: 0,
            tls_fingerprint: None,
        }
    }

    /// Runs frames over a page with a pending "Turn off approvals?" alert, pressing `keys`.
    fn alert_after(
        state: McpSettingsState,
        keys: &[egui::Key],
    ) -> (Vec<McpAction>, McpSettingsState) {
        let status = McpStatus::Disabled;
        let settings = McpSettings::default();
        let global = PermissionSet::read_only();
        let input = sample(&status, &settings, &[], &global);
        let mut state = state;
        let ctx = egui::Context::default();
        let mut actions = Vec::new();
        let run = |ctx: &egui::Context,
                   state: &mut McpSettingsState,
                   actions: &mut Vec<McpAction>,
                   events: Vec<egui::Event>,
                   time: f64| {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(600.0, 900.0))),
                time: Some(time),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(raw, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| body(ui, state, &input, actions));
            });
            output.textures_delta.clear();
        };
        run(&ctx, &mut state, &mut actions, Vec::new(), 0.0);
        run(&ctx, &mut state, &mut actions, Vec::new(), 0.1);
        for (index, key) in keys.iter().enumerate() {
            let press = |pressed| egui::Event::Key {
                key: *key,
                physical_key: Some(*key),
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            };
            let time = 0.2 + index as f64 * 0.2;
            run(&ctx, &mut state, &mut actions, vec![press(true)], time);
            run(
                &ctx,
                &mut state,
                &mut actions,
                vec![press(false)],
                time + 0.1,
            );
        }
        (actions, state)
    }

    fn unprotect_alert_after(keys: &[egui::Key]) -> (Vec<McpAction>, bool) {
        let (actions, state) = alert_after(
            McpSettingsState {
                confirm_unprotect: true,
                ..Default::default()
            },
            keys,
        );
        (actions, state.confirm_unprotect)
    }

    #[test]
    fn turning_off_approvals_asks_first_and_only_a_chosen_button_does_it() {
        let (actions, open) = unprotect_alert_after(&[]);
        assert!(actions.is_empty() && open);
        // The dialog starts on Cancel: Return alone cancels, changing nothing, and so does Escape.
        let (actions, open) = unprotect_alert_after(&[egui::Key::Enter]);
        assert!(actions.is_empty() && !open);
        let (actions, open) = unprotect_alert_after(&[egui::Key::Escape]);
        assert!(actions.is_empty() && !open);
        let (actions, open) = unprotect_alert_after(&[egui::Key::Tab, egui::Key::Enter]);
        assert_eq!(actions, [McpAction::SetRequireConfirmation(false)]);
        assert!(!open);
    }

    #[test]
    fn the_page_and_both_sheets_lay_out_without_acting_on_their_own() {
        let status = McpStatus::Running { port: 8765 };
        let settings = McpSettings {
            enabled: true,
            port: 8765,
            ..McpSettings::default()
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
            state.sheet = None;
            state.activity.open();
            assert!(
                frame(&mut state, &input, width).is_empty(),
                "activity sheet at {width}"
            );
        }
    }

    #[test]
    fn an_interface_is_one_address_of_this_mac_never_loopback_or_all_of_them() {
        assert_eq!(
            parse_interface(" 192.168.1.20 "),
            Ok(BindMode::Interface("192.168.1.20".parse().unwrap()))
        );
        assert_eq!(
            parse_interface("[fd00::20]"),
            Ok(BindMode::Interface("fd00::20".parse().unwrap()))
        );
        for bad in [
            "",
            "   ",
            "lan",
            "192.168.1",
            "0.0.0.0",
            "::",
            "224.0.0.1",
            "fe80::1",
        ] {
            assert!(
                parse_interface(bad).is_err(),
                "{bad:?} must not be accepted"
            );
        }
        let own = parse_interface("127.0.0.1").unwrap_err();
        assert!(own.contains("own address"), "{own}");
        assert!(
            parse_interface("0.0.0.0")
                .unwrap_err()
                .contains("one interface")
        );
    }

    #[test]
    fn a_detected_address_is_never_loopback_or_unspecified() {
        if let Some(address) = detect_local_address() {
            assert!(
                !address.is_loopback() && !address.is_unspecified(),
                "{address}"
            );
        }
    }

    #[test]
    fn remote_access_lays_out_in_every_state_without_acting_on_its_own() {
        let status = McpStatus::Running { port: 8765 };
        let lan = BindMode::interface("192.168.1.20".parse().unwrap()).unwrap();
        let on = McpSettings {
            enabled: true,
            bind: lan,
            remote_ceiling: PermissionSet::of([Permission::ChartsRead, Permission::AlertsRead]),
            ..McpSettings::default()
        };
        let off = McpSettings {
            enabled: true,
            ..McpSettings::default()
        };
        let mut remote = client("r", "Laptop", PermissionSet::read_only());
        remote.remote_allowed = true;
        remote.allowed_cidrs = vec![
            Cidr::parse("192.168.1.0/24").unwrap(),
            Cidr::parse("100.64.0.0/10").unwrap(),
            Cidr::parse("fd00::/8").unwrap(),
        ];
        let clients = [remote, client("l", "Local", PermissionSet::read_only())];
        let global = PermissionSet::read_only();
        let fingerprint = "AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89";

        for width in [360.0, 560.0, 900.0] {
            // Off, then switched on and waiting for an address, then on with a certificate.
            let input = sample(&status, &off, &clients, &global);
            let mut state = McpSettingsState::default();
            assert!(frame(&mut state, &input, width).is_empty());
            state.remote_editing = true;
            assert!(
                frame(&mut state, &input, width).is_empty(),
                "editing at {width}"
            );
            state.bind_error = Some("`lan` is not an IP address".into());
            assert!(frame(&mut state, &input, width).is_empty());

            let mut input = sample(&status, &on, &clients, &global);
            input.tls_fingerprint = Some(fingerprint);
            let mut state = McpSettingsState::default();
            assert!(frame(&mut state, &input, width).is_empty(), "on at {width}");
            assert_eq!(
                state.bind_text, "192.168.1.20",
                "the field shows the saved address"
            );
            for editing in [Editing::RemoteCeiling, Editing::Client("r".into())] {
                state.sheet = Some(editing);
                assert!(
                    frame(&mut state, &input, width).is_empty(),
                    "sheet at {width}"
                );
            }
            state.range_error = Some("`x` is not an address range".into());
            assert!(frame(&mut state, &input, width).is_empty());
        }
    }

    /// Every text one frame of the page paints.
    fn painted(input: &McpSettingsInput) -> Vec<String> {
        fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(text) => out.push(text.galley.text().to_owned()),
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| walk(shape, out)),
                _ => {}
            }
        }
        let ctx = egui::Context::default();
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(600.0, 6000.0))),
            ..Default::default()
        };
        let mut state = McpSettingsState::default();
        let mut actions = Vec::new();
        let mut out = Vec::new();
        for _ in 0..2 {
            out.clear();
            let mut output = ctx.run_ui(raw.clone(), |ui| {
                egui::ScrollArea::vertical()
                    .show(ui, |ui| body(ui, &mut state, input, &mut actions));
            });
            output.textures_delta.clear();
            for clipped in &output.shapes {
                walk(&clipped.shape, &mut out);
            }
        }
        out
    }

    #[test]
    fn a_remote_credential_is_marked_on_its_row_and_a_local_one_is_not() {
        let status = McpStatus::Disabled;
        let settings = McpSettings::default();
        let global = PermissionSet::read_only();
        let local = [client("l", "Local", PermissionSet::read_only())];
        let input = sample(&status, &settings, &local, &global);
        assert!(!painted(&input).iter().any(|text| text == "Remote"));

        let mut remote = client("r", "Laptop", PermissionSet::read_only());
        remote.remote_allowed = true;
        let clients = [remote, local[0].clone()];
        let input = sample(&status, &settings, &clients, &global);
        let marks = painted(&input)
            .iter()
            .filter(|text| *text == "Remote")
            .count();
        assert_eq!(marks, 1, "only the remote credential carries the mark");
    }

    #[test]
    fn a_row_says_which_limit_holds_a_granted_permission_back() {
        let status = McpStatus::Disabled;
        let settings = McpSettings::default();
        // Granted: read and create. The maximum allows both; the remote ceiling reads only.
        let everything = PermissionSet::everything();
        let mut remote = client(
            "r",
            "Laptop",
            PermissionSet::of([Permission::ChartsRead, Permission::ChartsCreate]),
        );
        remote.remote_allowed = true;
        let mut local = remote.clone();
        local.id = "l".into();
        local.name = "Local".into();
        local.remote_allowed = false;

        let texts = painted(&sample(&status, &settings, &[remote.clone()], &everything));
        assert!(
            texts
                .iter()
                .any(|t| t == "1 granted permission held back by the remote ceiling")
        );
        assert!(!texts.iter().any(|t| t.contains("held back by the maximum")));
        let texts = painted(&sample(&status, &settings, &[local], &everything));
        assert!(
            !texts.iter().any(|t| t.contains("held back")),
            "a local credential has no ceiling"
        );

        // Outside the maximum is the maximum's doing, not the ceiling's.
        let capped = PermissionSet::of([Permission::ChartsRead]);
        let texts = painted(&sample(&status, &settings, &[remote], &capped));
        assert!(
            texts
                .iter()
                .any(|t| t == "1 granted permission held back by the maximum")
        );
        assert!(!texts.iter().any(|t| t.contains("remote ceiling")));
    }

    #[test]
    fn the_page_tells_the_truth_about_where_it_listens() {
        let status = McpStatus::Disabled;
        let global = PermissionSet::read_only();
        let local = McpSettings::default();
        let texts = painted(&sample(&status, &local, &[], &global));
        assert!(texts.iter().any(|t| t == "127.0.0.1"));
        assert!(texts.iter().any(|t| t == "Remote connections"));
        assert!(
            !texts.iter().any(|t| t == "Interface"),
            "nothing to set while it is off"
        );
        assert!(texts.iter().any(|t| t == "http://127.0.0.1:8765/mcp"));
        assert!(
            !texts.iter().any(|t| t == "Certificate"),
            "no certificate on loopback"
        );

        let remote = McpSettings {
            bind: BindMode::interface("192.168.1.20".parse().unwrap()).unwrap(),
            ..McpSettings::default()
        };
        let mut input = sample(&status, &remote, &[], &global);
        input.tls_fingerprint = Some("AB:CD");
        let texts = painted(&input);
        assert!(texts.iter().any(|t| t == "192.168.1.20"));
        assert!(texts.iter().any(|t| t == "Interface"));
        assert!(texts.iter().any(|t| t == "https://192.168.1.20:8765/mcp"));
        assert!(texts.iter().any(|t| t == "Certificate"));
        assert!(
            texts.iter().any(|t| t == "AB:CD"),
            "the fingerprint is shown to be pinned"
        );
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
    fn the_edit_button_stays_on_the_title_line_however_much_the_maximum_holds() {
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
                let what = format!("{} at {width}pt", access_overview(set).headline);
                assert!(
                    (row.edit.center().y - row.title.center().y).abs() < 1.0,
                    "Edit is not level with the title, {what}: {:?} vs {:?}",
                    row.edit,
                    row.title
                );
                assert!(
                    row.summary.top() >= row.edit.bottom() - 0.5,
                    "the summary starts above the button's line, {what}"
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
                    row.summary.right() <= right + 0.5,
                    "the summary overflows the row, {what}: {} of {right}",
                    row.summary.right()
                );
                assert!(
                    !row.summary.intersects(row.edit),
                    "the summary covers Edit, {what}"
                );
                button_tops.push(row.edit.top());
            }
            assert!(
                button_tops
                    .windows(2)
                    .all(|pair| (pair[0] - pair[1]).abs() < 0.5),
                "the button moves with the size of the maximum at {width}pt: {button_tops:?}"
            );
        }
    }

    #[test]
    fn a_long_summary_wraps_under_its_capsule_and_never_runs_past_its_line() {
        let status = McpStatus::Disabled;
        let settings = McpSettings::default();
        let summary = |set: &PermissionSet, width: f32| {
            let input = sample(&status, &settings, &[], set);
            let mut state = McpSettingsState::default();
            laid_out(width, |ui| maximum_row(ui, &mut state, &input))
                .0
                .summary
        };
        let mixed = PermissionSet::of([
            Permission::ChartsRead,
            Permission::ChartsCreate,
            Permission::AlertsDelete,
            Permission::MarketsRead,
        ]);
        for width in [240.0, 300.0, 560.0] {
            let rect = summary(&mixed, width);
            assert!(rect.width() <= width + 1.0, "{rect:?} at {width}pt");
            assert!(rect.height() < 80.0, "a summary is a line or two: {rect:?}");
        }
        assert!(
            summary(&mixed, 240.0).height() >= summary(&mixed, 560.0).height(),
            "a narrow line wraps, a wide one does not need to"
        );
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
