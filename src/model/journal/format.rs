//! Text for journal figures. Pure, so the table and the exports agree and tests can pin them.

use rust_decimal::Decimal;

use super::calendar::{Civil, Offset, civil_from_days, day_number, weekday};

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

pub fn month_name(month: u8) -> &'static str {
    MONTHS[usize::from(month.clamp(1, 12)) - 1]
}

pub fn weekday_name(day: u8) -> &'static str {
    ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"][usize::from(day % 7)]
}

/// Groups the integer digits by thousands.
fn group(digits: &str) -> String {
    let mut out = String::new();
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// `value` with at least `min_dp` and at most `max_dp` decimals: trailing zeros past `min_dp` are
/// dropped. Integer digits are grouped. `-0` never appears.
pub fn decimal(value: Decimal, min_dp: u32, max_dp: u32) -> String {
    let rounded = value.round_dp(max_dp);
    let negative = rounded.is_sign_negative() && !rounded.is_zero();
    let text = rounded.abs().to_string();
    let (int_part, frac) = text.split_once('.').unwrap_or((&text, ""));
    let mut frac = frac.to_owned();
    while frac.len() > min_dp as usize && frac.ends_with('0') {
        frac.pop();
    }
    while frac.len() < min_dp as usize {
        frac.push('0');
    }
    let mut out = String::new();
    if negative {
        out.push('-');
    }
    out.push_str(&group(int_part));
    if !frac.is_empty() {
        out.push('.');
        out.push_str(&frac);
    }
    out
}

/// A price or quantity: up to eight decimals, none forced.
pub fn price(value: Decimal) -> String {
    decimal(value, 0, 8)
}

pub fn quantity(value: Decimal) -> String {
    decimal(value, 0, 8)
}

/// An amount of money: two decimals, or more when the figure is smaller than a cent.
pub fn money(value: Decimal) -> String {
    let dp = if value != Decimal::ZERO && value.abs() < Decimal::new(1, 2) {
        8
    } else {
        2
    };
    decimal(value, 2, dp)
}

/// `money` with an explicit sign: `+12.50`, `-3.00`, `0.00`.
pub fn signed_money(value: Decimal) -> String {
    let text = money(value);
    if value > Decimal::ZERO {
        format!("+{text}")
    } else {
        text
    }
}

pub fn percent(fraction: f64) -> String {
    if !fraction.is_finite() {
        return "\u{2014}".to_owned();
    }
    format!("{:.1}%", fraction * 100.0)
}

pub fn signed_percent(fraction: f64) -> String {
    if !fraction.is_finite() {
        return "\u{2014}".to_owned();
    }
    format!("{:+.2}%", fraction * 100.0)
}

pub fn ratio(value: f64) -> String {
    if value.is_finite() {
        format!("{value:.2}")
    } else {
        "\u{2014}".to_owned()
    }
}

pub fn r_multiple(value: f64) -> String {
    if value.is_finite() {
        format!("{value:+.2}R")
    } else {
        "\u{2014}".to_owned()
    }
}

/// `2d 3h`, `4h 05m`, `12m 30s`, `45s`, `0s`.
pub fn duration(ms: i64) -> String {
    let seconds = (ms.max(0)) / 1_000;
    let (days, hours, minutes, secs) = (
        seconds / 86_400,
        seconds % 86_400 / 3_600,
        seconds % 3_600 / 60,
        seconds % 60,
    );
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {minutes:02}m")
    } else if minutes > 0 {
        format!("{minutes}m {secs:02}s")
    } else {
        format!("{secs}s")
    }
}

/// `Nov 9, 2026 15:04` in the zone `offset` describes.
pub fn datetime(ts_ms: i64, offset: Offset<'_>) -> String {
    let day = day_number(ts_ms, offset);
    let civil = civil_from_days(day);
    let seconds = (ts_ms.div_euclid(1_000) + offset(ts_ms)).rem_euclid(86_400);
    format!(
        "{} {}, {} {:02}:{:02}",
        month_name(civil.month),
        civil.day,
        civil.year,
        seconds / 3_600,
        seconds % 3_600 / 60
    )
}

/// `Nov 9`, in the zone `offset` describes.
pub fn short_date(ts_ms: i64, offset: Offset<'_>) -> String {
    let civil = civil_from_days(day_number(ts_ms, offset));
    format!("{} {}", month_name(civil.month), civil.day)
}

/// `Nov 9` for a civil date.
pub fn short_civil(civil: Civil) -> String {
    format!("{} {}", month_name(civil.month), civil.day)
}

/// A figure for a chart axis: `950`, `1.2K`, `3.4M`, `-2.5K`.
pub fn compact(value: f64) -> String {
    if !value.is_finite() {
        return "\u{2014}".to_owned();
    }
    let abs = value.abs();
    let (scaled, suffix) = if abs >= 1e9 {
        (value / 1e9, "B")
    } else if abs >= 1e6 {
        (value / 1e6, "M")
    } else if abs >= 1e3 {
        (value / 1e3, "K")
    } else {
        (value, "")
    };
    let text = if suffix.is_empty() {
        if abs >= 100.0 || scaled.fract() == 0.0 {
            format!("{scaled:.0}")
        } else {
            format!("{scaled:.2}")
        }
    } else {
        let t = format!("{scaled:.1}");
        t.trim_end_matches(".0").to_owned()
    };
    format!("{text}{suffix}")
}

