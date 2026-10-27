//! Settings ▸ AI Assistant: which provider and model the Script Manager's assistant uses, and the
//! API key for it. The key goes to the Keychain through the shared `AiKeyStore`; provider, model
//! and server are the local `ai.prefs` setting, which the host saves when this page asks. Every
//! rule (what may be saved, what the badge says, what a test reports) is a plain method tested
//! against in-memory stores and a scripted provider.

use crate::service::keychain_ai_keys::{self, AiKeyStore};
use crate::service::llm::prefs::AiPrefs;
use crate::service::llm::{HttpProvider, LlmError, LlmProvider, ProviderKind, validate_base_url};
use crate::ui::alert_center::Tone;
use crate::ui::icons::Icon;
use crate::ui::settings_api_keys::{StatusMessage, destructive_button, field, link, status};
use crate::ui::settings_window::{card, section, status_badge};
use crate::ui::{kit, theme};
use eframe::egui;
use std::sync::mpsc::{Receiver, TryRecvError};

/// What the page asks the host to do.
#[derive(Clone, Debug, PartialEq)]
pub enum AiAction {
    SavePrefs(AiPrefs),
}

pub struct AiSettingsInput<'a> {
    pub prefs: &'a AiPrefs,
}

#[derive(Default)]
pub struct AiSettingsState {
    loaded: bool,
    /// The provider, model and server being edited; applied on Save.
    pub draft: AiPrefs,
    pub key_input: String,
    pub status: Option<StatusMessage>,
    test: Option<Receiver<Result<usize, String>>>,
}

impl AiSettingsState {
    /// Fills the draft from the saved prefs, once.
    pub fn load(&mut self, prefs: &AiPrefs) {
        if !self.loaded {
            self.loaded = true;
            self.draft = prefs.clone();
        }
    }

    pub fn kind(&self) -> ProviderKind {
        self.draft.provider
    }

    pub fn select(&mut self, kind: ProviderKind) {
        if self.draft.provider != kind {
            self.draft.provider = kind;
            self.key_input.clear();
            self.status = None;
        }
    }

    pub fn is_testing(&self) -> bool {
        self.test.is_some()
    }

    /// Why Save is refused, if it is.
    pub fn problem(&self) -> Option<String> {
        let kind = self.kind();
        if kind == ProviderKind::Compatible {
            if self.draft.compatible_base_url.trim().is_empty() {
                return Some("Enter the server's base URL.".to_owned());
            }
            if let Err(message) = validate_base_url(&self.draft.compatible_base_url) {
                return Some(message);
            }
            if self.draft.model(kind).is_empty() {
                return Some("Enter the model name your server expects.".to_owned());
            }
        }
        None
    }

    pub fn badge(&self, store: &AiKeyStore) -> (&'static str, Tone) {
        let kind = self.kind();
        if store.is_configured(kind) {
            ("API key saved", Tone::Good)
        } else if std::env::var(kind.env_var()).is_ok_and(|value| !value.trim().is_empty()) {
            ("Key from environment", Tone::Good)
        } else if kind.needs_key() {
            ("No API key", Tone::Neutral)
        } else {
            ("No key needed for a local server", Tone::Neutral)
        }
    }

