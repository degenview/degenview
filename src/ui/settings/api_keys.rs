//! Settings ▸ Alpaca, Settings ▸ Oanda and Settings ▸ CoinMarketCap: the API keys that unlock US
//! stock data, Oanda's forex and CFD markets, and raise the market-index rate limits. Pages over
//! the shared key stores; saving, removing and the connection tests happen here, since the stores
//! are process-wide, but every rule (what may be saved, what the status says) is a plain function
//! tested against in-memory stores.

use crate::i18n::tr;
use crate::model::DataSource;
use crate::service::ProviderError;
use crate::service::keychain::api_keys::{
    AlpacaCredentials, AlpacaCredentialsStore, CoinMarketCapKeyStore, CredentialPair,
    CredentialsStore, OandaCredentials, OandaCredentialsStore,
};
use crate::service::work_queue::{self, Pending};
use crate::ui::icons::{self, Icon};
use crate::ui::kit::loading::LoadingIndicator;
use crate::ui::kit::{Autosave, Tone, destructive_button, field, open_button};
use crate::ui::settings::brokers::step_row;
use crate::ui::settings::card::StatusMessage;
use crate::ui::settings::window::{card, section, status_message};
use crate::ui::{kit, theme};
use eframe::egui;

pub type AlpacaForm = CredentialForm<AlpacaCredentials>;
pub type OandaForm = CredentialForm<OandaCredentials>;

#[derive(Default)]
pub struct CredentialForm<C> {
    credentials: C,
    loaded: bool,
    is_saved: bool,
    status: Option<StatusMessage>,
    test: Option<Pending<Result<(), ProviderError>>>,
    autosave: Autosave,
}

#[derive(Default)]
pub struct CmcForm {
    pub api_key: String,
    loaded: bool,
    pub configured: bool,
    pub status: Option<StatusMessage>,
    test: Option<Pending<Result<(), ProviderError>>>,
    autosave: Autosave,
}

#[derive(Default)]
pub struct ApiKeysState {
    pub alpaca: AlpacaForm,
    pub oanda: OandaForm,
    pub cmc: CmcForm,
}

pub trait CredentialFormCopy: CredentialPair {
    const SOURCE: DataSource;
    fn saved_label() -> &'static str;
    fn removed_message() -> &'static str;
    fn remove_error(error: String) -> String;
}

impl CredentialFormCopy for AlpacaCredentials {
    const SOURCE: DataSource = DataSource::Alpaca;

    fn saved_label() -> &'static str {
        tr!("Keys saved")
    }

    fn removed_message() -> &'static str {
        tr!("Keys removed. Stock charts need keys to load.")
    }

    fn remove_error(error: String) -> String {
        tr!("Could not remove the keys: {error}", error = error)
    }
}

impl CredentialFormCopy for OandaCredentials {
    const SOURCE: DataSource = DataSource::Oanda;

    fn saved_label() -> &'static str {
        tr!("Credentials saved")
    }

    fn removed_message() -> &'static str {
        tr!("Credentials removed. Oanda charts need them to load.")
    }

    fn remove_error(error: String) -> String {
        tr!("Could not remove the credentials: {error}", error = error)
    }
}

