//! Presentation of paper-trading values: labels for the enums, the number formats, and the
//! colour rules. The engine keeps full precision; only what the screens show is rounded here.

// Transient: the paper-trading screens use this once they are hosted.
#![allow(dead_code)]

use crate::format::civil_from_days;
use crate::model::paper::{
    PaperCurrency, PaperInstrument, PaperOrderEventKind, PaperOrderRole, PaperOrderSide,
    PaperOrderStatus, PaperOrderType, PaperPositionSide, PaperTimeInForce,
};

impl PaperOrderSide {
    pub fn label(self) -> &'static str {
        match self {
            Self::Buy => "Buy",
            Self::Sell => "Sell",
        }
    }

    pub fn badge_text(self) -> &'static str {
        match self {
            Self::Buy => "BUY",
            Self::Sell => "SELL",
        }
    }

    pub fn is_buy(self) -> bool {
        self == Self::Buy
    }
}

impl PaperPositionSide {
    pub fn label(self) -> &'static str {
        match self {
            Self::Long => "Long",
            Self::Short => "Short",
        }
    }

    pub fn badge_text(self) -> &'static str {
        match self {
            Self::Long => "LONG",
            Self::Short => "SHORT",
        }
    }

    pub fn is_long(self) -> bool {
        self == Self::Long
    }
}

impl PaperOrderType {
    pub const ALL: [Self; 4] = [Self::Market, Self::Limit, Self::Stop, Self::StopLimit];

    pub fn label(self) -> &'static str {
        match self {
            Self::Market => "Market",
            Self::Limit => "Limit",
            Self::Stop => "Stop",
            Self::StopLimit => "Stop Limit",
        }
    }

    pub fn badge_text(self) -> &'static str {
        match self {
            Self::Market => "MKT",
            Self::Limit => "LMT",
            Self::Stop => "STP",
            Self::StopLimit => "STP LMT",
        }
    }

    /// Whether the ticket asks for a limit price.
    pub fn needs_limit_price(self) -> bool {
        matches!(self, Self::Limit | Self::StopLimit)
    }

    /// Whether the ticket asks for a stop price.
    pub fn needs_stop_price(self) -> bool {
        matches!(self, Self::Stop | Self::StopLimit)
    }
}

impl PaperTimeInForce {
    pub const ALL: [Self; 2] = [Self::Day, Self::GoodTilCanceled];

    pub fn label(self) -> &'static str {
        match self {
            Self::Day => "Day",
            Self::GoodTilCanceled => "Good till canceled",
        }
    }
}

impl PaperOrderRole {
    pub fn label(self) -> &'static str {
        match self {
            Self::Entry => "Entry",
            Self::TakeProfit => "Take profit",
            Self::StopLoss => "Stop loss",
        }
    }

    pub fn badge_text(self) -> &'static str {
        match self {
            Self::Entry => "ENTRY",
            Self::TakeProfit => "TP",
            Self::StopLoss => "SL",
        }
    }
}

/// How a status or event badge is toned.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BadgeTone {
    Good,
    Warning,
    Bad,
    Neutral,
}

impl PaperOrderStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::PendingSubmission => "Pending",
            Self::Working => "Working",
            Self::PartiallyFilled => "Partial",
            Self::Filled => "Filled",
            Self::PendingCancel => "Cancelling",
            Self::Canceled => "Canceled",
            Self::Rejected => "Rejected",
            Self::Expired => "Expired",
        }
    }

    pub fn tone(self) -> BadgeTone {
        match self {
            Self::Filled => BadgeTone::Good,
            Self::PartiallyFilled | Self::PendingCancel => BadgeTone::Warning,
            Self::Rejected => BadgeTone::Bad,
            Self::PendingSubmission | Self::Working | Self::Canceled | Self::Expired => {
                BadgeTone::Neutral
            }
        }
    }
}

impl PaperOrderEventKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Placed => "Placed",
            Self::Accepted => "Accepted",
            Self::PartiallyFilled => "Partial fill",
            Self::Filled => "Filled",
            Self::Modified => "Modified",
            Self::Canceled => "Canceled",
            Self::Rejected => "Rejected",
            Self::Expired => "Expired",
        }
    }

    pub fn tone(self) -> BadgeTone {
        match self {
            Self::Filled => BadgeTone::Good,
            Self::PartiallyFilled | Self::Modified => BadgeTone::Warning,
            Self::Rejected => BadgeTone::Bad,
            Self::Placed | Self::Accepted | Self::Canceled | Self::Expired => BadgeTone::Neutral,
        }
    }
}

