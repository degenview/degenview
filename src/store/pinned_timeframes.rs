//! The custom timeframes pinned in the timeframe picker (`kit::timeframe_picker`), kept as one
//! `setting` holding their labels. The list is shared by every chart tab and Pine Studio. The
//! presets are always listed and are never pinned.

use super::Store;
use crate::model::Timeframe;

/// Where the list is kept (`sync::record::policy` lists it as a portable setting).
pub const KEY: &str = "timeframePicker.pinned";

impl Store {
    /// The pinned timeframes in the order they were pinned. A label this build cannot read, a
    /// preset and a repeat are dropped.
    pub fn pinned_timeframes(&self) -> Vec<Timeframe> {
        let labels = self.setting::<Vec<String>>(KEY).unwrap_or_default();
        let mut pinned: Vec<Timeframe> = Vec::new();
        for timeframe in labels
            .iter()
            .filter_map(|label| Timeframe::parse(label).ok())
        {
            if !Timeframe::PRESETS.contains(&timeframe) && !pinned.contains(&timeframe) {
                pinned.push(timeframe);
            }
        }
        pinned
    }

    /// Pins `timeframe`; a preset or one already pinned changes nothing.
    pub fn pin_timeframe(&self, timeframe: Timeframe) -> rusqlite::Result<()> {
        let mut pinned = self.pinned_timeframes();
        if Timeframe::PRESETS.contains(&timeframe) || pinned.contains(&timeframe) {
            return Ok(());
        }
        pinned.push(timeframe);
        self.set_pinned_timeframes(&pinned)
    }

    /// Removes `timeframe` from the pinned ones.
    pub fn unpin_timeframe(&self, timeframe: Timeframe) -> rusqlite::Result<()> {
        let mut pinned = self.pinned_timeframes();
        let before = pinned.len();
        pinned.retain(|saved| *saved != timeframe);
        if pinned.len() == before {
            return Ok(());
        }
        self.set_pinned_timeframes(&pinned)
    }

    fn set_pinned_timeframes(&self, pinned: &[Timeframe]) -> rusqlite::Result<()> {
        let labels: Vec<String> = pinned.iter().map(|timeframe| timeframe.label()).collect();
        self.set_setting(KEY, &labels)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TimeUnit;

    fn custom(count: u16, unit: TimeUnit) -> Timeframe {
        Timeframe::new(count, unit).expect("in range")
    }

    #[test]
    fn a_pinned_timeframe_is_kept_by_its_label() {
        let store = Store::open_in_memory().expect("open");
        assert!(store.pinned_timeframes().is_empty());
        store.pin_timeframe(custom(22, TimeUnit::Minute)).unwrap();
        store.pin_timeframe(custom(2, TimeUnit::Day)).unwrap();
        assert_eq!(
            store.setting::<Vec<String>>(KEY),
            Some(vec!["22m".to_owned(), "2D".to_owned()])
        );
        assert_eq!(
            store.pinned_timeframes(),
            [custom(22, TimeUnit::Minute), custom(2, TimeUnit::Day)]
        );
    }

    #[test]
    fn presets_and_repeats_are_not_pinned() {
        let store = Store::open_in_memory().expect("open");
        store.pin_timeframe(Timeframe::HOUR_1).unwrap();
        store.pin_timeframe(custom(22, TimeUnit::Minute)).unwrap();
        store.pin_timeframe(custom(22, TimeUnit::Minute)).unwrap();
        assert_eq!(store.pinned_timeframes(), [custom(22, TimeUnit::Minute)]);
    }

    #[test]
    fn unpinning_removes_only_that_timeframe() {
        let store = Store::open_in_memory().expect("open");
        store.pin_timeframe(custom(22, TimeUnit::Minute)).unwrap();
        store.pin_timeframe(custom(3, TimeUnit::Hour)).unwrap();
        store.unpin_timeframe(custom(22, TimeUnit::Minute)).unwrap();
        assert_eq!(store.pinned_timeframes(), [custom(3, TimeUnit::Hour)]);
        store.unpin_timeframe(custom(9, TimeUnit::Hour)).unwrap();
        assert_eq!(store.pinned_timeframes(), [custom(3, TimeUnit::Hour)]);
    }

    #[test]
    fn a_label_that_does_not_parse_is_dropped_on_load() {
        let store = Store::open_in_memory().expect("open");
        store
            .set_setting(
                KEY,
                &["22m", "60s", "1h", "nonsense", "22m", "3h"].map(str::to_owned),
            )
            .unwrap();
        assert_eq!(
            store.pinned_timeframes(),
            [custom(22, TimeUnit::Minute), custom(3, TimeUnit::Hour)]
        );
    }

    #[test]
    fn the_cached_read_follows_a_write() {
        let store = Store::open_in_memory().expect("open");
        let first = store.pinned_timeframes_cached();
        assert!(first.is_empty());
        assert!(std::sync::Arc::ptr_eq(
            &first,
            &store.pinned_timeframes_cached()
        ));
        store.pin_timeframe(custom(22, TimeUnit::Minute)).unwrap();
        assert_eq!(
            *store.pinned_timeframes_cached(),
            [custom(22, TimeUnit::Minute)]
        );
    }
}
