//! Dates and times as the language in use writes them: month and weekday names, the order of day,
//! month and year, and the 12- or 24-hour clock, from CLDR through ICU4X (`icu_datetime`). Every
//! screen that writes a date calls these, so a language is never translated month by month.
//!
//! The callers hold civil fields (year, month, day, …) of the viewer's time zone, which is what
//! these take; the formatters are made once per language and kept.
//!
//! * English reads as it always did: `Oct 9, 2026`, `Oct 9, 14:03`, `Monday, October 9`.
//! * Times are on a 24-hour clock in every language ([`month_day_time`]), as a trading app shows
//!   them; [`month_day_clock`] is the one place that follows the language's own clock.
//! * ICU separates a time from its "PM" with U+202F, which fonts rarely have; it is written as a
//!   plain space.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use icu_calendar::Date;
use icu_datetime::DateTimeFormatter;
use icu_datetime::fieldsets::{E, MD, MDE, MDT, YM, YMD, YMDE};
use icu_datetime::options::TimePrecision;
use icu_locale_core::Locale as IcuLocale;
use icu_time::{DateTime, Time};

use super::{Locale, current};

/// Which clock a time is written on.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum Clock {
    /// 00:00 to 23:59 in every language.
    Day24,
    /// What the language uses: 2:03 PM in English, 14:03 in German.
    Language,
}

/// A formatter for each language and clock, made on first use by the thread that needs it (ICU4X
/// formatters share their data with `Rc`, so they stay on one thread; the interface draws on one).
/// One cache per kind of field set, because ICU4X types a formatter by the fields it writes.
macro_rules! formatter {
    ($name:ident, $fields:ty, $build:expr) => {
        fn $name(clock: Clock) -> Rc<DateTimeFormatter<$fields>> {
            type Cache = RefCell<HashMap<(Locale, Clock), Rc<DateTimeFormatter<$fields>>>>;
            thread_local! {
                static CACHE: Cache = RefCell::new(HashMap::new());
            }
            let locale = current();
            CACHE.with(|cache| {
                Rc::clone(
                    cache
                        .borrow_mut()
                        .entry((locale, clock))
                        .or_insert_with(|| {
                            let fields: $fields = $build;
                            // The locale is one of the table's tags, which ICU4X has data for, and any it
                            // lacks falls back to the root locale, so this does not fail.
                            let formatter = DateTimeFormatter::try_new(
                                icu_locale(locale, clock).into(),
                                fields,
                            )
                            .or_else(|_| {
                                DateTimeFormatter::try_new(IcuLocale::UNKNOWN.into(), $build)
                            })
                            .expect("ICU4X formats every date field set in the root locale");
                            Rc::new(formatter)
                        }),
                )
            })
        }
    };
}

formatter!(year_month_day_formatter, YMD, YMD::medium());
formatter!(month_day_formatter, MD, MD::medium());
formatter!(
    month_day_minutes_formatter,
    MDT,
    MDT::medium().with_time_precision(TimePrecision::Minute)
);
formatter!(
    month_day_seconds_formatter,
    MDT,
    MDT::medium().with_time_precision(TimePrecision::Second)
);
formatter!(weekday_month_day_formatter, MDE, MDE::long());
formatter!(weekday_month_day_year_formatter, YMDE, YMDE::long());
formatter!(month_year_formatter, YM, YM::long());
formatter!(weekday_long_formatter, E, E::long());
formatter!(weekday_short_formatter, E, E::short());

/// The locale ICU4X is asked for: the language's, with the 24-hour clock when that is wanted.
fn icu_locale(locale: Locale, clock: Clock) -> IcuLocale {
    match clock {
        Clock::Language => locale.icu(),
        Clock::Day24 => format!("{}-u-hc-h23", locale.icu())
            .parse()
            .unwrap_or_else(|_| locale.icu()),
    }
}

fn iso(year: i64, month: i64, day: i64) -> Option<Date<icu_calendar::Iso>> {
    Date::try_new_iso(
        i32::try_from(year).ok()?,
        u8::try_from(month).ok()?,
        u8::try_from(day).ok()?,
    )
    .ok()
}

