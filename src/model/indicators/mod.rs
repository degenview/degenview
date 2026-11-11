pub mod catalog;

use crate::model::Kline;
use serde::{Deserialize, Serialize};

pub const RSI_PERIOD: usize = 14;
pub const RSI_OVERBOUGHT: f64 = 70.0;
pub const RSI_OVERSOLD: f64 = 30.0;
pub const BOLLINGER_PERIOD: usize = 20;
pub const BOLLINGER_MULTIPLIER: f64 = 2.0;
pub const SUPERTREND_PERIOD: usize = 10;
pub const SUPERTREND_MULTIPLIER: f64 = 3.0;
pub const EMA_DEFAULT_PERIOD: usize = 20;
pub const EMA_PERIODS: [usize; 5] = [9, 20, 50, 100, 200];

/// Which built-in indicators a chart draws.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct IndicatorConfig {
    pub show_volume: bool,
    pub show_rsi: bool,
    pub show_ema: bool,
    pub ema_period: usize,
    pub show_bollinger: bool,
    pub show_trend_flips: bool,
    #[serde(default)]
    pub hide_volume: bool,
    #[serde(default)]
    pub hide_rsi: bool,
    #[serde(default)]
    pub hide_ema: bool,
    #[serde(default)]
    pub hide_bollinger: bool,
    #[serde(default)]
    pub hide_trend_flips: bool,
}

impl Default for IndicatorConfig {
    fn default() -> Self {
        Self {
            show_volume: false,
            show_rsi: false,
            show_ema: false,
            ema_period: EMA_DEFAULT_PERIOD,
            show_bollinger: false,
            show_trend_flips: false,
            hide_volume: false,
            hide_rsi: false,
            hide_ema: false,
            hide_bollinger: false,
            hide_trend_flips: false,
        }
    }
}

impl IndicatorConfig {
    /// The flags shown in the settings sheet mean "applied". Rendering additionally
    /// respects the header eye controls, while the chips remain available to restore them.
    pub fn drawn(self) -> Self {
        Self {
            show_volume: self.show_volume && !self.hide_volume,
            show_rsi: self.show_rsi && !self.hide_rsi,
            show_ema: self.show_ema && !self.hide_ema,
            show_bollinger: self.show_bollinger && !self.hide_bollinger,
            show_trend_flips: self.show_trend_flips && !self.hide_trend_flips,
            ..self
        }
    }
}

/// One of the chart's built-in indicators. The single list the chart UI and the MCP server
/// both draw from: each can be applied once per chart, and only EMA takes a parameter.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BuiltIn {
    Volume,
    Rsi,
    Ema,
    Bollinger,
    TrendFlips,
}

impl BuiltIn {
    pub const ALL: [Self; 5] = [
        Self::Volume,
        Self::Rsi,
        Self::Ema,
        Self::Bollinger,
        Self::TrendFlips,
    ];

