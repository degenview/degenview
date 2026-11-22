//! What the person chose in Settings: follow the system, or one language.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::locale::Locale;

/// The language setting. Saved as `"system"` or the locale's tag.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Preference {
    /// The first of the system's languages the app has.
    #[default]
    System,
    Fixed(Locale),
}

const SYSTEM: &str = "system";

impl Serialize for Preference {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(match self {
            Self::System => SYSTEM,
            Self::Fixed(locale) => locale.tag(),
        })
    }
}

impl<'de> Deserialize<'de> for Preference {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        if text == SYSTEM {
            return Ok(Self::System);
        }
        Locale::from_tag(&text)
            .map(Self::Fixed)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown language {text:?}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_choice_is_saved_as_a_word_or_a_tag() {
        assert_eq!(
            serde_json::to_string(&Preference::System).unwrap(),
            "\"system\""
        );
        assert_eq!(
            serde_json::to_string(&Preference::Fixed(Locale::PSEUDO)).unwrap(),
            "\"en-XA\""
        );
    }

    #[test]
    fn every_choice_reads_back_as_it_was_saved() {
        let all = std::iter::once(Preference::System).chain(Locale::all().map(Preference::Fixed));
        for preference in all {
            let saved = serde_json::to_string(&preference).unwrap();
            assert_eq!(
                serde_json::from_str::<Preference>(&saved).unwrap(),
                preference
            );
        }
    }

    #[test]
    fn a_language_this_build_does_not_have_is_an_error_not_the_default() {
        assert!(serde_json::from_str::<Preference>("\"tlh\"").is_err());
        assert!(serde_json::from_str::<Preference>("7").is_err());
    }
}
