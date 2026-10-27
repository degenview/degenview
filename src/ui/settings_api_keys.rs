//! Settings ▸ Alpaca and Settings ▸ CoinMarketCap: the API keys that unlock US stock data and
//! raise the market-index rate limits. Pages over the shared key stores; saving, removing and the
//! CoinMarketCap connection test happen here, since the stores are process-wide, but every rule
//! (what may be saved, what the status says) is a plain function tested against in-memory stores.

// Transient: reachable once the Settings window is shown.
#![allow(dead_code)]

use crate::service::keychain_api_keys::{
    AlpacaCredentials, AlpacaCredentialsStore, CoinMarketCapKeyStore,
};
use crate::ui::alert_center::Tone;
use crate::ui::icons::{self, Icon};
use crate::ui::settings_window::{card, footer_row, section, status_badge};
use crate::ui::{kit, theme};
use eframe::egui;
use std::sync::mpsc::{Receiver, TryRecvError};

#[derive(Clone, Debug, PartialEq)]
pub struct StatusMessage {
    pub text: String,
    pub is_error: bool,
}

impl StatusMessage {
    pub(crate) fn ok(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            is_error: false,
        }
    }

    pub(crate) fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            is_error: true,
        }
    }
}

#[derive(Default)]
pub struct AlpacaForm {
    pub key_id: String,
    pub secret_key: String,
    /// Whether the saved credentials have been read into the fields yet (reading may prompt).
    loaded: bool,
    pub is_saved: bool,
    pub status: Option<StatusMessage>,
}

#[derive(Default)]
pub struct CmcForm {
    pub api_key: String,
    loaded: bool,
    pub configured: bool,
    pub status: Option<StatusMessage>,
    test: Option<Receiver<Result<(), String>>>,
}

#[derive(Default)]
pub struct ApiKeysState {
    pub alpaca: AlpacaForm,
    pub cmc: CmcForm,
}

// MARK: Alpaca rules

impl AlpacaForm {
    /// Fills the fields from the saved credentials, once.
    pub fn load(&mut self, store: &AlpacaCredentialsStore) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        self.is_saved = store.is_configured();
        if self.is_saved {
            let saved = store.credentials();
            self.key_id = saved.key_id;
            self.secret_key = saved.secret_key;
        }
    }

    pub fn can_save(&self) -> bool {
        !self.key_id.trim().is_empty() && !self.secret_key.trim().is_empty()
    }

    pub fn badge(&self) -> (&'static str, Tone) {
        if self.is_saved {
            ("Keys saved", Tone::Good)
        } else {
            ("Not configured", Tone::Neutral)
        }
    }

    pub fn save(&mut self, store: &AlpacaCredentialsStore) {
        let credentials = AlpacaCredentials {
            key_id: self.key_id.clone(),
            secret_key: self.secret_key.clone(),
        };
        self.status = Some(match store.save(&credentials) {
            Ok(()) => {
                self.is_saved = true;
                StatusMessage::ok(crate::platform::saved_securely())
            }
            Err(error) => StatusMessage::error(format!("Could not save: {}", error.0)),
        });
    }

    pub fn remove(&mut self, store: &AlpacaCredentialsStore) {
        self.status = Some(match store.save(&AlpacaCredentials::default()) {
            Ok(()) => {
                self.key_id.clear();
                self.secret_key.clear();
                self.is_saved = false;
                StatusMessage::ok("Keys removed. Stock charts need keys to load.")
            }
            Err(error) => StatusMessage::error(format!("Could not remove the keys: {}", error.0)),
        });
    }
}

// MARK: CoinMarketCap rules

impl CmcForm {
    pub fn load(&mut self, store: &CoinMarketCapKeyStore) {
        if !self.loaded {
            self.loaded = true;
            self.configured = store.is_configured();
        }
    }

    pub fn can_save(&self) -> bool {
        !self.api_key.trim().is_empty()
    }

    pub fn is_testing(&self) -> bool {
        self.test.is_some()
    }

    pub fn badge(&self) -> (&'static str, Tone) {
        if self.configured {
            ("API key configured", Tone::Good)
        } else {
            ("Public API mode", Tone::Neutral)
        }
    }