impl<C: CredentialFormCopy> CredentialForm<C> {
    fn load(&mut self, store: &CredentialsStore<C>) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        self.is_saved = store.is_configured();
        if self.is_saved {
            self.credentials = store.credentials();
        }
    }

    fn can_save(&self) -> bool {
        self.credentials.is_configured()
    }

    fn badge(&self) -> (&'static str, Tone) {
        if self.is_saved {
            (C::saved_label(), Tone::Good)
        } else {
            (tr!("Not configured"), Tone::Neutral)
        }
    }

    fn save(&mut self, store: &CredentialsStore<C>) {
        self.status = Some(match store.save(&self.credentials) {
            Ok(()) => {
                self.is_saved = self.can_save();
                StatusMessage::saved_securely()
            }
            Err(error) => StatusMessage::error(tr!("Could not save: {error}", error = error.0)),
        });
    }

    fn save_if_due(&mut self, ctx: &egui::Context, store: &CredentialsStore<C>) {
        if self.autosave.due(ctx) && self.can_save() {
            self.save(store);
        }
    }

    fn is_testing(&self) -> bool {
        self.test.is_some()
    }

    fn start_test(
        &mut self,
        repaint: Option<&egui::Context>,
        test: impl FnOnce(String, String) -> Result<(), ProviderError> + Send + 'static,
    ) {
        if self.is_testing() {
            return;
        }
        self.status = None;
        let (first, second) = self.credentials.fields();
        let (first, second) = (first.to_owned(), second.to_owned());
        self.test = Some(work_queue::start_for(C::SOURCE, repaint, move || {
            test(first, second)
        }));
    }

    fn poll_test(&mut self) {
        take_test_answer(&mut self.test, &mut self.status);
    }

    fn remove(&mut self, store: &CredentialsStore<C>) {
        self.autosave.cancel();
        self.status = Some(match store.save(&C::default()) {
            Ok(()) => {
                self.credentials = C::default();
                self.is_saved = false;
                StatusMessage::ok(C::removed_message())
            }
            Err(error) => StatusMessage::error(C::remove_error(error.0)),
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
            (tr!("API key configured"), Tone::Good)
        } else {
            (tr!("Public API mode"), Tone::Neutral)
        }
    }

    pub fn field_title(&self) -> &'static str {
        if self.configured {
            tr!("Replace API Key")
        } else {
            tr!("API Key")
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
                StatusMessage::saved_securely()
            }
            Err(error) => StatusMessage::error(tr!("Could not save: {error}", error = error.0)),
        });
    }

    pub fn remove(&mut self, store: &CoinMarketCapKeyStore) {
        self.status = Some(match store.remove() {
            Ok(()) => {
                self.api_key.clear();
                self.configured = false;
                StatusMessage::ok(tr!("Key removed. Public API mode is active."))
            }
            Err(error) => {
                StatusMessage::error(tr!("Could not remove the key: {error}", error = error.0))
            }
        });
    }

    /// Runs `test` on a worker that may wait on the user (it reads the saved key, which may
    /// prompt); `poll_test` picks up the answer.
    pub fn start_test(
        &mut self,
        repaint: Option<&egui::Context>,
        test: impl FnOnce() -> Result<(), ProviderError> + Send + 'static,
    ) {
        if self.test.is_some() {
            return;
        }
        self.status = None;
        self.test = Some(work_queue::SNAPSHOTS.start(repaint, test));
    }

    /// Takes the connection test's answer when it has arrived.
    pub fn poll_test(&mut self) {
        take_test_answer(&mut self.test, &mut self.status);
    }
}

/// Turns a finished connection test into the status under its button.
fn take_test_answer(
    test: &mut Option<Pending<Result<(), ProviderError>>>,
    status: &mut Option<StatusMessage>,
) {
    let Some(pending) = test else {
        return;
    };
    let Some(outcome) = pending.poll_or(|| {
        Err(ProviderError::other(tr!(
            "The connection test stopped unexpectedly."
        )))
    }) else {
        return;
    };
    *test = None;
    *status = Some(match outcome {
        Ok(()) => StatusMessage::ok(tr!("Connection successful.")),
        Err(error) => StatusMessage::error(error.to_string()),
    });
}

// MARK: Views

/// A link with a leading "open" glyph: gray and underlined, as every link is.
pub(crate) fn link(ui: &mut egui::Ui, label: &str, url: &str) {
    let response = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            icons::inline(ui, Icon::ArrowUpRightSquare, 12.0, theme::text_secondary());
            ui.add(
                egui::Label::new(
                    kit::text(
                        label,
                        kit::size::BODY,
                        kit::REGULAR,
                        theme::text_secondary(),
                    )
                    .underline(),
                )
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

pub fn alpaca_body(ui: &mut egui::Ui, state: &mut ApiKeysState, store: &AlpacaCredentialsStore) {
    let form = &mut state.alpaca;
    form.load(store);
    form.poll_test();
    if form.is_testing() {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(150));
    }
    let (text, tone) = form.badge();
    kit::status_badge(ui, text, tone);
    ui.add_space(18.0);
    section(ui, tr!("API credentials"), None, |ui| {
        card(ui, |ui| {
            ui.vertical(|ui| {
                let key_id = field(
                    ui,
                    "alpaca-key-id",
                    tr!("API Key ID"),
                    None,
                    "PK…",
                    &mut form.credentials.key_id,
                    false,
                );
                form.autosave.watch(&key_id, false);
                ui.add_space(12.0);
                let secret = field(
                    ui,
                    "alpaca-secret",
                    tr!("Secret Key"),
                    Some(crate::platform::credential_stored_hint()),
                    "",
                    &mut form.credentials.secret_key,
                    true,
                );
                form.autosave.watch(&secret, true);
            });
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                if form.is_testing() {
                    ui.add(LoadingIndicator::new().size(14.0));
                } else if kit::sheet_button(ui, tr!("Test Connection"), false, form.can_save())
                    .clicked()
                {
                    form.start_test(Some(ui.ctx()), |key_id, secret| {
                        crate::service::alpaca_test_connection(&key_id, &secret)
                    });
                }
                if form.is_saved && destructive_button(ui, tr!("Remove Keys")).clicked() {
                    form.remove(store);
                }
            });
        });
        if let Some(status) = &form.status {
            ui.add_space(8.0);
            status_message(ui, &status.text(), status.is_error);
        }
    });
    // Both keys are needed, so nothing is saved until both are there.
    form.save_if_due(ui.ctx(), store);
    section(ui, tr!("Getting your keys"), None, |ui| {
        card(ui, |ui| {
            step_row(ui, 1, tr!("Create a free Alpaca account."));
            step_row(
                ui,
                2,
                tr!("Open the API Keys section of the dashboard and generate a key."),
            );
            step_row(
                ui,
                3,
                tr!("Copy both values here. Alpaca only shows the secret once."),
            );
            link(
                ui,
                tr!("Open the Alpaca dashboard"),
                "https://app.alpaca.markets/",
            );
        });
    });
}

