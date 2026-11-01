//! The "Add Webhook" / "Edit Webhook" sheet: a name, where to post, an optional secret
//! and headers, and whether it is on. A pure view: it edits a `WebhookEditorState` the
//! host owns and reports `Save` or `Cancel`. All validation lives in `WebhookEditorState`
//! (and in `webhook_policy`), so the sheet cannot disagree with the store.

// Transient: the Settings window hosts this once the webhook store is wired.
#![allow(dead_code)]

use crate::model::webhook::{EndpointId, WebhookEndpoint, WebhookHeader, WebhookHttpMethod};
use crate::model::webhook_policy::{
    self, MAXIMUM_HEADERS, SECRET_TOKEN, SecretPlacement, WebhookHeaderError, WebhookSecretError,
};
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, BOLD, REGULAR, SEMIBOLD, size};
use crate::ui::sheet;
use crate::ui::theme;
use eframe::egui::{self, Color32, FontId, Id, Pos2, Rect, RichText, Sense, Stroke, Vec2};

/// What the sheet hands back when the user saves: already trimmed and stripped of blank
/// header rows, ready for the store.
#[derive(Clone, Debug, PartialEq)]
pub struct WebhookEditorResult {
    /// `None` creates a new endpoint.
    pub id: Option<EndpointId>,
    pub name: String,
    pub description: String,
    pub url: String,
    pub method: WebhookHttpMethod,
    pub headers: Vec<WebhookHeader>,
    /// Empty means "no secret"; the store removes any saved one.
    pub secret: String,
    pub is_enabled: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum WebhookEditorOutcome {
    Save(WebhookEditorResult),
    Cancel,
}

/// The one-line status under the secret field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SecretStatus {
    Problem(WebhookSecretError),
    /// `{{secret}}` is used but no value is entered.
    MissingValue,
    Used(String),
    /// A value is entered but nothing uses it.
    NotUsedYet,
    Idle,
}

#[derive(Clone, Debug)]
pub struct WebhookEditorState {
    existing_id: Option<EndpointId>,
    pub name: String,
    pub description: String,
    pub url: String,
    pub method: WebhookHttpMethod,
    pub secret: String,
    pub headers: Vec<WebhookHeader>,
    pub headers_expanded: bool,
    pub is_enabled: bool,
    /// Set by the host when saving failed, shown under the form.
    pub failure: Option<String>,
    secret_revealed: bool,
    /// A half-typed address is not an error yet: the message waits for the field to lose focus.
    url_has_focus: bool,
    focus_name: bool,
    /// A header row whose name should take the cursor next frame.
    focus_header: Option<u64>,
}

impl WebhookEditorState {
    pub fn new_blank() -> Self {
        Self {
            existing_id: None,
            name: String::new(),
            description: String::new(),
            url: String::new(),
            method: WebhookHttpMethod::Post,
            secret: String::new(),
            headers: Vec::new(),
            headers_expanded: false,
            is_enabled: true,
            failure: None,
            secret_revealed: false,
            url_has_focus: false,
            focus_name: true,
            focus_header: None,
        }
    }

    /// `secret` is the value currently in secure storage (the Keychain on macOS) for this endpoint, if any.
    pub fn editing(endpoint: &WebhookEndpoint, secret: Option<String>) -> Self {
        Self {
            existing_id: Some(endpoint.id),
            name: endpoint.name.clone(),
            description: endpoint.description.clone(),
            url: endpoint.url.clone(),
            method: endpoint.method,
            secret: secret.unwrap_or_default(),
            headers: endpoint.headers.clone(),
            headers_expanded: !endpoint.headers.is_empty(),
            is_enabled: endpoint.is_enabled,
            failure: None,
            secret_revealed: false,
            url_has_focus: false,
            focus_name: false,
            focus_header: None,
        }
    }

    pub fn is_new(&self) -> bool {
        self.existing_id.is_none()
    }

    // MARK: Validation

    pub fn trimmed_url(&self) -> &str {
        self.url.trim()
    }

    pub fn filled_headers(&self) -> Vec<WebhookHeader> {
        self.headers
            .iter()
            .filter(|header| !header.is_blank())
            .cloned()
            .collect()
    }