    pub fn field_title(&self) -> &'static str {
        if self.configured {
            "Replace API Key"
        } else {
            "API Key"
        }
    }

    pub fn field_hint(&self) -> &'static str {
        if self.configured {
            crate::platform::credential_saved_hint()
        } else {
            crate::platform::credential_stored_hint()
        }
    }

    pub fn save(&mut self, store: &CoinMarketCapKeyStore) {
        self.status = Some(match store.save(&self.api_key) {
            Ok(()) => {
                self.configured = true;
                self.api_key.clear();
                StatusMessage::ok(crate::platform::saved_securely())
            }
            Err(error) => StatusMessage::error(format!("Could not save: {}", error.0)),
        });
    }

    pub fn remove(&mut self, store: &CoinMarketCapKeyStore) {
        self.status = Some(match store.remove() {
            Ok(()) => {
                self.api_key.clear();
                self.configured = false;
                StatusMessage::ok("Key removed. Public API mode is active.")
            }
            Err(error) => StatusMessage::error(format!("Could not remove the key: {}", error.0)),
        });
    }

    /// Runs `test` off the UI thread; `poll_test` picks up the answer.
    pub fn start_test(&mut self, test: impl FnOnce() -> Result<(), String> + Send + 'static) {
        if self.test.is_some() {
            return;
        }
        self.status = None;
        let (sender, receiver) = std::sync::mpsc::channel();
        self.test = Some(receiver);
        std::thread::spawn(move || {
            let _ = sender.send(test());
        });
    }

    /// Takes the connection test's answer when it has arrived.
    pub fn poll_test(&mut self) {
        let Some(receiver) = &self.test else {
            return;
        };
        let outcome = match receiver.try_recv() {
            Ok(outcome) => outcome,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                Err("The connection test stopped unexpectedly.".to_owned())
            }
        };
        self.test = None;
        self.status = Some(match outcome {
            Ok(()) => StatusMessage::ok("Connection successful."),
            Err(message) => StatusMessage::error(message),
        });
    }
}

// MARK: Views

/// A labelled text field in the app's field style. A secret field hides its text and offers an
/// eye button to reveal it, so a pasted key can be checked before saving.
pub(crate) fn field(
    ui: &mut egui::Ui,
    title: &str,
    hint: Option<&str>,
    prompt: &str,
    text: &mut String,
    secret: bool,
) {
    let id = egui::Id::new(("settings-field", title));
    let revealed_id = id.with("revealed");
    let mut revealed: bool = ui.data(|data| data.get_temp(revealed_id)).unwrap_or(false);
    ui.spacing_mut().item_spacing.y = 0.0;
    ui.label(kit::text(
        title,
        kit::size::SUBHEADLINE + 1.0,
        kit::MEDIUM,
        theme::text_primary(),
    ));
    ui.add_space(6.0);
    let focused = ui.memory(|memory| memory.has_focus(id));
    egui::Frame::new()
        .fill(theme::with_alpha(theme::editor_background(), 0.7))
        .corner_radius(8.0)
        .stroke(egui::Stroke::new(
            1.0,
            if focused {
                theme::with_alpha(kit::ACCENT, 0.7)
            } else {
                kit::primary(0.15)
            },
        ))
        .inner_margin(egui::Margin::symmetric(10, 0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_height(36.0);
            ui.horizontal_centered(|ui| {
                let eye = if secret {
                    22.0 + ui.spacing().item_spacing.x
                } else {
                    0.0
                };
                ui.add(
                    egui::TextEdit::singleline(text)
                        .id(id)
                        .frame(egui::Frame::NONE)
                        .font(egui::FontId::proportional(kit::size::BODY))
                        .text_color(theme::text_primary())
                        .hint_text(kit::text(
                            prompt,
                            kit::size::BODY,
                            kit::REGULAR,
                            theme::text_tertiary(),
                        ))
                        .password(secret && !revealed)
                        .desired_width(ui.available_width() - eye),
                );
                if secret {
                    let (icon, tip) = if revealed {
                        (Icon::EyeSlash, format!("Hide {title}"))
                    } else {
                        (Icon::Eye, format!("Show {title}"))
                    };
                    if icons::button(ui, icon, 22.0, theme::text_secondary(), &tip).clicked() {
                        revealed = !revealed;
                        ui.data_mut(|data| data.insert_temp(revealed_id, revealed));
                    }
                }
            });
        });
    if let Some(hint) = hint {
        ui.add_space(6.0);
        ui.label(kit::text(
            hint,
            kit::size::CAPTION + 0.5,
            kit::REGULAR,
            theme::text_secondary(),
        ));
    }
}

/// A numbered instruction line.
fn step(ui: &mut egui::Ui, number: usize, text: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let (badge, _) = ui.allocate_exact_size(egui::Vec2::splat(20.0), egui::Sense::hover());
        ui.painter()
            .circle_filled(badge.center(), 10.0, theme::with_alpha(kit::ACCENT, 0.14));
        let digit = kit::galley(
            ui,
            &number.to_string(),
            kit::size::CAPTION + 0.5,
            kit::BOLD,
            kit::ACCENT,
            20.0,
        );
        ui.painter()
            .galley(badge.center() - digit.size() / 2.0, digit, kit::ACCENT);
        ui.label(kit::text(text, 12.0, kit::REGULAR, theme::text_secondary()));
    });
}

