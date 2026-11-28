//! The chart time zone the user chose (Settings ▸ Appearance ▸ Time zone), kept as one
//! `setting`.

use super::Store;
use crate::model::timezone::ChartTimezone;

pub const KEY: &str = "chart.timezone";

impl Store {
    /// The saved chart time zone; follows the system when none was saved or when the saved value
    /// cannot be decoded.
    pub fn chart_timezone(&self) -> ChartTimezone {
        self.setting::<ChartTimezone>(KEY).unwrap_or_default()
    }

    pub fn set_chart_timezone(&self, tz: ChartTimezone) -> rusqlite::Result<()> {
        self.set_setting(KEY, &tz)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_saved_follows_the_system() {
        let store = Store::open_in_memory().expect("store");
        assert_eq!(store.chart_timezone(), ChartTimezone::System);
    }

    #[test]
    fn a_choice_is_read_back() {
        let store = Store::open_in_memory().expect("store");
        store
            .set_chart_timezone(ChartTimezone::Tokyo)
            .expect("save");
        assert_eq!(store.chart_timezone(), ChartTimezone::Tokyo);
        store
            .set_chart_timezone(ChartTimezone::System)
            .expect("save");
        assert_eq!(store.chart_timezone(), ChartTimezone::System);
    }

    #[test]
    fn an_unreadable_value_falls_back_to_system() {
        let store = Store::open_in_memory().expect("store");
        store.set_setting(KEY, &"mars_olympus").expect("save");
        assert_eq!(store.chart_timezone(), ChartTimezone::System);
    }
}
