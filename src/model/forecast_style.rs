//! The settings of the forecasting, volume and measurement drawings that are not a line, a fill or a
//! font: a position's account and risk, a ghost feed's candles, a VWAP's source and bands, a
//! profile's rows and value area, what a measurer prints.
//!
//! Each is a small `Copy` struct inside [`DrawingStyle`](super::drawing_style::DrawingStyle), left
//! out of a stored drawing while it holds its default (so a rectangle carries none of them), brought
//! into range by `sanitized()` on every write, and saved by a template with the rest of the style.
//! Colours are the Charting Library's defaults where it states them (**lib**) and chosen here
//! otherwise (**inferred**). Pure: no egui types.

use super::chart_type::{PriceSource, Rgba};
use serde::{Deserialize, Serialize};

/// `value` when finite and within `range`, clamped into it; `fallback` when it is not a number.
fn within(value: f32, range: (f32, f32), fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(range.0, range.1)
    } else {
        fallback
    }
}

/// `value` when finite and within `range`, clamped into it; `fallback` when it is not a number.
fn within64(value: f64, range: (f64, f64), fallback: f64) -> f64 {
    if value.is_finite() {
        value.clamp(range.0, range.1)
    } else {
        fallback
    }
}

// MARK: Positions

/// A Long or Short position's account, risk and colours, and the Position forecast's result colours.
///
/// The sizing is TradingView's (see [`position`](super::position)); the defaults are the Charting
/// Library's: an account of 1,000, risking 25 %, a lot of one.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct PositionOptions {
    pub account: f64,
    pub lot: f64,
    pub risk: f64,
    /// Risk is a percentage of the account (`true`) or an amount of it.
    pub risk_percent: bool,
    pub leverage: f64,
    /// Decimal places of the quantity.
    pub qty_precision: u8,
    /// Shorter stats tags.
    pub compact: bool,
    /// Show the stats on an unselected position too.
    pub always_stats: bool,
    /// The profit zone and the loss zone.
    pub profit: Rgba,
    pub stop: Rgba,
    /// A forecast's result boxes.
    pub success: Rgba,
    pub failure: Rgba,
    /// The plate behind a stats tag.
    pub label: Rgba,
}

/// The most an account or a risk amount may be.
pub const MAX_ACCOUNT: f64 = 1.0e12;
/// The most decimal places of a quantity.
pub const MAX_QTY_PRECISION: u8 = 8;

impl Default for PositionOptions {
    fn default() -> Self {
        Self {
            account: 1_000.0,
            lot: 1.0,
            risk: 25.0,
            risk_percent: true,
            leverage: 1.0,
            qty_precision: 2,
            compact: false,
            always_stats: false,
            profit: [8, 153, 129, 51],
            stop: [242, 54, 69, 51],
            success: [76, 175, 80, 255],
            failure: [242, 54, 69, 255],
            label: [88, 88, 88, 255],
        }
    }
}

impl PositionOptions {
    pub fn sanitized(mut self) -> Self {
        let default = Self::default();
        self.account = within64(self.account, (0.0, MAX_ACCOUNT), default.account);
        self.lot = within64(self.lot, (1e-9, 1e9), default.lot);
        let risk_limit = if self.risk_percent {
            100.0
        } else {
            MAX_ACCOUNT
        };
        self.risk = within64(self.risk, (0.0, risk_limit), default.risk);
        self.leverage = within64(self.leverage, (0.01, 1_000.0), default.leverage);
        self.qty_precision = self.qty_precision.min(MAX_QTY_PRECISION);
        self
    }
}

// MARK: Ghost feed

/// A ghost feed's size and candles. The average range is a percentage of the starting price:
/// TradingView sizes it in ticks, which this build has no real size for.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct GhostOptions {
    /// The average high-to-low of a candle, in percent of the first price.
    pub average: f32,
    /// How much the range and the path wander, in percent (0 to 100).
    pub variance: f32,
    pub up: Rgba,
    pub down: Rgba,
    pub up_border: Rgba,
    pub down_border: Rgba,
    pub wick: Rgba,
    pub draw_border: bool,
    pub draw_wick: bool,
}

impl Default for GhostOptions {
    fn default() -> Self {
        // The library's: 50 % transparent bodies in a pale green and red, solid borders.
        Self {
            average: 0.5,
            variance: 50.0,
            up: [172, 229, 220, 128],
            down: [250, 161, 164, 128],
            up_border: [8, 153, 129, 255],
            down_border: [242, 54, 69, 255],
            wick: [128, 128, 128, 255],
            draw_border: true,
            draw_wick: true,
        }
    }
}

impl GhostOptions {
    pub fn sanitized(mut self) -> Self {
        let default = Self::default();
        self.average = within(self.average, (0.0, 100.0), default.average);
        self.variance = within(self.variance, (0.0, 100.0), default.variance);
        self
    }
}