    /// Why the address is unacceptable, in the editor's wording; `None` when empty or fine.
    pub fn url_problem(&self) -> Option<&'static str> {
        let url = self.trimmed_url();
        if url.is_empty() {
            return None;
        }
        webhook_policy::validate_templates(url, &[]).map(webhook_policy::url_message)
    }

    pub fn url_is_valid(&self) -> bool {
        !self.trimmed_url().is_empty() && self.url_problem().is_none()
    }

    fn visible_url_problem(&self) -> Option<&'static str> {
        if self.url_has_focus {
            None
        } else {
            self.url_problem()
        }
    }

    pub fn uses_secret(&self) -> bool {
        webhook_policy::references_secret(self.trimmed_url(), &self.filled_headers())
    }

    pub fn secret_problem(&self) -> Option<WebhookSecretError> {
        if self.secret.is_empty() {
            None
        } else {
            webhook_policy::validate_secret(&self.secret)
        }
    }

    pub fn secret_is_valid(&self) -> bool {
        self.secret_problem().is_none() && (!self.uses_secret() || !self.secret.is_empty())
    }

    pub fn name_is_valid(&self) -> bool {
        !self.name.trim().is_empty()
    }

    pub fn can_save(&self) -> bool {
        self.name_is_valid()
            && self.url_is_valid()
            && webhook_policy::headers_are_valid(&self.filled_headers())
            && self.secret_is_valid()
    }

    /// The first problem with each filled header row, by id.
    pub fn header_errors(&self) -> Vec<(u64, WebhookHeaderError)> {
        webhook_policy::header_errors(&self.filled_headers())
    }

    pub fn secret_status(&self) -> SecretStatus {
        if let Some(problem) = self.secret_problem() {
            return SecretStatus::Problem(problem);
        }
        let url_uses = webhook_policy::contains_secret(&self.url);
        let header_uses = self
            .headers
            .iter()
            .filter(|header| webhook_policy::contains_secret(&header.value))
            .count();
        if url_uses || header_uses > 0 {
            if self.secret.is_empty() {
                return SecretStatus::MissingValue;
            }
            let mut places = Vec::new();
            if url_uses {
                places.push("the URL".to_owned());
            }
            if header_uses > 0 {
                places.push(if header_uses == 1 {
                    "1 header".to_owned()
                } else {
                    format!("{header_uses} headers")
                });
            }
            return SecretStatus::Used(format!("Used in {}", places.join(" and ")));
        }
        if self.secret.is_empty() {
            SecretStatus::Idle
        } else {
            SecretStatus::NotUsedYet
        }
    }

    /// A header that looks like a credential, typed in plain while no secret is saved.
    pub fn plain_credential(&self) -> Option<WebhookHeader> {
        if !self.secret.is_empty() {
            return None;
        }
        let filled = self.filled_headers();
        webhook_policy::plain_credential_header(&filled).cloned()
    }

    // MARK: Edits

    pub fn move_to_secret(&mut self, header_id: u64) {
        let Some(header) = self
            .headers
            .iter_mut()
            .find(|header| header.id == header_id)
        else {
            return;
        };
        let (secret, value) = webhook_policy::moving_to_secret(header);
        header.value = value;
        self.secret = secret;
    }

    /// The "Use secret…" shortcuts.
    pub fn apply_placement(&mut self, choice: &SecretPlacement) {
        let edit = webhook_policy::apply_placement(choice, &self.url, &self.headers);
        let had = self.headers.len();
        self.url = edit.url;
        self.headers = edit.headers;
        // A row the menu added with no name yet gets the cursor.
        if let Some(added) = self
            .headers
            .last()
            .filter(|added| self.headers.len() > had && added.name.is_empty())
        {
            self.focus_header = Some(added.id);
        }
        if edit.touches_headers {
            self.headers_expanded = true;
        }
    }

    pub fn add_header(&mut self) {
        if self.headers.len() < MAXIMUM_HEADERS {
            let header = WebhookHeader::new("", "");
            self.focus_header = Some(header.id);
            self.headers.push(header);
            self.headers_expanded = true;
        }
    }

    pub fn remove_header(&mut self, header_id: u64) {
        self.headers.retain(|header| header.id != header_id);
    }

    /// The saved form of this sheet, or `None` while it cannot be saved.
    pub fn result(&self) -> Option<WebhookEditorResult> {
        self.can_save().then(|| WebhookEditorResult {
            id: self.existing_id,
            name: self.name.trim().to_owned(),
            description: self.description.trim().to_owned(),
            url: self.trimmed_url().to_owned(),
            method: self.method,
            headers: self.filled_headers(),
            secret: self.secret.clone(),
            is_enabled: self.is_enabled,
        })
    }
}

// MARK: View

const FORM_WIDTH: f32 = 480.0;
const MAXIMUM_FORM_HEIGHT: f32 = 470.0;
/// The shortest the form scrolls in, however small the window.
const MINIMUM_FORM_HEIGHT: f32 = 160.0;
/// Everything in the sheet but the form (title, gaps, footer), plus a margin to the window edge.
const SHEET_CHROME: f32 = 163.0 + 2.0 * 20.0;
/// The space between the form's sections.
const SECTION_GAP: f32 = 18.0;
/// A field's leading glyph, and the gap after it.
const FIELD_ICON: f32 = 13.0;