pub fn oanda_body(ui: &mut egui::Ui, state: &mut ApiKeysState, store: &OandaCredentialsStore) {
    let form = &mut state.oanda;
    form.load(store);
    form.poll_test();
    if form.is_testing() {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(150));
    }
    let (text, tone) = form.badge();
    kit::status_badge(ui, text, tone);
    ui.add_space(18.0);
    section(ui, tr!("API credentials"), None, |ui| {
        card(ui, |ui| {
            ui.vertical(|ui| {
                let account_id = field(
                    ui,
                    "oanda-account-id",
                    tr!("Account ID"),
                    None,
                    "001-…",
                    &mut form.credentials.account_id,
                    false,
                );
                form.autosave.watch(&account_id, false);
                ui.add_space(12.0);
                let token = field(
                    ui,
                    "oanda-access-token",
                    tr!("Access Token"),
                    Some(crate::platform::credential_stored_hint()),
                    "",
                    &mut form.credentials.access_token,
                    true,
                );
                form.autosave.watch(&token, true);
            });
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                if form.is_testing() {
                    ui.add(LoadingIndicator::new().size(14.0));
                } else if kit::sheet_button(ui, tr!("Test Connection"), false, form.can_save())
                    .clicked()
                {
                    form.start_test(Some(ui.ctx()), |account_id, token| {
                        crate::service::oanda_test_connection(&account_id, &token)
                    });
                }
                if form.is_saved && destructive_button(ui, tr!("Remove Credentials")).clicked() {
                    form.remove(store);
                }
            });
        });
        if let Some(status) = &form.status {
            ui.add_space(8.0);
            status_message(ui, &status.text(), status.is_error);
        }
    });
    // Both fields are needed, so nothing is saved until both are there.
    form.save_if_due(ui.ctx(), store);
    section(ui, tr!("Getting your credentials"), None, |ui| {
        card(ui, |ui| {
            step_row(ui, 1, tr!("Create an Oanda account."));
            step_row(
                ui,
                2,
                tr!("In Account Management, open Personal Access Tokens and generate a token."),
            );
            step_row(
                ui,
                3,
                tr!("Copy the account ID and the token here. Oanda only shows the token once."),
            );
            link(
                ui,
                tr!("Open the Oanda API documentation"),
                "https://developer.oanda.com/",
            );
        });
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
    kit::status_badge(ui, text, tone);
    ui.add_space(18.0);
    let (title, hint) = (form.field_title(), form.field_hint());
    section(ui, tr!("API access"), None, |ui| {
        card(ui, |ui| {
            ui.vertical(|ui| {
                let key = field(
                    ui,
                    "cmc-key",
                    title,
                    Some(hint),
                    "",
                    &mut form.api_key,
                    true,
                );
                form.autosave.watch(&key, true);
            });
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                if form.is_testing() {
                    ui.add(LoadingIndicator::new().size(14.0));
                } else if kit::sheet_button(ui, tr!("Test Connection"), false, true).clicked() {
                    form.start_test(Some(ui.ctx()), crate::service::market::cmc::test_connection);
                }
                open_button(
                    ui,
                    tr!("Get API Key"),
                    "https://pro.coinmarketcap.com/signup/",
                );
                if form.configured && destructive_button(ui, tr!("Remove Key")).clicked() {
                    form.remove(store);
                    form.autosave.cancel();
                }
            });
        });
        if let Some(status) = &form.status {
            ui.add_space(8.0);
            status_message(ui, &status.text(), status.is_error);
        }
    });
    if form.autosave.due(ui.ctx()) && form.can_save() {
        form.save(store);
    }
    kit::notice_card(
        ui,
        Icon::InfoCircle,
        theme::text_secondary(),
        tr!("A key is optional"),
        Some(tr!(
            "CoinMarketCap works without an API key. Adding one gives DegenView higher API rate limits."
        )),
        None,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::keychain::api_keys::MemoryBackend;
    use std::time::{Duration, Instant};

    fn alpaca_store() -> AlpacaCredentialsStore {
        AlpacaCredentialsStore::with_backend(Box::new(MemoryBackend::default()))
    }

    fn cmc_store() -> CoinMarketCapKeyStore {
        CoinMarketCapKeyStore::with_backend(Box::new(MemoryBackend::default()))
    }

    fn oanda_store() -> OandaCredentialsStore {
        OandaCredentialsStore::with_backend(Box::new(MemoryBackend::default()))
    }

    #[test]
    fn alpaca_saves_only_with_both_fields_and_reports_it() {
        let store = alpaca_store();
        let mut form = AlpacaForm::default();
        form.load(&store);
        assert_eq!(form.badge(), ("Not configured", Tone::Neutral));
        assert!(!form.can_save());
        form.credentials.key_id = " PK1 ".into();
        assert!(!form.can_save(), "the secret is missing too");
        form.credentials.secret_key = "S1".into();
        assert!(form.can_save());
        form.save(&store);
        assert_eq!(form.status, Some(StatusMessage::saved_securely()));
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
            (
                form.credentials.key_id.as_str(),
                form.credentials.secret_key.as_str()
            ),
            ("PK9", "S9")
        );
        form.credentials.key_id = "typed".into();
        form.load(&store);
        assert_eq!(
            form.credentials.key_id, "typed",
            "a second load does not overwrite edits"
        );
        form.remove(&store);
        assert!(
            !form.is_saved
                && form.credentials.key_id.is_empty()
                && form.credentials.secret_key.is_empty()
        );
        assert_eq!(
            form.status,
            Some(StatusMessage::ok(
                "Keys removed. Stock charts need keys to load."
            ))
        );
        assert!(!store.is_configured());
    }

    #[test]
    fn oanda_saves_only_with_both_fields_and_loads_them_back_once() {
        let store = oanda_store();
        let mut form = OandaForm::default();
        form.load(&store);
        assert_eq!(form.badge(), ("Not configured", Tone::Neutral));
        assert!(!form.can_save());
        form.credentials.account_id = " 001-001 ".into();
        assert!(!form.can_save(), "the token is missing too");
        form.credentials.access_token = "tok".into();
        assert!(form.can_save());
        form.save(&store);
        assert_eq!(form.status, Some(StatusMessage::saved_securely()));
        assert_eq!(form.badge(), ("Credentials saved", Tone::Good));
        assert_eq!(store.credentials().account_id, "001-001", "saved trimmed");

        let mut reloaded = OandaForm::default();
        reloaded.load(&store);
        assert!(reloaded.is_saved);
        assert_eq!(
            (
                reloaded.credentials.account_id.as_str(),
                reloaded.credentials.access_token.as_str()
            ),
            ("001-001", "tok")
        );
        reloaded.credentials.account_id = "typed".into();
        reloaded.load(&store);
        assert_eq!(
            reloaded.credentials.account_id, "typed",
            "a second load keeps edits"
        );
        reloaded.remove(&store);
        assert!(!reloaded.is_saved && reloaded.credentials.account_id.is_empty());
        assert_eq!(
            reloaded.status,
            Some(StatusMessage::ok(
                "Credentials removed. Oanda charts need them to load."
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
        form.start_test(None, || Ok(()));
        assert!(form.is_testing());
        assert_eq!(form.status, None, "a new test clears the old message");
        form.start_test(None, || {
            Err(ProviderError::other("ignored: one test at a time"))
        });
        wait_for_test(&mut form);
        assert!(!form.is_testing());
        assert_eq!(
            form.status,
            Some(StatusMessage::ok("Connection successful."))
        );

        form.start_test(None, || Err(ProviderError::other("Invalid API key")));
        wait_for_test(&mut form);
        assert_eq!(form.status, Some(StatusMessage::error("Invalid API key")));
    }

    /// A setup step is longer in another language and wraps under its number; it does not run past
    /// the card.
    #[test]
    fn the_setup_steps_wrap_inside_the_page_in_the_pseudo_locale() {
        use crate::i18n::{Locale, with_locale};
        use crate::ui::render_tests::Screen;
        use crate::ui::trade::pseudo_text::painted_boxes;
        with_locale(Locale::PSEUDO, || {
            let store = alpaca_store();
            let mut state = ApiKeysState::default();
            let width = 540.0;
            let mut screen = Screen::new(egui::vec2(width, 1_100.0));
            let mut last = None;
            for _ in 0..3 {
                last = Some(screen.run(Vec::new(), |ui| alpaca_body(ui, &mut state, &store)));
            }
            for (text, rect) in painted_boxes(&last.expect("a frame")) {
                assert!(
                    rect.right() <= width,
                    "{text:?} runs past the page: {rect:?}"
                );
            }
        });
    }
}