fn date_and_time(
    (year, month, day): (i64, i64, i64),
    (hour, minute, second): (i64, i64, i64),
) -> Option<DateTime<icu_calendar::Iso>> {
    Some(DateTime {
        date: iso(year, month, day)?,
        time: Time::try_new(
            u8::try_from(hour).ok()?,
            u8::try_from(minute).ok()?,
            u8::try_from(second).ok()?,
            0,
        )
        .ok()?,
    })
}

/// The date as numbers, for a field ICU4X cannot be given (a day that does not exist).
fn numbers(year: i64, month: i64, day: i64) -> String {
    format!("{year}-{month:02}-{day:02}")
}

fn plain_spaces(text: String) -> String {
    text.replace('\u{202f}', " ")
}

/// "Oct 9, 2026"
pub fn date(year: i64, month: i64, day: i64) -> String {
    iso(year, month, day).map_or_else(
        || numbers(year, month, day),
        |date| {
            plain_spaces(
                year_month_day_formatter(Clock::Day24)
                    .format(&date)
                    .to_string(),
            )
        },
    )
}

/// "Sep 27" (the year only picks the calendar day; it is not written)
pub fn month_day(year: i64, month: i64, day: i64) -> String {
    iso(year, month, day).map_or_else(
        || numbers(year, month, day),
        |date| plain_spaces(month_day_formatter(Clock::Day24).format(&date).to_string()),
    )
}

/// "Oct 9, 14:03", or "Oct 9, 14:03:07" with `seconds`, on a 24-hour clock.
pub fn month_day_time(date: (i64, i64, i64), time: (i64, i64, i64), seconds: bool) -> String {
    let Some(moment) = date_and_time(date, time) else {
        return numbers(date.0, date.1, date.2);
    };
    let formatter = if seconds {
        month_day_seconds_formatter(Clock::Day24)
    } else {
        month_day_minutes_formatter(Clock::Day24)
    };
    plain_spaces(formatter.format(&moment).to_string())
}

/// "Oct 9, 2:03 PM" in English, "9. Okt., 14:03" in German: the language's own clock.
pub fn month_day_clock(date: (i64, i64, i64), (hour, minute): (i64, i64)) -> String {
    let Some(moment) = date_and_time(date, (hour, minute, 0)) else {
        return numbers(date.0, date.1, date.2);
    };
    plain_spaces(
        month_day_minutes_formatter(Clock::Language)
            .format(&moment)
            .to_string(),
    )
}

/// "Monday, October 9", or "Monday, October 9, 2025" with `year`. The weekday is the date's own.
pub fn weekday_month_day(date: (i64, i64, i64), year: bool) -> String {
    let (y, m, d) = date;
    let Some(day) = iso(y, m, d) else {
        return numbers(y, m, d);
    };
    plain_spaces(if year {
        weekday_month_day_year_formatter(Clock::Day24)
            .format(&day)
            .to_string()
    } else {
        weekday_month_day_formatter(Clock::Day24)
            .format(&day)
            .to_string()
    })
}

/// "October 2026"
pub fn month_year(year: i64, month: i64) -> String {
    iso(year, month, 1).map_or_else(
        || format!("{year}-{month:02}"),
        |date| plain_spaces(month_year_formatter(Clock::Day24).format(&date).to_string()),
    )
}

/// A day of a week counted from Monday, the way the pickers lay it out: any Monday and the days
/// after it. 1 January 2024 was a Monday.
fn weekday_date(day: usize) -> Option<Date<icu_calendar::Iso>> {
    Date::try_new_iso(2024, 1, 1 + u8::try_from(day.min(6)).ok()?).ok()
}

/// "Monday" for 0 … "Sunday" for 6; a day past 6 is Sunday.
pub fn weekday_name(day: usize) -> String {
    weekday_date(day).map_or_else(String::new, |date| {
        plain_spaces(
            weekday_long_formatter(Clock::Day24)
                .format(&date)
                .to_string(),
        )
    })
}