/// `November 2026`.
pub fn month_title(year: i32, month: u8) -> String {
    const FULL: [&str; 12] = [
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
    format!("{} {year}", FULL[usize::from(month.clamp(1, 12)) - 1])
}

/// `Sun, Nov 9, 2026`.
pub fn date(civil: Civil) -> String {
    let days = super::calendar::days_from_civil(civil);
    format!(
        "{}, {} {}, {}",
        weekday_name(weekday(days)),
        month_name(civil.month),
        civil.day,
        civil.year
    )
}

pub fn iso_date(civil: Civil) -> String {
    format!("{:04}-{:02}-{:02}", civil.year, civil.month, civil.day)
}

/// `YYYY-MM-DD` to a civil date; `None` for anything that is not a real day.
pub fn parse_iso_date(text: &str) -> Option<Civil> {
    let mut parts = text.trim().split('-');
    let year: i32 = parts.next()?.parse().ok()?;
    let month: u8 = parts.next()?.parse().ok()?;
    let day: u8 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&month) {
        return None;
    }
    (day >= 1 && day <= super::calendar::days_in_month(year, month)).then_some(Civil {
        year,
        month,
        day,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::journal::calendar::{days_from_civil, utc};

    fn d(text: &str) -> Decimal {
        text.parse().unwrap()
    }

    #[test]
    fn decimals_group_trim_and_never_print_negative_zero() {
        assert_eq!(decimal(d("1234567.5"), 2, 8), "1,234,567.50");
        assert_eq!(decimal(d("0.10000000"), 0, 8), "0.1");
        assert_eq!(decimal(d("100"), 0, 8), "100");
        assert_eq!(decimal(d("-0.000000001"), 2, 8), "0.00");
        assert_eq!(decimal(d("-1234.5"), 2, 2), "-1,234.50");
        assert_eq!(decimal(d("999"), 0, 2), "999");
        assert_eq!(decimal(d("0.123456789"), 0, 8), "0.12345679");
    }

    #[test]
    fn money_shows_sub_cent_amounts_instead_of_rounding_them_away() {
        assert_eq!(money(d("12.5")), "12.50");
        assert_eq!(money(d("0")), "0.00");
        assert_eq!(money(d("0.004")), "0.004");
        assert_eq!(signed_money(d("12.5")), "+12.50");
        assert_eq!(signed_money(d("-3")), "-3.00");
        assert_eq!(signed_money(d("0")), "0.00");
    }

    #[test]
    fn ratios_percentages_and_r_are_signed_and_finite_only() {
        assert_eq!(percent(0.4571), "45.7%");
        assert_eq!(signed_percent(-0.0123), "-1.23%");
        assert_eq!(r_multiple(1.8), "+1.80R");
        assert_eq!(r_multiple(-0.5), "-0.50R");
        assert_eq!(percent(f64::NAN), "\u{2014}");
        assert_eq!(ratio(f64::INFINITY), "\u{2014}");
        assert_eq!(ratio(4.0), "4.00");
    }

    #[test]
    fn durations_pick_the_two_most_useful_units() {
        assert_eq!(duration(0), "0s");
        assert_eq!(duration(45_000), "45s");
        assert_eq!(duration(750_000), "12m 30s");
        assert_eq!(duration(14_700_000), "4h 05m");
        assert_eq!(duration(2 * 86_400_000 + 3 * 3_600_000), "2d 3h");
        assert_eq!(duration(-5), "0s");
    }

    #[test]
    fn datetimes_follow_the_zone() {
        let ts = days_from_civil(Civil {
            year: 2026,
            month: 11,
            day: 9,
        }) * 86_400_000
            + (15 * 3_600 + 4 * 60) * 1_000;
        assert_eq!(datetime(ts, &utc), "Nov 9, 2026 15:04");
        assert_eq!(datetime(ts, &|_| 10 * 3_600), "Nov 10, 2026 01:04");
        assert_eq!(datetime(ts, &|_| -16 * 3_600), "Nov 8, 2026 23:04");
    }

    #[test]
    fn axis_figures_are_short() {
        assert_eq!(compact(0.0), "0");
        assert_eq!(compact(950.0), "950");
        assert_eq!(compact(1_200.0), "1.2K");
        assert_eq!(compact(-2_500.0), "-2.5K");
        assert_eq!(compact(3_400_000.0), "3.4M");
        assert_eq!(compact(2_000.0), "2K");
        assert_eq!(compact(0.25), "0.25");
        assert_eq!(compact(f64::NAN), "\u{2014}");
        assert_eq!(month_title(2026, 11), "November 2026");
        assert_eq!(
            short_civil(Civil {
                year: 2026,
                month: 11,
                day: 9
            }),
            "Nov 9"
        );
        assert_eq!(short_date(0, &utc), "Jan 1");
    }

    #[test]
    fn dates_print_and_parse() {
        let civil = Civil {
            year: 2026,
            month: 11,
            day: 9,
        };
        assert_eq!(date(civil), "Mon, Nov 9, 2026");
        assert_eq!(iso_date(civil), "2026-11-09");
        assert_eq!(parse_iso_date("2026-11-09"), Some(civil));
        assert_eq!(
            parse_iso_date(" 2024-02-29 "),
            Some(Civil {
                year: 2024,
                month: 2,
                day: 29
            })
        );
        for bad in [
            "2026-02-29",
            "2026-13-01",
            "2026-00-10",
            "nope",
            "2026-11",
            "2026-11-09-1",
            "",
        ] {
            assert_eq!(parse_iso_date(bad), None, "{bad}");
        }
    }
}