impl PaperCurrency {
    pub const ALL: [Self; 6] = [
        Self::Usd,
        Self::Eur,
        Self::Gbp,
        Self::Jpy,
        Self::Usdt,
        Self::Usdc,
    ];

    pub fn code(self) -> &'static str {
        match self {
            Self::Usd => "USD",
            Self::Eur => "EUR",
            Self::Gbp => "GBP",
            Self::Jpy => "JPY",
            Self::Usdt => "USDT",
            Self::Usdc => "USDC",
        }
    }

    fn prefix(self) -> &'static str {
        match self {
            Self::Usd => "$",
            Self::Eur => "€",
            Self::Gbp => "£",
            Self::Jpy => "¥",
            Self::Usdt => "USDT ",
            Self::Usdc => "USDC ",
        }
    }

    fn fraction_digits(self) -> usize {
        if self == Self::Jpy { 0 } else { 2 }
    }
}

// MARK: Number formats

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

/// `magnitude` to between 0 and `max` fraction digits, grouped, trailing zeros dropped.
fn grouped(magnitude: f64, max: usize) -> String {
    let mut text = format!("{magnitude:.max$}");
    if text.contains('.') {
        text = text.trim_end_matches('0').trim_end_matches('.').to_owned();
    }
    match text.split_once('.') {
        Some((whole, fraction)) => format!("{}.{fraction}", group(whole)),
        None => group(&text),
    }
}

/// The decimal places an increment such as `0.01` or `0.00000001` is written with.
pub fn decimal_places(increment: f64) -> usize {
    if increment <= 0.0 || !increment.is_finite() {
        return 0;
    }
    for digits in 0..=16 {
        let scale = 10_f64.powi(digits);
        let scaled = increment * scale;
        if (scaled - scaled.round()).abs() < 1e-9 * scaled.abs().max(1.0) {
            return digits as usize;
        }
    }
    16
}

/// `$1,234.50`, with half-even rounding to the currency's cents.
pub fn money(value: f64, currency: PaperCurrency) -> String {
    let digits = currency.fraction_digits();
    let magnitude = round_half_even(value.abs(), digits);
    let mut text = format!("{magnitude:.digits$}");
    if let Some((whole, fraction)) = text.split_once('.') {
        text = format!("{}.{fraction}", group(whole));
    } else {
        text = group(&text);
    }
    let rounds_to_zero = !text.chars().any(|c| c.is_ascii_digit() && c != '0');
    let sign = if value < 0.0 && !rounds_to_zero {
        "-"
    } else {
        ""
    };
    format!("{sign}{}{text}", currency.prefix())
}

/// `+$12.00` for a gain; a loss keeps its minus; zero has no sign.
pub fn signed_money(value: f64, currency: PaperCurrency) -> String {
    let text = money(value, currency);
    if value > 0.0 && !text.contains("0.00")
        || value > 0.0 && text.chars().any(|c| c.is_ascii_digit() && c != '0')
    {
        format!("+{text}")
    } else {
        text
    }
}

/// A ratio as a percentage with two decimals: `0.23349` → "23.35%".
pub fn percent(ratio: f64) -> String {
    format!("{:.2}%", round_half_even(ratio * 100.0, 2))
}

/// Like `percent`, with a leading `+` for gains.
pub fn signed_percent(ratio: f64) -> String {
    let text = percent(ratio);
    if ratio > 0.0 {
        format!("+{text}")
    } else {
        text
    }
}

/// A price at the instrument's tick size.
pub fn price(value: f64, instrument: &PaperInstrument) -> String {
    decimal(value, instrument.tick_size)
}

/// A quantity at the instrument's quantity step.
pub fn quantity(value: f64, instrument: &PaperInstrument) -> String {
    decimal(value, instrument.quantity_increment)
}

fn decimal(value: f64, increment: f64) -> String {
    let digits = decimal_places(increment).min(16);
    let text = grouped(round_half_even(value.abs(), digits), digits);
    let rounds_to_zero = !text.chars().any(|c| c.is_ascii_digit() && c != '0');
    format!(
        "{}{text}",
        if value < 0.0 && !rounds_to_zero {
            "-"
        } else {
            ""
        }
    )
}

