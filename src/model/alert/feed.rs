#![allow(dead_code)]
//! What the Alerts window and the sidebar bell show about alerts that already fired:
//! how many are unseen, and the history bucketed under day headings.

use crate::format::civil_from_days;
use crate::model::alert::AlertDomainEvent;
use std::collections::HashSet;

/// The `setting` key that holds the last-seen timestamp.
pub const LAST_SEEN_KEY: &str = "alerts.lastSeenAt";

/// How many alerts have fired since the user last looked at the Alerts window; drives the
/// red bubble on the sidebar bell.
///
/// "Seen" is a timestamp, not a flag on the events: the alert snapshot rewrites its event
/// rows, and the agent can fire alerts while the app is closed. Those arrive through a
/// reload after launch and count when they postdate it. Each method that changes the
/// timestamp returns the new value, for the caller to save under `LAST_SEEN_KEY`.
#[derive(Debug)]
pub struct UnseenAlerts {
    last_seen_ms: i64,
    unseen: HashSet<u64>,
    window_active: bool,
}

impl UnseenAlerts {
    /// A first run has nothing to compare against, so history from before this build is
    /// not "new": the second value is the timestamp to save when there was none.
    pub fn new(saved_last_seen_ms: Option<i64>, now_ms: i64) -> (Self, Option<i64>) {
        let (last_seen_ms, to_save) = match saved_last_seen_ms {
            Some(saved) => (saved, None),
            None => (now_ms, Some(now_ms)),
        };
        (
            Self {
                last_seen_ms,
                unseen: HashSet::new(),
                window_active: false,
            },
            to_save,
        )
    }

    pub fn count(&self) -> usize {
        self.unseen.len()
    }

    pub fn last_seen_ms(&self) -> i64 {
        self.last_seen_ms
    }

    pub fn window_is_active(&self) -> bool {
        self.window_active
    }

    /// The Alerts window became the key window: everything so far is seen, and so is what
    /// arrives while it stays.
    pub fn window_did_become_active(&mut self, now_ms: i64) -> i64 {
        self.window_active = true;
        self.mark_all_seen(now_ms)
    }

    pub fn window_did_resign(&mut self) {
        self.window_active = false;
    }

    /// Feeds one published event. Returns a new last-seen timestamp to save, if it moved.
    pub fn receive(&mut self, event: AlertDomainEvent) -> Option<i64> {
        if event.occurred_at_ms() <= self.last_seen_ms {
            return None;
        }
        if self.window_active {
            return Some(self.mark_all_seen(event.occurred_at_ms()));
        }
        self.unseen.insert(event.event_id());
        None
    }

    fn mark_all_seen(&mut self, up_to_ms: i64) -> i64 {
        self.unseen.clear();
        self.last_seen_ms = self.last_seen_ms.max(up_to_ms);
        self.last_seen_ms
    }
}

// MARK: Day grouping

pub struct DaySection<T> {
    /// Midnight of the day, in the viewer's time zone, as a UTC instant.
    pub day_start_ms: i64,
    pub title: String,
    pub items: Vec<T>,
}

const WEEKDAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const DAY_MS: i64 = 86_400_000;

/// Buckets time-stamped items under "Today", "Yesterday" and dated headings, newest day
/// first. Items inside a day keep the newest-first order of the whole list; an empty input
/// gives no sections. `utc_offset_secs` is the viewer's offset from UTC.
pub fn day_sections<T>(
    items: Vec<T>,
    date_ms: impl Fn(&T) -> i64,
    now_ms: i64,
    utc_offset_secs: i64,
) -> Vec<DaySection<T>> {
    let offset_ms = utc_offset_secs * 1_000;
    let day_start = |ms: i64| (ms + offset_ms).div_euclid(DAY_MS) * DAY_MS - offset_ms;
    let today = day_start(now_ms);

    let mut ordered = items;
    ordered.sort_by_key(|item| std::cmp::Reverse(date_ms(item)));

    let mut sections: Vec<DaySection<T>> = Vec::new();
    for item in ordered {
        let day = day_start(date_ms(&item));
        match sections.last_mut() {
            Some(section) if section.day_start_ms == day => section.items.push(item),
            _ => sections.push(DaySection {
                day_start_ms: day,
                title: day_title(day, today, offset_ms),
                items: vec![item],
            }),
        }
    }
    sections
}

fn day_title(day_start_ms: i64, today_start_ms: i64, offset_ms: i64) -> String {
    if day_start_ms == today_start_ms {
        return "Today".to_owned();
    }
    if day_start_ms == today_start_ms - DAY_MS {
        return "Yesterday".to_owned();
    }
    let days = (day_start_ms + offset_ms).div_euclid(DAY_MS);
    let (year, month, day) = civil_from_days(days);
    let (today_year, ..) = civil_from_days((today_start_ms + offset_ms).div_euclid(DAY_MS));
    // 1970-01-01 was a Thursday; Monday = 0.
    let weekday = WEEKDAYS[(days + 3).rem_euclid(7) as usize];
    let mut title = format!("{weekday}, {} {day}", MONTHS[(month - 1) as usize]);
    if year != today_year {
        title.push_str(&format!(", {year}"));
    }
    title
}

