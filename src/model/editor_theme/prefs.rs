//! What a person has chosen for the editor: which theme and which typography.

use super::{BuiltInTheme, ThemeId, Typography};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;

/// The `setting` key the choices are stored under.
pub const KEY: &str = "editor.prefs";

/// The theme the editor wears.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum ThemeSelection {
    /// Dark in a dark app, Light in a light app: what the editor did before themes, and what an
    /// upgrade starts as.
    #[default]
    Auto,
    BuiltIn(BuiltInTheme),
    Custom(ThemeId),
}

const CUSTOM_PREFIX: &str = "custom.";

impl ThemeSelection {
    /// The text stored and written to files: `auto`, `builtin.dracula`, `custom.1730000000000`.
    pub fn as_text(self) -> String {
        match self {
            Self::Auto => "auto".to_owned(),
            Self::BuiltIn(theme) => theme.id().to_owned(),
            Self::Custom(id) => format!("{CUSTOM_PREFIX}{id}"),
        }
    }

    /// The selection a text names. Anything else (a theme from a newer version, a mistake) is
    /// `Auto`, so a bad value can never leave the editor without a theme.
    pub fn from_text(text: &str) -> Self {
        if let Some(theme) = BuiltInTheme::from_id(text) {
            return Self::BuiltIn(theme);
        }
        text.strip_prefix(CUSTOM_PREFIX)
            .and_then(|id| id.parse().ok())
            .map_or(Self::Auto, Self::Custom)
    }
}

impl Serialize for ThemeSelection {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.as_text())
    }
}

impl<'de> Deserialize<'de> for ThemeSelection {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        Ok(value.as_str().map_or(Self::Auto, Self::from_text))
    }
}

/// The editor's choices, stored whole under [`KEY`]. Every field is read on its own: a value this
/// version cannot read costs that field only, and fields it does not know are kept.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct EditorPrefs {
    pub version: u32,
    pub selected: ThemeSelection,
    #[serde(deserialize_with = "super::typography::tolerant")]
    pub typography: Typography,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

pub const VERSION: u32 = 1;

impl Default for EditorPrefs {
    fn default() -> Self {
        Self {
            version: VERSION,
            selected: ThemeSelection::Auto,
            typography: Typography::default(),
            extra: BTreeMap::new(),
        }
    }
}

impl EditorPrefs {
    /// These with every value made valid. A newer file keeps its version number: this build
    /// writes back what it read and only changes what the person changed.
    pub fn sanitized(mut self) -> Self {
        self.typography = self.typography.sanitized();
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selections_round_trip_as_text() {
        for selection in [
            ThemeSelection::Auto,
            ThemeSelection::BuiltIn(BuiltInTheme::Dark),
            ThemeSelection::BuiltIn(BuiltInTheme::Dracula),
            ThemeSelection::Custom(1_730_000_000_000_000_000),
        ] {
            assert_eq!(ThemeSelection::from_text(&selection.as_text()), selection);
        }
    }

    #[test]
    fn a_selection_that_names_nothing_is_automatic() {
        for text in [
            "",
            "builtin.nope",
            "custom.",
            "custom.abc",
            "custom.-1",
            "dracula",
            "AUTO",
        ] {
            assert_eq!(
                ThemeSelection::from_text(text),
                ThemeSelection::Auto,
                "{text:?}"
            );
        }
        let prefs: EditorPrefs = serde_json::from_str(r#"{"selected": 12}"#).unwrap();
        assert_eq!(prefs.selected, ThemeSelection::Auto);
    }

    #[test]
    fn nothing_stored_is_what_the_editor_always_was() {
        let prefs: EditorPrefs = serde_json::from_str("{}").unwrap();
        assert_eq!(prefs, EditorPrefs::default());
        assert_eq!(prefs.selected, ThemeSelection::Auto);
        assert_eq!(prefs.typography, Typography::default());
    }

    #[test]
    fn what_cannot_be_read_costs_that_field_only() {
        let prefs: EditorPrefs = serde_json::from_str(
            r#"{"selected":"builtin.dracula","typography":"sideways","version":1}"#,
        )
        .unwrap();
        assert_eq!(
            prefs.selected,
            ThemeSelection::BuiltIn(BuiltInTheme::Dracula)
        );
        assert_eq!(prefs.typography, Typography::default());
    }

    #[test]
    fn fields_from_a_newer_version_are_kept() {
        let text = r#"{"version":2,"selected":"auto","wrap":{"on":true}}"#;
        let prefs: EditorPrefs = serde_json::from_str(text).unwrap();
        assert_eq!(prefs.version, 2);
        let written = serde_json::to_value(&prefs).unwrap();
        assert_eq!(written["wrap"]["on"], true);
        assert_eq!(written["version"], 2);
    }

    #[test]
    fn sanitizing_clamps_the_typography() {
        let mut prefs = EditorPrefs::default();
        prefs.typography.font_size = 99.0;
        assert_eq!(
            prefs.sanitized().typography.font_size,
            super::super::MAX_FONT_SIZE
        );
    }
}