    pub fn field_title(&self, store: &AiKeyStore) -> &'static str {
        if store.is_configured(self.kind()) {
            "Replace API Key"
        } else {
            "API Key"
        }
    }

    pub fn field_hint(&self, store: &AiKeyStore) -> &'static str {
        if store.is_configured(self.kind()) {
            crate::platform::credential_saved_hint()
        } else {
            crate::platform::credential_stored_hint()
        }
    }

    pub fn can_save(&self) -> bool {
        self.problem().is_none()
    }

    /// Saves a typed key to the Keychain and returns the prefs for the host to store. A key the
    /// Keychain refused is still usable until the app quits, and the message says so.
    pub fn save(&mut self, store: &AiKeyStore) -> Option<AiAction> {
        if let Some(problem) = self.problem() {
            self.status = Some(StatusMessage::error(problem));
            return None;
        }
        let kind = self.kind();
        self.status = Some(if self.key_input.trim().is_empty() {
            StatusMessage::ok("Saved.")
        } else {
            match store.save(kind, &self.key_input) {
                Ok(()) => StatusMessage::ok(crate::platform::saved_securely()),
                Err(error) => StatusMessage::error(format!(
                    "The key could not be saved for later ({}). It works until you quit DegenView.",
                    error.0
                )),
            }
        });
        self.key_input.clear();
        self.draft.set_model(kind, &self.draft.model(kind));
        Some(AiAction::SavePrefs(self.draft.clone()))
    }

    pub fn remove_key(&mut self, store: &AiKeyStore) {
        store.remove(self.kind());
        self.key_input.clear();
        self.status = Some(StatusMessage::ok("Key removed."));
    }

    /// The provider the Test button talks to: the typed key if there is one, else the saved or
    /// environment key.
    pub fn provider_for_test(
        &self,
        store: &AiKeyStore,
        env: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Box<dyn LlmProvider>, LlmError> {
        let kind = self.kind();
        let typed = self.key_input.trim();
        let key = if typed.is_empty() {
            keychain_ai_keys::key_with(store, kind, env)
        } else {
            Some(typed.to_owned())
        };
        let provider = HttpProvider::new(kind, self.draft.base_url(kind), key)?;
        Ok(Box::new(provider))
    }

    /// Lists the key's models off the UI thread; `poll_test` picks up the answer. Listing spends
    /// no tokens, so it is a free check that the key and the address work.
    pub fn start_test(&mut self, provider: Result<Box<dyn LlmProvider>, LlmError>) {
        if self.test.is_some() {
            return;
        }
        self.status = None;
        let provider = match provider {
            Ok(provider) => provider,
            Err(error) => {
                self.status = Some(StatusMessage::error(error.to_string()));
                return;
            }
        };
        let (sender, receiver) = std::sync::mpsc::channel();
        self.test = Some(receiver);
        std::thread::spawn(move || {
            let outcome = provider
                .list_models()
                .map(|models| models.len())
                .map_err(|error| error.to_string());
            let _ = sender.send(outcome);
        });
    }

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
            Ok(1) => StatusMessage::ok("Connection successful. 1 model available."),
            Ok(count) => {
                StatusMessage::ok(format!("Connection successful. {count} models available."))
            }
            Err(message) => StatusMessage::error(message),
        });
    }
}

// MARK: Views

fn key_page(kind: ProviderKind) -> Option<(&'static str, &'static str)> {
    match kind {
        ProviderKind::OpenAi => Some((
            "Get an OpenAI API key",
            "https://platform.openai.com/api-keys",
        )),
        ProviderKind::Anthropic => Some((
            "Get an Anthropic API key",
            "https://console.anthropic.com/settings/keys",
        )),
        ProviderKind::Compatible => None,
    }
}