    /// The stable identifier (`volume`, `rsi`, `ema`, `bollinger`, `trend_flips`).
    pub fn id(self) -> &'static str {
        match self {
            Self::Volume => "volume",
            Self::Rsi => "rsi",
            Self::Ema => "ema",
            Self::Bollinger => "bollinger",
            Self::TrendFlips => "trend_flips",
        }
    }

    pub fn menu_title(self) -> &'static str {
        match self {
            Self::Volume => "Volume",
            Self::Rsi => "RSI",
            Self::Ema => "EMA",
            Self::Bollinger => "Bollinger Bands",
            Self::TrendFlips => "Trend Flips",
        }
    }

    /// Whether it draws on the price pane (`true`) or in a pane of its own.
    pub fn is_overlay(self) -> bool {
        match self {
            Self::Volume | Self::Rsi => false,
            Self::Ema | Self::Bollinger | Self::TrendFlips => true,
        }
    }

    /// Volume has nothing to draw on a line chart (prediction markets, CoinMarketCap).
    pub fn supports_line_charts(self) -> bool {
        self != Self::Volume
    }

    pub fn chip_title(self, config: &IndicatorConfig) -> String {
        match self {
            Self::Volume => "Vol".into(),
            Self::Rsi => format!("RSI {RSI_PERIOD}"),
            Self::Ema => format!("EMA {}", config.ema_period),
            Self::Bollinger => "BB".into(),
            Self::TrendFlips => "Trend Flips".into(),
        }
    }

    pub fn applied(self, config: &IndicatorConfig) -> bool {
        match self {
            Self::Volume => config.show_volume,
            Self::Rsi => config.show_rsi,
            Self::Ema => config.show_ema,
            Self::Bollinger => config.show_bollinger,
            Self::TrendFlips => config.show_trend_flips,
        }
    }

    pub fn hidden(self, config: &IndicatorConfig) -> bool {
        match self {
            Self::Volume => config.hide_volume,
            Self::Rsi => config.hide_rsi,
            Self::Ema => config.hide_ema,
            Self::Bollinger => config.hide_bollinger,
            Self::TrendFlips => config.hide_trend_flips,
        }
    }

    pub fn set_applied(self, config: &mut IndicatorConfig, applied: bool) {
        match self {
            Self::Volume => config.show_volume = applied,
            Self::Rsi => config.show_rsi = applied,
            Self::Ema => config.show_ema = applied,
            Self::Bollinger => config.show_bollinger = applied,
            Self::TrendFlips => config.show_trend_flips = applied,
        }
        if applied {
            self.set_hidden(config, false);
        }
    }

    fn set_hidden(self, config: &mut IndicatorConfig, hidden: bool) {
        match self {
            Self::Volume => config.hide_volume = hidden,
            Self::Rsi => config.hide_rsi = hidden,
            Self::Ema => config.hide_ema = hidden,
            Self::Bollinger => config.hide_bollinger = hidden,
            Self::TrendFlips => config.hide_trend_flips = hidden,
        }
    }

    pub fn toggle_visible(self, config: &mut IndicatorConfig) {
        self.set_hidden(config, !self.hidden(config));
    }
}

#[derive(Clone, Debug, Default)]
pub struct BollingerSeries {
    pub upper: Vec<Option<f64>>,
    pub middle: Vec<Option<f64>>,
    pub lower: Vec<Option<f64>>,
}

/// Indicator readings aligned 1:1 with the visible candles. Empty means off.
#[derive(Clone, Debug, Default)]
pub struct IndicatorSeries {
    pub rsi: Vec<Option<f64>>,
    pub ema: Vec<Option<f64>>,
    pub bollinger: Option<BollingerSeries>,
    /// `Some(true)` bullish flip, `Some(false)` bearish.
    pub trend_flips: Vec<Option<bool>>,
}

impl IndicatorSeries {
    /// Compute every enabled indicator over the full `candles` buffer, then trim to
    /// the visible `[start, start + len)` tail so period indicators are warm at the
    /// left edge.
    pub fn compute(
        candles: &[Kline],
        start: usize,
        len: usize,
        config: &IndicatorConfig,
    ) -> IndicatorSeries {
        let mut series = IndicatorSeries::default();
        if candles.is_empty() {
            return series;
        }
        let end = (start + len).min(candles.len());

        if config.show_rsi {
            series.rsi = rsi(candles, RSI_PERIOD)[start..end].to_vec();
        }
        if config.show_ema {
            series.ema = ema(candles, config.ema_period)[start..end].to_vec();
        }
        if config.show_bollinger {
            let (upper, middle, lower) = bollinger(candles, BOLLINGER_PERIOD, BOLLINGER_MULTIPLIER);
            series.bollinger = Some(BollingerSeries {
                upper: upper[start..end].to_vec(),
                middle: middle[start..end].to_vec(),
                lower: lower[start..end].to_vec(),
            });
        }
        if config.show_trend_flips {
            series.trend_flips =
                supertrend_flips(candles, SUPERTREND_PERIOD, SUPERTREND_MULTIPLIER)[start..end]
                    .to_vec();
        }
        series
    }
}

/// Exponential moving average of closes, seeded with the simple average of the
/// first `period` closes.
pub fn ema(candles: &[Kline], period: usize) -> Vec<Option<f64>> {
    let mut values = vec![None; candles.len()];
    if period == 0 || candles.len() < period {
        return values;
    }
    let mut seed = 0.0;
    for candle in &candles[..period] {
        seed += candle.close;
    }
    seed /= period as f64;
    values[period - 1] = Some(seed);

    let alpha = 2.0 / (period as f64 + 1.0);
    let mut previous = seed;
    for i in period..candles.len() {
        previous = candles[i].close * alpha + previous * (1.0 - alpha);
        values[i] = Some(previous);
    }
    values
}