// MARK: VWAP

/// One band of an anchored VWAP, a multiple of the standard deviation either side of it.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct VwapBand {
    pub enabled: bool,
    pub multiplier: f32,
    pub color: Rgba,
}

impl Default for VwapBand {
    fn default() -> Self {
        Self {
            enabled: false,
            multiplier: 1.0,
            color: [76, 175, 80, 255],
        }
    }
}

/// The most standard deviations a band may be.
pub const MAX_BAND_MULTIPLIER: f32 = 50.0;

/// An anchored VWAP's source and bands. The defaults are the library's: HLC3, bands at 1, 2 and 3
/// standard deviations in green, olive and teal. Only the first band is on (**inferred**).
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct VwapOptions {
    pub source: PriceSource,
    pub bands: [VwapBand; 3],
    /// Fill between each enabled band and its mirror.
    pub fill: bool,
}

impl Default for VwapOptions {
    fn default() -> Self {
        Self {
            source: PriceSource::Hlc3,
            bands: [
                VwapBand {
                    enabled: true,
                    multiplier: 1.0,
                    color: [76, 175, 80, 255],
                },
                VwapBand {
                    enabled: false,
                    multiplier: 2.0,
                    color: [128, 128, 0, 255],
                },
                VwapBand {
                    enabled: false,
                    multiplier: 3.0,
                    color: [0, 137, 123, 255],
                },
            ],
            fill: true,
        }
    }
}

impl VwapOptions {
    pub fn sanitized(mut self) -> Self {
        let default = Self::default();
        for (band, fallback) in self.bands.iter_mut().zip(default.bands) {
            band.multiplier = within(
                band.multiplier,
                (0.0, MAX_BAND_MULTIPLIER),
                fallback.multiplier,
            );
        }
        self
    }
}

// MARK: Volume profile

/// How a profile's rows are sized.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RowsLayout {
    /// A number of rows over the range.
    #[default]
    Rows,
    /// Rows of a number of ticks each, as many as it takes.
    Ticks,
}

impl RowsLayout {
    pub const ALL: [Self; 2] = [Self::Rows, Self::Ticks];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Rows => "Number of rows",
            Self::Ticks => "Ticks per row",
        }
    }
}

/// What a profile's bars show.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileVolume {
    /// Each row's volume in one colour.
    Total,
    /// Each row split into the volume of up bars and of down bars.
    #[default]
    UpDown,
    /// Up volume less down volume, in the colour of the side that has more.
    Delta,
}

impl ProfileVolume {
    pub const ALL: [Self; 3] = [Self::Total, Self::UpDown, Self::Delta];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Total => "Total",
            Self::UpDown => "Up/Down",
            Self::Delta => "Delta",
        }
    }
}

/// The most ticks one row may be.
pub const MAX_ROW_TICKS: u32 = 100_000;

/// A volume profile's rows, value area and look. Defaults: 24 rows, up and down volume, a 70 %
/// value area (TradingView's), the histogram 70 % of the range wide from its left edge
/// (**inferred**), the point of control and the value area's edges drawn.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct ProfileOptions {
    pub layout: RowsLayout,
    pub rows: u32,
    pub ticks: u32,
    pub volume: ProfileVolume,
    /// Percent of the total volume the value area holds.
    pub value_area: f32,
    /// How wide the longest bar is, in percent of the range's width.
    pub width: f32,
    /// Grow the histogram leftwards from the range's right edge instead.
    pub right: bool,
    pub poc: bool,
    pub value_area_lines: bool,
    /// Run the point of control on to the right edge of the plot.
    pub extend_poc: bool,
    pub up: Rgba,
    pub down: Rgba,
    pub poc_color: Rgba,
    pub value_area_color: Rgba,
}

impl Default for ProfileOptions {
    fn default() -> Self {
        Self {
            layout: RowsLayout::Rows,
            rows: 24,
            ticks: 1,
            volume: ProfileVolume::UpDown,
            value_area: 70.0,
            width: 70.0,
            right: false,
            poc: true,
            value_area_lines: true,
            extend_poc: false,
            up: [41, 98, 255, 153],
            down: [251, 192, 45, 153],
            poc_color: [242, 54, 69, 255],
            value_area_color: [41, 98, 255, 255],
        }
    }
}

impl ProfileOptions {
    pub fn sanitized(mut self) -> Self {
        let default = Self::default();
        self.rows = self.rows.clamp(1, super::volume_profile::MAX_ROWS);
        self.ticks = self.ticks.clamp(1, MAX_ROW_TICKS);
        self.value_area = within(self.value_area, (0.0, 100.0), default.value_area);
        self.width = within(self.width, (5.0, 100.0), default.width);
        self
    }
}

// MARK: Measurers

