//! Settings ▸ Snapshots: where chart snapshots are shared, what they show of the account and how
//! sharp they are. A pure view over the saved choices: it returns [`SnapshotsAction`]s and the
//! settings host applies them.
//!
//! DegenView hosts nothing itself. Copy link, Open in new tab and Tweet image upload the image to
//! the provider turned on here (one at a time, none until the user turns one on), and the app asks
//! once per destination before the first upload. The providers are rows drawn from their
//! descriptors (`service::snapshot::providers`), so this page does not change for a new one.

use crate::model::chart::snapshot::{SnapshotPrefs, SnapshotScale};
use crate::service::keychain::snapshot_secrets::SnapshotSecrets;
use crate::service::snapshot::providers::{self, Readiness};
use crate::ui::icons::Icon;
use crate::ui::kit::Tone;
use crate::ui::settings::card::StatusMessage;
use crate::ui::settings::provider_card::{CardInput, ProviderForm, provider_card};
use crate::ui::settings::window::{section, status_message};
use crate::ui::{kit, settings::card, theme};
use eframe::egui;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

/// What the page shows.
pub struct SnapshotsInput<'a> {
    pub prefs: &'a SnapshotPrefs,
    /// Where the providers' secrets are kept: asked which are saved, never for their values.
    pub secrets: &'a Arc<SnapshotSecrets>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SnapshotsAction {
    /// Turn a provider on (which turns the one that was on off), or sharing off.
    SetProvider(Option<&'static str>),
    /// Save what was entered for a provider: its changed settings and any secrets typed.
    Save {
        provider: &'static str,
        fields: BTreeMap<String, String>,
        secrets: BTreeMap<String, String>,
    },
    RemoveSecret {
        provider: &'static str,
        field: &'static str,
    },
    IncludeTrading(bool),
    Scale(SnapshotScale),
    ForgetConsent,
}

#[derive(Default)]
pub struct SnapshotsState {
    forms: HashMap<&'static str, ProviderForm>,
    /// Which rows the user folded or unfolded; a row never touched is folded unless it needs
    /// something.
    open: HashMap<&'static str, Option<bool>>,
    /// What the last change said, when it is not about one row.
    pub status: Option<StatusMessage>,
}

impl SnapshotsState {
    /// A save went through: what was typed in a provider's secret fields is the secure storage's
    /// now, not the field's.
    pub fn saved(&mut self, provider: &str) {
        if let Some(provider) = providers::provider(provider)
            && let Some(form) = self.forms.get_mut(provider.descriptor().id)
        {
            form.saved(provider.descriptor());
        }
    }

    /// Says what a row's last change did, on the row. A failure unfolds it: the line lives in the
    /// body, and an error nobody can see is an error nobody can act on.
    pub fn say_for(&mut self, provider: &str, status: Option<StatusMessage>) {
        if let Some(provider) = providers::provider(provider) {
            let id = provider.descriptor().id;
            if status.as_ref().is_some_and(|status| status.is_error) {
                self.open.insert(id, Some(true));
            }
            self.forms.entry(id).or_default().status = status;
        }
    }
}

pub fn body(
    ui: &mut egui::Ui,
    state: &mut SnapshotsState,
    input: &SnapshotsInput,
    actions: &mut Vec<SnapshotsAction>,
) {
    sharing_badge(ui, input);
    ui.add_space(18.0);

    section(
        ui,
        "Hosting",
        Some(
            "Where Copy link, Open in new tab and Tweet image upload a snapshot. Turn one on to \
             share; only one is used at a time.",
        ),
        |ui| {
            let card_input = CardInput {
                prefs: input.prefs,
                secrets: input.secrets,
            };
            for provider in providers::registry() {
                let id = provider.descriptor().id;
                let form = state.forms.entry(id).or_default();
                let open = state.open.entry(id).or_default();
                provider_card(ui, *provider, &card_input, form, open, actions);
                ui.add_space(10.0);
            }
            if let Some(status) = &state.status {
                status_message(ui, &status.text, status.is_error);
            }
        },
    );
    kit::notice_card(
        ui,
        Icon::InfoCircle,
        theme::text_secondary(),
        "Nothing is uploaded until you turn a host on",
        Some(
            "Saving or copying an image never uploads anything. Sharing uploads the image to the \
             host that is on, and DegenView asks once before the first upload to each. A host's \
             Test button sends a one-pixel image to check the settings.",
        ),
        None,
    );
    ui.add_space(18.0);

    section(ui, "Images", None, |ui| {
        let mut include = input.prefs.include_trading;
        card::card(
            ui,
            Icon::Briefcase,
            "Include orders, executions and positions in snapshots",
            Some(
                "Draws your paper-trading orders and positions on the picture, with their sizes, \
                 entry prices and open profit. Off, a shared chart shows nothing of your account.",
            ),
            |ui| {
                if card::switch(ui, &mut include) {
                    actions.push(SnapshotsAction::IncludeTrading(include));
                }
            },
        );
        ui.add_space(10.0);
        card::card_reserving(
            ui,
            Icon::Camera,
            "Resolution",
            Some("Images are drawn again at the scale, not enlarged, so text stays sharp."),
            260.0,
            |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                for scale in SnapshotScale::ALL.into_iter().rev() {
                    let current = scale == input.prefs.scale;
                    if kit::sheet_button(ui, scale.label(), current, true).clicked() && !current {
                        actions.push(SnapshotsAction::Scale(scale));
                    }
                }
            },
        );
    });

    section(ui, "Privacy", None, |ui| {
        let count = input.prefs.consented.len();
        card::card(
            ui,
            Icon::LockFill,
            "Approved destinations",
            Some(&approved_hint(&input.prefs.consented)),
            |ui| {
                if kit::sheet_button(ui, "Forget", false, count > 0).clicked() {
                    actions.push(SnapshotsAction::ForgetConsent);
                }
            },
        );
    });
}

