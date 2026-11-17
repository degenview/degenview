//! How the user wants paper trading to behave on this Mac. Local (`paper.prefs` is never synced):
//! whether this Mac makes a sound is not another Mac's business.

use serde::{Deserialize, Serialize};

pub const KEY: &str = "paper.prefs";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct PaperPrefs {
    /// Play a sound when a paper order is filled.
    pub fill_sound: bool,
}

impl Default for PaperPrefs {
    fn default() -> Self {
        Self { fill_sound: true }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sound_is_on_by_default_and_for_a_value_saved_without_it() {
        assert!(PaperPrefs::default().fill_sound);
        let saved: PaperPrefs = serde_json::from_str("{}").unwrap();
        assert!(saved.fill_sound);
    }

    #[test]
    fn a_choice_survives_a_round_trip() {
        let muted = PaperPrefs { fill_sound: false };
        let text = serde_json::to_string(&muted).unwrap();
        assert_eq!(serde_json::from_str::<PaperPrefs>(&text).unwrap(), muted);
    }
}