pub fn show(ctx: &egui::Context, state: &mut WebhookEditorState) -> Option<WebhookEditorOutcome> {
    let mut outcome = None;
    let title = if state.is_new() {
        "Add Webhook"
    } else {
        "Edit Webhook"
    };
    sheet::show(ctx, Id::new("webhook-editor"), FORM_WIDTH, |ui| {
        ui.add_space(24.0);
        sheet::inset(ui, |ui| {
            sheet::header(
                ui,
                Icon::ArrowUpRightSquare,
                title,
                "Alerts post their message here over HTTP.",
                theme::ACCENT,
            )
        });
        ui.add_space(SECTION_GAP);
        // The form scrolls rather than push the sheet past the window it sits in.
        let form_height = (ctx.content_rect().height() - SHEET_CHROME)
            .clamp(MINIMUM_FORM_HEIGHT, MAXIMUM_FORM_HEIGHT);
        egui::ScrollArea::vertical()
            .max_height(form_height)
            .auto_shrink([false, true])
            .show(ui, |ui| sheet::inset(ui, |ui| form(ui, state)));
        ui.add_space(SECTION_GAP);
        ui.separator();
        sheet::inset(ui, |ui| {
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    let label = if state.is_new() {
                        "Add Webhook"
                    } else {
                        "Save Changes"
                    };
                    if kit::sheet_button(ui, label, true, state.can_save()).clicked() {
                        outcome = state.result().map(WebhookEditorOutcome::Save);
                    }
                    if kit::sheet_button(ui, "Cancel", false, true).clicked() {
                        outcome = Some(WebhookEditorOutcome::Cancel);
                    }
                });
            });
            ui.add_space(14.0);
        });
    });
    // A menu that is open takes Return and Escape for itself.
    if !egui::Popup::is_any_open(ctx) {
        if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            outcome = Some(WebhookEditorOutcome::Cancel);
        } else if outcome.is_none() && ctx.input(|input| input.key_pressed(egui::Key::Enter)) {
            outcome = state.result().map(WebhookEditorOutcome::Save);
        }
    }
    outcome
}

fn form(ui: &mut egui::Ui, state: &mut WebhookEditorState) {
    ui.spacing_mut().item_spacing.y = 0.0;

    section_title(ui, "Name");
    ui.add_space(8.0);
    let name_id = Id::new("webhook-name");
    field(ui, name_id, false, |ui| {
        icons::inline(ui, Icon::Tag, FIELD_ICON, theme::text_secondary());
        let width = ui.available_width();
        let response = input(ui, name_id, &mut state.name, "Trading Bot", false, width);
        if state.focus_name {
            response.request_focus();
            state.focus_name = false;
        }
    });
    ui.add_space(SECTION_GAP);

    url_section(ui, state);
    ui.add_space(SECTION_GAP);
    secret_section(ui, state);

    if let Some(credential) = state.plain_credential() {
        ui.add_space(10.0);
        let title = format!("“{}” looks like a credential", credential.name);
        if kit::notice_card(
            ui,
            Icon::Key,
            kit::ORANGE,
            &title,
            Some(&format!(
                "Save it as the secret so it is kept {} instead of in plain text.",
                crate::platform::in_secret_store()
            )),
            Some("Move to Secret"),
        ) {
            state.move_to_secret(credential.id);
        }
    }
    ui.add_space(SECTION_GAP);

    headers_section(ui, state);
    ui.add_space(SECTION_GAP);

    section_title(ui, "Description");
    ui.add_space(8.0);
    let description_id = Id::new("webhook-description");
    field(ui, description_id, false, |ui| {
        icons::inline(ui, Icon::List, FIELD_ICON, theme::text_secondary());
        let width = ui.available_width();
        input(
            ui,
            description_id,
            &mut state.description,
            "Optional",
            false,
            width,
        );
    });
    ui.add_space(SECTION_GAP);

    enabled_card(ui, &mut state.is_enabled);

    if let Some(failure) = &state.failure {
        ui.add_space(SECTION_GAP);
        kit::notice_card(
            ui,
            Icon::TriangleFill,
            kit::RED,
            "This webhook couldn't be saved",
            Some(failure),
            None,
        );
    }
}

// MARK: Pieces

fn section_title(ui: &mut egui::Ui, text: &str) {
    ui.label(kit::text(
        text,
        size::SUBHEADLINE + 1.0,
        SEMIBOLD,
        theme::text_primary(),
    ));
}

/// A section title with its "Optional" tag, and `trailing` pushed to the right edge.
fn optional_title(ui: &mut egui::Ui, text: &str, trailing: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        section_title(ui, text);
        ui.label(kit::text(
            "Optional",
            size::CAPTION + 0.5,
            REGULAR,
            theme::text_tertiary(),
        ));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), trailing);
    });
}

