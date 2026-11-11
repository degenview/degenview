//! What a watchlist row shows for one market. Transient: never stored with a
//! watchlist, and a missing value stays `None` so the row shows a dash, not zero.

use crate::format;
use crate::model::{DataSource, PriceFormat};

/// Text for a value the provider did not supply.
pub const MISSING: &str = "–";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChangeBasis {
    Rolling24h,
    PreviousClose,
    Window,
}

impl ChangeBasis {
    pub fn for_source(source: DataSource) -> Self {
        match source {
            DataSource::Binance
            | DataSource::Coinbase
            | DataSource::CoinGecko
            | DataSource::DexScreener => Self::Rolling24h,
            DataSource::Alpaca => Self::PreviousClose,
            DataSource::Polymarket | DataSource::Kalshi | DataSource::CoinMarketCap => Self::Window,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Rolling24h => "Change over the last 24 hours",
            Self::PreviousClose => "Change since the previous close",
            Self::Window => "Change over the market's comparison window",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Freshness {
    Live,
    Stale,
    MarketClosed,
    Unavailable,
    Unsupported,
}

impl Freshness {
    pub fn label(self) -> &'static str {
        match self {
            Self::Live => "Live",
            Self::Stale => "Stale",
            Self::MarketClosed => "Market closed",
            Self::Unavailable => "No quote",
            Self::Unsupported => "No quotes for this market",
        }
    }

    /// Whether the price can be shown as current.
    pub fn is_current(self) -> bool {
        matches!(self, Self::Live | Self::MarketClosed)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VolumeKind {
    QuoteCurrency,
    Shares,
    Base,
}

/// A provider's answer for one market, before it becomes a row value. Mirrors the
/// Swift `SourceQuote`: only what the quote endpoints really return.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SourceQuote {
    pub price: f64,
    pub previous_day_price: Option<f64>,
    pub volume_24h: Option<f64>,
    pub volume_kind: Option<VolumeKind>,
    pub timestamp_ms: Option<i64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WatchlistQuote {
    pub last: Option<f64>,
    pub change: Option<f64>,
    pub change_percent: Option<f64>,
    pub change_basis: Option<ChangeBasis>,
    pub volume: Option<f64>,
    pub volume_kind: Option<VolumeKind>,
    pub timestamp_ms: Option<i64>,
    pub freshness: Freshness,
    /// Why a quote is missing, when the provider said ("Add your Alpaca API key in Settings").
    pub note: Option<String>,
}

impl Default for WatchlistQuote {
    fn default() -> Self {
        Self {
            last: None,
            change: None,
            change_percent: None,
            change_basis: None,
            volume: None,
            volume_kind: None,
            timestamp_ms: None,
            freshness: Freshness::Unavailable,
            note: None,
        }
    }
}

impl WatchlistQuote {
    pub fn unsupported(note: Option<String>) -> Self {
        Self {
            freshness: Freshness::Unsupported,
            note,
            ..Self::default()
        }
    }

    /// The same quote re-labelled, for the last known value of a source that stopped answering.
    pub fn with_freshness(&self, freshness: Freshness) -> Self {
        Self {
            freshness,
            ..self.clone()
        }
    }

    /// A provider's answer as a row value. Change is the provider's own reference (rolling
    /// 24h, or the previous close for stocks); with no usable reference it stays `None`
    /// rather than being guessed from other data.
    pub fn from_source(source: DataSource, quote: &SourceQuote, received_at_ms: i64) -> Self {
        let (mut change, mut percent) = (None, None);
        if let Some(previous) = quote.previous_day_price
            && previous.is_finite()
            && quote.price.is_finite()
        {
            if source.is_prediction_market() {
                let delta = quote.price - previous;
                change = Some(delta);
                // Percentage points of probability.
                percent = Some(delta * 100.0);
            } else if previous > 0.0 {
                change = Some(quote.price - previous);
                percent = Some((quote.price / previous - 1.0) * 100.0);
            }
        }
        Self {
            last: Some(quote.price),
            change,
            change_percent: percent,
            change_basis: Some(ChangeBasis::for_source(source)),
            volume: quote.volume_24h,
            volume_kind: quote.volume_kind,
            timestamp_ms: quote.timestamp_ms.or(Some(received_at_ms)),
            freshness: evaluate_freshness(source, received_at_ms, received_at_ms),
            note: None,
        }
    }
}

// MARK: Freshness

/// Seconds between refreshes of a source while a sidebar is visible.
pub fn refresh_interval_secs(source: DataSource) -> u64 {
    match source {
        DataSource::Binance | DataSource::Coinbase | DataSource::Alpaca => 5,
        DataSource::DexScreener => 10,
        DataSource::CoinGecko | DataSource::Polymarket | DataSource::Kalshi => 30,
        DataSource::CoinMarketCap => 0,
    }
}

/// How long after its last refresh a price is still shown as current: a few poll
/// intervals, so one missed round does not dim a row but a real outage does.
pub fn maximum_age_secs(source: DataSource) -> i64 {
    match source {
        DataSource::Binance | DataSource::Alpaca => 45,
        DataSource::DexScreener => 60,
        // Coinbase is refreshed from REST every 30 seconds when the socket has been quiet.
        DataSource::Coinbase => 90,
        DataSource::CoinGecko | DataSource::Polymarket | DataSource::Kalshi => 120,
        DataSource::CoinMarketCap => 0,
    }
}

/// `Stale` once the last refresh is older than the source's window; a stock outside US
/// hours reads `MarketClosed` (its last price is the close, still current); else `Live`.
pub fn evaluate_freshness(source: DataSource, received_at_ms: i64, now_ms: i64) -> Freshness {
    if now_ms - received_at_ms > maximum_age_secs(source) * 1_000 {
        return Freshness::Stale;
    }
    if source == DataSource::Alpaca && !is_us_market_open(now_ms) {
        return Freshness::MarketClosed;
    }
    Freshness::Live
}

/// Regular US session, Monday to Friday 9:30 to 16:00 Eastern. Holidays are not
/// modelled, so on one a stock reads "live" at its last close.
pub fn is_us_market_open(epoch_ms: i64) -> bool {
    let local = epoch_ms.div_euclid(1_000) + eastern_offset_secs(epoch_ms);
    let days = local.div_euclid(86_400);
    // 1970-01-01 was a Thursday; 0 = Monday.
    let weekday = (days + 3).rem_euclid(7);
    if weekday >= 5 {
        return false;
    }
    let minutes = local.rem_euclid(86_400) / 60;
    (9 * 60 + 30..16 * 60).contains(&minutes)
}

/// UTC offset of America/New_York at `epoch_ms`: -5h, or -4h during US daylight time
/// (02:00 local on the second Sunday of March to 02:00 on the first Sunday of November).
fn eastern_offset_secs(epoch_ms: i64) -> i64 {
    let seconds = epoch_ms.div_euclid(1_000);
    let (year, ..) = format::civil_from_days(seconds.div_euclid(86_400));
    // Transitions in UTC: 02:00 EST = 07:00 UTC (spring); 02:00 EDT = 06:00 UTC (autumn).
    let spring = nth_sunday(year, 3, 2) * 86_400 + 7 * 3_600;
    let autumn = nth_sunday(year, 11, 1) * 86_400 + 6 * 3_600;
    if (spring..autumn).contains(&seconds) {
        -4 * 3_600
    } else {
        -5 * 3_600
    }
}

/// Days since the epoch of the `nth` Sunday of a month.
fn nth_sunday(year: i64, month: i64, nth: i64) -> i64 {
    let first = format::days_from_civil(year, month, 1);
    // Sunday is weekday 6 with Monday = 0.
    let weekday = (first + 3).rem_euclid(7);
    first + (6 - weekday).rem_euclid(7) + (nth - 1) * 7
}

// MARK: Formatting

/// Direction as a glyph, so it never rests on red and green alone.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Direction {
    Up,
    Down,
    Flat,
}

impl Direction {
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Up => "▲",
            Self::Down => "▼",
            Self::Flat => "",
        }
    }
}

pub fn direction(quote: Option<&WatchlistQuote>) -> Direction {
    let value = quote.and_then(|quote| quote.change_percent.or(quote.change));
    match value {
        Some(value) if value > 0.0 => Direction::Up,
        Some(value) if value < 0.0 => Direction::Down,
        _ => Direction::Flat,
    }
}

fn is_probability(source: Option<DataSource>) -> bool {
    source.is_some_and(DataSource::is_prediction_market)
}

/// 0…1 → "66.5%". Clamped, since a stale quote can overshoot slightly.
fn probability_text(value: f64) -> String {
    format!("{:.1}%", (value * 100.0).clamp(0.0, 100.0))
}

pub fn last_text(quote: Option<&WatchlistQuote>, source: Option<DataSource>) -> String {
    quote.and_then(|quote| quote.last).map_or_else(
        || MISSING.to_owned(),
        |last| {
            if is_probability(source) {
                probability_text(last)
            } else {
                PriceFormat::AUTO.format(last)
            }
        },
    )
}

/// "+12.50" / "-0.0031": signed, at the precision the last price is shown with; "+7.0 pp" for
/// probabilities.
pub fn change_text(quote: Option<&WatchlistQuote>, source: Option<DataSource>) -> String {
    quote.and_then(|quote| quote.change).map_or_else(
        || MISSING.to_owned(),
        |change| signed_change(change, source),
    )
}

fn signed_change(value: f64, source: Option<DataSource>) -> String {
    let sign = if value < 0.0 { "-" } else { "+" };
    if is_probability(source) {
        format!("{sign}{:.1} pp", value.abs() * 100.0)
    } else {
        format!("{sign}{}", PriceFormat::AUTO.format(value.abs()))
    }
}

/// Prediction markets move in percentage points of probability, not percent of a price.
pub fn percent_text(quote: Option<&WatchlistQuote>, source: Option<DataSource>) -> String {
    let Some(quote) = quote else {
        return MISSING.to_owned();
    };
    let Some(percent) = quote.change_percent else {
        return MISSING.to_owned();
    };
    if let (true, Some(change)) = (is_probability(source), quote.change) {
        return signed_change(change, source);
    }
    format!("{percent:+.2}%")
}

pub fn volume_text(quote: Option<&WatchlistQuote>) -> String {
    let Some(quote) = quote else {
        return MISSING.to_owned();
    };
    let Some(volume) = quote.volume else {
        return MISSING.to_owned();
    };
    let text = compact_number(volume);
    if quote.volume_kind == Some(VolumeKind::QuoteCurrency) {
        format!("${text}")
    } else {
        text
    }
}

/// 1,234 → "1.23K", 5,600,000 → "5.6M".
pub fn compact_number(value: f64) -> String {
    const UNITS: [(f64, &str); 4] = [
        (1_000_000_000_000.0, "T"),
        (1_000_000_000.0, "B"),
        (1_000_000.0, "M"),
        (1_000.0, "K"),
    ];
    for (divisor, suffix) in UNITS {
        if value.abs() >= divisor {
            return format!("{}{suffix}", trimmed(value / divisor));
        }
    }
    trimmed(value)
}

/// At most two decimals, trailing zeros dropped.
fn trimmed(value: f64) -> String {
    let text = format!("{value:.2}");
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2024-01-15 (Monday) 15:00 UTC = 10:00 EST.
    const MONDAY_10AM_EST: i64 = 1_705_330_800_000;

    fn source_quote(price: f64, previous: Option<f64>) -> SourceQuote {
        SourceQuote {
            price,
            previous_day_price: previous,
            ..SourceQuote::default()
        }
    }

    #[test]
    fn change_is_measured_against_the_provider_reference() {
        let quote = WatchlistQuote::from_source(
            DataSource::Binance,
            &source_quote(110.0, Some(100.0)),
            MONDAY_10AM_EST,
        );
        assert_eq!(quote.change, Some(10.0));
        assert!((quote.change_percent.unwrap() - 10.0).abs() < 1e-9);
        assert_eq!(quote.change_basis, Some(ChangeBasis::Rolling24h));
        assert_eq!(quote.freshness, Freshness::Live);
    }

    #[test]
    fn missing_reference_stays_missing_never_zero() {
        let quote = WatchlistQuote::from_source(
            DataSource::Coinbase,
            &source_quote(5.0, None),
            MONDAY_10AM_EST,
        );
        assert_eq!((quote.change, quote.change_percent), (None, None));
        assert_eq!(change_text(Some(&quote), None), MISSING);
        assert_eq!(percent_text(Some(&quote), None), MISSING);
        // A zero previous price is not a usable reference either.
        let zero = WatchlistQuote::from_source(
            DataSource::Binance,
            &source_quote(5.0, Some(0.0)),
            MONDAY_10AM_EST,
        );
        assert_eq!(zero.change_percent, None);
    }

    #[test]
    fn prediction_markets_move_in_percentage_points() {
        let quote = WatchlistQuote::from_source(
            DataSource::Polymarket,
            &source_quote(0.62, Some(0.55)),
            MONDAY_10AM_EST,
        );
        assert!((quote.change.unwrap() - 0.07).abs() < 1e-9);
        assert!((quote.change_percent.unwrap() - 7.0).abs() < 1e-9);
        assert_eq!(
            percent_text(Some(&quote), Some(DataSource::Polymarket)),
            "+7.0 pp"
        );
        assert_eq!(
            last_text(Some(&quote), Some(DataSource::Polymarket)),
            "62.0%"
        );
        // A drop keeps its sign through the price formatter's magnitude-only rules.
        let down = WatchlistQuote::from_source(
            DataSource::Kalshi,
            &source_quote(0.40, Some(0.50)),
            MONDAY_10AM_EST,
        );
        assert_eq!(
            percent_text(Some(&down), Some(DataSource::Kalshi)),
            "-10.0 pp"
        );
    }

    #[test]
    fn freshness_windows_and_market_hours() {
        let received = MONDAY_10AM_EST;
        assert_eq!(
            evaluate_freshness(DataSource::Binance, received, received + 45_000),
            Freshness::Live
        );
        assert_eq!(
            evaluate_freshness(DataSource::Binance, received, received + 46_000),
            Freshness::Stale
        );
        assert_eq!(
            evaluate_freshness(DataSource::Coinbase, received, received + 90_000),
            Freshness::Live
        );
        assert_eq!(
            evaluate_freshness(DataSource::Alpaca, received, received),
            Freshness::Live
        );
        // Sunday 2024-01-14 15:00 UTC: closed, and a closed price is current.
        let sunday = received - 86_400_000;
        let closed = evaluate_freshness(DataSource::Alpaca, sunday, sunday);
        assert_eq!(closed, Freshness::MarketClosed);
        assert!(closed.is_current());
    }

    #[test]
    fn us_market_hours_follow_daylight_time() {
        // Monday 2024-01-15 14:29 UTC = 09:29 EST (closed); 14:30 UTC = 09:30 (open).
        assert!(!is_us_market_open(1_705_328_940_000));
        assert!(is_us_market_open(1_705_329_000_000));
        // 21:00 UTC = 16:00 EST: closed again.
        assert!(!is_us_market_open(1_705_352_400_000));
        // Monday 2024-07-15 13:30 UTC = 09:30 EDT (open); 13:29 UTC is closed.
        assert!(is_us_market_open(1_721_050_200_000));
        assert!(!is_us_market_open(1_721_050_140_000));
        // Saturday 2024-07-13 15:00 UTC.
        assert!(!is_us_market_open(1_720_882_800_000));
    }

    #[test]
    fn daylight_time_boundaries_for_2024() {
        // Second Sunday of March 2024 is the 10th; first Sunday of November is the 3rd.
        assert_eq!(nth_sunday(2024, 3, 2), format::days_from_civil(2024, 3, 10));
        assert_eq!(
            nth_sunday(2024, 11, 1),
            format::days_from_civil(2024, 11, 3)
        );
        let before = format::days_from_civil(2024, 3, 10) * 86_400_000 + 6 * 3_600_000;
        let after = format::days_from_civil(2024, 3, 10) * 86_400_000 + 7 * 3_600_000;
        assert_eq!(eastern_offset_secs(before), -5 * 3_600);
        assert_eq!(eastern_offset_secs(after), -4 * 3_600);
    }

    #[test]
    fn column_text() {
        let quote = WatchlistQuote {
            last: Some(67_432.19),
            change: Some(-12.5),
            change_percent: Some(-1.234),
            volume: Some(5_600_000.0),
            volume_kind: Some(VolumeKind::QuoteCurrency),
            freshness: Freshness::Live,
            ..WatchlistQuote::default()
        };
        assert_eq!(last_text(Some(&quote), None), "67,432");
        assert_eq!(change_text(Some(&quote), None), "-12.50");
        // Large moves lose the cents' fourth place, small ones keep two significant digits.
        let big = WatchlistQuote {
            change: Some(792.01),
            ..quote.clone()
        };
        assert_eq!(change_text(Some(&big), None), "+792.01");
        let small = WatchlistQuote {
            change: Some(-0.003_14),
            ..quote.clone()
        };
        assert_eq!(change_text(Some(&small), None), "-0.0031");
        assert_eq!(
            percent_text(Some(&quote), Some(DataSource::Binance)),
            "-1.23%"
        );
        assert_eq!(volume_text(Some(&quote)), "$5.6M");
        assert_eq!(direction(Some(&quote)), Direction::Down);
        assert_eq!(last_text(None, None), MISSING);
        assert_eq!(volume_text(None), MISSING);
        assert_eq!(direction(None), Direction::Flat);
    }

    #[test]
    fn compact_numbers() {
        assert_eq!(compact_number(1_234.0), "1.23K");
        assert_eq!(compact_number(5_600_000.0), "5.6M");
        assert_eq!(compact_number(2_000_000_000_000.0), "2T");
        assert_eq!(compact_number(999.0), "999");
        assert_eq!(compact_number(0.5), "0.5");
    }
}
