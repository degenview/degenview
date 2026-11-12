//! Money and rates as text. What an account is worth is the crate's `valuation`; this is only how
//! the figures are written.

use degenview_brokers::Decimal;

/// `1,234.56` with the currency's mark: `$` and `€` and `£` lead, anything else follows as a code.
pub fn money(value: Decimal, currency: &str) -> String {
    let rounded = value.round_dp(2);
    let text = format!("{:.2}", rounded.abs());
    let (whole, fraction) = text.split_once('.').unwrap_or((&text, "00"));
    let mut grouped = String::new();
    for (index, digit) in whole.chars().enumerate() {
        if index > 0 && (whole.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    let sign = if rounded.is_sign_negative() && !rounded.is_zero() {
        "-"
    } else {
        ""
    };
    match currency {
        "USD" => format!("{sign}${grouped}.{fraction}"),
        "EUR" => format!("{sign}€{grouped}.{fraction}"),
        "GBP" => format!("{sign}£{grouped}.{fraction}"),
        code => format!("{sign}{grouped}.{fraction} {code}"),
    }
}

/// A fee rate as a percent: `0.006` is `0.60%`.
pub fn percent(rate: Decimal) -> String {
    format!("{:.2}%", (rate * Decimal::ONE_HUNDRED).round_dp(2))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(text: &str) -> Decimal {
        text.parse().unwrap()
    }

    #[test]
    fn money_groups_thousands_and_marks_the_currency() {
        assert_eq!(money(d("1234567.891"), "USD"), "$1,234,567.89");
        assert_eq!(money(d("0.5"), "EUR"), "€0.50");
        assert_eq!(money(d("-12.3"), "GBP"), "-£12.30");
        assert_eq!(money(d("1000"), "USDC"), "1,000.00 USDC");
        assert_eq!(money(d("-0.001"), "USD"), "$0.00");
    }

    #[test]
    fn a_fee_rate_is_a_percent() {
        assert_eq!(percent(d("0.006")), "0.60%");
        assert_eq!(percent(d("0.0035")), "0.35%");
        assert_eq!(percent(d("0")), "0.00%");
    }
}
