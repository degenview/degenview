//! Settings ▸ Webhooks: the reusable endpoints that price alerts and Pine alerts can post
//! to. A pure view returning `WebhookSettingsAction`s; the host owns the store, sends the
//! test (through the same `deliver` alerts use, in test mode) and feeds the result back.

// Transient: the Settings window hosts this once the webhook store is wired.
#![allow(dead_code)]

use crate::model::webhook::{
    EndpointId, WebhookDeliveryError, WebhookDeliveryResult, WebhookEndpoint,
};
use crate::ui::icons::{self, Icon};
use crate::ui::settings::window::{card, section};
use crate::ui::webhook::editor::{
    self, WebhookEditorOutcome, WebhookEditorResult, WebhookEditorState,
};
use crate::ui::{kit, theme};
use eframe::egui::{self, Rect, Sense, Vec2};
use std::collections::HashMap;

pub const DEFAULT_TEST_PAYLOAD: &str = r#"{"event":"DegenView webhook test"}"#;

/// What the last Test button press found out.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum WebhookTestState {
    #[default]
    NotTested,
    Testing,
    Delivered {
        status: Option<u16>,
        duration_secs: f64,
    },
    Failed {
        error: WebhookDeliveryError,
        status: Option<u16>,
        duration_secs: f64,
    },
    TimedOut {
        duration_secs: f64,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tone {
    Neutral,
    Good,
    Bad,
}

impl WebhookTestState {
    pub fn from_result(result: &WebhookDeliveryResult) -> Self {
        match result.error {
            None => Self::Delivered {
                status: result.status_code,
                duration_secs: result.duration_secs,
            },
            Some(WebhookDeliveryError::Timeout) => Self::TimedOut {
                duration_secs: result.duration_secs,
            },
            Some(error) => Self::Failed {
                error,
                status: result.status_code,
                duration_secs: result.duration_secs,
            },
        }
    }

    pub fn title(&self) -> &'static str {
        match self {
            Self::NotTested => "Not tested",
            Self::Testing => "Testing…",
            Self::Delivered { .. } => "Delivered",
            Self::Failed { .. } => "Failed",
            Self::TimedOut { .. } => "Timed out",
        }
    }

    /// `HTTP 200 · 143 ms`, `3.0 s`, or a short reason. Never a raw system error.
    pub fn detail(&self) -> Option<String> {
        match self {
            Self::NotTested | Self::Testing => None,
            Self::Delivered {
                status,
                duration_secs,
            } => Some(join(
                status.map(|s| format!("HTTP {s}")),
                milliseconds(*duration_secs),
            )),
            Self::TimedOut { duration_secs } => Some(format!("{duration_secs:.1} s")),
            Self::Failed {
                error,
                status,
                duration_secs,
            } => Some(match error {
                WebhookDeliveryError::HttpFailure => join(
                    status.map(|s| format!("HTTP {s}")),
                    milliseconds(*duration_secs),
                ),
                WebhookDeliveryError::NetworkFailure => "Could not connect".to_owned(),
                WebhookDeliveryError::InvalidUrl
                | WebhookDeliveryError::UnsupportedScheme
                | WebhookDeliveryError::UnsupportedPort => "Invalid address".to_owned(),
                WebhookDeliveryError::SecretUnavailable => {
                    format!("Secret not found {}", crate::platform::in_secret_store())
                }
                WebhookDeliveryError::InvalidHeader => "Invalid header".to_owned(),
                WebhookDeliveryError::SecretInHost => "Secret can't be in the host".to_owned(),
                WebhookDeliveryError::MissingEndpoint => "Webhook not found".to_owned(),
                WebhookDeliveryError::DisabledEndpoint => "Webhook is off".to_owned(),
                WebhookDeliveryError::Timeout => format!("{duration_secs:.1} s"),
            }),
        }
    }

    pub fn tone(&self) -> Tone {
        match self {
            Self::NotTested | Self::Testing => Tone::Neutral,
            Self::Delivered { .. } => Tone::Good,
            Self::Failed { .. } | Self::TimedOut { .. } => Tone::Bad,
        }
    }
}

