//! How the portfolio screens write amounts: one reporting currency, one privacy switch.
//!
//! Amounts mask when privacy mode is on; percentages and a position's *share* (allocation,
//! return) stay readable, matching the Holdings table.

// Transient: the portfolio views and the store wire this up next.
#![allow(dead_code)]

use crate::model::portfolio::PortfolioCurrency;

/// What an amount is masked with in privacy mode.
pub const MASK: &str = "••••••••";

/// Every currency a portfolio can report in.
pub const PORTFOLIO_CURRENCIES: [PortfolioCurrency; 6] = [
    PortfolioCurrency::Usd,
    PortfolioCurrency::Eur,
    PortfolioCurrency::Gbp,
    PortfolioCurrency::Jpy,
    PortfolioCurrency::Chf,
    PortfolioCurrency::Btc,
];

/// Groups digits in threes: `1234567` → `1,234,567`.
fn group(digits: &str) -> String {
    let mut output = String::new();
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            output.push(',');
        }
        output.push(character);
    }
    output
}

/// `magnitude` with between `min` and `max` fraction digits (trailing zeros trimmed down to
/// `min`) and thousands separators.
fn grouped(magnitude: f64, min: usize, max: usize) -> String {
    let mut text = format!("{magnitude:.max$}");
    if let Some(dot) = text.find('.') {
        let keep = dot + 1 + min;
        while text.len() > keep && text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
    }
    match text.split_once('.') {
        Some((whole, fraction)) => format!("{}.{fraction}", group(whole)),
        None => group(&text),
    }
}

/// `"-"` for a value that would print as a negative number, `""` otherwise (a value that
/// rounds to zero carries no sign).
fn sign_for(value: f64, rendered: &str) -> &'static str {
    if value < 0.0 && rendered.chars().any(|c| c.is_ascii_digit() && c != '0') {
        "-"
    } else {
        ""
    }
}

impl PortfolioCurrency {
    /// ISO code.
    pub fn code(self) -> &'static str {
        match self {
            Self::Usd => "USD",
            Self::Eur => "EUR",
            Self::Gbp => "GBP",
            Self::Jpy => "JPY",
            Self::Chf => "CHF",
            Self::Btc => "BTC",
        }
    }

    fn symbol_prefix(self) -> &'static str {
        match self {
            Self::Usd => "$",
            Self::Eur => "€",
            Self::Gbp => "£",
            Self::Jpy => "¥",
            Self::Chf => "CHF ",
            Self::Btc => "BTC ",
        }
    }

    /// `$1,234.00`; yen has no fraction, bitcoin up to eight decimals.
    pub fn format(self, value: f64) -> String {
        let (min, max) = match self {
            Self::Jpy => (0, 0),
            Self::Btc => (0, 8),
            _ => (2, 2),
        };
        let body = grouped(value.abs(), min, max);
        format!("{}{}{body}", sign_for(value, &body), self.symbol_prefix())
    }

    /// A per-unit price. Same as `format`, except under one unit it keeps up to eight
    /// decimals, so a micro-cap token reads `$0.00002` rather than `$0.00`.
    pub fn format_price(self, value: f64) -> String {
        if self == Self::Btc || value == 0.0 || value.abs() >= 1.0 {
            return self.format(value);
        }
        let body = grouped(value.abs(), 2, 8);
        format!("{}{}{body}", sign_for(value, &body), self.symbol_prefix())
    }

    /// Short form for chart axes: `135.84K`. The currency is stated beside the chart.
    pub fn format_compact(self, value: f64) -> String {
        if self == Self::Btc {
            return self.format(value);
        }
        for (divisor, suffix) in [(1e9, "B"), (1e6, "M"), (1e3, "K")] {
            if value.abs() >= divisor {
                return format!("{}{suffix}", trimmed(value / divisor));
            }
        }
        trimmed(value)
    }
}