/// The rounded well every input sits in. The ring turns accent while the input with `id` has
/// focus (the field's own, or one of its children's) and red when `invalid`.
fn field_with(
    ui: &mut egui::Ui,
    focus_ids: &[Id],
    invalid: bool,
    contents: impl FnOnce(&mut egui::Ui),
) {
    let focused = focus_ids
        .iter()
        .any(|id| ui.memory(|memory| memory.has_focus(*id)));
    let mut frame = kit::field_frame(focused);
    if invalid {
        frame = frame.stroke(Stroke::new(1.0, theme::with_alpha(kit::RED, 0.7)));
    }
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            contents(ui);
        });
    });
}

fn field(ui: &mut egui::Ui, id: Id, invalid: bool, contents: impl FnOnce(&mut egui::Ui)) {
    field_with(ui, &[id], invalid, contents);
}

/// A bare single-line input for use inside a [`field`]; `mono` is for addresses and tokens.
fn input(
    ui: &mut egui::Ui,
    id: Id,
    value: &mut String,
    hint: &str,
    mono: bool,
    width: f32,
) -> egui::Response {
    input_with(ui, id, value, hint, mono, false, width)
}

fn input_with(
    ui: &mut egui::Ui,
    id: Id,
    value: &mut String,
    hint: &str,
    mono: bool,
    masked: bool,
    width: f32,
) -> egui::Response {
    let font = if mono {
        FontId::monospace(size::BODY - 0.5)
    } else {
        FontId::proportional(size::BODY)
    };
    ui.add(
        egui::TextEdit::singleline(value)
            .id(id)
            .frame(egui::Frame::NONE)
            .font(font.clone())
            .text_color(theme::text_primary())
            .hint_text(RichText::new(hint).font(font).color(theme::text_tertiary()))
            .password(masked)
            .desired_width(width.max(40.0)),
    )
}

/// A one-line message under a field: a glyph in `color`, then the text, which wraps.
fn status_line(ui: &mut egui::Ui, icon: Icon, color: Color32, message: &str) {
    ui.add_space(7.0);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.vertical(|ui| {
            ui.add_space(1.5);
            icons::inline(ui, icon, 11.0, color);
        });
        ui.add(egui::Label::new(kit::text(message, size::CAPTION + 0.5, REGULAR, color)).wrap());
    });
}