fn milliseconds(duration_secs: f64) -> String {
    format!("{} ms", (duration_secs * 1000.0).round() as i64)
}

fn join(status: Option<String>, timing: String) -> String {
    match status {
        Some(status) => format!("{status} · {timing}"),
        None => timing,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum WebhookSettingsAction {
    /// Send `message` to this endpoint in test mode, then call `set_test_result`.
    Test {
        endpoint: Box<WebhookEndpoint>,
        message: String,
    },
    Toggle(EndpointId),
    Delete(EndpointId),
    /// Create (`id == None`) or update an endpoint; on failure set the sheet's `failure`.
    Save(WebhookEditorResult),
}

pub struct WebhookSettingsInput<'a> {
    pub endpoints: &'a [WebhookEndpoint],
    /// Whether secure storage (the Keychain on macOS) holds a secret for this endpoint.
    pub has_secret: &'a dyn Fn(EndpointId) -> bool,
    /// The secret's value, read only when the editor opens.
    pub secret_for_edit: &'a dyn Fn(EndpointId) -> Option<String>,
    /// How many alerts use this endpoint, for the delete confirmation.
    pub reference_count: &'a dyn Fn(EndpointId) -> usize,
    pub load_failed: bool,
}

pub struct WebhookSettingsState {
    pub test_payload: String,
    editor: Option<WebhookEditorState>,
    pending_delete: Option<EndpointId>,
    test_states: HashMap<EndpointId, WebhookTestState>,
    pub failure: Option<String>,
}

impl Default for WebhookSettingsState {
    fn default() -> Self {
        Self {
            test_payload: DEFAULT_TEST_PAYLOAD.to_owned(),
            editor: None,
            pending_delete: None,
            test_states: HashMap::new(),
            failure: None,
        }
    }
}

impl WebhookSettingsState {
    pub fn set_test_result(&mut self, result: &WebhookDeliveryResult) {
        self.test_states
            .insert(result.endpoint_id, WebhookTestState::from_result(result));
    }

    pub fn test_state(&self, endpoint: EndpointId) -> WebhookTestState {
        self.test_states.get(&endpoint).copied().unwrap_or_default()
    }

    /// A save the store refused: keep the sheet open and say why.
    pub fn save_failed(&mut self, reason: String) {
        if let Some(editor) = &mut self.editor {
            editor.failure = Some(reason.clone());
        }
        self.failure = Some(reason);
    }

    /// A save that went through: close the sheet.
    pub fn save_succeeded(&mut self) {
        self.editor = None;
        self.failure = None;
    }

    pub fn forget(&mut self, endpoint: EndpointId) {
        self.test_states.remove(&endpoint);
    }

    pub fn is_editing(&self) -> bool {
        self.editor.is_some()
    }
}

/// The delete confirmation's wording, with a usage count.
pub fn delete_message(references: usize) -> String {
    match references {
        0 => "This webhook isn’t used by any alert.".to_owned(),
        1 => "This webhook is currently used by 1 alert. Deleting it will stop webhook delivery for that alert."
            .to_owned(),
        count => format!(
            "This webhook is currently used by {count} alerts. Deleting it will stop webhook delivery for those alerts."
        ),
    }
}

impl Tone {
    fn kit(self) -> kit::Tone {
        match self {
            Self::Neutral => kit::Tone::Neutral,
            Self::Good => kit::Tone::Good,
            Self::Bad => kit::Tone::Bad,
        }
    }
}

/// The Webhooks page's scrolling content: the endpoints, the test message and how delivery
/// works, plus the editor sheet and the delete confirmation when they are open.
pub fn body(
    ui: &mut egui::Ui,
    state: &mut WebhookSettingsState,
    input: &WebhookSettingsInput,
) -> Vec<WebhookSettingsAction> {
    let mut actions = Vec::new();

    if input.load_failed {
        kit::notice_card(
            ui,
            Icon::TriangleFill,
            kit::ORANGE,
            "Saved webhooks could not be read",
            Some(
                "Changes are turned off so nothing is overwritten. Restart DegenView to try again.",
            ),
            None,
        );
        ui.add_space(18.0);
    }

    section(
        ui,
        "Endpoints",
        Some(
            "Each alert chooses which of these it posts to. Turning one off pauses it everywhere.",
        ),
        |ui| {
            if input.endpoints.is_empty() {
                card(ui, |ui| {
                    ui.label(kit::text(
                        "No webhooks yet. Add one to start sending alerts to your own server.",
                        kit::size::SUBHEADLINE + 1.0,
                        kit::REGULAR,
                        theme::text_secondary(),
                    ));
                });
            }
            for (index, endpoint) in input.endpoints.iter().enumerate() {
                if index > 0 {
                    ui.add_space(10.0);
                }
                endpoint_row(ui, state, input, endpoint, &mut actions);
            }
            // Under the list it adds to, with a failure beside it.
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                if kit::push_button(ui, Some(Icon::Plus), "Add Webhook", !input.load_failed)
                    .clicked()
                {
                    state.editor = Some(WebhookEditorState::new_blank());
                }
                if let Some(failure) = &state.failure {
                    crate::ui::settings::window::status_message(ui, failure, true);
                }
            });
        },
    );

    section(
        ui,
        "Test message",
        Some(
            "Sent by each Test button. Valid JSON goes out as application/json, anything else as text/plain.",
        ),
        |ui| {
            card(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut state.test_payload)
                        .frame(egui::Frame::NONE)
                        .font(egui::FontId::monospace(kit::size::BODY - 0.5))
                        .text_color(theme::text_primary())
                        .desired_rows(1)
                        .desired_width(f32::INFINITY),
                );
            });
        },
    );

    kit::notice_card(
        ui,
        Icon::InfoCircle,
        theme::text_secondary(),
        "How it works",
        Some(&format!(
            "Alerts send their final message as the request body. A secret is kept {} and added \
             to the URL or a header where you place it. Each alert triggers one attempt per webhook, \
             with a 3 second limit and no retries, so a slow or failing server can never send the \
             same message twice.",
            crate::platform::in_secret_store()
        )),
        None,
    );

    if let Some(editor) = &mut state.editor {
        match editor::show(ui.ctx(), editor) {
            Some(WebhookEditorOutcome::Save(result)) => {
                actions.push(WebhookSettingsAction::Save(result));
            }
            Some(WebhookEditorOutcome::Cancel) => state.editor = None,
            None => {}
        }
    }
    delete_dialog(ui.ctx(), state, input, &mut actions);
    actions
}

