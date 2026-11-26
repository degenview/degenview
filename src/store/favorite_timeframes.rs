//! The favorite timeframes shown directly on the toolbar and starred in the timeframe picker.
//! Kept as one `setting` holding their labels (e.g. ["15m", "30m", "1h", "4h", "1D", "1W"]).

use super::Store;
use crate::model::Timeframe;

/// Where the list is kept (`sync::record::policy` lists it as a portable setting).
pub const KEY: &str = "timeframePicker.favorites";

impl Store {
    /// The favorite timeframes in sorted order.
    pub fn favorite_timeframes(&self) -> Vec<Timeframe> {
        let labels = self.setting::<Vec<String>>(KEY).unwrap_or_default();
        let mut favorites: Vec<Timeframe> = Vec::new();
        for timeframe in labels
            .iter()
            .filter_map(|label| Timeframe::parse(label).ok())
        {
            if !favorites.contains(&timeframe) {
                favorites.push(timeframe);
            }
        }
        favorites.sort();
        favorites
    }

    /// Toggles `timeframe` in the favorite list.
    pub fn toggle_favorite_timeframe(&self, timeframe: Timeframe) -> rusqlite::Result<()> {
        let mut favorites = self.favorite_timeframes();
        if let Some(pos) = favorites.iter().position(|t| *t == timeframe) {
            favorites.remove(pos);
        } else {
            favorites.push(timeframe);
        }
        favorites.sort();
        self.set_favorite_timeframes(&favorites)
    }

    pub fn set_favorite_timeframes(&self, favorites: &[Timeframe]) -> rusqlite::Result<()> {
        let labels: Vec<String> = favorites.iter().map(|tf| tf.label()).collect();
        self.set_setting(KEY, &labels)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn favorite_timeframes_toggle_and_sort() {
        let store = Store::open_in_memory().expect("open");
        assert!(store.favorite_timeframes().is_empty());

        store
            .toggle_favorite_timeframe(Timeframe::HOUR_1)
            .expect("toggle 1h");
        store
            .toggle_favorite_timeframe(Timeframe::MINUTE_15)
            .expect("toggle 15m");
        // Check sorted by duration: 15m comes before 1h.
        assert_eq!(
            store.favorite_timeframes(),
            vec![Timeframe::MINUTE_15, Timeframe::HOUR_1]
        );

        // Toggle 1h off.
        store
            .toggle_favorite_timeframe(Timeframe::HOUR_1)
            .expect("toggle 1h off");
        assert_eq!(store.favorite_timeframes(), vec![Timeframe::MINUTE_15]);
    }
}