fn round_half_even(value: f64, digits: usize) -> f64 {
    let scale = 10_f64.powi(digits as i32);
    let scaled = value * scale;
    let floor = scaled.floor();
    let difference = scaled - floor;
    let rounded = if (difference - 0.5).abs() < 1e-9 {
        if floor % 2.0 == 0.0 {
            floor
        } else {
            floor + 1.0
        }
    } else {
        scaled.round()
    };
    rounded / scale
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// "Oct 9, 14:03:07" in the viewer's time zone.
pub fn timestamp(epoch_ms: i64, utc_offset_secs: i64) -> String {
    let seconds = epoch_ms.div_euclid(1_000) + utc_offset_secs;
    let (_, month, day) = civil_from_days(seconds.div_euclid(86_400));
    let rest = seconds.rem_euclid(86_400);
    format!(
        "{} {day}, {:02}:{:02}:{:02}",
        MONTHS[(month - 1) as usize],
        rest / 3_600,
        rest % 3_600 / 60,
        rest % 60
    )
}

// MARK: Colour rules

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PnlTone {
    Profit,
    Loss,
    Flat,
}

/// Green above zero, red below, quiet at zero.
pub fn pnl_tone(value: f64) -> PnlTone {
    if value > 0.0 {
        PnlTone::Profit
    } else if value < 0.0 {
        PnlTone::Loss
    } else {
        PnlTone::Flat
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BufferLevel {
    Normal,
    Thin,
    Critical,
}

/// Margin buffer is available funds over equity: plenty is quiet, thin is orange, nearly out
/// is red.
pub fn margin_buffer_level(ratio: f64) -> BufferLevel {
    if ratio < 0.1 {
        BufferLevel::Critical
    } else if ratio < 0.25 {
        BufferLevel::Thin
    } else {
        BufferLevel::Normal
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;

    fn instrument(tick: f64, step: f64) -> PaperInstrument {
        let mut instrument = PaperInstrument::chart("BTCUSDT", "Bitcoin", DataSource::Binance);
        instrument.tick_size = tick;
        instrument.quantity_increment = step;
        instrument
    }

    #[test]
    fn increments_give_their_decimal_places() {
        for (increment, places) in [
            (1.0, 0),
            (10.0, 0),
            (0.1, 1),
            (0.01, 2),
            (0.001, 3),
            (0.000_000_01, 8),
            (0.25, 2),
            (0.0, 0),
            (-1.0, 0),
            (f64::NAN, 0),
        ] {
            assert_eq!(decimal_places(increment), places, "{increment}");
        }
    }

    #[test]
    fn money_groups_digits_and_rounds_half_even() {
        assert_eq!(money(1_234.5, PaperCurrency::Usd), "$1,234.50");
        assert_eq!(money(-12.0, PaperCurrency::Usd), "-$12.00");
        assert_eq!(
            money(0.125, PaperCurrency::Usd),
            "$0.12",
            "half-even: 12 is even"
        );
        assert_eq!(money(0.135, PaperCurrency::Usd), "$0.14");
        assert_eq!(money(2.5, PaperCurrency::Jpy), "¥2");
        assert_eq!(money(3.5, PaperCurrency::Jpy), "¥4");
        assert_eq!(money(100_000.0, PaperCurrency::Usdt), "USDT 100,000.00");
        assert_eq!(
            money(-0.001, PaperCurrency::Usd),
            "$0.00",
            "a negative that rounds to zero has no sign"
        );
        assert_eq!(money(5.0, PaperCurrency::Eur), "€5.00");
    }

    #[test]
    fn only_gains_get_a_plus() {
        assert_eq!(signed_money(12.0, PaperCurrency::Usd), "+$12.00");
        assert_eq!(signed_money(-12.0, PaperCurrency::Usd), "-$12.00");
        assert_eq!(signed_money(0.0, PaperCurrency::Usd), "$0.00");
        assert_eq!(signed_money(0.001, PaperCurrency::Usd), "$0.00");
        assert_eq!(signed_money(0.01, PaperCurrency::Usd), "+$0.01");
    }

    #[test]
    fn percentages_keep_two_decimals() {
        assert_eq!(percent(0.233_49), "23.35%");
        assert_eq!(percent(-0.041), "-4.10%");
        assert_eq!(percent(0.0), "0.00%");
        assert_eq!(signed_percent(0.233_49), "+23.35%");
        assert_eq!(signed_percent(-0.041), "-4.10%");
        assert_eq!(signed_percent(0.0), "0.00%");
    }

    #[test]
    fn prices_and_quantities_follow_the_instruments_increments() {
        let btc = instrument(0.01, 0.000_001);
        assert_eq!(
            price(67_432.1, &btc),
            "67,432.1",
            "trailing zeros are dropped"
        );
        assert_eq!(price(67_432.126, &btc), "67,432.13");
        assert_eq!(quantity(0.5, &btc), "0.5");
        assert_eq!(quantity(0.123_456_789, &btc), "0.123457");
        let share = instrument(0.01, 1.0);
        assert_eq!(quantity(10.0, &share), "10");
        assert_eq!(quantity(1_500.0, &share), "1,500");
        assert_eq!(price(-0.004, &share), "0", "rounds to zero without a sign");
        assert_eq!(price(-3.0, &share), "-3");
    }

    #[test]
    fn timestamps_are_month_day_and_a_clock_in_the_viewers_zone() {
        // 2024-01-15 23:30:07 UTC.
        let ms = 1_705_361_407_000;
        assert_eq!(timestamp(ms, 0), "Jan 15, 23:30:07");
        assert_eq!(timestamp(ms, 2 * 3_600), "Jan 16, 01:30:07");
        assert_eq!(timestamp(0, 0), "Jan 1, 00:00:00");
    }

    #[test]
    fn labels_match_the_swift_app() {
        assert_eq!(PaperOrderSide::Buy.badge_text(), "BUY");
        assert_eq!(PaperPositionSide::Short.badge_text(), "SHORT");
        assert_eq!(PaperOrderType::StopLimit.label(), "Stop Limit");
        assert_eq!(PaperOrderType::StopLimit.badge_text(), "STP LMT");
        assert_eq!(
            PaperTimeInForce::GoodTilCanceled.label(),
            "Good till canceled"
        );
        assert_eq!(PaperOrderRole::TakeProfit.badge_text(), "TP");
        assert_eq!(PaperOrderStatus::PendingCancel.label(), "Cancelling");
        assert_eq!(PaperOrderEventKind::PartiallyFilled.label(), "Partial fill");
        assert_eq!(PaperCurrency::Usdc.code(), "USDC");
    }

    #[test]
    fn statuses_and_events_are_toned() {
        assert_eq!(PaperOrderStatus::Filled.tone(), BadgeTone::Good);
        assert_eq!(PaperOrderStatus::PartiallyFilled.tone(), BadgeTone::Warning);
        assert_eq!(PaperOrderStatus::Rejected.tone(), BadgeTone::Bad);
        assert_eq!(PaperOrderStatus::Working.tone(), BadgeTone::Neutral);
        assert_eq!(PaperOrderEventKind::Modified.tone(), BadgeTone::Warning);
        assert_eq!(PaperOrderEventKind::Canceled.tone(), BadgeTone::Neutral);
    }

    #[test]
    fn the_ticket_asks_for_the_prices_an_order_type_needs() {
        assert!(
            !PaperOrderType::Market.needs_limit_price()
                && !PaperOrderType::Market.needs_stop_price()
        );
        assert!(
            PaperOrderType::Limit.needs_limit_price() && !PaperOrderType::Limit.needs_stop_price()
        );
        assert!(
            !PaperOrderType::Stop.needs_limit_price() && PaperOrderType::Stop.needs_stop_price()
        );
        assert!(
            PaperOrderType::StopLimit.needs_limit_price()
                && PaperOrderType::StopLimit.needs_stop_price()
        );
    }

    #[test]
    fn pnl_and_margin_buffer_tones() {
        assert_eq!(pnl_tone(1.0), PnlTone::Profit);
        assert_eq!(pnl_tone(-1.0), PnlTone::Loss);
        assert_eq!(pnl_tone(0.0), PnlTone::Flat);
        assert_eq!(margin_buffer_level(0.05), BufferLevel::Critical);
        assert_eq!(margin_buffer_level(0.1), BufferLevel::Thin);
        assert_eq!(margin_buffer_level(0.2499), BufferLevel::Thin);
        assert_eq!(margin_buffer_level(0.25), BufferLevel::Normal);
    }
}
