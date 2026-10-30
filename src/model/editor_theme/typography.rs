//! How the editor sets its text: font family, size and line height. A person sets these once for
//! the editor ([`Typography`]); a theme may carry its own ([`TypographyOverride`]) for the fields it
//! names, so a shared theme can bring its font along.
//!
//! Ligatures, font weight and tab width are not settings: egui's text engine shapes with the
//! font's default features and has no weight axis for a plain font, and the editor indents with
//! four spaces. A control for any of them would do nothing.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};

pub const MIN_FONT_SIZE: f32 = 9.0;
pub const MAX_FONT_SIZE: f32 = 32.0;
/// The size the editor has always used.
pub const DEFAULT_FONT_SIZE: f32 = 13.0;
pub const MAX_FAMILY_CHARS: usize = 100;

/// The height of a line of code, as a multiple of the font size.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LineHeight {
    /// Tighter than the font's own.
    Compact,
    /// Whatever the font asks for.
    #[default]
    Default,
    Relaxed,
    Loose,
}

impl LineHeight {
    pub const ALL: [Self; 4] = [Self::Compact, Self::Default, Self::Relaxed, Self::Loose];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Compact => "Compact",
            Self::Default => "Default",
            Self::Relaxed => "Relaxed",
            Self::Loose => "Loose",
        }
    }

    /// The multiple of the font size, or `None` for the font's own height.
    pub const fn factor(self) -> Option<f32> {
        match self {
            Self::Compact => Some(1.15),
            Self::Default => None,
            Self::Relaxed => Some(1.5),
            Self::Loose => Some(1.75),
        }
    }

    /// The name a theme file uses.
    pub const fn key(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Default => "default",
            Self::Relaxed => "relaxed",
            Self::Loose => "loose",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|height| height.key() == key)
    }
}

/// A family name as a person or a file may give it: trimmed, not empty, not absurdly long, no
/// control characters. `None` when it is none of those.
pub fn clean_family(raw: &str) -> Option<String> {
    let family = raw.trim();
    (!family.is_empty()
        && family.chars().count() <= MAX_FAMILY_CHARS
        && !family.chars().any(char::is_control))
    .then(|| family.to_owned())
}

/// A size in range, rounded to the nearest half point; a size that is not a number is the default.
pub fn clamp_size(size: f32) -> f32 {
    if size.is_nan() {
        return DEFAULT_FONT_SIZE;
    }
    ((size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE)) * 2.0).round() / 2.0
}

/// A field that falls back to its default when what is stored cannot be read, so one value this
/// version does not understand never loses the rest.
pub(super) fn tolerant<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned + Default,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).unwrap_or_default())
}

/// A size that falls back to the default when what is stored is not a number.
fn tolerant_size<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f32, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(value.as_f64().map_or(DEFAULT_FONT_SIZE, |size| size as f32))
}

/// The editor's typography.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Typography {
    /// `None` is the system's monospace font: SF Mono on a Mac, the bundled one elsewhere.
    #[serde(deserialize_with = "tolerant")]
    pub font_family: Option<String>,
    #[serde(deserialize_with = "tolerant_size")]
    pub font_size: f32,
    #[serde(deserialize_with = "tolerant")]
    pub line_height: LineHeight,
}

impl Default for Typography {
    fn default() -> Self {
        Self {
            font_family: None,
            font_size: DEFAULT_FONT_SIZE,
            line_height: LineHeight::Default,
        }
    }
}

impl Typography {
    /// This with every value made valid.
    pub fn sanitized(mut self) -> Self {
        self.font_family = self.font_family.as_deref().and_then(clean_family);
        self.font_size = clamp_size(self.font_size);
        self
    }

    /// This, with the fields `theme` names replacing the ones here.
    pub fn with(&self, theme: &TypographyOverride) -> Self {
        Self {
            font_family: theme
                .font_family
                .clone()
                .or_else(|| self.font_family.clone()),
            font_size: theme.font_size.unwrap_or(self.font_size),
            line_height: theme.line_height.unwrap_or(self.line_height),
        }
        .sanitized()
    }
}

/// The typography a theme carries: only the fields it names.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TypographyOverride {
    #[serde(deserialize_with = "tolerant", skip_serializing_if = "Option::is_none")]
    pub font_family: Option<String>,
    #[serde(deserialize_with = "tolerant", skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f32>,
    #[serde(deserialize_with = "tolerant", skip_serializing_if = "Option::is_none")]
    pub line_height: Option<LineHeight>,
}

