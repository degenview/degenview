//! Month and weekday names in the language in use. Every screen that writes a date takes its names
//! from here, so a language is translated once.

use super::tr_c;

/// "Jan" for month 1 … "Dec" for month 12; a month outside 1–12 is the nearest one.
pub fn month_abbreviation(month: i64) -> &'static str {
    match month.clamp(1, 12) {
        1 => tr_c!("month abbreviation", "Jan"),
        2 => tr_c!("month abbreviation", "Feb"),
        3 => tr_c!("month abbreviation", "Mar"),
        4 => tr_c!("month abbreviation", "Apr"),
        5 => tr_c!("month abbreviation", "May"),
        6 => tr_c!("month abbreviation", "Jun"),
        7 => tr_c!("month abbreviation", "Jul"),
        8 => tr_c!("month abbreviation", "Aug"),
        9 => tr_c!("month abbreviation", "Sep"),
        10 => tr_c!("month abbreviation", "Oct"),
        11 => tr_c!("month abbreviation", "Nov"),
        12 => tr_c!("month abbreviation", "Dec"),
        _ => tr_c!("month abbreviation", "Dec"),
    }
}

/// "January" for month 1 … "December" for month 12; a month outside 1–12 is the nearest one.
pub fn month_name(month: i64) -> &'static str {
    match month.clamp(1, 12) {
        1 => tr_c!("month name", "January"),
        2 => tr_c!("month name", "February"),
        3 => tr_c!("month name", "March"),
        4 => tr_c!("month name", "April"),
        5 => tr_c!("month name", "May"),
        6 => tr_c!("month name", "June"),
        7 => tr_c!("month name", "July"),
        8 => tr_c!("month name", "August"),
        9 => tr_c!("month name", "September"),
        10 => tr_c!("month name", "October"),
        11 => tr_c!("month name", "November"),
        12 => tr_c!("month name", "December"),
        _ => tr_c!("month name", "December"),
    }
}

/// "Monday" for 0 … "Sunday" for 6, the way a week is counted from Monday; a day past 6 is Sunday.
pub fn weekday_name(day: usize) -> &'static str {
    match day {
        0 => tr_c!("weekday name", "Monday"),
        1 => tr_c!("weekday name", "Tuesday"),
        2 => tr_c!("weekday name", "Wednesday"),
        3 => tr_c!("weekday name", "Thursday"),
        4 => tr_c!("weekday name", "Friday"),
        5 => tr_c!("weekday name", "Saturday"),
        6 => tr_c!("weekday name", "Sunday"),
        _ => tr_c!("weekday name", "Sunday"),
    }
}

/// "M" for Monday … "S" for Sunday: the column heads of a calendar.
pub fn weekday_initial(day: usize) -> &'static str {
    match day {
        0 => tr_c!("weekday initial", "M"),
        1 => tr_c!("weekday initial", "T"),
        2 => tr_c!("weekday initial", "W"),
        3 => tr_c!("weekday initial", "T"),
        4 => tr_c!("weekday initial", "F"),
        5 => tr_c!("weekday initial", "S"),
        6 => tr_c!("weekday initial", "S"),
        _ => tr_c!("weekday initial", "S"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{Locale, with_locale};

    #[test]
    fn english_names_run_from_one_month_and_from_monday() {
        assert_eq!(month_abbreviation(1), "Jan");
        assert_eq!(month_abbreviation(12), "Dec");
        assert_eq!(month_name(9), "September");
        assert_eq!(weekday_name(0), "Monday");
        assert_eq!(weekday_name(6), "Sunday");
        assert_eq!(weekday_initial(1), "T");
    }

    #[test]
    fn a_number_out_of_range_is_the_nearest_name() {
        assert_eq!(month_abbreviation(0), "Jan");
        assert_eq!(month_abbreviation(-3), "Jan");
        assert_eq!(month_abbreviation(13), "Dec");
        assert_eq!(month_name(99), "December");
        assert_eq!(weekday_name(7), "Sunday");
        assert_eq!(weekday_initial(7), "S");
    }

    #[test]
    fn the_names_are_looked_up_in_the_language_in_use() {
        with_locale(Locale::Pseudo, || {
            assert_eq!(month_abbreviation(1), "[Ĵåñ ~~]");
            assert_ne!(weekday_name(2), "Wednesday");
        });
    }
}