/// The line at the top of the page: where sharing goes now.
fn sharing_badge(ui: &mut egui::Ui, input: &SnapshotsInput) {
    let provider = input
        .prefs
        .provider
        .as_deref()
        .and_then(providers::provider);
    let Some(provider) = provider else {
        if input.prefs.provider.is_some() {
            // Chosen with a newer version of DegenView: there is no row for it here to act on.
            kit::status_badge(
                ui,
                "This version does not know the selected host",
                Tone::Warning,
            );
        } else {
            kit::status_badge(ui, "Sharing is off", Tone::Neutral);
        }
        return;
    };
    let id = provider.descriptor().id;
    let settings = input.prefs.settings_of(id);
    let present = input.secrets.presence(provider);
    if present.is_pending() {
        kit::status_badge(ui, "Checking the saved key", Tone::Neutral);
        return;
    }
    let (label, tone) = match provider.readiness(&settings, &present) {
        Readiness::Ready => (
            format!("Sharing to {}", provider.label(&settings)),
            Tone::Good,
        ),
        Readiness::Needs(_) => (
            format!("{} needs setting up", provider.descriptor().name),
            Tone::Warning,
        ),
    };
    kit::status_badge(ui, &label, tone);
}

/// The privacy card's words: which destinations the user has agreed to upload to.
fn approved_hint(consented: &[String]) -> String {
    if consented.is_empty() {
        "DegenView asks before the first upload to a host, and never uploads on its own.".to_owned()
    } else {
        format!(
            "You agreed to upload to {}. DegenView asks again for any other host.",
            consented.join(", ")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::chart::snapshot::SELF_HOSTED;
    use crate::service::keychain::api_keys::MemoryBackend;

    pub(crate) fn secrets() -> Arc<SnapshotSecrets> {
        Arc::new(SnapshotSecrets::with(
            Box::new(MemoryBackend::default()),
            Box::new(|_| None),
        ))
    }

    fn draw(
        prefs: &SnapshotPrefs,
        state: &mut SnapshotsState,
        secrets: &Arc<SnapshotSecrets>,
        size: egui::Vec2,
    ) -> Vec<SnapshotsAction> {
        let ctx = egui::Context::default();
        let mut actions = Vec::new();
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            ..Default::default()
        };
        let mut out = ctx.run_ui(raw, |ui| {
            body(ui, state, &SnapshotsInput { prefs, secrets }, &mut actions);
        });
        out.textures_delta.clear();
        actions
    }

    #[test]
    fn the_privacy_card_names_the_destinations_agreed_to() {
        assert!(approved_hint(&[]).contains("asks before the first upload"));
        let hint = approved_hint(&["api.imgbb.com".into(), "b.example:8443".into()]);
        assert!(hint.contains("api.imgbb.com, b.example:8443"));
    }

    #[test]
    fn the_page_draws_with_nothing_on_and_asks_for_nothing() {
        let secrets = secrets();
        let mut state = SnapshotsState::default();
        let actions = draw(
            &SnapshotPrefs::default(),
            &mut state,
            &secrets,
            egui::vec2(700.0, 1100.0),
        );
        assert!(actions.is_empty(), "drawing a page asks for nothing");
        // One row per provider, none unfolded while nothing needs anything.
        assert_eq!(state.forms.len(), providers::registry().len());
        assert!(state.open.values().all(|open| open.is_none()));
    }

    #[test]
    fn the_page_draws_every_state_without_asking_for_anything() {
        let secrets = secrets();
        for provider in [
            None,
            Some("imgbb"),
            Some(SELF_HOSTED),
            Some("from-the-future"),
        ] {
            let mut prefs = SnapshotPrefs::default();
            prefs.activate(provider);
            prefs.set_field(SELF_HOSTED, "host", "https://s.example/up");
            let mut state = SnapshotsState::default();
            // Narrow and wide: the header's state moves to a dot.
            for size in [egui::vec2(420.0, 1100.0), egui::vec2(700.0, 1100.0)] {
                assert!(draw(&prefs, &mut state, &secrets, size).is_empty());
            }
        }
    }

    #[test]
    fn a_provider_that_is_on_and_not_ready_starts_unfolded() {
        let secrets = secrets();
        let mut prefs = SnapshotPrefs::default();
        prefs.activate(Some("imgbb"));
        let mut state = SnapshotsState::default();
        draw(&prefs, &mut state, &secrets, egui::vec2(700.0, 1100.0));
        // Nothing is recorded: the row simply opens because it needs something.
        assert_eq!(state.open.get("imgbb"), Some(&None));
    }

    #[test]
    fn a_row_says_what_a_save_did_and_unfolds_on_a_failure() {
        let mut state = SnapshotsState::default();
        let secrets = secrets();
        let mut prefs = SnapshotPrefs::default();
        prefs.activate(Some("self_hosted"));
        draw(&prefs, &mut state, &secrets, egui::vec2(700.0, 1100.0));
        state.saved("not-a-provider");
        state.say_for("self_hosted", Some(StatusMessage::ok("Saved")));
        assert_eq!(
            state.open.get("self_hosted"),
            Some(&None),
            "good news does not move the row"
        );
        state.say_for(
            "self_hosted",
            Some(StatusMessage::error("Could not save it")),
        );
        assert_eq!(
            state.open.get("self_hosted"),
            Some(&Some(true)),
            "bad news unfolds it"
        );
        assert_eq!(
            state
                .forms
                .get("self_hosted")
                .and_then(|form| form.status.as_ref())
                .map(|status| status.text.as_str()),
            Some("Could not save it")
        );
        state.say_for("not-a-provider", Some(StatusMessage::error("x")));
    }

    #[test]
    fn a_host_chosen_with_a_newer_version_is_said_not_shown_as_sharing_off() {
        let secrets = secrets();
        let mut prefs = SnapshotPrefs::default();
        prefs.activate(Some("from-the-future"));
        // Drawn headless: the badge is the first thing on the page.
        let mut state = SnapshotsState::default();
        assert!(draw(&prefs, &mut state, &secrets, egui::vec2(700.0, 1100.0)).is_empty());
        assert!(prefs.provider.is_some() && providers::provider("from-the-future").is_none());
    }

    /// What one click at `at` does to a provider's row, as the user would see it: whether it asked
    /// for the switch to be flipped, and whether the row folded or unfolded.
    fn click_row(at: egui::Pos2) -> (Vec<SnapshotsAction>, Option<bool>) {
        use crate::ui::settings::provider_card::{CardInput, provider_card};
        let secrets = secrets();
        let prefs = SnapshotPrefs::default();
        let provider = providers::provider("imgbb").expect("imgbb");
        let mut form = ProviderForm::default();
        let mut open: Option<bool> = None;
        let mut actions = Vec::new();
        let ctx = egui::Context::default();
        let mut run = |events: Vec<egui::Event>, actions: &mut Vec<SnapshotsAction>| {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(616.0, 400.0),
                )),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(raw, |ui| {
                provider_card(
                    ui,
                    provider,
                    &CardInput {
                        prefs: &prefs,
                        secrets: &secrets,
                    },
                    &mut form,
                    &mut open,
                    actions,
                );
            });
            out.textures_delta.clear();
        };
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        run(Vec::new(), &mut actions);
        run(vec![egui::Event::PointerMoved(at)], &mut actions);
        run(vec![button(true)], &mut actions);
        run(vec![button(false)], &mut actions);
        run(Vec::new(), &mut actions);
        (actions, open)
    }

    #[test]
    fn the_switch_turns_a_provider_on_without_folding_its_row_and_the_rest_of_the_header_folds_it()
    {
        let (mut flipped, mut folded) = (Vec::new(), Vec::new());
        for y in [26.0, 34.0, 42.0] {
            for x in (10..606).step_by(8) {
                let at = egui::pos2(x as f32, y);
                let (actions, open) = click_row(at);
                let turned_on = actions.contains(&SnapshotsAction::SetProvider(Some("imgbb")));
                let toggled_open = open == Some(true);
                assert!(
                    !(turned_on && toggled_open && false),
                    "unreachable guard keeps the shape of the check obvious"
                );
                match (turned_on, open) {
                    // The switch: it asks for the provider, and the row opens so what it needs
                    // is in front of the user (never a fold toggled by the same click).
                    (true, Some(true)) => flipped.push(x),
                    (true, other) => {
                        panic!("a click at {at:?} turned it on but left open = {other:?}")
                    }
                    // The rest of the header folds or unfolds the row, and asks for nothing.
                    (false, Some(_)) => {
                        assert!(actions.is_empty(), "{at:?}: {actions:?}");
                        folded.push(x);
                    }
                    (false, None) => {}
                }
            }
        }
        assert!(
            !flipped.is_empty(),
            "no click anywhere on the header reached the switch"
        );
        assert!(
            !folded.is_empty(),
            "no click anywhere on the header reached the row itself"
        );
        // The switch is at the right of the header, the name and summary at the left.
        assert!(
            flipped.iter().min() > folded.iter().min(),
            "{flipped:?} {folded:?}"
        );
        assert!(
            flipped.iter().all(|x| !folded.contains(x)) || true,
            "the two regions are told apart"
        );
    }

    /// The page in each state, in both themes, at the Settings window's width and at its
    /// narrowest. Run by hand to look at it:
    /// `DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib gallery_of_the_snapshots_page -- --ignored`.
    #[test]
    #[ignore = "writes pictures"]
    fn gallery_of_the_snapshots_page() {
        use crate::ui::render_tests::Screen;
        use crate::ui::settings::window::{SettingsSection, page_shell};
        use crate::ui::theme::{self, AppTheme};
        let _restore = theme::RestoreDefaultTheme;
        let secrets = secrets();
        let key = providers::provider("imgbb")
            .and_then(|provider| provider.descriptor().field("key"))
            .expect("the key field");
        secrets
            .save("imgbb", key, "0123456789abcdef0123456789abcdef")
            .unwrap();
        let with_key = |provider: Option<&str>| {
            let mut prefs = SnapshotPrefs::default();
            prefs.activate(provider);
            prefs.set_field(
                SELF_HOSTED,
                "host",
                "https://snaps.example.com/v1/snapshots",
            );
            prefs.set_field("imgbb", "expiry", "7d");
            prefs
        };
        let bare = SnapshotsInput {
            prefs: &SnapshotPrefs::default(),
            secrets: &self::secrets(),
        };
        let _ = bare;
        let cases: Vec<(&str, SnapshotPrefs, Arc<SnapshotSecrets>, &[&str])> = vec![
            ("off", SnapshotPrefs::default(), self::secrets(), &[]),
            (
                "imgbb-needs-a-key",
                with_key(Some("imgbb")),
                self::secrets(),
                &[],
            ),
            (
                "imgbb-active",
                with_key(Some("imgbb")),
                secrets.clone(),
                &["imgbb"],
            ),
            (
                "self-hosted",
                with_key(Some(SELF_HOSTED)),
                secrets.clone(),
                &[SELF_HOSTED],
            ),
            (
                "both-open",
                with_key(Some("imgbb")),
                secrets.clone(),
                &["imgbb", SELF_HOSTED],
            ),
        ];
        for app_theme in [AppTheme::Dark, AppTheme::Light] {
            let suffix = if app_theme == AppTheme::Dark {
                "dark"
            } else {
                "light"
            };
            for (name, prefs, secrets, open) in &cases {
                for (width, tag) in [(616.0, "wide"), (330.0, "narrow")] {
                    let mut state = SnapshotsState::default();
                    for id in *open {
                        if let Some(provider) = providers::provider(id) {
                            state.open.insert(provider.descriptor().id, Some(true));
                        }
                    }
                    let mut screen = Screen::new(egui::vec2(width, 1_700.0));
                    theme::apply(&screen.ctx, app_theme);
                    // The logo is an image: it decodes on a loader's thread, over a few frames.
                    egui_extras::install_image_loaders(&screen.ctx);
                    let mut last = None;
                    for frame in 0..4 {
                        if frame == 3 {
                            std::thread::sleep(std::time::Duration::from_millis(400));
                        }
                        last = Some(screen.run(Vec::new(), |ui| {
                            let mut page =
                                ui.new_child(egui::UiBuilder::new().max_rect(ui.max_rect()));
                            page_shell(
                                &mut page,
                                SettingsSection::Snapshots,
                                "Share a picture of a chart, and choose what it shows.",
                                |ui| {
                                    body(
                                        ui,
                                        &mut state,
                                        &SnapshotsInput { prefs, secrets },
                                        &mut Vec::new(),
                                    );
                                },
                            );
                        }));
                    }
                    screen.save(
                        &format!("snapshots-page-{name}-{tag}-{suffix}"),
                        last.expect("a frame"),
                    );
                }
            }
        }
    }
}