/// A link with a leading "open" glyph, in the accent colour.
pub(crate) fn link(ui: &mut egui::Ui, label: &str, url: &str) {
    let response = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            icons::inline(ui, Icon::ArrowUpRightSquare, 12.0, kit::ACCENT);
            ui.add(
                egui::Label::new(kit::text(label, kit::size::BODY, kit::REGULAR, kit::ACCENT))
                    .sense(egui::Sense::click()),
            )
        })
        .inner
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(url);
    if response.clicked() {
        ui.ctx().open_url(egui::OpenUrl::new_tab(url));
    }
}

/// What a footer says about `status`: an error long enough to need it is cut to its headline (the
/// provider's paragraph is the hover text), anything else is shown as it is.
pub(crate) fn footer_status(status: &Option<StatusMessage>) -> Option<(String, bool)> {
    let status = status.as_ref()?;
    Some((footer_text(&status.text, status.is_error), status.is_error))
}

pub(crate) fn footer_text(text: &str, is_error: bool) -> String {
    if is_error && text.chars().count() > 70 {
        crate::service::llm::summary::summarize(text).headline
    } else {
        text.to_owned()
    }
}

/// A destructive footer button: the regular button with red text.
pub(crate) fn destructive_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let galley = kit::galley(
        ui,
        label,
        kit::size::BODY,
        kit::REGULAR,
        kit::RED,
        f32::INFINITY,
    );
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2((galley.size().x + 32.0).max(76.0), 28.0),
        egui::Sense::click(),
    );
    let response = crate::ui::accessibility::label(response, label);
    ui.painter().rect_filled(
        rect,
        7.0,
        kit::primary(if response.is_pointer_button_down_on() {
            0.18
        } else if response.hovered() {
            0.14
        } else {
            0.1
        }),
    );
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, kit::RED);
    response
}

pub fn alpaca_body(ui: &mut egui::Ui, state: &mut ApiKeysState, store: &AlpacaCredentialsStore) {
    let form = &mut state.alpaca;
    form.load(store);
    let (text, tone) = form.badge();
    status_badge(ui, text, tone);
    ui.add_space(18.0);
    section(ui, "API credentials", None, |ui| {
        card(ui, |ui| {
            ui.vertical(|ui| {
                field(ui, "API Key ID", None, "PK…", &mut form.key_id, false);
                ui.add_space(12.0);
                field(
                    ui,
                    "Secret Key",
                    Some(crate::platform::credential_stored_hint()),
                    "",
                    &mut form.secret_key,
                    true,
                );
            });
        });
    });
    section(ui, "Getting your keys", None, |ui| {
        card(ui, |ui| {
            step(ui, 1, "Create a free Alpaca account.");
            step(
                ui,
                2,
                "Open the API Keys section of the dashboard and generate a key.",
            );
            step(
                ui,
                3,
                "Copy both values here — Alpaca only shows the secret once.",
            );
            link(
                ui,
                "Open the Alpaca dashboard",
                "https://app.alpaca.markets/",
            );
        });
    });
}

pub fn alpaca_footer(ui: &mut egui::Ui, state: &mut ApiKeysState, store: &AlpacaCredentialsStore) {
    let form = &mut state.alpaca;
    footer_row(ui, footer_status(&form.status), |ui| {
        if kit::sheet_button(ui, "Save", true, form.can_save()).clicked() {
            form.save(store);
        }
        if form.is_saved && destructive_button(ui, "Remove Keys").clicked() {
            form.remove(store);
        }
    });
}

pub fn coinmarketcap_body(
    ui: &mut egui::Ui,
    state: &mut ApiKeysState,
    store: &CoinMarketCapKeyStore,
) {
    let form = &mut state.cmc;
    form.load(store);
    form.poll_test();
    if form.is_testing() {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(150));
    }
    let (text, tone) = form.badge();
    status_badge(ui, text, tone);
    ui.add_space(18.0);
    let (title, hint) = (form.field_title(), form.field_hint());
    section(ui, "API access", None, |ui| {
        card(ui, |ui| {
            ui.vertical(|ui| field(ui, title, Some(hint), "", &mut form.api_key, true));
        });
    });
    kit::notice_card(
        ui,
        Icon::InfoCircle,
        kit::ACCENT,
        "A key is optional",
        Some(
            "CoinMarketCap works without an API key. Adding one gives DegenView higher API rate limits.",
        ),
        None,
    );
    ui.add_space(18.0);
    link(
        ui,
        "Get an API key",
        "https://pro.coinmarketcap.com/signup/",
    );
}