/// What a measurer prints. Every one is on by default, as the library has them.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct MeasureOptions {
    /// The price difference.
    pub price: bool,
    pub percent: bool,
    /// The difference in ticks (TradingView's pips).
    pub ticks: bool,
    pub bars: bool,
    /// The time between the anchors.
    pub time: bool,
    pub volume: bool,
    /// The plate behind the label.
    pub label: Rgba,
}

impl Default for MeasureOptions {
    fn default() -> Self {
        Self {
            price: true,
            percent: true,
            ticks: true,
            bars: true,
            time: true,
            volume: true,
            label: [88, 88, 88, 255],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_the_charting_librarys() {
        let position = PositionOptions::default();
        assert_eq!(
            (position.account, position.lot, position.risk),
            (1_000.0, 1.0, 25.0)
        );
        assert!(position.risk_percent && !position.compact && !position.always_stats);
        assert_eq!(position.profit, [8, 153, 129, 51]);
        assert_eq!(position.success, [76, 175, 80, 255]);
        let ghost = GhostOptions::default();
        assert_eq!(ghost.up, [172, 229, 220, 128]);
        assert_eq!(ghost.up_border, [8, 153, 129, 255]);
        let vwap = VwapOptions::default();
        assert_eq!(vwap.source, PriceSource::Hlc3);
        let multipliers: Vec<f32> = vwap.bands.iter().map(|band| band.multiplier).collect();
        assert_eq!(multipliers, [1.0, 2.0, 3.0]);
        assert_eq!(ProfileOptions::default().rows, 24);
        assert_eq!(ProfileOptions::default().value_area, 70.0);
        let measure = MeasureOptions::default();
        assert!(measure.price && measure.percent && measure.ticks);
        assert!(measure.bars && measure.time && measure.volume);
    }

    #[test]
    fn nothing_sanitizes_into_something_that_is_not_a_number_or_out_of_range() {
        let wild = PositionOptions {
            account: f64::NAN,
            lot: -5.0,
            risk: 1e99,
            leverage: 0.0,
            qty_precision: 200,
            ..PositionOptions::default()
        }
        .sanitized();
        assert_eq!(wild.account, 1_000.0);
        assert!(wild.lot > 0.0);
        assert_eq!(wild.risk, 100.0, "a percentage cannot be over 100");
        assert!(wild.leverage >= 0.01);
        assert_eq!(wild.qty_precision, MAX_QTY_PRECISION);
        let amount = PositionOptions {
            risk_percent: false,
            risk: 5_000.0,
            ..PositionOptions::default()
        }
        .sanitized();
        assert_eq!(amount.risk, 5_000.0, "an amount may be more than 100");
        let ghost = GhostOptions {
            average: f32::INFINITY,
            variance: 900.0,
            ..GhostOptions::default()
        }
        .sanitized();
        assert_eq!(ghost.average, 0.5);
        assert_eq!(ghost.variance, 100.0);
        let mut vwap = VwapOptions::default();
        vwap.bands[1].multiplier = f32::NAN;
        vwap.bands[2].multiplier = 1e9;
        let vwap = vwap.sanitized();
        assert_eq!(vwap.bands[1].multiplier, 2.0);
        assert_eq!(vwap.bands[2].multiplier, MAX_BAND_MULTIPLIER);
        let profile = ProfileOptions {
            rows: 0,
            ticks: 0,
            value_area: -3.0,
            width: 0.0,
            ..ProfileOptions::default()
        }
        .sanitized();
        assert_eq!((profile.rows, profile.ticks), (1, 1));
        assert_eq!(profile.value_area, 0.0);
        assert_eq!(profile.width, 5.0);
        assert_eq!(
            ProfileOptions {
                rows: 1_000_000,
                ..ProfileOptions::default()
            }
            .sanitized()
            .rows,
            super::super::volume_profile::MAX_ROWS
        );
    }

    #[test]
    fn a_default_is_unchanged_by_sanitizing_and_round_trips() {
        assert_eq!(
            PositionOptions::default().sanitized(),
            PositionOptions::default()
        );
        assert_eq!(GhostOptions::default().sanitized(), GhostOptions::default());
        assert_eq!(VwapOptions::default().sanitized(), VwapOptions::default());
        assert_eq!(
            ProfileOptions::default().sanitized(),
            ProfileOptions::default()
        );
        let json = serde_json::to_string(&ProfileOptions::default()).unwrap();
        assert_eq!(
            serde_json::from_str::<ProfileOptions>(&json).unwrap(),
            ProfileOptions::default()
        );
        // A row that names only some fields reads the rest as the default.
        let partial: PositionOptions = serde_json::from_str(r#"{"account": 5000.0}"#).unwrap();
        assert_eq!(partial.account, 5_000.0);
        assert_eq!(partial.risk, 25.0);
    }
}