/// A small accent action, like a borderless button with a glyph.
fn link_button(ui: &mut egui::Ui, icon: Icon, label: &str, enabled: bool) -> egui::Response {
    let color = if enabled {
        kit::ACCENT
    } else {
        theme::text_tertiary()
    };
    let text = kit::galley(
        ui,
        label,
        size::CAPTION + 1.0,
        SEMIBOLD,
        color,
        f32::INFINITY,
    );
    let size = Vec2::new(text.size().x + 17.0, 18.0);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    let response = crate::ui::accessibility::label(response, label);
    let color = if response.is_pointer_button_down_on() {
        theme::with_alpha(color, 0.7)
    } else {
        color
    };
    icons::paint(
        ui.painter(),
        Rect::from_center_size(
            Pos2::new(rect.left() + 5.5, rect.center().y),
            Vec2::splat(11.0),
        ),
        icon,
        color,
    );
    ui.painter().galley(
        Pos2::new(rect.left() + 17.0, rect.center().y - text.size().y / 2.0),
        text,
        color,
    );
    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

// MARK: Address

fn url_section(ui: &mut egui::Ui, state: &mut WebhookEditorState) {
    section_title(ui, "Webhook URL");
    ui.add_space(8.0);
    let url_id = Id::new("webhook-url");
    let invalid = state.visible_url_problem().is_some();
    field(ui, url_id, invalid, |ui| {
        method_menu(ui, &mut state.method);
        let (divider, _) = ui.allocate_exact_size(Vec2::new(1.0, 16.0), Sense::hover());
        ui.painter().rect_filled(divider, 0.0, kit::hairline());
        let width = ui.available_width() - 14.0 - 8.0;
        let response = input(
            ui,
            url_id,
            &mut state.url,
            "https://example.com/webhook",
            true,
            width,
        );
        state.url_has_focus = response.has_focus();
        let (mark, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
        if state.url_is_valid() {
            icons::paint(ui.painter(), mark, Icon::CheckCircleFill, kit::GREEN);
        }
    });
    if let Some(problem) = state.visible_url_problem() {
        status_line(ui, Icon::TriangleFill, kit::RED, problem);
    } else if state.url_is_valid() && webhook_policy::contains_secret(state.trimmed_url()) {
        // What will actually be requested, with the secret hidden.
        ui.add_space(7.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            icons::inline(ui, Icon::ArrowDownRight, 11.0, theme::text_tertiary());
            ui.add(
                egui::Label::new(
                    RichText::new(webhook_policy::masked_url(state.trimmed_url()))
                        .font(FontId::monospace(size::CAPTION + 0.5))
                        .color(theme::text_secondary()),
                )
                .truncate(),
            )
            .on_hover_text("What will be requested, with the secret hidden");
        });
    }
}

/// The HTTP method as a compact dropdown at the front of the address field.
fn method_menu(ui: &mut egui::Ui, method: &mut WebhookHttpMethod) {
    let label = ui.painter().layout_no_wrap(
        method.as_str().to_owned(),
        FontId::monospace(size::BODY - 1.0),
        kit::ACCENT,
    );
    let size = Vec2::new(label.size().x + 5.0 + 9.0, 18.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let response =
        crate::ui::accessibility::label(response, format!("HTTP method, {}", method.as_str()))
            .on_hover_cursor(egui::CursorIcon::PointingHand);
    let color = if response.is_pointer_button_down_on() {
        theme::with_alpha(kit::ACCENT, 0.7)
    } else {
        kit::ACCENT
    };
    ui.painter().galley(
        Pos2::new(rect.left(), rect.center().y - label.size().y / 2.0),
        label.clone(),
        color,
    );
    icons::paint(
        ui.painter(),
        Rect::from_center_size(
            Pos2::new(rect.right() - 4.0, rect.center().y),
            Vec2::splat(9.0),
        ),
        Icon::ChevronUpDown,
        color,
    );
    egui::Popup::menu(&response)
        .frame(kit::menu_frame())
        .gap(8.0)
        .show(|ui| {
            ui.set_width(104.0);
            ui.spacing_mut().item_spacing.y = 0.0;
            for choice in WebhookHttpMethod::ALL {
                if kit::menu_item_marked(ui, Icon::Check, choice.as_str(), choice == *method) {
                    *method = choice;
                    ui.close();
                }
            }
        });
    response.on_hover_text("HTTP method");
}

// MARK: Secret

fn secret_section(ui: &mut egui::Ui, state: &mut WebhookEditorState) {
    optional_title(ui, "Secret", |ui| {
        let button = link_button(ui, Icon::Plus, "Use secret", true)
            .on_hover_text("Put the secret in the URL or in a header");
        egui::Popup::menu(&button)
            .frame(kit::menu_frame())
            .align(egui::RectAlign::BOTTOM_END)
            .gap(4.0)
            .show(|ui| {
                ui.set_width(236.0);
                ui.spacing_mut().item_spacing.y = 0.0;
                let choices = [
                    (
                        Icon::Link,
                        "Add to the URL as ?token=",
                        SecretPlacement::UrlParameter("token".to_owned()),
                    ),
                    (
                        Icon::Key,
                        "Send as a Bearer token",
                        SecretPlacement::BearerToken,
                    ),
                    (
                        Icon::Key,
                        "Send as an X-API-Key header",
                        SecretPlacement::ApiKey,
                    ),
                    (
                        Icon::List,
                        "Send in a custom header…",
                        SecretPlacement::CustomHeader,
                    ),
                ];
                for (index, (icon, label, choice)) in choices.into_iter().enumerate() {
                    if index == 1 {
                        kit::menu_separator(ui);
                    }
                    if kit::menu_item(ui, icon, label, true, false) {
                        state.apply_placement(&choice);
                        ui.close();
                    }
                }
            });
    });
    ui.add_space(8.0);

    let status = state.secret_status();
    let secret_id = Id::new("webhook-secret");
    let invalid = matches!(status, SecretStatus::Problem(_));
    field(ui, secret_id, invalid, |ui| {
        icons::inline(ui, Icon::LockFill, 12.0, theme::text_secondary());
        let width = ui.available_width() - 22.0 - 8.0;
        input_with(
            ui,
            secret_id,
            &mut state.secret,
            "API token or key",
            state.secret_revealed,
            !state.secret_revealed,
            width,
        );
        let (slot, _) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
        let (icon, tip) = if state.secret_revealed {
            (Icon::EyeSlash, "Hide")
        } else {
            (Icon::Eye, "Show")
        };
        if kit::hover_button(ui, slot, secret_id.with("eye"), icon, tip, true, true).clicked() {
            state.secret_revealed = !state.secret_revealed;
        }
    });

    match status {
        SecretStatus::Problem(problem) => status_line(
            ui,
            Icon::TriangleFill,
            kit::RED,
            &webhook_policy::secret_message(problem),
        ),
        SecretStatus::MissingValue => status_line(
            ui,
            Icon::TriangleFill,
            kit::RED,
            &format!("Add the secret value that {SECRET_TOKEN} stands for."),
        ),
        SecretStatus::Used(places) => status_line(ui, Icon::CheckCircleFill, kit::GREEN, &places),
        SecretStatus::NotUsedYet => status_line(
            ui,
            Icon::InfoCircle,
            kit::ORANGE,
            "Not used yet. Choose where it goes with Use secret.",
        ),
        SecretStatus::Idle => {
            ui.add_space(7.0);
            ui.add(
                egui::Label::new(kit::text(
                    format!(
                        "Kept {} and inserted wherever you place it, in the URL or a header.",
                        crate::platform::in_secret_store()
                    ),
                    size::CAPTION + 0.5,
                    REGULAR,
                    theme::text_secondary(),
                ))
                .wrap(),
            );
        }
    }
}

// MARK: Headers

fn headers_section(ui: &mut egui::Ui, state: &mut WebhookEditorState) {
    let errors = state.header_errors();
    let expanded = state.headers_expanded;

    let toggle = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            icons::inline(
                ui,
                if expanded {
                    Icon::ChevronDown
                } else {
                    Icon::ChevronRight
                },
                10.0,
                theme::text_secondary(),
            );
            section_title(ui, "Headers");
            if !state.headers.is_empty() {
                egui::Frame::new()
                    .fill(kit::primary(0.08))
                    .corner_radius(9.0)
                    .inner_margin(egui::Margin::symmetric(6, 1))
                    .show(ui, |ui| {
                        ui.label(kit::text(
                            state.headers.len().to_string(),
                            size::CAPTION,
                            BOLD,
                            theme::text_secondary(),
                        ));
                    });
            }
            if !expanded && !errors.is_empty() {
                icons::inline(ui, Icon::TriangleFill, 11.0, kit::RED);
            }
            ui.label(kit::text(
                "Optional",
                size::CAPTION + 0.5,
                REGULAR,
                theme::text_tertiary(),
            ));
        })
        .response;
    let toggle = ui
        .interact(
            toggle.rect,
            Id::new("webhook-headers-toggle"),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let toggle =
        crate::ui::accessibility::label(toggle, format!("Headers, {} set", state.headers.len()));
    if toggle.clicked() {
        state.headers_expanded = !state.headers_expanded;
    }
    if !state.headers_expanded {
        return;
    }

    ui.add_space(10.0);
    if state.headers.is_empty() {
        ui.label(kit::text(
            "No custom headers yet.",
            size::CAPTION + 0.5,
            REGULAR,
            theme::text_secondary(),
        ));
        ui.add_space(8.0);
    }
    let focus_target = state.focus_header.take();
    let mut remove = None;
    for header in &mut state.headers {
        let error = errors
            .iter()
            .find(|(id, _)| *id == header.id)
            .map(|(_, error)| *error);
        let name_error = error.is_some_and(is_name_error);
        let value_error = error.is_some() && !name_error;
        let name_id = Id::new(("webhook-header-name", header.id));
        let value_id = Id::new(("webhook-header-value", header.id));
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.allocate_ui(Vec2::new(150.0, 0.0), |ui| {
                field(ui, name_id, name_error, |ui| {
                    let width = ui.available_width();
                    let response = input(ui, name_id, &mut header.name, "Header-Name", true, width);
                    if focus_target == Some(header.id) {
                        response.request_focus();
                    }
                });
            });
            let width = ui.available_width() - 22.0 - 8.0;
            ui.allocate_ui(Vec2::new(width.max(60.0), 0.0), |ui| {
                field(ui, value_id, value_error, |ui| {
                    let inner = ui.available_width() - 20.0 - 8.0;
                    input(ui, value_id, &mut header.value, "Value", true, inner);
                    let (slot, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
                    if kit::hover_button(
                        ui,
                        slot,
                        value_id.with("secret"),
                        Icon::Key,
                        "Insert the secret",
                        true,
                        true,
                    )
                    .clicked()
                    {
                        header.value.push_str(SECRET_TOKEN);
                    }
                });
            });
            let (slot, _) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
            if kit::hover_button(
                ui,
                slot,
                name_id.with("remove"),
                Icon::Trash,
                "Remove header",
                true,
                true,
            )
            .clicked()
            {
                remove = Some(header.id);
            }
        });
        if let Some(error) = error {
            status_line(
                ui,
                Icon::TriangleFill,
                kit::RED,
                &webhook_policy::header_message(error),
            );
        }
        ui.add_space(8.0);
    }
    if let Some(id) = remove {
        state.remove_header(id);
    }
    let can_add = state.headers.len() < MAXIMUM_HEADERS;
    let add = link_button(ui, Icon::Plus, "Add header", can_add);
    let add = if can_add {
        add.on_hover_text("Add a header")
    } else {
        add.on_hover_text(format!("At most {MAXIMUM_HEADERS} headers"))
    };
    if can_add && add.clicked() {
        state.add_header();
    }
}