/// At most two decimals, trailing zeros dropped.
fn trimmed(value: f64) -> String {
    let text = format!("{value:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// Green for gains, red for losses.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tone {
    Gain,
    Loss,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum QuantitySign {
    #[default]
    None,
    Plus,
    Minus,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PortfolioValueFormatter {
    pub currency: PortfolioCurrency,
    pub privacy: bool,
}

impl PortfolioValueFormatter {
    pub fn new(currency: PortfolioCurrency, privacy: bool) -> Self {
        Self { currency, privacy }
    }

    fn sensitive(&self, text: String) -> String {
        if self.privacy { MASK.to_owned() } else { text }
    }

    /// A currency amount, masked in privacy mode.
    pub fn money(&self, value: f64) -> String {
        self.sensitive(self.currency.format(value))
    }

    /// A per-unit price (cost, quote), masked in privacy mode.
    pub fn price(&self, value: f64) -> String {
        self.sensitive(self.currency.format_price(value))
    }

    /// "+$1,234.00" / "−$12.00"; zero carries no sign.
    pub fn signed_money(&self, value: f64) -> String {
        if self.privacy {
            return MASK.to_owned();
        }
        let text = self.currency.format(value.abs());
        let rounds_to_zero = !text.chars().any(|c| c.is_ascii_digit() && c != '0');
        if value > 0.0 && !rounds_to_zero {
            format!("+{text}")
        } else if value < 0.0 && !rounds_to_zero {
            format!("−{text}")
        } else {
            text
        }
    }

    /// A fraction as a percentage: `0.1234` → "12.34%".
    pub fn percent(&self, fraction: f64, digits: usize) -> String {
        format!("{:.digits$}%", fraction * 100.0)
    }

    /// "+12.34%" / "−3.10%".
    pub fn signed_percent(&self, fraction: f64, digits: usize) -> String {
        let text = self.percent(fraction.abs(), digits);
        let rounds_to_zero = !text.chars().any(|c| c.is_ascii_digit() && c != '0');
        if fraction > 0.0 && !rounds_to_zero {
            format!("+{text}")
        } else if fraction < 0.0 && !rounds_to_zero {
            format!("−{text}")
        } else {
            text
        }
    }

    /// An asset quantity: grouped, at most eight decimals, no trailing zeros; masked in
    /// privacy mode. `sign` marks inflows and outflows ("+0.5", "−0.2").
    pub fn quantity(&self, value: f64, sign: QuantitySign) -> String {
        if self.privacy {
            return MASK.to_owned();
        }
        let negative = if value < 0.0 { "-" } else { "" };
        let text = format!("{negative}{}", grouped(value.abs(), 0, 8));
        match sign {
            QuantitySign::None => text,
            QuantitySign::Plus => format!("+{text}"),
            QuantitySign::Minus => format!("−{text}"),
        }
    }

    /// Gains and losses, never for zero or unknown, and never while values are hidden — the
    /// color would give the sign away.
    pub fn tone(&self, value: Option<f64>) -> Option<Tone> {
        if self.privacy {
            return None;
        }
        match value {
            Some(value) if value > 0.0 => Some(Tone::Gain),
            Some(value) if value < 0.0 => Some(Tone::Loss),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use PortfolioCurrency::*;

    #[test]
    fn currencies_format_like_the_swift_app() {
        assert_eq!(Usd.format(1_234.5), "$1,234.50");
        assert_eq!(Usd.format(-12.0), "-$12.00");
        assert_eq!(Usd.format(0.0), "$0.00");
        assert_eq!(Eur.format(1_000_000.0), "€1,000,000.00");
        assert_eq!(Gbp.format(0.5), "£0.50");
        assert_eq!(Chf.format(10.0), "CHF 10.00");
        assert_eq!(Jpy.format(10_234.6), "¥10,235");
        assert_eq!(Btc.format(0.123_456_789_9), "BTC 0.12345679");
        assert_eq!(Btc.format(2.0), "BTC 2");
    }

    #[test]
    fn a_negative_that_rounds_to_zero_has_no_sign() {
        assert_eq!(Usd.format(-0.001), "$0.00");
        assert_eq!(Jpy.format(-0.4), "¥0");
    }

    #[test]
    fn unit_prices_keep_the_decimals_of_sub_unit_assets() {
        assert_eq!(Usd.format_price(0.000_02), "$0.00002");
        assert_eq!(Usd.format_price(0.5), "$0.50");
        assert_eq!(Usd.format_price(0.123_456_789), "$0.12345679");
        assert_eq!(
            Usd.format_price(1.5),
            "$1.50",
            "one unit and up reads as money"
        );
        assert_eq!(Usd.format_price(0.0), "$0.00");
        assert_eq!(Usd.format_price(-0.00005), "-$0.00005");
        assert_eq!(Btc.format_price(0.5), "BTC 0.5");
    }

    #[test]
    fn axis_labels_are_compact() {
        assert_eq!(Usd.format_compact(135_840.0), "135.84K");
        assert_eq!(Usd.format_compact(2_500_000.0), "2.5M");
        assert_eq!(Usd.format_compact(3_000_000_000.0), "3B");
        assert_eq!(Usd.format_compact(999.0), "999");
        assert_eq!(Usd.format_compact(-1_500.0), "-1.5K");
        assert_eq!(Btc.format_compact(1_500.0), "BTC 1,500");
    }

    #[test]
    fn privacy_masks_amounts_but_not_shares() {
        let open = PortfolioValueFormatter::new(Usd, false);
        let hidden = PortfolioValueFormatter::new(Usd, true);
        assert_eq!(open.money(1_234.5), "$1,234.50");
        assert_eq!(hidden.money(1_234.5), MASK);
        assert_eq!(hidden.price(1.0), MASK);
        assert_eq!(hidden.signed_money(5.0), MASK);
        assert_eq!(hidden.quantity(1.5, QuantitySign::Plus), MASK);
        assert_eq!(
            hidden.percent(0.255, 1),
            "25.5%",
            "allocation stays readable"
        );
        assert_eq!(hidden.signed_percent(-0.031, 2), "−3.10%");
        assert_eq!(
            hidden.tone(Some(5.0)),
            None,
            "a colour would give the sign away"
        );
        assert_eq!(open.tone(Some(5.0)), Some(Tone::Gain));
    }

    #[test]
    fn signed_amounts_use_a_real_minus_and_no_sign_for_zero() {
        let f = PortfolioValueFormatter::new(Usd, false);
        assert_eq!(f.signed_money(1_234.0), "+$1,234.00");
        assert_eq!(f.signed_money(-12.0), "−$12.00");
        assert_eq!(f.signed_money(0.0), "$0.00");
        assert_eq!(f.signed_money(0.001), "$0.00", "rounds to zero, no plus");
        assert_eq!(f.signed_percent(0.1234, 2), "+12.34%");
        assert_eq!(f.signed_percent(-0.031, 2), "−3.10%");
        assert_eq!(f.signed_percent(0.0, 2), "0.00%");
        assert_eq!(f.signed_percent(0.000_001, 2), "0.00%");
        assert_eq!(f.percent(0.5, 0), "50%");
    }

    #[test]
    fn quantities_are_grouped_with_up_to_a_satoshi_of_precision() {
        let f = PortfolioValueFormatter::new(Usd, false);
        assert_eq!(f.quantity(1_234.5, QuantitySign::None), "1,234.5");
        assert_eq!(f.quantity(0.000_000_01, QuantitySign::None), "0.00000001");
        assert_eq!(f.quantity(0.000_000_001, QuantitySign::None), "0");
        assert_eq!(f.quantity(10.0, QuantitySign::Plus), "+10");
        assert_eq!(f.quantity(0.2, QuantitySign::Minus), "−0.2");
        assert_eq!(f.quantity(-3.0, QuantitySign::None), "-3");
    }

    #[test]
    fn tones_ignore_zero_and_unknown() {
        let f = PortfolioValueFormatter::new(Usd, false);
        assert_eq!(f.tone(Some(-0.01)), Some(Tone::Loss));
        assert_eq!(f.tone(Some(0.0)), None);
        assert_eq!(f.tone(None), None);
    }

    #[test]
    fn codes_and_the_list_of_currencies_agree() {
        let codes: Vec<&str> = PORTFOLIO_CURRENCIES.iter().map(|c| c.code()).collect();
        assert_eq!(codes, ["USD", "EUR", "GBP", "JPY", "CHF", "BTC"]);
    }
}