/// One endpoint as a card. The words are one column on the left, each line saying one thing: the
/// name, what it is for, where it posts (the whole address), and what the last test found. The
/// two controls, Test and the menu, sit at the right end, centred on the card.
fn endpoint_row(
    ui: &mut egui::Ui,
    state: &mut WebhookSettingsState,
    input: &WebhookSettingsInput,
    endpoint: &WebhookEndpoint,
    actions: &mut Vec<WebhookSettingsAction>,
) {
    let test = state.test_state(endpoint.id);
    ui.scope(|ui| {
        if !endpoint.is_enabled {
            ui.multiply_opacity(0.75);
        }
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                ui.vertical(|ui| {
                    // The Test button, the menu and their gaps keep their room.
                    ui.set_max_width((ui.available_width() - 120.0).max(140.0));
                    ui.spacing_mut().item_spacing.y = 5.0;
                    // Lines sit close: a row is as tall as its text, not a control.
                    ui.spacing_mut().interact_size.y = 16.0;
                    name_line(ui, endpoint);
                    if !endpoint.description.is_empty() {
                        ui.label(kit::text(
                            &endpoint.description,
                            kit::size::CAPTION + 0.5,
                            kit::REGULAR,
                            theme::text_secondary(),
                        ));
                    }
                    address_line(ui, input, endpoint);
                    result_line(ui, test);
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    let (slot, _) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::hover());
                    more_menu(ui, slot, state, input, endpoint, actions);
                    test_button(ui, state, endpoint, test, actions);
                });
            });
        });
    });
}

