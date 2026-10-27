//! A Pine session string such as `"0930-1600"`, `"1800-0600"` (overnight),
//! `"0930-1130,1300-1600"` or `"0930-1600:23456"` (Monday to Friday; Sunday is 1). `"24x7"` is
//! always open. A port of `Builtins/PineSession.swift`.

use super::calendar::Components;
use std::collections::BTreeSet;

#[derive(Clone, Debug, Eq, PartialEq)]
struct Window {
    start: u32,
    end: u32,
    days: BTreeSet<u32>,
}

impl Window {
    fn contains(&self, minute: u32, weekday: u32) -> bool {
        if self.start == self.end {
            return self.days.contains(&weekday);
        }
        if self.start < self.end {
            return minute >= self.start && minute < self.end && self.days.contains(&weekday);
        }
        if minute >= self.start {
            return self.days.contains(&weekday);
        }
        // The after-midnight part of an overnight window belongs to the day it opened on.
        minute < self.end
            && self
                .days
                .contains(&if weekday == 1 { 7 } else { weekday - 1 })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TradingSession {
    windows: Vec<Window>,
}

fn all_days() -> BTreeSet<u32> {
    (1..=7).collect()
}

impl TradingSession {
    /// None when the text is not a session.
    pub fn parse(text: &str) -> Option<Self> {
        let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        if compact.eq_ignore_ascii_case("24x7") {
            return Some(Self {
                windows: vec![Window {
                    start: 0,
                    end: 0,
                    days: all_days(),
                }],
            });
        }
        let windows: Option<Vec<Window>> = compact.split(',').map(window).collect();
        windows
            .filter(|windows| !windows.is_empty())
            .map(|windows| Self { windows })
    }

    /// Whether the moment described by `components` falls inside the session.
    pub fn contains(&self, components: &Components) -> bool {
        let minute = (components.hour * 60 + components.minute) as u32;
        let weekday = components.weekday as u32;
        self.windows
            .iter()
            .any(|window| window.contains(minute, weekday))
    }
}

fn window(text: &str) -> Option<Window> {
    let sides: Vec<&str> = text.split(':').collect();
    if sides.len() > 2 {
        return None;
    }
    let (start, end) = sides[0].split_once('-')?;
    let (start, end) = (minute_of_day(start)?, minute_of_day(end)?);
    let mut days = all_days();
    if let Some(digits) = sides.get(1) {
        let parsed: Option<BTreeSet<u32>> = digits
            .chars()
            .map(|c| c.to_digit(10).filter(|d| (1..=7).contains(d)))
            .collect();
        days = parsed.filter(|set| !digits.is_empty() && !set.is_empty())?;
    }
    Some(Window { start, end, days })
}

fn minute_of_day(text: &str) -> Option<u32> {
    if text.len() != 4 || !text.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let value: u32 = text.parse().ok()?;
    let (hour, minute) = (value / 100, value % 100);
    (minute < 60 && hour <= 24 && (hour < 24 || minute == 0)).then_some(hour * 60 + minute)
}

/// When a market trades, which is what `session.*` reads: the regular session, the extended hours
/// around it and the zone they are kept in. Known for the two kinds of symbol the app lists:
/// crypto trades round the clock on UTC, and US stocks keep New York's hours.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketSessions {
    zone: &'static str,
    regular: TradingSession,
    premarket: Option<TradingSession>,
    postmarket: Option<TradingSession>,
}

impl MarketSessions {
    /// The sessions of a `syminfo.type`: `"stock"` is a US stock, anything else is taken to trade
    /// all day on UTC like the crypto the app mostly lists.
    pub fn for_kind(kind: &str) -> Self {
        let parse = |text: &str| TradingSession::parse(text).expect("a constant session parses");
        if kind == "stock" {
            Self {
                zone: "America/New_York",
                regular: parse("0930-1600:23456"),
                premarket: Some(parse("0400-0930:23456")),
                postmarket: Some(parse("1600-2000:23456")),
            }
        } else {
            Self {
                zone: "Etc/UTC",
                regular: parse("24x7"),
                premarket: None,
                postmarket: None,
            }
        }
    }

    /// The zone the sessions are kept in, as `syminfo.timezone` names it.
    pub fn zone(&self) -> &'static str {
        self.zone
    }

    fn local(&self, millis: i64) -> Components {
        super::time::components(
            millis,
            super::time::zone_offset_secs(Some(self.zone), millis),
        )
    }