pub fn coinmarketcap_footer(
    ui: &mut egui::Ui,
    state: &mut ApiKeysState,
    store: &CoinMarketCapKeyStore,
) {
    let form = &mut state.cmc;
    footer_row(ui, footer_status(&form.status), |ui| {
        if kit::sheet_button(ui, "Save", true, form.can_save()).clicked() {
            form.save(store);
        }
        let testing = form.is_testing();
        if testing {
            ui.add(
                egui::Spinner::new()
                    .size(14.0)
                    .color(theme::text_secondary()),
            );
        } else if kit::sheet_button(ui, "Test Connection", false, true).clicked() {
            form.start_test(crate::service::cmc::test_connection);
        }
        if form.configured && destructive_button(ui, "Remove Key").clicked() {
            form.remove(store);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::keychain_api_keys::MemoryBackend;
    use std::time::{Duration, Instant};

    fn alpaca_store() -> AlpacaCredentialsStore {
        AlpacaCredentialsStore::with_backend(Box::new(MemoryBackend::default()))
    }

    fn cmc_store() -> CoinMarketCapKeyStore {
        CoinMarketCapKeyStore::with_backend(Box::new(MemoryBackend::default()))
    }

    #[test]
    fn alpaca_saves_only_with_both_fields_and_reports_it() {
        let store = alpaca_store();
        let mut form = AlpacaForm::default();
        form.load(&store);
        assert_eq!(form.badge(), ("Not configured", Tone::Neutral));
        assert!(!form.can_save());
        form.key_id = " PK1 ".into();
        assert!(!form.can_save(), "the secret is missing too");
        form.secret_key = "S1".into();
        assert!(form.can_save());
        form.save(&store);
        assert_eq!(
            form.status,
            Some(StatusMessage::ok(crate::platform::saved_securely()))
        );
        assert_eq!(form.badge(), ("Keys saved", Tone::Good));
        assert_eq!(store.credentials().key_id, "PK1", "saved trimmed");
    }

    #[test]
    fn alpaca_loads_saved_keys_into_the_fields_once_and_can_remove_them() {
        let store = alpaca_store();
        store
            .save(&AlpacaCredentials {
                key_id: "PK9".into(),
                secret_key: "S9".into(),
            })
            .unwrap();
        let mut form = AlpacaForm::default();
        form.load(&store);
        assert!(form.is_saved);
        assert_eq!(
            (form.key_id.as_str(), form.secret_key.as_str()),
            ("PK9", "S9")
        );
        form.key_id = "typed".into();
        form.load(&store);
        assert_eq!(
            form.key_id, "typed",
            "a second load does not overwrite edits"
        );
        form.remove(&store);
        assert!(!form.is_saved && form.key_id.is_empty() && form.secret_key.is_empty());
        assert_eq!(
            form.status,
            Some(StatusMessage::ok(
                "Keys removed. Stock charts need keys to load."
            ))
        );
        assert!(!store.is_configured());
    }

    #[test]
    fn coinmarketcap_saves_clears_the_field_and_flips_the_badge() {
        let store = cmc_store();
        let mut form = CmcForm::default();
        form.load(&store);
        assert_eq!(form.badge(), ("Public API mode", Tone::Neutral));
        assert_eq!(form.field_title(), "API Key");
        assert!(!form.can_save());
        form.api_key = "  abc ".into();
        assert!(form.can_save());
        form.save(&store);
        assert!(
            form.api_key.is_empty(),
            "the secret does not linger in the field"
        );
        assert_eq!(form.badge(), ("API key configured", Tone::Good));
        assert_eq!(form.field_title(), "Replace API Key");
        assert_eq!(form.field_hint(), crate::platform::credential_saved_hint());
        assert_eq!(store.api_key().as_deref(), Some("abc"));
        form.remove(&store);
        assert_eq!(
            form.status,
            Some(StatusMessage::ok("Key removed. Public API mode is active."))
        );
        assert!(!form.configured && store.api_key().is_none());
    }

    fn wait_for_test(form: &mut CmcForm) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while form.is_testing() && Instant::now() < deadline {
            form.poll_test();
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn the_connection_test_runs_in_the_background_and_reports_either_way() {
        let mut form = CmcForm {
            status: Some(StatusMessage::error("old")),
            ..CmcForm::default()
        };
        form.start_test(|| Ok(()));
        assert!(form.is_testing());
        assert_eq!(form.status, None, "a new test clears the old message");
        form.start_test(|| Err("ignored: one test at a time".into()));
        wait_for_test(&mut form);
        assert!(!form.is_testing());
        assert_eq!(
            form.status,
            Some(StatusMessage::ok("Connection successful."))
        );

        form.start_test(|| Err("Invalid API key".into()));
        wait_for_test(&mut form);
        assert_eq!(form.status, Some(StatusMessage::error("Invalid API key")));
    }
}