/// The name, and a quiet "Off" beside it when alerts skip this webhook.
fn name_line(ui: &mut egui::Ui, endpoint: &WebhookEndpoint) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(kit::text(
            &endpoint.name,
            kit::size::SUBHEADLINE + 1.0,
            kit::SEMIBOLD,
            theme::text_primary(),
        ));
        if !endpoint.is_enabled {
            let text = kit::galley(
                ui,
                "Off",
                kit::size::CAPTION,
                kit::SEMIBOLD,
                theme::text_secondary(),
                f32::INFINITY,
            );
            let (pill, response) =
                ui.allocate_exact_size(text.size() + Vec2::new(12.0, 4.0), Sense::hover());
            ui.painter()
                .rect_filled(pill, pill.height() / 2.0, kit::primary(0.08));
            ui.painter().galley(
                pill.center() - text.size() / 2.0,
                text,
                theme::text_secondary(),
            );
            response.on_hover_text("Disabled: alerts skip this webhook");
        }
    });
}

/// The method and the whole address, then the marks of what else the request carries.
fn address_line(ui: &mut egui::Ui, input: &WebhookSettingsInput, endpoint: &WebhookEndpoint) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let method = kit::galley(
            ui,
            endpoint.method.as_str(),
            kit::size::CAPTION,
            kit::BOLD,
            theme::text_secondary(),
            60.0,
        );
        let (pill, _) =
            ui.allocate_exact_size(method.size() + Vec2::new(10.0, 2.0), Sense::hover());
        ui.painter().rect_filled(pill, 4.0, kit::primary(0.08));
        ui.painter().galley(
            pill.center() - method.size() / 2.0,
            method,
            theme::text_secondary(),
        );
        ui.add(
            egui::Label::new(
                egui::RichText::new(&endpoint.url)
                    .font(egui::FontId::monospace(kit::size::CAPTION))
                    .color(theme::text_secondary()),
            )
            .truncate(),
        )
        .on_hover_text(&endpoint.url);
        if (input.has_secret)(endpoint.id) {
            icons::inline(ui, Icon::LockFill, 10.0, theme::text_secondary()).on_hover_text(
                format!("Uses a secret kept {}", crate::platform::in_secret_store()),
            );
        }
        if !endpoint.headers.is_empty() {
            ui.label(kit::text(
                if endpoint.headers.len() == 1 {
                    "1 header".to_owned()
                } else {
                    format!("{} headers", endpoint.headers.len())
                },
                kit::size::CAPTION,
                kit::REGULAR,
                theme::text_tertiary(),
            ));
        }
    });
}

/// What the last test found, in the same column as the rest: a dot in the result's colour, the
/// result, and its detail. An untested webhook says so in a faint grey, not in a pill of its own.
fn result_line(ui: &mut egui::Ui, test: WebhookTestState) {
    let tone = test.tone().kit();
    let neutral = test.tone() == Tone::Neutral;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (dot, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
        ui.painter().circle_filled(
            dot.center(),
            3.5,
            if neutral {
                theme::text_tertiary()
            } else {
                tone.dot()
            },
        );
        ui.label(kit::text(
            test.title(),
            kit::size::CAPTION + 0.5,
            kit::SEMIBOLD,
            if neutral {
                theme::text_tertiary()
            } else {
                tone.text()
            },
        ));
        if let Some(detail) = test.detail() {
            ui.label(kit::text(
                format!("\u{b7}  {detail}"),
                kit::size::CAPTION + 0.5,
                kit::REGULAR,
                theme::text_secondary(),
            ));
        }
    });
}