impl TypographyOverride {
    pub fn is_empty(&self) -> bool {
        self.font_family.is_none() && self.font_size.is_none() && self.line_height.is_none()
    }

    pub fn sanitized(mut self) -> Self {
        self.font_family = self.font_family.as_deref().and_then(clean_family);
        self.font_size = self.font_size.filter(|size| !size.is_nan()).map(clamp_size);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_kept_in_range_and_to_half_points() {
        assert_eq!(clamp_size(13.0), 13.0);
        assert_eq!(clamp_size(2.0), MIN_FONT_SIZE);
        assert_eq!(clamp_size(400.0), MAX_FONT_SIZE);
        assert_eq!(clamp_size(13.26), 13.5);
        assert_eq!(clamp_size(f32::NAN), DEFAULT_FONT_SIZE);
        assert_eq!(clamp_size(f32::INFINITY), MAX_FONT_SIZE);
        assert_eq!(clamp_size(f32::NEG_INFINITY), MIN_FONT_SIZE);
    }

    #[test]
    fn families_are_cleaned() {
        assert_eq!(
            clean_family("  JetBrains Mono "),
            Some("JetBrains Mono".into())
        );
        assert_eq!(clean_family("Fira Código"), Some("Fira Código".into()));
        assert_eq!(clean_family("   "), None);
        assert_eq!(clean_family("a\nb"), None);
        assert_eq!(clean_family(&"x".repeat(MAX_FAMILY_CHARS + 1)), None);
        assert!(clean_family(&"x".repeat(MAX_FAMILY_CHARS)).is_some());
    }

    #[test]
    fn line_heights_have_names_and_factors() {
        assert_eq!(LineHeight::Default.factor(), None);
        assert!(LineHeight::Compact.factor() < LineHeight::Relaxed.factor());
        assert!(LineHeight::Relaxed.factor() < LineHeight::Loose.factor());
        for height in LineHeight::ALL {
            assert_eq!(LineHeight::from_key(height.key()), Some(height));
        }
    }

    #[test]
    fn a_theme_overrides_only_what_it_names() {
        let base = Typography {
            font_family: Some("Menlo".into()),
            font_size: 14.0,
            line_height: LineHeight::Relaxed,
        };
        let none = TypographyOverride::default();
        assert!(none.is_empty());
        assert_eq!(base.with(&none), base);
        let some = TypographyOverride {
            font_size: Some(18.0),
            ..Default::default()
        };
        let merged = base.with(&some);
        assert_eq!(merged.font_size, 18.0);
        assert_eq!(merged.font_family.as_deref(), Some("Menlo"));
        assert_eq!(merged.line_height, LineHeight::Relaxed);
    }

    #[test]
    fn unreadable_fields_fall_back_alone() {
        let text = r#"{"fontFamily": 7, "fontSize": 16.0, "lineHeight": "enormous"}"#;
        let typography: Typography = serde_json::from_str(text).unwrap();
        assert_eq!(typography.font_family, None);
        assert_eq!(typography.font_size, 16.0);
        let text = r#"{"fontFamily": "Menlo", "fontSize": "big"}"#;
        let typography: Typography = serde_json::from_str(text).unwrap();
        assert_eq!(typography.font_family.as_deref(), Some("Menlo"));
        assert_eq!(typography.font_size, DEFAULT_FONT_SIZE);
        assert_eq!(typography.line_height, LineHeight::Default);
        let empty: Typography = serde_json::from_str("{}").unwrap();
        assert_eq!(empty, Typography::default());
    }

    #[test]
    fn sanitizing_makes_everything_valid() {
        let wild = Typography {
            font_family: Some("\u{7}".into()),
            font_size: 1000.0,
            line_height: LineHeight::Loose,
        }
        .sanitized();
        assert_eq!(wild.font_family, None);
        assert_eq!(wild.font_size, MAX_FONT_SIZE);
        let theme = TypographyOverride {
            font_family: Some("  ".into()),
            font_size: Some(f32::NAN),
            line_height: None,
        }
        .sanitized();
        assert!(theme.is_empty());
    }

    #[test]
    fn an_override_writes_only_its_fields() {
        let some = TypographyOverride {
            font_size: Some(15.0),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_string(&some).unwrap(),
            r#"{"fontSize":15.0}"#
        );
    }
}