/// Relative Strength Index with Wilder's smoothing (α = 1/period).
pub fn rsi(candles: &[Kline], period: usize) -> Vec<Option<f64>> {
    let mut values = vec![None; candles.len()];
    if period == 0 || candles.len() <= period {
        return values;
    }
    let mut average_gain = 0.0;
    let mut average_loss = 0.0;
    for i in 1..=period {
        let change = candles[i].close - candles[i - 1].close;
        if change >= 0.0 {
            average_gain += change;
        } else {
            average_loss -= change;
        }
    }
    average_gain /= period as f64;
    average_loss /= period as f64;
    values[period] = Some(rsi_from(average_gain, average_loss));

    if candles.len() <= period + 1 {
        return values;
    }
    let weight = (period - 1) as f64;
    for i in (period + 1)..candles.len() {
        let change = candles[i].close - candles[i - 1].close;
        average_gain = (average_gain * weight + change.max(0.0)) / period as f64;
        average_loss = (average_loss * weight + (-change).max(0.0)) / period as f64;
        values[i] = Some(rsi_from(average_gain, average_loss));
    }
    values
}

fn rsi_from(gain: f64, loss: f64) -> f64 {
    if loss > 0.0 {
        100.0 - 100.0 / (1.0 + gain / loss)
    } else if gain > 0.0 {
        100.0
    } else {
        50.0
    }
}

/// A series of optional readings, aligned with the candles.
type OptionSeries = Vec<Option<f64>>;

/// Bollinger bands: an SMA with `multiplier` population standard deviations either
/// side. Population rather than sample deviation (divide by N). The variance is a
/// two-pass sum of squared deviations, which stays accurate at BTC's magnitude
/// where `E[x²] − E[x]²` cancels to noise.
pub fn bollinger(
    candles: &[Kline],
    period: usize,
    multiplier: f64,
) -> (OptionSeries, OptionSeries, OptionSeries) {
    let blank = vec![None; candles.len()];
    let mut upper = blank.clone();
    let mut middle = blank.clone();
    let mut lower = blank;
    if period < 2 || candles.len() < period {
        return (upper, middle, lower);
    }
    let window = period as f64;
    for i in (period - 1)..candles.len() {
        let start = i + 1 - period;
        let mut sum = 0.0;
        for candle in &candles[start..=i] {
            sum += candle.close;
        }
        let mean = sum / window;
        let mut squared_deviation = 0.0;
        for candle in &candles[start..=i] {
            let d = candle.close - mean;
            squared_deviation += d * d;
        }
        let deviation = (squared_deviation / window).sqrt() * multiplier;
        middle[i] = Some(mean);
        upper[i] = Some(mean + deviation);
        lower[i] = Some(mean - deviation);
    }
    (upper, middle, lower)
}

/// Supertrend regime changes using a Wilder-smoothed ATR. The final candle's signal
/// is suppressed (its values can still change).
pub fn supertrend_flips(candles: &[Kline], period: usize, multiplier: f64) -> Vec<Option<bool>> {
    let mut flips = vec![None; candles.len()];
    if period == 0 || multiplier <= 0.0 || candles.len() < period {
        return flips;
    }
    let mut true_ranges = vec![0.0; candles.len()];
    for i in 0..candles.len() {
        let candle = &candles[i];
        let high_low = candle.high - candle.low;
        true_ranges[i] = if i == 0 {
            high_low
        } else {
            let previous_close = candles[i - 1].close;
            high_low
                .max((candle.high - previous_close).abs())
                .max((candle.low - previous_close).abs())
        };
    }

    let seed_index = period - 1;
    let mut atr = true_ranges[0..=seed_index].iter().sum::<f64>() / period as f64;
    let seed_midpoint = (candles[seed_index].high + candles[seed_index].low) / 2.0;
    let mut upper_band = seed_midpoint + multiplier * atr;
    let mut lower_band = seed_midpoint - multiplier * atr;
    let mut is_bullish = false;
    let mut was_upper_band = true;

    if candles.len() <= period {
        return flips;
    }
    for i in period..candles.len() {
        atr = (atr * (period - 1) as f64 + true_ranges[i]) / period as f64;
        let candle = &candles[i];
        let midpoint = (candle.high + candle.low) / 2.0;
        let basic_upper = midpoint + multiplier * atr;
        let basic_lower = midpoint - multiplier * atr;
        let previous_close = candles[i - 1].close;
        if basic_upper < upper_band || previous_close > upper_band {
            upper_band = basic_upper;
        }
        if basic_lower > lower_band || previous_close < lower_band {
            lower_band = basic_lower;
        }
        let previous_direction = is_bullish;
        is_bullish = if was_upper_band {
            candle.close > upper_band
        } else {
            candle.close >= lower_band
        };
        was_upper_band = !is_bullish;
        if is_bullish != previous_direction {
            flips[i] = Some(is_bullish);
        }
    }
    let last = flips.len() - 1;
    flips[last] = None;
    flips
}