/// Test, or a spinner while it runs.
fn test_button(
    ui: &mut egui::Ui,
    state: &mut WebhookSettingsState,
    endpoint: &WebhookEndpoint,
    test: WebhookTestState,
    actions: &mut Vec<WebhookSettingsAction>,
) {
    if test == WebhookTestState::Testing {
        ui.add(
            egui::Spinner::new()
                .size(14.0)
                .color(theme::text_secondary()),
        );
    } else if kit::push_button(ui, None, "Test", true)
        .on_hover_text("Send the test payload to this webhook, even if it is disabled")
        .clicked()
    {
        state
            .test_states
            .insert(endpoint.id, WebhookTestState::Testing);
        actions.push(WebhookSettingsAction::Test {
            endpoint: Box::new(endpoint.clone()),
            message: state.test_payload.clone(),
        });
    }
}

/// The ellipsis menu: edit, enable or disable, delete.
fn more_menu(
    ui: &mut egui::Ui,
    slot: Rect,
    state: &mut WebhookSettingsState,
    input: &WebhookSettingsInput,
    endpoint: &WebhookEndpoint,
    actions: &mut Vec<WebhookSettingsAction>,
) {
    let response = kit::hover_button(
        ui,
        slot,
        egui::Id::new(("webhook-more", endpoint.id)),
        Icon::Ellipsis,
        &format!("More actions for {}", endpoint.name),
        true,
        true,
    );
    kit::fitted_menu(
        ui,
        &response,
        egui::RectAlign::BOTTOM_END,
        &["Edit\u{2026}", "Disable", "Delete\u{2026}"],
        |ui| {
            if kit::menu_item(ui, Icon::Pencil, "Edit…", true, false) {
                state.editor = Some(WebhookEditorState::editing(
                    endpoint,
                    (input.secret_for_edit)(endpoint.id),
                ));
                ui.close();
            }
            let (icon, label) = if endpoint.is_enabled {
                (Icon::PauseFill, "Disable")
            } else {
                (Icon::PlayFill, "Enable")
            };
            if kit::menu_item(ui, icon, label, true, false) {
                actions.push(WebhookSettingsAction::Toggle(endpoint.id));
                ui.close();
            }
            kit::menu_separator(ui);
            if kit::menu_item(ui, Icon::Trash, "Delete…", true, true) {
                state.pending_delete = Some(endpoint.id);
                ui.close();
            }
        },
    );
}

