//! Which provider and model the assistant uses. Saved as the local `ai.prefs` setting: it names
//! the user's own account and server, so it never syncs. The API keys are not here; they live in
//! the Keychain (`keychain_ai_keys`).

use super::ProviderKind;
use crate::store::Store;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AiPrefs {
    pub provider: ProviderKind,
    pub openai_model: String,
    pub anthropic_model: String,
    pub compatible_model: String,
    /// The server the compatible provider talks to.
    pub compatible_base_url: String,
    /// Apply a clean change from an Edit request to the editor without asking. Off (Manual) until
    /// the user turns it on. A safety choice about this user's own scripts, so it stays on this Mac
    /// with the rest of `ai.prefs`.
    pub auto_apply: bool,
}

impl AiPrefs {
    pub const KEY: &'static str = "ai.prefs";

    pub fn load(store: &Store) -> Self {
        store.setting(Self::KEY).unwrap_or_default()
    }

    pub fn save(&self, store: &Store) -> Result<(), String> {
        store
            .set_setting(Self::KEY, self)
            .map_err(|error| error.to_string())
    }

    /// The model for `kind`: what the user typed, else that provider's suggestion.
    pub fn model(&self, kind: ProviderKind) -> String {
        let typed = match kind {
            ProviderKind::OpenAi => &self.openai_model,
            ProviderKind::Anthropic => &self.anthropic_model,
            ProviderKind::Compatible => &self.compatible_model,
        }
        .trim();
        if typed.is_empty() {
            kind.default_model().to_owned()
        } else {
            typed.to_owned()
        }
    }

    pub fn set_model(&mut self, kind: ProviderKind, model: &str) {
        let model = model.trim().to_owned();
        match kind {
            ProviderKind::OpenAi => self.openai_model = model,
            ProviderKind::Anthropic => self.anthropic_model = model,
            ProviderKind::Compatible => self.compatible_model = model,
        }
    }

    /// The base URL to use for `kind`; empty means the provider's own.
    pub fn base_url(&self, kind: ProviderKind) -> &str {
        match kind {
            ProviderKind::Compatible => self.compatible_base_url.trim(),
            _ => "",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_or_partial_setting_decodes_to_defaults() {
        let prefs: AiPrefs = serde_json::from_str("{}").unwrap();
        assert_eq!(prefs, AiPrefs::default());
        let prefs: AiPrefs = serde_json::from_str(r#"{"provider":"anthropic"}"#).unwrap();
        assert_eq!(prefs.provider, ProviderKind::Anthropic);
    }

    #[test]
    fn auto_apply_is_off_unless_it_was_saved_on() {
        assert!(!AiPrefs::default().auto_apply);
        let old: AiPrefs = serde_json::from_str(r#"{"provider":"openAi"}"#).unwrap();
        assert!(!old.auto_apply);
        let on: AiPrefs = serde_json::from_str(r#"{"autoApply":true}"#).unwrap();
        assert!(on.auto_apply);
    }

    #[test]
    fn the_model_falls_back_to_the_providers_suggestion() {
        let mut prefs = AiPrefs::default();
        assert_eq!(
            prefs.model(ProviderKind::Anthropic),
            ProviderKind::Anthropic.default_model()
        );
        prefs.set_model(ProviderKind::Anthropic, "  my-model ");
        assert_eq!(prefs.model(ProviderKind::Anthropic), "my-model");
        assert_eq!(
            prefs.model(ProviderKind::OpenAi),
            ProviderKind::OpenAi.default_model()
        );
        prefs.set_model(ProviderKind::Anthropic, "   ");
        assert_eq!(
            prefs.model(ProviderKind::Anthropic),
            ProviderKind::Anthropic.default_model()
        );
    }

    #[test]
    fn only_the_compatible_provider_has_its_own_base_url() {
        let prefs = AiPrefs {
            compatible_base_url: " http://localhost:11434/v1 ".to_owned(),
            ..AiPrefs::default()
        };
        assert_eq!(
            prefs.base_url(ProviderKind::Compatible),
            "http://localhost:11434/v1"
        );
        assert_eq!(prefs.base_url(ProviderKind::OpenAi), "");
    }

    #[test]
    fn prefs_round_trip_through_the_store_and_stay_local() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(AiPrefs::load(&store), AiPrefs::default());
        let mut prefs = AiPrefs {
            provider: ProviderKind::Compatible,
            ..AiPrefs::default()
        };
        prefs.set_model(ProviderKind::Compatible, "llama");
        prefs.auto_apply = true;
        prefs.save(&store).unwrap();
        assert_eq!(AiPrefs::load(&store), prefs);
        assert_eq!(
            crate::sync::policy::setting_policy(AiPrefs::KEY),
            crate::sync::policy::SettingPolicy::Local
        );
    }
}