#[cfg(test)]
mod tests {
    #[test]
    fn built_in_ids_are_stable_and_describe_placement() {
        let ids: Vec<_> = BuiltIn::ALL.iter().map(|kind| kind.id()).collect();
        assert_eq!(ids, ["volume", "rsi", "ema", "bollinger", "trend_flips"]);
        assert!(BuiltIn::Ema.is_overlay() && BuiltIn::Bollinger.is_overlay());
        assert!(!BuiltIn::Rsi.is_overlay() && !BuiltIn::Volume.is_overlay());
        assert!(!BuiltIn::Volume.supports_line_charts());
    }

    use super::*;

    fn candle(close: f64, high: f64, low: f64) -> Kline {
        Kline {
            open_time: 0,
            close_time: 0,
            open: close,
            high,
            low,
            close,
            volume: 0.0,
        }
    }

    #[test]
    fn ema_seeds_with_simple_average() {
        let candles: Vec<Kline> = (1..=5)
            .map(|i| candle(i as f64, i as f64, i as f64))
            .collect();
        let values = ema(&candles, 3);
        assert_eq!(values[0], None);
        assert_eq!(values[1], None);
        assert_eq!(values[2], Some(2.0)); // seed = (1+2+3)/3
        assert!(values[4].is_some());
    }

    #[test]
    fn rsi_pins_to_100_on_all_gains() {
        let candles: Vec<Kline> = (0..20)
            .map(|i| candle(i as f64 + 10.0, i as f64 + 10.0, i as f64 + 10.0))
            .collect();
        let values = rsi(&candles, 14);
        assert_eq!(values[14], Some(100.0));
    }

    #[test]
    fn bollinger_upper_above_middle_above_lower() {
        let candles: Vec<Kline> = (0..30)
            .map(|i| {
                let v = (i % 10) as f64;
                candle(v, v + 0.5, v - 0.5)
            })
            .collect();
        let (upper, middle, lower) = bollinger(&candles, 20, 2.0);
        let i = 29;
        let (u, m, l) = (upper[i].unwrap(), middle[i].unwrap(), lower[i].unwrap());
        assert!(u >= m && m >= l);
    }

    /// Prints what the indicators cost per frame (they are recomputed over every candle each frame).
    /// Run with `cargo test --release --lib indicator_frame_cost -- --ignored --nocapture`.
    #[test]
    #[ignore = "a measurement, not a check"]
    fn indicator_frame_cost_report() {
        let config = IndicatorConfig {
            show_volume: true,
            show_rsi: true,
            show_ema: true,
            show_bollinger: true,
            show_trend_flips: true,
            ..IndicatorConfig::default()
        };
        for count in [500, 10_000, 100_000] {
            let candles: Vec<Kline> = (0..count)
                .map(|index| {
                    let close = 100.0 + (index as f64 * 0.01).sin() * 20.0;
                    Kline {
                        open_time: index as i64 * 3_600_000,
                        close_time: (index as i64 + 1) * 3_600_000 - 1,
                        open: close,
                        high: close + 1.0,
                        low: close - 1.0,
                        close,
                        volume: 1.0,
                    }
                })
                .collect();
            let started = std::time::Instant::now();
            let runs = 20;
            for _ in 0..runs {
                std::hint::black_box(IndicatorSeries::compute(
                    &candles,
                    count - 120,
                    120,
                    &config,
                ));
            }
            println!(
                "{count:>7} candles: {:?} per frame",
                started.elapsed() / runs
            );
        }
    }
}