#[cfg(test)]
mod tests {
    use super::*;

    fn price_event(id: u64, at_ms: i64) -> AlertDomainEvent {
        AlertDomainEvent::PriceAlertTriggered {
            event_id: id,
            at_ms,
        }
    }

    #[test]
    fn a_first_run_treats_old_history_as_seen() {
        let (unseen, save) = UnseenAlerts::new(None, 1_000);
        assert_eq!(save, Some(1_000));
        assert_eq!(unseen.last_seen_ms(), 1_000);
        let (resumed, save) = UnseenAlerts::new(Some(500), 1_000);
        assert_eq!((resumed.last_seen_ms(), save), (500, None));
    }

    #[test]
    fn only_events_after_the_last_look_count_and_each_counts_once() {
        let (mut unseen, _) = UnseenAlerts::new(Some(1_000), 5_000);
        assert_eq!(unseen.receive(price_event(1, 900)), None);
        assert_eq!(
            unseen.receive(price_event(2, 1_000)),
            None,
            "at the timestamp is not after it"
        );
        assert_eq!(unseen.count(), 0);
        unseen.receive(price_event(3, 2_000));
        unseen.receive(price_event(3, 2_000));
        unseen.receive(AlertDomainEvent::ScriptAlertTriggered {
            event_id: 4,
            at_ms: 2_500,
        });
        assert_eq!(
            unseen.count(),
            2,
            "a repeated id (a reload) is not counted twice"
        );
    }

    #[test]
    fn opening_the_window_clears_the_count_and_moves_the_timestamp() {
        let (mut unseen, _) = UnseenAlerts::new(Some(1_000), 5_000);
        unseen.receive(price_event(1, 2_000));
        assert_eq!(unseen.window_did_become_active(3_000), 3_000);
        assert_eq!(unseen.count(), 0);
        // Re-activating with an older clock never moves the timestamp backwards.
        unseen.window_did_resign();
        assert_eq!(unseen.window_did_become_active(2_000), 3_000);
    }

    #[test]
    fn alerts_firing_while_the_window_is_key_are_seen_at_once() {
        let (mut unseen, _) = UnseenAlerts::new(Some(1_000), 5_000);
        unseen.window_did_become_active(3_000);
        assert_eq!(unseen.receive(price_event(9, 4_000)), Some(4_000));
        assert_eq!(unseen.count(), 0);
        // After the window resigns, new ones count again.
        unseen.window_did_resign();
        unseen.receive(price_event(10, 5_000));
        assert_eq!(unseen.count(), 1);
        // A late-arriving older event is ignored.
        unseen.receive(price_event(11, 3_500));
        assert_eq!(unseen.count(), 1);
    }

    // 2024-01-17 (Wednesday) 12:00 UTC.
    const NOW: i64 = 1_705_492_800_000;

    #[test]
    fn items_group_under_today_yesterday_and_dated_headings() {
        let items = vec![
            ("older", NOW - 3 * DAY_MS),
            ("today-early", NOW - 2 * 3_600_000),
            ("yesterday", NOW - DAY_MS),
            ("today-late", NOW - 1_000),
            ("last-year", NOW - 400 * DAY_MS),
        ];
        let sections = day_sections(items, |item| item.1, NOW, 0);
        let titles: Vec<&str> = sections.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "Today",
                "Yesterday",
                "Sunday, January 14",
                "Tuesday, December 13, 2022"
            ]
        );
        let today: Vec<&str> = sections[0].items.iter().map(|item| item.0).collect();
        assert_eq!(
            today,
            ["today-late", "today-early"],
            "newest first inside a day"
        );
    }

    #[test]
    fn days_follow_the_viewers_time_zone() {
        // 23:30 UTC on Jan 16 is already Jan 17 in UTC+2.
        let late = NOW - 12 * 3_600_000 - 1_800_000;
        assert_eq!(
            day_sections(vec![late], |ms| *ms, NOW, 0)[0].title,
            "Yesterday"
        );
        assert_eq!(
            day_sections(vec![late], |ms| *ms, NOW, 2 * 3_600)[0].title,
            "Today"
        );
        assert_eq!(
            day_sections(vec![late], |ms| *ms, NOW, -5 * 3_600)[0].title,
            "Yesterday"
        );
    }

    #[test]
    fn empty_input_has_no_sections_and_days_before_the_epoch_work() {
        assert!(day_sections(Vec::<i64>::new(), |ms| *ms, NOW, 0).is_empty());
        let sections = day_sections(vec![-1_000_i64], |ms| *ms, 0, 0);
        assert_eq!(sections[0].title, "Yesterday");
        assert_eq!(sections[0].day_start_ms, -DAY_MS);
    }
}