pub fn body(ui: &mut egui::Ui, state: &mut AiSettingsState, input: &AiSettingsInput) {
    let store = AiKeyStore::shared();
    state.load(input.prefs);
    state.poll_test();
    if state.is_testing() {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(150));
    }
    let (text, tone) = state.badge(store);
    status_badge(ui, text, tone);
    ui.add_space(18.0);

    section(ui, "Provider", None, |ui| {
        card(ui, |ui| {
            let options: Vec<(ProviderKind, &str)> = ProviderKind::ALL
                .into_iter()
                .map(|kind| (kind, kind.title()))
                .collect();
            let mut selected = state.kind();
            kit::segmented(ui, egui::Id::new("ai-provider"), &options, &mut selected);
            state.select(selected);
            let kind = state.kind();
            let mut model = state.draft.model(kind);
            let suggestion = kind.default_model();
            let hint = (!suggestion.is_empty())
                .then(|| format!("Model names change often. Suggested: {suggestion}."));
            ui.vertical(|ui| field(ui, "Model", hint.as_deref(), suggestion, &mut model, false));
            state.draft.set_model(kind, &model);
            if kind == ProviderKind::Compatible {
                ui.vertical(|ui| {
                    field(
                        ui,
                        "Server Base URL",
                        Some(
                            "OpenRouter, Ollama, LM Studio and similar. Use https://, or http:// for a server on this Mac.",
                        ),
                        "http://localhost:11434/v1",
                        &mut state.draft.compatible_base_url,
                        false,
                    )
                });
            }
        });
    });
    ui.add_space(18.0);

    let (title, hint) = (state.field_title(store), state.field_hint(store));
    section(ui, "API access", None, |ui| {
        card(ui, |ui| {
            ui.vertical(|ui| field(ui, title, Some(hint), "", &mut state.key_input, true));
        });
    });
    let provider = state.kind().title();
    kit::notice_card(
        ui,
        Icon::InfoCircle,
        kit::ACCENT,
        "Your script is sent to the provider",
        Some(&format!(
            "When you ask the assistant for a change, the open script and your message are sent to {provider} with your key. DegenView has no server in between, and you pay {provider} for the usage."
        )),
        None,
    );
    if let Some((label, url)) = key_page(state.kind()) {
        ui.add_space(18.0);
        link(ui, label, url);
    }
}