    /// Whether the bar that opens at `millis` is part of the regular session.
    pub fn is_regular(&self, millis: i64) -> bool {
        self.regular.contains(&self.local(millis))
    }

    pub fn is_premarket(&self, millis: i64) -> bool {
        self.premarket
            .as_ref()
            .is_some_and(|session| session.contains(&self.local(millis)))
    }

    pub fn is_postmarket(&self, millis: i64) -> bool {
        self.postmarket
            .as_ref()
            .is_some_and(|session| session.contains(&self.local(millis)))
    }

    fn same_day(&self, first: i64, second: i64) -> bool {
        let (a, b) = (self.local(first), self.local(second));
        (a.year, a.month, a.day) == (b.year, b.month, b.day)
    }

    /// Whether the bar opening at `millis` starts a regular session: it is in one and the bar
    /// before it (`previous`, when there is one) was not in the same day's.
    pub fn starts_regular_session(&self, millis: i64, previous: Option<i64>) -> bool {
        self.is_regular(millis)
            && !previous
                .is_some_and(|before| self.is_regular(before) && self.same_day(before, millis))
    }

    /// Whether the bar opening at `millis` and lasting `length` ends a regular session: it is in
    /// one and the bar after it is not in the same day's.
    pub fn ends_regular_session(&self, millis: i64, length: i64) -> bool {
        let next = millis + length;
        self.is_regular(millis) && !(self.is_regular(next) && self.same_day(millis, next))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(hour: i64, minute: i64, weekday: i64) -> Components {
        Components {
            year: 2024,
            month: 3,
            day: 5,
            hour,
            minute,
            second: 0,
            weekday,
        }
    }

    fn session(text: &str) -> TradingSession {
        TradingSession::parse(text).unwrap_or_else(|| panic!("{text} should parse"))
    }

    #[test]
    fn a_plain_window_is_half_open() {
        let regular = session("0930-1600");
        assert!(!regular.contains(&at(9, 29, 3)));
        assert!(regular.contains(&at(9, 30, 3)));
        assert!(regular.contains(&at(15, 59, 3)));
        assert!(
            !regular.contains(&at(16, 0, 3)),
            "the end minute is outside"
        );
        assert!(
            regular.contains(&at(12, 0, 1)),
            "every day unless restricted"
        );
    }

    #[test]
    fn days_restrict_a_window() {
        let weekdays = session("0930-1600:23456");
        assert!(weekdays.contains(&at(10, 0, 2)), "Monday");
        assert!(weekdays.contains(&at(10, 0, 6)), "Friday");
        assert!(!weekdays.contains(&at(10, 0, 1)), "Sunday is 1");
        assert!(!weekdays.contains(&at(10, 0, 7)), "Saturday");
    }

    #[test]
    fn an_overnight_window_spills_into_the_next_day() {
        let overnight = session("1800-0600:23456");
        assert!(overnight.contains(&at(20, 0, 2)), "Monday evening");
        assert!(
            overnight.contains(&at(3, 0, 3)),
            "Tuesday 03:00 belongs to Monday's window"
        );
        assert!(
            !overnight.contains(&at(3, 0, 2)),
            "Monday 03:00 would need Sunday's window"
        );
        assert!(
            overnight.contains(&at(3, 0, 7)),
            "Saturday 03:00 belongs to Friday's window"
        );
        assert!(!overnight.contains(&at(12, 0, 3)));
        let sunday_wraps = session("2200-0200:7");
        assert!(
            sunday_wraps.contains(&at(1, 0, 1)),
            "Sunday 01:00 belongs to Saturday's window"
        );
    }

    #[test]
    fn several_windows_and_the_always_open_session() {
        let split = session("0930-1130,1300-1600");
        assert!(split.contains(&at(10, 0, 3)) && split.contains(&at(14, 0, 3)));
        assert!(!split.contains(&at(12, 0, 3)));
        let always = session("24x7");
        assert!(always.contains(&at(0, 0, 1)) && always.contains(&at(23, 59, 7)));
        assert!(
            session(" 24X7 ").contains(&at(5, 0, 4)),
            "case and spaces are ignored"
        );
        let midnight = session("0000-2400");
        assert!(midnight.contains(&at(23, 59, 4)));
        let all_day = session("0900-0900");
        assert!(all_day.contains(&at(3, 0, 4)), "equal ends mean all day");
    }

    #[test]
    fn malformed_sessions_are_rejected() {
        for bad in [
            "",
            "0930",
            "0930-",
            "09:30-16:00",
            "0930-1600:",
            "0930-1600:8",
            "0930-1600:0",
            "0930-1600:12a",
            "2500-0100",
            "0960-1000",
            "2401-2402",
            "930-1600",
            "0930-1600:1:2",
            "0930-1130,",
            "abcd-efgh",
        ] {
            assert!(TradingSession::parse(bad).is_none(), "{bad:?}");
        }
    }

    const MINUTE: i64 = 60_000;
    const HOUR: i64 = 60 * MINUTE;

    /// A New York wall-clock time on Tuesday 2024-03-05 (UTC-5 before the March change), in UTC ms.
    fn new_york(hour: i64, minute: i64) -> i64 {
        crate::pine::calendar::make(2024, 3, 5, hour + 5, minute, 0, 0).unwrap()
    }

    #[test]
    fn a_us_stock_trades_9_30_to_16_00_new_york_time_with_extended_hours_around_it() {
        let stock = MarketSessions::for_kind("stock");
        assert_eq!(stock.zone(), "America/New_York");
        assert!(stock.is_premarket(new_york(4, 0)) && stock.is_premarket(new_york(9, 29)));
        assert!(!stock.is_premarket(new_york(9, 30)) && !stock.is_premarket(new_york(3, 59)));
        assert!(stock.is_regular(new_york(9, 30)) && stock.is_regular(new_york(15, 59)));
        assert!(!stock.is_regular(new_york(16, 0)) && !stock.is_regular(new_york(9, 29)));
        assert!(stock.is_postmarket(new_york(16, 0)) && stock.is_postmarket(new_york(19, 59)));
        assert!(!stock.is_postmarket(new_york(20, 0)));
        // Saturday: nothing trades.
        let saturday = crate::pine::calendar::make(2024, 3, 9, 15, 0, 0, 0).unwrap();
        assert!(!stock.is_regular(saturday) && !stock.is_premarket(saturday));
    }

    #[test]
    fn daylight_saving_moves_the_session_in_utc() {
        let stock = MarketSessions::for_kind("stock");
        // 2024-07-02 13:30 UTC is 09:30 in New York (UTC-4).
        let summer = crate::pine::calendar::make(2024, 7, 2, 13, 30, 0, 0).unwrap();
        assert!(stock.is_regular(summer));
        assert!(!stock.is_regular(summer - MINUTE));
    }

    #[test]
    fn the_first_and_last_bars_of_a_regular_session_are_found_from_their_neighbours() {
        let stock = MarketSessions::for_kind("stock");
        let open = new_york(9, 30);
        // Hourly bars: 09:30 starts the session (the one before is premarket), 10:30 does not.
        assert!(stock.starts_regular_session(open, Some(open - 30 * MINUTE)));
        assert!(!stock.starts_regular_session(open + HOUR, Some(open)));
        assert!(
            stock.starts_regular_session(open, None),
            "the first bar of the data"
        );
        // Yesterday's last bar is in a regular session too, but on another day.
        let yesterday = new_york(15, 30) - 24 * HOUR;
        assert!(stock.starts_regular_session(open, Some(yesterday)));
        // 15:30 + 30 minutes is 16:00, past the close; 14:30 is followed by 15:30.
        assert!(stock.ends_regular_session(new_york(15, 30), 30 * MINUTE));
        assert!(!stock.ends_regular_session(new_york(14, 30), 30 * MINUTE));
        assert!(
            !stock.ends_regular_session(new_york(17, 0), HOUR),
            "after hours is not regular"
        );
    }

    #[test]
    fn crypto_trades_all_day_and_a_session_is_a_utc_day() {
        let crypto = MarketSessions::for_kind("crypto");
        assert_eq!(crypto.zone(), "Etc/UTC");
        let midnight = crate::pine::calendar::make(2024, 3, 5, 0, 0, 0, 0).unwrap();
        assert!(crypto.is_regular(midnight) && !crypto.is_premarket(midnight));
        assert!(!crypto.is_postmarket(midnight));
        assert!(crypto.starts_regular_session(midnight, Some(midnight - HOUR)));
        assert!(!crypto.starts_regular_session(midnight + HOUR, Some(midnight)));
        assert!(
            crypto.ends_regular_session(midnight - HOUR, HOUR),
            "23:00 closes the day"
        );
        assert!(!crypto.ends_regular_session(midnight, HOUR));
        assert_eq!(MarketSessions::for_kind("anything else"), crypto);
    }
}