fn delete_dialog(
    ctx: &egui::Context,
    state: &mut WebhookSettingsState,
    input: &WebhookSettingsInput,
    actions: &mut Vec<WebhookSettingsAction>,
) {
    let Some(id) = state.pending_delete else {
        return;
    };
    let Some(endpoint) = input.endpoints.iter().find(|endpoint| endpoint.id == id) else {
        state.pending_delete = None;
        return;
    };
    let title = format!("Delete “{}”?", endpoint.name);
    let message = delete_message((input.reference_count)(id));
    match kit::confirm(
        ctx,
        egui::Id::new("webhook-delete"),
        &kit::Confirm {
            icon: Icon::Trash,
            title: &title,
            message: &message,
            destructive: "Delete",
        },
    ) {
        Some(kit::ConfirmOutcome::Confirmed) => {
            actions.push(WebhookSettingsAction::Delete(id));
            state.pending_delete = None;
        }
        Some(kit::ConfirmOutcome::Cancelled) => state.pending_delete = None,
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(
        error: Option<WebhookDeliveryError>,
        status: Option<u16>,
        secs: f64,
    ) -> WebhookDeliveryResult {
        WebhookDeliveryResult {
            endpoint_id: 1,
            timestamp_ms: 0,
            duration_secs: secs,
            status_code: status,
            error,
        }
    }

    #[test]
    fn a_delivered_test_reports_status_and_timing() {
        let state = WebhookTestState::from_result(&result(None, Some(200), 0.1434));
        assert_eq!(state.title(), "Delivered");
        assert_eq!(state.detail().as_deref(), Some("HTTP 200 · 143 ms"));
        assert_eq!(state.tone(), Tone::Good);
        let no_status = WebhookTestState::from_result(&result(None, None, 0.05));
        assert_eq!(no_status.detail().as_deref(), Some("50 ms"));
    }

    #[test]
    fn failures_get_short_human_reasons_never_raw_errors() {
        use WebhookDeliveryError::*;
        let cases = [
            (HttpFailure, Some(500), "HTTP 500 · 20 ms"),
            (HttpFailure, None, "20 ms"),
            (NetworkFailure, None, "Could not connect"),
            (InvalidUrl, None, "Invalid address"),
            (UnsupportedScheme, None, "Invalid address"),
            (UnsupportedPort, None, "Invalid address"),
            (
                SecretUnavailable,
                None,
                &format!("Secret not found {}", crate::platform::in_secret_store()),
            ),
            (InvalidHeader, None, "Invalid header"),
            (SecretInHost, None, "Secret can't be in the host"),
            (MissingEndpoint, None, "Webhook not found"),
            (DisabledEndpoint, None, "Webhook is off"),
        ];
        for (error, status, expected) in cases {
            let state = WebhookTestState::from_result(&result(Some(error), status, 0.02));
            assert_eq!(state.title(), "Failed", "{error:?}");
            assert_eq!(state.detail().as_deref(), Some(expected), "{error:?}");
            assert_eq!(state.tone(), Tone::Bad);
        }
    }

    #[test]
    fn a_timeout_is_its_own_state() {
        let state =
            WebhookTestState::from_result(&result(Some(WebhookDeliveryError::Timeout), None, 3.02));
        assert_eq!(state.title(), "Timed out");
        assert_eq!(state.detail().as_deref(), Some("3.0 s"));
        assert_eq!(state.tone(), Tone::Bad);
    }

    #[test]
    fn untested_and_testing_have_no_detail() {
        for state in [WebhookTestState::NotTested, WebhookTestState::Testing] {
            assert_eq!(state.detail(), None);
            assert_eq!(state.tone(), Tone::Neutral);
        }
        assert_eq!(WebhookTestState::NotTested.title(), "Not tested");
        assert_eq!(WebhookTestState::Testing.title(), "Testing…");
    }

    #[test]
    fn the_state_remembers_results_per_endpoint_and_can_forget_them() {
        let mut state = WebhookSettingsState::default();
        assert_eq!(state.test_state(1), WebhookTestState::NotTested);
        state.set_test_result(&result(None, Some(204), 0.2));
        assert_eq!(state.test_state(1).title(), "Delivered");
        assert_eq!(state.test_state(2), WebhookTestState::NotTested);
        state.forget(1);
        assert_eq!(state.test_state(1), WebhookTestState::NotTested);
    }

    #[test]
    fn the_default_test_message_is_json() {
        assert!(crate::model::webhook::policy::is_json(DEFAULT_TEST_PAYLOAD));
        assert_eq!(
            WebhookSettingsState::default().test_payload,
            DEFAULT_TEST_PAYLOAD
        );
    }

    #[test]
    fn deleting_says_how_many_alerts_lose_their_webhook() {
        assert_eq!(delete_message(0), "This webhook isn’t used by any alert.");
        assert!(
            delete_message(1)
                .contains("1 alert. Deleting it will stop webhook delivery for that alert.")
        );
        let many = delete_message(3);
        assert!(many.contains("3 alerts") && many.contains("those alerts"));
    }

    #[test]
    fn a_failed_save_keeps_the_sheet_open_with_the_reason() {
        let mut state = WebhookSettingsState::default();
        state.save_failed("ignored: nothing open".into());
        assert!(!state.is_editing());
        state.editor = Some(WebhookEditorState::new_blank());
        state.save_failed("Couldn't save".into());
        assert_eq!(
            state.editor.as_ref().and_then(|e| e.failure.as_deref()),
            Some("Couldn't save")
        );
        state.save_succeeded();
        assert!(!state.is_editing());
    }

    fn endpoint(id: EndpointId, name: &str, url: &str, enabled: bool) -> WebhookEndpoint {
        WebhookEndpoint {
            id,
            name: name.to_owned(),
            description: String::new(),
            url: url.to_owned(),
            method: Default::default(),
            headers: Vec::new(),
            is_enabled: enabled,
            created_at_ms: 0,
            updated_at_ms: 0,
        }
    }

    /// The page with endpoints in each state, and the editor sheet, in both themes; with
    /// `DEGENVIEW_GALLERY_DIR` set they are written out to look at.
    #[test]
    fn the_page_and_the_sheet_draw_in_both_themes() {
        use crate::ui::render_tests::{Screen, gallery_dir};
        use crate::ui::theme::AppTheme;
        let _theme = crate::ui::theme::RestoreDefaultTheme;
        let mut first = endpoint(
            1,
            "Trading bot",
            "http://localhost/hooks/alerts/btc?token={{secret}}",
            true,
        );
        first.description = "Posts BTC alerts to my bot".to_owned();
        first.headers.push(crate::model::webhook::WebhookHeader {
            id: 1,
            name: "X-Source".into(),
            value: "degenview".into(),
        });
        first.headers.push(crate::model::webhook::WebhookHeader {
            id: 2,
            name: String::new(),
            value: String::new(),
        });
        let endpoints = vec![
            first,
            endpoint(
                2,
                "Discord",
                "https://discord.com/api/webhooks/123456/abcdef",
                true,
            ),
            endpoint(3, "Staging", "https://staging.example.com/alerts", false),
        ];
        let has_secret = |id: EndpointId| id == 1;
        let secret_for_edit = |_: EndpointId| None;
        let reference_count = |_: EndpointId| 0;
        for (name, app_theme) in [("dark", AppTheme::Dark), ("light", AppTheme::Light)] {
            for sheet in [false, true] {
                let mut screen = Screen::new(Vec2::new(616.0, if sheet { 760.0 } else { 700.0 }));
                crate::ui::theme::apply(&screen.ctx, app_theme);
                let mut state = WebhookSettingsState::default();
                state.set_test_result(&WebhookDeliveryResult {
                    endpoint_id: 1,
                    timestamp_ms: 0,
                    duration_secs: 0.143,
                    status_code: Some(200),
                    error: None,
                });
                state.set_test_result(&WebhookDeliveryResult {
                    endpoint_id: 3,
                    timestamp_ms: 0,
                    duration_secs: 0.02,
                    status_code: None,
                    error: Some(WebhookDeliveryError::NetworkFailure),
                });
                if sheet {
                    state.editor = Some(WebhookEditorState::editing(&endpoints[0], None));
                }
                let mut last = None;
                for _ in 0..3 {
                    last = Some(screen.run(Vec::new(), |ui| {
                        let input = WebhookSettingsInput {
                            endpoints: &endpoints,
                            has_secret: &has_secret,
                            secret_for_edit: &secret_for_edit,
                            reference_count: &reference_count,
                            load_failed: false,
                        };
                        egui::Frame::new()
                            .inner_margin(egui::Margin::same(24))
                            .show(ui, |ui| {
                                body(ui, &mut state, &input);
                            });
                    }));
                }
                if gallery_dir().is_some() {
                    screen.save(
                        &format!("webhooks-{}-{name}", if sheet { "sheet" } else { "page" }),
                        last.expect("a frame"),
                    );
                }
            }
        }
    }
}