pub fn footer(ui: &mut egui::Ui, state: &mut AiSettingsState, actions: &mut Vec<AiAction>) {
    let store = AiKeyStore::shared();
    status(ui, &state.status);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        if kit::sheet_button(ui, "Save", true, state.can_save()).clicked()
            && let Some(action) = state.save(store)
        {
            actions.push(action);
        }
        if state.is_testing() {
            ui.add(
                egui::Spinner::new()
                    .size(14.0)
                    .color(theme::text_secondary()),
            );
        } else if kit::sheet_button(ui, "Test Connection", false, state.can_save()).clicked() {
            let provider = state.provider_for_test(store, &|name| std::env::var(name).ok());
            state.start_test(provider);
        }
        if store.is_configured(state.kind()) && destructive_button(ui, "Remove Key").clicked() {
            state.remove_key(store);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::keychain_api_keys::MemoryBackend;
    use crate::service::llm::mock::MockProvider;
    use std::time::{Duration, Instant};

    fn store() -> AiKeyStore {
        AiKeyStore::with_backend(Box::new(MemoryBackend::default()))
    }

    fn state() -> AiSettingsState {
        let mut state = AiSettingsState::default();
        state.load(&AiPrefs::default());
        state
    }

    fn wait_for_test(state: &mut AiSettingsState) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while state.is_testing() {
            assert!(Instant::now() < deadline, "the test never finished");
            state.poll_test();
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn the_draft_is_read_from_the_saved_prefs_once() {
        let mut state = AiSettingsState::default();
        let saved = AiPrefs {
            provider: ProviderKind::Anthropic,
            ..AiPrefs::default()
        };
        state.load(&saved);
        assert_eq!(state.kind(), ProviderKind::Anthropic);
        state.draft.provider = ProviderKind::OpenAi;
        state.load(&saved);
        assert_eq!(state.kind(), ProviderKind::OpenAi);
    }

    #[test]
    fn saving_stores_the_typed_key_and_hands_back_the_prefs() {
        let store = store();
        let mut state = state();
        state.key_input = "  sk-test ".to_owned();
        let action = state.save(&store);
        assert_eq!(store.key(ProviderKind::OpenAi).as_deref(), Some("sk-test"));
        assert_eq!(state.key_input, "");
        assert!(!state.status.as_ref().unwrap().is_error);
        let Some(AiAction::SavePrefs(prefs)) = action else {
            panic!("expected prefs");
        };
        assert_eq!(prefs.provider, ProviderKind::OpenAi);
        assert_eq!(prefs.openai_model, ProviderKind::OpenAi.default_model());
    }

    #[test]
    fn saving_with_no_new_key_keeps_the_saved_one() {
        let store = store();
        store.save(ProviderKind::OpenAi, "kept").unwrap();
        let mut state = state();
        assert!(state.save(&store).is_some());
        assert_eq!(store.key(ProviderKind::OpenAi).as_deref(), Some("kept"));
    }

    #[test]
    fn the_compatible_provider_needs_a_safe_url_and_a_model() {
        let store = store();
        let mut state = state();
        state.select(ProviderKind::Compatible);
        assert!(!state.can_save());
        assert!(state.save(&store).is_none());
        assert!(state.status.as_ref().unwrap().is_error);

        state.draft.compatible_base_url = "http://example.com/v1".to_owned();
        assert!(state.problem().unwrap().contains("https://"));

        state.draft.compatible_base_url = "http://localhost:11434/v1".to_owned();
        assert!(state.problem().unwrap().contains("model"));

        state.draft.set_model(ProviderKind::Compatible, "llama3");
        assert_eq!(state.problem(), None);
        assert!(state.save(&store).is_some());
        // A local server needs no key.
        assert!(!store.is_configured(ProviderKind::Compatible));
    }

    #[test]
    fn switching_provider_drops_the_half_typed_key_and_old_status() {
        let mut state = state();
        state.key_input = "sk-half".to_owned();
        state.status = Some(StatusMessage::error("old"));
        state.select(ProviderKind::Anthropic);
        assert_eq!(state.key_input, "");
        assert!(state.status.is_none());
        // Picking the same one again changes nothing.
        state.key_input = "typed".to_owned();
        state.select(ProviderKind::Anthropic);
        assert_eq!(state.key_input, "typed");
    }

    #[test]
    fn removing_the_key_clears_it_and_the_badge_follows() {
        let store = store();
        let mut state = state();
        assert_eq!(state.badge(&store).0, "No API key");
        store.save(ProviderKind::OpenAi, "k").unwrap();
        assert_eq!(state.badge(&store).0, "API key saved");
        state.remove_key(&store);
        assert_eq!(state.badge(&store).0, "No API key");
        assert_eq!(state.field_title(&store), "API Key");
    }

    #[test]
    fn the_test_uses_the_typed_key_before_the_saved_one() {
        let store = store();
        store.save(ProviderKind::OpenAi, "saved").unwrap();
        let mut state = state();
        // With nothing typed, the saved key is enough to build a provider.
        assert!(state.provider_for_test(&store, &|_| None).is_ok());
        // With no key anywhere, building one fails before any request.
        let empty = self::store();
        assert!(matches!(
            state.provider_for_test(&empty, &|_| None),
            Err(LlmError::Config(_))
        ));
        state.key_input = "typed".to_owned();
        assert!(state.provider_for_test(&empty, &|_| None).is_ok());
    }

    #[test]
    fn a_connection_test_reports_the_model_count_or_the_error() {
        let mut state = state();
        let mut mock = MockProvider::replying(vec![]);
        mock.models = Ok(vec!["a".to_owned(), "b".to_owned()]);
        state.start_test(Ok(Box::new(mock)));
        assert!(state.is_testing());
        wait_for_test(&mut state);
        assert_eq!(
            state.status,
            Some(StatusMessage::ok(
                "Connection successful. 2 models available."
            ))
        );

        let mut mock = MockProvider::replying(vec![]);
        mock.models = Err(LlmError::Auth("bad key".to_owned()));
        state.start_test(Ok(Box::new(mock)));
        wait_for_test(&mut state);
        let status = state.status.clone().unwrap();
        assert!(status.is_error);
        assert!(status.text.contains("bad key"));
    }

    #[test]
    fn a_test_with_no_key_says_so_without_starting() {
        let mut state = state();
        state.start_test(Err(LlmError::Config("Add your key.".to_owned())));
        assert!(!state.is_testing());
        assert_eq!(state.status, Some(StatusMessage::error("Add your key.")));
    }
}