/// Whether `error` is about the header's name rather than its value.
fn is_name_error(error: WebhookHeaderError) -> bool {
    !matches!(
        error,
        WebhookHeaderError::InvalidValue | WebhookHeaderError::ValueTooLong
    )
}

// MARK: Enabled

fn enabled_card(ui: &mut egui::Ui, is_enabled: &mut bool) {
    let tint = if *is_enabled {
        theme::text_primary()
    } else {
        theme::text_secondary()
    };
    egui::Frame::new()
        .fill(theme::quaternary(0.3))
        .corner_radius(12.0)
        .stroke(Stroke::new(1.0, kit::hairline()))
        .inner_margin(egui::Margin::symmetric(14, 11))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                let (badge, _) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::hover());
                ui.painter()
                    .rect_filled(badge, 8.0, theme::with_alpha(tint, 0.12));
                icons::paint(
                    ui.painter(),
                    Rect::from_center_size(badge.center(), Vec2::splat(14.0)),
                    if *is_enabled {
                        Icon::Bolt
                    } else {
                        Icon::BoltSlash
                    },
                    tint,
                );
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.label(kit::text(
                        "Enabled",
                        size::SUBHEADLINE + 1.0,
                        SEMIBOLD,
                        theme::text_primary(),
                    ));
                    ui.label(kit::text(
                        if *is_enabled {
                            "Alerts that use this webhook will post to it."
                        } else {
                            "Paused: alerts skip this webhook."
                        },
                        size::CAPTION + 0.5,
                        REGULAR,
                        theme::text_secondary(),
                    ));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    kit::switch(ui, is_enabled, "Enabled");
                });
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filled() -> WebhookEditorState {
        let mut state = WebhookEditorState::new_blank();
        state.name = "  Trading Bot ".into();
        state.url = " https://example.com/hook ".into();
        state
    }

    #[test]
    fn a_blank_sheet_cannot_be_saved_and_a_name_plus_url_can() {
        let mut state = WebhookEditorState::new_blank();
        assert!(state.is_new());
        assert!(!state.can_save());
        state.name = "Bot".into();
        assert!(!state.can_save(), "no address yet");
        state.url = "https://example.com/hook".into();
        assert!(state.can_save());
        state.name = "   ".into();
        assert!(!state.can_save(), "a blank name is no name");
    }

    #[test]
    fn the_result_is_trimmed_and_drops_blank_header_rows() {
        let mut state = filled();
        state.description = "  my bridge ".into();
        state.headers = vec![
            WebhookHeader::new("X-Trace", "abc"),
            WebhookHeader::new("", ""),
            WebhookHeader::new("  ", " "),
        ];
        let result = state.result().unwrap();
        assert_eq!(result.id, None);
        assert_eq!(result.name, "Trading Bot");
        assert_eq!(result.description, "my bridge");
        assert_eq!(result.url, "https://example.com/hook");
        assert_eq!(result.headers.len(), 1);
        assert!(result.is_enabled);
        assert_eq!(result.method, WebhookHttpMethod::Post);
    }

    #[test]
    fn address_problems_use_the_editors_wording() {
        let mut state = filled();
        state.url = "ftp://example.com".into();
        assert_eq!(
            state.url_problem(),
            Some("Use an http:// or https:// address.")
        );
        assert!(!state.url_is_valid());
        state.url = "https://{{secret}}.example.com".into();
        assert!(state.url_problem().unwrap().contains("host"));
        state.url = "".into();
        assert_eq!(state.url_problem(), None, "empty is not an error yet");
        assert!(!state.url_is_valid());
    }

    #[test]
    fn a_half_typed_address_stays_quiet_until_the_field_loses_focus() {
        let mut state = filled();
        state.url = "htt".into();
        state.url_has_focus = true;
        assert_eq!(state.visible_url_problem(), None);
        state.url_has_focus = false;
        assert!(state.visible_url_problem().is_some());
    }

    #[test]
    fn using_the_secret_requires_a_value_and_a_value_must_be_valid() {
        let mut state = filled();
        state.url = "https://example.com/hook?token={{secret}}".into();
        assert!(state.uses_secret());
        assert_eq!(state.secret_status(), SecretStatus::MissingValue);
        assert!(!state.can_save());
        state.secret = "abc".into();
        assert_eq!(
            state.secret_status(),
            SecretStatus::Used("Used in the URL".into())
        );
        assert!(state.can_save());
        state.secret = "a\nb".into();
        assert_eq!(
            state.secret_status(),
            SecretStatus::Problem(WebhookSecretError::ControlCharacters)
        );
        assert!(!state.can_save());
    }

    #[test]
    fn an_unused_secret_is_allowed_but_flagged() {
        let mut state = filled();
        assert_eq!(state.secret_status(), SecretStatus::Idle);
        state.secret = "abc".into();
        assert_eq!(state.secret_status(), SecretStatus::NotUsedYet);
        assert!(state.can_save());
    }

    #[test]
    fn the_status_names_every_place_the_secret_is_used() {
        let mut state = filled();
        state.secret = "abc".into();
        state.apply_placement(&SecretPlacement::UrlParameter("token".into()));
        state.apply_placement(&SecretPlacement::BearerToken);
        state.apply_placement(&SecretPlacement::ApiKey);
        assert_eq!(
            state.secret_status(),
            SecretStatus::Used("Used in the URL and 2 headers".into())
        );
        assert!(
            state.headers_expanded,
            "a header placement opens the headers"
        );
        assert!(state.url.contains("?token={{secret}}"));
    }

    #[test]
    fn use_secret_shortcuts_do_not_open_headers_for_a_url_choice() {
        let mut state = filled();
        state.apply_placement(&SecretPlacement::UrlParameter("key".into()));
        assert!(!state.headers_expanded);
        assert!(state.headers.is_empty());
    }

    #[test]
    fn header_errors_block_saving() {
        let mut state = filled();
        state.headers = vec![
            WebhookHeader::new("Content-Type", "x"),
            WebhookHeader::new("X-Fine", "y"),
        ];
        assert_eq!(state.header_errors().len(), 1);
        assert!(!state.can_save());
        state.headers.remove(0);
        assert!(state.can_save());
        // A header whose value uses the secret counts as using it.
        state
            .headers
            .push(WebhookHeader::new("X-Key", "{{secret}}"));
        assert!(!state.can_save(), "no secret value yet");
    }

    #[test]
    fn header_rows_are_capped_and_removable() {
        let mut state = filled();
        for _ in 0..15 {
            state.add_header();
        }
        assert_eq!(state.headers.len(), MAXIMUM_HEADERS);
        let id = state.headers[3].id;
        state.remove_header(id);
        assert_eq!(state.headers.len(), MAXIMUM_HEADERS - 1);
        assert!(state.headers.iter().all(|header| header.id != id));
    }

    #[test]
    fn a_plain_credential_can_be_moved_into_the_secret() {
        let mut state = filled();
        state.headers = vec![WebhookHeader::new("Authorization", "Bearer abc123")];
        let credential = state.plain_credential().unwrap();
        assert_eq!(credential.name, "Authorization");
        state.move_to_secret(credential.id);
        assert_eq!(state.secret, "abc123");
        assert_eq!(state.headers[0].value, "Bearer {{secret}}");
        assert!(
            state.plain_credential().is_none(),
            "nothing left to nudge about"
        );
        assert!(state.can_save());
        // Once a secret exists the nudge stays away.
        state.headers.push(WebhookHeader::new("X-Token", "plain"));
        assert!(state.plain_credential().is_none());
    }

    #[test]
    fn editing_starts_from_the_saved_endpoint_and_keeps_its_id() {
        let mut endpoint = WebhookEndpoint::new("Bot", "https://example.com/a?t={{secret}}", 0);
        endpoint
            .headers
            .push(WebhookHeader::new("X-Key", "{{secret}}"));
        endpoint.method = WebhookHttpMethod::Put;
        endpoint.is_enabled = false;
        let state = WebhookEditorState::editing(&endpoint, Some("tok".into()));
        assert!(!state.is_new());
        assert!(
            state.headers_expanded,
            "a webhook that already has headers opens them"
        );
        assert_eq!(state.secret, "tok");
        let result = state.result().unwrap();
        assert_eq!(result.id, Some(endpoint.id));
        assert_eq!(result.method, WebhookHttpMethod::Put);
        assert!(!result.is_enabled);
        // No saved secret: the field starts empty and the sheet will not save until filled.
        let missing = WebhookEditorState::editing(&endpoint, None);
        assert!(!missing.can_save());
    }

    fn draw(state: &mut WebhookEditorState) -> Option<WebhookEditorOutcome> {
        let ctx = egui::Context::default();
        let mut outcome = None;
        // Two frames: the first lays the fields out, the second reads their focus.
        for _ in 0..2 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                outcome = show(ui.ctx(), state);
            });
            output.textures_delta.clear();
        }
        outcome
    }

    #[test]
    fn the_sheet_draws_blank_filled_invalid_and_with_headers() {
        let mut state = WebhookEditorState::new_blank();
        assert_eq!(draw(&mut state), None);
        state.name = "Bot".into();
        state.url = "https://example.com/hook?token={{secret}}".into();
        state.secret = "abc".into();
        state.headers = vec![
            WebhookHeader::new("X-Key", "{{secret}}"),
            WebhookHeader::new("Content-Type", "x"),
        ];
        state.headers_expanded = true;
        state.failure = Some("The Keychain is locked.".into());
        assert_eq!(draw(&mut state), None);
        state.url = "ftp://nope".into();
        state.secret_revealed = true;
        state.is_enabled = false;
        assert_eq!(draw(&mut state), None);
        state.headers_expanded = false;
        assert_eq!(draw(&mut state), None);
    }

    #[test]
    fn a_new_header_row_takes_the_cursor_in_its_name() {
        let mut state = filled();
        state.add_header();
        assert_eq!(state.focus_header, Some(state.headers[0].id));
        draw(&mut state);
        assert_eq!(state.focus_header, None, "the request is spent");

        let mut state = filled();
        state.apply_placement(&SecretPlacement::CustomHeader);
        assert_eq!(
            state.focus_header,
            state.headers.last().map(|header| header.id)
        );
    }
}