/// "Mon" for 0 … "Sun" for 6; a day past 6 is Sunday.
pub fn weekday_short(day: usize) -> String {
    weekday_date(day).map_or_else(String::new, |date| {
        plain_spaces(
            weekday_short_formatter(Clock::Day24)
                .format(&date)
                .to_string(),
        )
    })
}

/// "M" for Monday … "S" for Sunday: the column heads of a calendar. The first letter of the
/// language's short name, capitalised.
pub fn weekday_initial(day: usize) -> String {
    weekday_short(day)
        .chars()
        .next()
        .map(|first| first.to_uppercase().collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::with_locale;

    /// The English the app showed before dates went through ICU4X; these are what it keeps.
    #[test]
    fn english_dates_read_as_they_always_did() {
        assert_eq!(date(2026, 10, 9), "Oct 9, 2026");
        assert_eq!(month_day(2026, 9, 27), "Sep 27");
        assert_eq!(
            month_day_time((2026, 10, 9), (14, 3, 7), false),
            "Oct 9, 14:03"
        );
        assert_eq!(
            month_day_time((2026, 10, 9), (14, 3, 7), true),
            "Oct 9, 14:03:07"
        );
        assert_eq!(
            month_day_time((2026, 1, 2), (0, 5, 0), false),
            "Jan 2, 00:05"
        );
        assert_eq!(weekday_month_day((2026, 10, 9), false), "Friday, October 9");
        assert_eq!(
            weekday_month_day((2025, 10, 9), true),
            "Thursday, October 9, 2025"
        );
        assert_eq!(month_year(2026, 10), "October 2026");
        assert_eq!(month_year(2026, 5), "May 2026");
    }

    #[test]
    fn english_weeks_run_from_monday() {
        let names: Vec<String> = (0..7).map(weekday_name).collect();
        assert_eq!(
            names,
            [
                "Monday",
                "Tuesday",
                "Wednesday",
                "Thursday",
                "Friday",
                "Saturday",
                "Sunday"
            ]
        );
        let initials: String = (0..7).map(weekday_initial).collect();
        assert_eq!(initials, "MTWTFSS");
        assert_eq!(
            (0..7).map(weekday_short).collect::<Vec<_>>().join(" "),
            "Mon Tue Wed Thu Fri Sat Sun"
        );
        assert_eq!(weekday_name(9), "Sunday");
    }

    #[test]
    fn the_language_clock_is_twelve_hours_in_english_with_plain_spaces() {
        assert_eq!(month_day_clock((2026, 10, 9), (14, 3)), "Oct 9, 2:03 PM");
        assert_eq!(month_day_clock((2026, 10, 9), (0, 5)), "Oct 9, 12:05 AM");
        assert_eq!(month_day_clock((2026, 10, 9), (12, 0)), "Oct 9, 12:00 PM");
    }

    #[test]
    fn a_date_that_does_not_exist_is_written_as_numbers() {
        assert_eq!(date(2026, 2, 31), "2026-02-31");
        assert_eq!(month_day(2026, 13, 1), "2026-13-01");
        assert_eq!(month_year(2026, 0), "2026-00");
    }

    #[test]
    fn the_pseudo_locale_writes_dates_as_english() {
        with_locale(Locale::PSEUDO, || {
            assert_eq!(date(2026, 10, 9), "Oct 9, 2026");
        });
    }

    #[test]
    fn a_language_writes_dates_its_own_way() {
        // The formatters are made for the language in use; any CLDR locale shows the difference
        // the table's languages will.
        let german = |year, month, day| -> String {
            let formatter = DateTimeFormatter::try_new(
                "de".parse::<IcuLocale>().expect("tag").into(),
                YMD::medium(),
            )
            .expect("german");
            formatter
                .format(&iso(year, month, day).expect("date"))
                .to_string()
        };
        assert_eq!(german(2026, 10, 9), "09.10.2026");
    }
}
