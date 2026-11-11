//! How a chart draws its price data, and the per-type settings that go with it.
//!
//! The chart type is a presentation choice kept apart from the market data: the provider's candles
//! stay canonical, and the types that construct their own bars (Heikin Ashi, Renko, Kagi, Line
//! Break, Point & Figure, Range) derive them in [`crate::model::chart::synthetic`]. [`ChartStyle`] rides inside
//! [`ChartAppearance`](super::ChartAppearance), so it is saved with the chart, the saved layouts
//! and the layout fingerprint without a second persistence path. Every type keeps its own
//! settings: switching away and back finds them as they were left.
//!
//! Pure: no egui types. Colours are `[r, g, b, a]` bytes; `None` follows the chart's up/down
//! palette (and, for borders and wicks, the body colour).

use crate::model::{DataSource, Kline};
use serde::{Deserialize, Serialize};

/// A straight (unpremultiplied) RGBA colour.
pub type Rgba = [u8; 4];

/// What a chart draws its bars as.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChartType {
    #[default]
    Candles,
    HollowCandles,
    Bars,
    HighLowBars,
    VolumeCandles,
    HeikinAshi,
    Renko,
    Kagi,
    LineBreak,
    PointAndFigure,
    Range,
    Line,
    LineWithMarkers,
    StepLine,
    Area,
    HlcArea,
    Baseline,
    Columns,
}

/// Where a chart type sits in the selector.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChartGroup {
    Standard,
    LinesAndAreas,
    PriceBased,
}

impl ChartGroup {
    pub const ALL: [Self; 3] = [Self::Standard, Self::LinesAndAreas, Self::PriceBased];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Standard => "Standard",
            Self::LinesAndAreas => "Lines & Areas",
            Self::PriceBased => "Price-Based",
        }
    }
}

/// Which series a chart type reads, and so what a script on it sees.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DataClass {
    /// Draws the provider's own open/high/low/close.
    StandardOhlc,
    /// Draws one value of the provider's own bars (or their high/low/close) as a line or area.
    StandardLine,
    /// Draws bars constructed from the provider's: scripts and indicators run over those.
    Synthetic,
}

/// The chart kind Pine reports through `chart.is_*`. Every standard type reports `Standard`:
/// scripts see the real OHLC whichever way it is drawn.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChartKind {
    #[default]
    Standard,
    HeikinAshi,
    Renko,
    Kagi,
    LineBreak,
    PointAndFigure,
    Range,
}

impl ChartKind {
    pub const fn is_standard(self) -> bool {
        matches!(self, Self::Standard)
    }
}

impl ChartType {
    pub const ALL: [Self; 18] = [
        Self::Candles,
        Self::HollowCandles,
        Self::Bars,
        Self::HighLowBars,
        Self::VolumeCandles,
        Self::Line,
        Self::LineWithMarkers,
        Self::StepLine,
        Self::Area,
        Self::HlcArea,
        Self::Baseline,
        Self::Columns,
        Self::HeikinAshi,
        Self::Renko,
        Self::LineBreak,
        Self::Kagi,
        Self::PointAndFigure,
        Self::Range,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Candles => "Candles",
            Self::HollowCandles => "Hollow Candles",
            Self::Bars => "Bars",
            Self::HighLowBars => "High-Low Bars",
            Self::VolumeCandles => "Volume Candles",
            Self::HeikinAshi => "Heikin Ashi",
            Self::Renko => "Renko",
            Self::Kagi => "Kagi",
            Self::LineBreak => "Line Break",
            Self::PointAndFigure => "Point & Figure",
            Self::Range => "Range",
            Self::Line => "Line",
            Self::LineWithMarkers => "Line with Markers",
            Self::StepLine => "Step Line",
            Self::Area => "Area",
            Self::HlcArea => "HLC Area",
            Self::Baseline => "Baseline",
            Self::Columns => "Columns",
        }
    }

    /// The name this type goes by in saved rows and over MCP.
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Candles => "candles",
            Self::HollowCandles => "hollow_candles",
            Self::Bars => "bars",
            Self::HighLowBars => "high_low_bars",
            Self::VolumeCandles => "volume_candles",
            Self::HeikinAshi => "heikin_ashi",
            Self::Renko => "renko",
            Self::Kagi => "kagi",
            Self::LineBreak => "line_break",
            Self::PointAndFigure => "point_and_figure",
            Self::Range => "range",
            Self::Line => "line",
            Self::LineWithMarkers => "line_with_markers",
            Self::StepLine => "step_line",
            Self::Area => "area",
            Self::HlcArea => "hlc_area",
            Self::Baseline => "baseline",
            Self::Columns => "columns",
        }
    }

    pub const fn group(self) -> ChartGroup {
        match self {
            Self::Candles
            | Self::HollowCandles
            | Self::Bars
            | Self::HighLowBars
            | Self::VolumeCandles => ChartGroup::Standard,
            Self::Line
            | Self::LineWithMarkers
            | Self::StepLine
            | Self::Area
            | Self::HlcArea
            | Self::Baseline
            | Self::Columns => ChartGroup::LinesAndAreas,
            Self::HeikinAshi
            | Self::Renko
            | Self::Kagi
            | Self::LineBreak
            | Self::PointAndFigure
            | Self::Range => ChartGroup::PriceBased,
        }
    }

    pub fn in_group(group: ChartGroup) -> impl Iterator<Item = Self> {
        Self::ALL
            .into_iter()
            .filter(move |kind| kind.group() == group)
    }

    pub const fn data_class(self) -> DataClass {
        match self.group() {
            ChartGroup::Standard => DataClass::StandardOhlc,
            ChartGroup::LinesAndAreas => DataClass::StandardLine,
            ChartGroup::PriceBased => DataClass::Synthetic,
        }
    }

    /// Whether the type builds its own bars, which scripts and indicators then run over.
    pub const fn is_synthetic(self) -> bool {
        matches!(self.data_class(), DataClass::Synthetic)
    }

    /// What Pine's `chart.is_*` reads on this type.
    pub const fn kind(self) -> ChartKind {
        match self {
            Self::HeikinAshi => ChartKind::HeikinAshi,
            Self::Renko => ChartKind::Renko,
            Self::Kagi => ChartKind::Kagi,
            Self::LineBreak => ChartKind::LineBreak,
            Self::PointAndFigure => ChartKind::PointAndFigure,
            Self::Range => ChartKind::Range,
            _ => ChartKind::Standard,
        }
    }

    /// Why `source` cannot show this type, or `None` when it can.
    ///
    /// Sources that draw a line (prediction markets, CoinMarketCap indices) have no open, high,
    /// low or volume to build any of these from, so they keep their own line.
    pub fn unsupported_reason(self, source: DataSource) -> Option<&'static str> {
        if source.is_line_chart() {
            return Some("This market is drawn as a line");
        }
        if self == Self::VolumeCandles && !source.provides_volume() {
            return Some("This source reports no volume");
        }
        None
    }

    pub fn supported_by(self, source: DataSource) -> bool {
        self.unsupported_reason(source).is_none()
    }
}

/// The value of a bar a line, area, baseline or column chart follows.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PriceSource {
    Open,
    High,
    Low,
    #[default]
    Close,
    Hl2,
    Hlc3,
    Ohlc4,
}

impl PriceSource {
    pub const ALL: [Self; 7] = [
        Self::Open,
        Self::High,
        Self::Low,
        Self::Close,
        Self::Hl2,
        Self::Hlc3,
        Self::Ohlc4,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Open => "Open",
            Self::High => "High",
            Self::Low => "Low",
            Self::Close => "Close",
            Self::Hl2 => "HL2",
            Self::Hlc3 => "HLC3",
            Self::Ohlc4 => "OHLC4",
        }
    }

    pub fn value(self, bar: &Kline) -> f64 {
        match self {
            Self::Open => bar.open,
            Self::High => bar.high,
            Self::Low => bar.low,
            Self::Close => bar.close,
            Self::Hl2 => (bar.high + bar.low) / 2.0,
            Self::Hlc3 => (bar.high + bar.low + bar.close) / 3.0,
            Self::Ohlc4 => (bar.open + bar.high + bar.low + bar.close) / 4.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LineDash {
    #[default]
    Solid,
    Dashed,
    Dotted,
}

impl LineDash {
    pub const ALL: [Self; 3] = [Self::Solid, Self::Dashed, Self::Dotted];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Solid => "Solid",
            Self::Dashed => "Dashed",
            Self::Dotted => "Dotted",
        }
    }
}

/// How a brick, reversal or range size is chosen.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SizeMethod {
    /// A fixed amount in price units.
    Traditional,
    /// The Average True Range over `atr_length` bars, as of the newest bar.
    #[default]
    Atr,
    /// A percentage of the newest close.
    Percent,
}

impl SizeMethod {
    pub const ALL: [Self; 3] = [Self::Traditional, Self::Atr, Self::Percent];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Traditional => "Traditional",
            Self::Atr => "ATR",
            Self::Percent => "Percent of price",
        }
    }
}

/// A brick/reversal/range size: the method plus the value each method reads.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct SizeRule {
    pub method: SizeMethod,
    /// `Traditional`: the size in price units.
    pub amount: f64,
    /// `Atr`: the averaging length.
    pub atr_length: u32,
    /// `Percent`: the size as a percentage of the newest close.
    pub percent: f64,
}

impl Eq for SizeRule {}

impl Default for SizeRule {
    fn default() -> Self {
        Self {
            method: SizeMethod::Atr,
            amount: 1.0,
            atr_length: 14,
            percent: 1.0,
        }
    }
}

impl SizeRule {
    /// The size in price units over `bars`, or `None` when there is nothing to size from.
    ///
    /// ATR and percentage sizes read the newest bar, so the size (and with it the whole
    /// construction) is a function of the series alone and does not depend on what is visible.
    pub fn resolve(&self, bars: &[Kline]) -> Option<f64> {
        let last = bars.last()?;
        let size = match self.method {
            SizeMethod::Traditional => self.amount,
            SizeMethod::Atr => average_true_range(bars, self.atr_length.max(1) as usize),
            SizeMethod::Percent => last.close.abs() * self.percent / 100.0,
        };
        (size.is_finite() && size > 0.0).then_some(size)
    }

    /// Clamps hand-edited or stale values into the range the builders accept.
    pub fn sanitized(mut self) -> Self {
        self.amount = finite_or(self.amount, 1.0).max(f64::MIN_POSITIVE);
        self.percent = finite_or(self.percent, 1.0).clamp(0.0001, 100.0);
        self.atr_length = self.atr_length.clamp(1, 500);
        self
    }
}

fn finite_or(value: f64, fallback: f64) -> f64 {
    if value.is_finite() { value } else { fallback }
}

/// Wilder's Average True Range over the newest `length` bars' worth of smoothing. With fewer bars
/// than `length`, the mean of the true ranges there are.
pub fn average_true_range(bars: &[Kline], length: usize) -> f64 {
    let mut average = 0.0;
    let mut count = 0usize;
    let mut previous_close: Option<f64> = None;
    for bar in bars {
        let range = bar.high - bar.low;
        let true_range = match previous_close {
            Some(close) => range
                .max((bar.high - close).abs())
                .max((bar.low - close).abs()),
            None => range,
        };
        previous_close = Some(bar.close);
        count += 1;
        if count <= length {
            average += (true_range - average) / count as f64;
        } else {
            average = (average * (length as f64 - 1.0) + true_range) / length as f64;
        }
    }
    average
}

/// Colours and visibility for candle bodies, borders and wicks.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct CandleSettings {
    pub body: bool,
    pub border: bool,
    pub wick: bool,
    pub up_body: Option<Rgba>,
    pub down_body: Option<Rgba>,
    pub up_border: Option<Rgba>,
    pub down_border: Option<Rgba>,
    pub up_wick: Option<Rgba>,
    pub down_wick: Option<Rgba>,
    /// Colour by the close against the previous close instead of against the open.
    pub color_by_previous_close: bool,
}

impl Default for CandleSettings {
    fn default() -> Self {
        Self {
            body: true,
            border: true,
            wick: true,
            up_body: None,
            down_body: None,
            up_border: None,
            down_border: None,
            up_wick: None,
            down_wick: None,
            color_by_previous_close: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct BarSettings {
    /// Draw the stem one pixel wide whatever the zoom.
    pub thin: bool,
    /// The open tick on the left and close tick on the right.
    pub ticks: bool,
    pub up: Option<Rgba>,
    pub down: Option<Rgba>,
    pub color_by_previous_close: bool,
    /// High-Low bars only: label the highest high and lowest low on screen.
    pub labels: bool,
}

impl Default for BarSettings {
    fn default() -> Self {
        Self {
            thin: false,
            ticks: true,
            up: None,
            down: None,
            color_by_previous_close: false,
            labels: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct LineSettings {
    pub source: PriceSource,
    pub color: Option<Rgba>,
    /// In points, 1…=4.
    pub width: u8,
    pub dash: LineDash,
    /// Radius of the markers of the marker chart, in points.
    pub marker_size: u8,
}

impl Default for LineSettings {
    fn default() -> Self {
        Self {
            source: PriceSource::Close,
            color: None,
            width: 2,
            dash: LineDash::Solid,
            marker_size: 3,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct AreaSettings {
    pub source: PriceSource,
    pub line: Option<Rgba>,
    pub width: u8,
    /// The fill under the line: `top` at the line fading to `bottom` at the foot of the pane.
    pub top: Option<Rgba>,
    pub bottom: Option<Rgba>,
}

impl Default for AreaSettings {
    fn default() -> Self {
        Self {
            source: PriceSource::Close,
            line: None,
            width: 2,
            top: None,
            bottom: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct HlcAreaSettings {
    pub high_line: Option<Rgba>,
    pub low_line: Option<Rgba>,
    pub close_line: Option<Rgba>,
    /// The fill between the high and low lines.
    pub fill: Option<Rgba>,
    pub width: u8,
}

impl Default for HlcAreaSettings {
    fn default() -> Self {
        Self {
            high_line: None,
            low_line: None,
            close_line: None,
            fill: None,
            width: 2,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct BaselineSettings {
    pub source: PriceSource,
    /// Where the baseline sits, as a percentage of the plot's height from its bottom.
    pub level_percent: f64,
    pub top_line: Option<Rgba>,
    pub bottom_line: Option<Rgba>,
    pub top_fill: Option<Rgba>,
    pub bottom_fill: Option<Rgba>,
    pub width: u8,
}

impl Eq for BaselineSettings {}

impl Default for BaselineSettings {
    fn default() -> Self {
        Self {
            source: PriceSource::Close,
            level_percent: 50.0,
            top_line: None,
            bottom_line: None,
            top_fill: None,
            bottom_fill: None,
            width: 2,
        }
    }
}

impl BaselineSettings {
    pub fn level_fraction(&self) -> f64 {
        finite_or(self.level_percent, 50.0).clamp(0.0, 100.0) / 100.0
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct ColumnSettings {
    pub source: PriceSource,
    pub up: Option<Rgba>,
    pub down: Option<Rgba>,
    pub color_by_previous_close: bool,
}

impl Default for ColumnSettings {
    fn default() -> Self {
        Self {
            source: PriceSource::Close,
            up: None,
            down: None,
            color_by_previous_close: true,
        }
    }
}

/// Which price of each source bar a price-based construction follows.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstructionSource {
    /// The close of each bar.
    #[default]
    Close,
    /// Each bar's open, then its extremes, then its close.
    HighLow,
}

impl ConstructionSource {
    pub const ALL: [Self; 2] = [Self::Close, Self::HighLow];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Close => "Close",
            Self::HighLow => "High/Low",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct RenkoSettings {
    pub size: SizeRule,
    pub source: ConstructionSource,
    /// Draw the source bars' highs and lows as wicks on the bricks.
    pub wicks: bool,
    pub candle: CandleSettings,
}

impl Default for RenkoSettings {
    fn default() -> Self {
        Self {
            size: SizeRule::default(),
            source: ConstructionSource::Close,
            wicks: false,
            candle: CandleSettings::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct KagiSettings {
    /// The reversal amount.
    pub size: SizeRule,
    pub source: ConstructionSource,
    pub up: Option<Rgba>,
    pub down: Option<Rgba>,
    pub width: u8,
}

impl Default for KagiSettings {
    fn default() -> Self {
        Self {
            size: SizeRule::default(),
            source: ConstructionSource::Close,
            up: None,
            down: None,
            width: 3,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct LineBreakSettings {
    /// How many previous lines a close has to break (three by default).
    pub lines: u8,
    pub candle: CandleSettings,
}

impl Default for LineBreakSettings {
    fn default() -> Self {
        Self {
            lines: 3,
            candle: CandleSettings::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct PointFigureSettings {
    pub size: SizeRule,
    /// Boxes a move against the column has to cover to start the next one (three by default).
    pub reversal: u8,
    pub source: ConstructionSource,
    pub up: Option<Rgba>,
    pub down: Option<Rgba>,
    pub outlines: bool,
}

impl Default for PointFigureSettings {
    fn default() -> Self {
        Self {
            size: SizeRule::default(),
            reversal: 3,
            source: ConstructionSource::Close,
            up: None,
            down: None,
            outlines: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RangeLook {
    #[default]
    Candles,
    Bars,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct RangeSettings {
    /// The high-to-low span a bar covers before the next one opens.
    pub size: SizeRule,
    pub look: RangeLook,
    pub candle: CandleSettings,
    pub bars: BarSettings,
}

impl Default for RangeSettings {
    fn default() -> Self {
        Self {
            size: SizeRule {
                method: SizeMethod::Percent,
                percent: 0.5,
                ..SizeRule::default()
            },
            look: RangeLook::Candles,
            candle: CandleSettings::default(),
            bars: BarSettings::default(),
        }
    }
}

/// The chart type and every type's own settings. Old rows without it decode as plain candles.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct ChartStyle {
    pub chart_type: ChartType,
    pub candles: CandleSettings,
    pub hollow: CandleSettings,
    pub bars: BarSettings,
    pub high_low: BarSettings,
    pub volume_candles: CandleSettings,
    pub heikin_ashi: CandleSettings,
    pub line: LineSettings,
    pub line_markers: LineSettings,
    pub step_line: LineSettings,
    pub area: AreaSettings,
    pub hlc_area: HlcAreaSettings,
    pub baseline: BaselineSettings,
    pub columns: ColumnSettings,
    pub renko: RenkoSettings,
    pub kagi: KagiSettings,
    pub line_break: LineBreakSettings,
    pub point_figure: PointFigureSettings,
    pub range: RangeSettings,
}

impl ChartStyle {
    /// The candle settings of the selected type, for the types that draw candle bodies.
    pub fn candle_settings(&self) -> Option<&CandleSettings> {
        Some(match self.chart_type {
            ChartType::Candles => &self.candles,
            ChartType::HollowCandles => &self.hollow,
            ChartType::VolumeCandles => &self.volume_candles,
            ChartType::HeikinAshi => &self.heikin_ashi,
            ChartType::Renko => &self.renko.candle,
            ChartType::LineBreak => &self.line_break.candle,
            ChartType::Range if self.range.look == RangeLook::Candles => &self.range.candle,
            _ => return None,
        })
    }

    pub fn candle_settings_mut(&mut self) -> Option<&mut CandleSettings> {
        Some(match self.chart_type {
            ChartType::Candles => &mut self.candles,
            ChartType::HollowCandles => &mut self.hollow,
            ChartType::VolumeCandles => &mut self.volume_candles,
            ChartType::HeikinAshi => &mut self.heikin_ashi,
            ChartType::Renko => &mut self.renko.candle,
            ChartType::LineBreak => &mut self.line_break.candle,
            ChartType::Range if self.range.look == RangeLook::Candles => &mut self.range.candle,
            _ => return None,
        })
    }

    /// The bar settings of the selected type, for the types that draw OHLC bars.
    pub fn bar_settings(&self) -> Option<&BarSettings> {
        Some(match self.chart_type {
            ChartType::Bars => &self.bars,
            ChartType::HighLowBars => &self.high_low,
            ChartType::Range if self.range.look == RangeLook::Bars => &self.range.bars,
            _ => return None,
        })
    }

    pub fn bar_settings_mut(&mut self) -> Option<&mut BarSettings> {
        Some(match self.chart_type {
            ChartType::Bars => &mut self.bars,
            ChartType::HighLowBars => &mut self.high_low,
            ChartType::Range if self.range.look == RangeLook::Bars => &mut self.range.bars,
            _ => return None,
        })
    }

    /// The line settings of the line-style types.
    pub fn line_settings(&self) -> Option<&LineSettings> {
        Some(match self.chart_type {
            ChartType::Line => &self.line,
            ChartType::LineWithMarkers => &self.line_markers,
            ChartType::StepLine => &self.step_line,
            _ => return None,
        })
    }

    pub fn line_settings_mut(&mut self) -> Option<&mut LineSettings> {
        Some(match self.chart_type {
            ChartType::Line => &mut self.line,
            ChartType::LineWithMarkers => &mut self.line_markers,
            ChartType::StepLine => &mut self.step_line,
            _ => return None,
        })
    }

    /// The value of a bar the selected line-style, area, baseline or column type follows.
    pub fn value_source(&self) -> PriceSource {
        match self.chart_type {
            ChartType::Line | ChartType::LineWithMarkers | ChartType::StepLine => self
                .line_settings()
                .map_or(PriceSource::Close, |settings| settings.source),
            ChartType::Area => self.area.source,
            ChartType::Baseline => self.baseline.source,
            ChartType::Columns => self.columns.source,
            _ => PriceSource::Close,
        }
    }

    /// The up and down colours of the selected type, for what the chart draws beside its bars
    /// (volume bars, the last-price tag, trend-flip markers): the type's own colours where it has
    /// them, else `base` (the chart's saved bullish/bearish palette) for whichever side is unset.
    pub fn up_down(&self, base: (Rgba, Rgba)) -> (Rgba, Rgba) {
        let (up, down) = match self.chart_type {
            ChartType::Candles
            | ChartType::HollowCandles
            | ChartType::VolumeCandles
            | ChartType::HeikinAshi
            | ChartType::Renko
            | ChartType::LineBreak => self.candle_settings().map_or((None, None), |settings| {
                (settings.up_body, settings.down_body)
            }),
            ChartType::Range => match self.range.look {
                RangeLook::Candles => (self.range.candle.up_body, self.range.candle.down_body),
                RangeLook::Bars => (self.range.bars.up, self.range.bars.down),
            },
            ChartType::Bars => (self.bars.up, self.bars.down),
            ChartType::HighLowBars => (self.high_low.up, self.high_low.down),
            ChartType::Columns => (self.columns.up, self.columns.down),
            ChartType::Kagi => (self.kagi.up, self.kagi.down),
            ChartType::PointAndFigure => (self.point_figure.up, self.point_figure.down),
            ChartType::Baseline => (self.baseline.top_line, self.baseline.bottom_line),
            ChartType::HlcArea => (self.hlc_area.high_line, self.hlc_area.low_line),
            ChartType::Line
            | ChartType::LineWithMarkers
            | ChartType::StepLine
            | ChartType::Area => (None, None),
        };
        (up.unwrap_or(base.0), down.unwrap_or(base.1))
    }

    /// The type the chart can draw for `source`: the selected one, else plain candles.
    pub fn effective_type(&self, source: DataSource) -> ChartType {
        if self.chart_type.supported_by(source) {
            self.chart_type
        } else {
            ChartType::Candles
        }
    }

    /// Puts the selected type's own settings back to their defaults; every other type keeps its.
    pub fn reset_selected(&mut self) {
        let fresh = Self::default();
        match self.chart_type {
            ChartType::Candles => self.candles = fresh.candles,
            ChartType::HollowCandles => self.hollow = fresh.hollow,
            ChartType::Bars => self.bars = fresh.bars,
            ChartType::HighLowBars => self.high_low = fresh.high_low,
            ChartType::VolumeCandles => self.volume_candles = fresh.volume_candles,
            ChartType::HeikinAshi => self.heikin_ashi = fresh.heikin_ashi,
            ChartType::Renko => self.renko = fresh.renko,
            ChartType::Kagi => self.kagi = fresh.kagi,
            ChartType::LineBreak => self.line_break = fresh.line_break,
            ChartType::PointAndFigure => self.point_figure = fresh.point_figure,
            ChartType::Range => self.range = fresh.range,
            ChartType::Line => self.line = fresh.line,
            ChartType::LineWithMarkers => self.line_markers = fresh.line_markers,
            ChartType::StepLine => self.step_line = fresh.step_line,
            ChartType::Area => self.area = fresh.area,
            ChartType::HlcArea => self.hlc_area = fresh.hlc_area,
            ChartType::Baseline => self.baseline = fresh.baseline,
            ChartType::Columns => self.columns = fresh.columns,
        }
    }

    /// Heals values a hand-edited or older row could leave the builders unable to use.
    pub fn sanitized(mut self) -> Self {
        self.renko.size = self.renko.size.sanitized();
        self.kagi.size = self.kagi.size.sanitized();
        self.point_figure.size = self.point_figure.size.sanitized();
        self.range.size = self.range.size.sanitized();
        self.line_break.lines = self.line_break.lines.clamp(1, 20);
        self.point_figure.reversal = self.point_figure.reversal.clamp(1, 20);
        self.baseline.level_percent =
            finite_or(self.baseline.level_percent, 50.0).clamp(0.0, 100.0);
        for line in [&mut self.line, &mut self.line_markers, &mut self.step_line] {
            line.width = line.width.clamp(1, 4);
            line.marker_size = line.marker_size.clamp(1, 8);
        }
        self.area.width = self.area.width.clamp(1, 4);
        self.hlc_area.width = self.hlc_area.width.clamp(1, 4);
        self.baseline.width = self.baseline.width.clamp(1, 4);
        self.kagi.width = self.kagi.width.clamp(1, 4);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(open: f64, high: f64, low: f64, close: f64) -> Kline {
        Kline {
            open_time: 0,
            close_time: 1,
            open,
            high,
            low,
            close,
            volume: 1.0,
        }
    }

    #[test]
    fn every_type_has_a_group_and_the_selector_lists_each_once() {
        let mut seen = std::collections::HashSet::new();
        for group in ChartGroup::ALL {
            for kind in ChartType::in_group(group) {
                assert!(seen.insert(kind), "{kind:?} listed twice");
            }
        }
        assert_eq!(seen.len(), ChartType::ALL.len());
    }

    #[test]
    fn slugs_are_the_serialized_names() {
        for kind in ChartType::ALL {
            assert_eq!(
                serde_json::to_string(&kind).unwrap(),
                format!("\"{}\"", kind.slug())
            );
        }
    }

    #[test]
    fn only_the_price_based_types_are_synthetic_and_report_a_chart_kind() {
        for kind in ChartType::ALL {
            assert_eq!(
                kind.is_synthetic(),
                kind.group() == ChartGroup::PriceBased,
                "{kind:?}"
            );
            assert_eq!(kind.kind().is_standard(), !kind.is_synthetic(), "{kind:?}");
        }
        assert_eq!(ChartType::Line.kind(), ChartKind::Standard);
        assert_eq!(ChartType::Renko.kind(), ChartKind::Renko);
    }

    #[test]
    fn line_markets_and_volumeless_sources_refuse_what_they_cannot_draw() {
        assert!(!ChartType::Renko.supported_by(DataSource::Polymarket));
        assert!(!ChartType::Candles.supported_by(DataSource::CoinMarketCap));
        assert!(!ChartType::VolumeCandles.supported_by(DataSource::CoinGecko));
        assert!(ChartType::VolumeCandles.supported_by(DataSource::Binance));
        assert!(ChartType::HeikinAshi.supported_by(DataSource::CoinGecko));
        let style = ChartStyle {
            chart_type: ChartType::Kagi,
            ..ChartStyle::default()
        };
        assert_eq!(style.effective_type(DataSource::Kalshi), ChartType::Candles);
        assert_eq!(style.effective_type(DataSource::Binance), ChartType::Kagi);
    }

    #[test]
    fn a_row_without_a_style_decodes_as_default_candles() {
        let style: ChartStyle = serde_json::from_str("{}").unwrap();
        assert_eq!(style, ChartStyle::default());
        assert_eq!(style.chart_type, ChartType::Candles);
        let partial: ChartStyle =
            serde_json::from_str(r#"{"chart_type":"renko","renko":{"size":{"amount":10.0}}}"#)
                .unwrap();
        assert_eq!(partial.chart_type, ChartType::Renko);
        assert_eq!(partial.renko.size.amount, 10.0);
        assert_eq!(partial.renko.size.atr_length, 14);
    }

    #[test]
    fn each_type_keeps_its_settings_across_a_switch() {
        let mut style = ChartStyle {
            chart_type: ChartType::Renko,
            ..ChartStyle::default()
        };
        style.renko.size.method = SizeMethod::Traditional;
        style.renko.size.amount = 10.0;
        style.chart_type = ChartType::Candles;
        style.candles.up_body = Some([1, 2, 3, 255]);
        style.chart_type = ChartType::Renko;
        assert_eq!(style.renko.size.amount, 10.0);
        assert_eq!(style.renko.size.method, SizeMethod::Traditional);
        let encoded = serde_json::to_string(&style).unwrap();
        let decoded: ChartStyle = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, style);
    }

    #[test]
    fn resetting_a_type_leaves_the_others_alone() {
        let mut style = ChartStyle::default();
        style.candles.wick = false;
        style.renko.wicks = true;
        style.chart_type = ChartType::Renko;
        style.reset_selected();
        assert!(!style.renko.wicks);
        assert!(!style.candles.wick);
        for kind in ChartType::ALL {
            let mut each = ChartStyle {
                chart_type: kind,
                ..ChartStyle::default()
            };
            each.reset_selected();
            assert_eq!(
                each,
                ChartStyle {
                    chart_type: kind,
                    ..ChartStyle::default()
                }
            );
        }
    }

    #[test]
    fn shared_colours_follow_the_selected_types_own_up_and_down() {
        let base = ([0, 200, 0, 255], [200, 0, 0, 255]);
        let mut style = ChartStyle::default();
        // Nothing set: the saved palette.
        for kind in ChartType::ALL {
            style.chart_type = kind;
            assert_eq!(style.up_down(base), base, "{kind:?}");
        }
        style.candles.up_body = Some([1, 1, 1, 255]);
        style.renko.candle.down_body = Some([2, 2, 2, 255]);
        style.bars.up = Some([3, 3, 3, 255]);
        style.kagi.down = Some([4, 4, 4, 255]);
        style.baseline.top_line = Some([5, 5, 5, 255]);
        style.range.look = RangeLook::Bars;
        style.range.bars.down = Some([6, 6, 6, 255]);
        let pick = |style: &mut ChartStyle, kind| {
            style.chart_type = kind;
            style.up_down(base)
        };
        // One side set: the other keeps the palette.
        assert_eq!(
            pick(&mut style, ChartType::Candles),
            ([1, 1, 1, 255], base.1)
        );
        assert_eq!(pick(&mut style, ChartType::Renko), (base.0, [2, 2, 2, 255]));
        assert_eq!(pick(&mut style, ChartType::Bars), ([3, 3, 3, 255], base.1));
        assert_eq!(pick(&mut style, ChartType::Kagi), (base.0, [4, 4, 4, 255]));
        assert_eq!(
            pick(&mut style, ChartType::Baseline),
            ([5, 5, 5, 255], base.1)
        );
        assert_eq!(pick(&mut style, ChartType::Range), (base.0, [6, 6, 6, 255]));
        // Another type's colours do not leak: Heikin Ashi has none of its own.
        assert_eq!(pick(&mut style, ChartType::HeikinAshi), base);
        // Lines have no up/down.
        assert_eq!(pick(&mut style, ChartType::Line), base);
    }

    #[test]
    fn price_sources_average_the_bar() {
        let bar = bar(10.0, 16.0, 8.0, 12.0);
        assert_eq!(PriceSource::Close.value(&bar), 12.0);
        assert_eq!(PriceSource::Hl2.value(&bar), 12.0);
        assert_eq!(PriceSource::Hlc3.value(&bar), 12.0);
        assert_eq!(PriceSource::Ohlc4.value(&bar), 11.5);
    }

    #[test]
    fn atr_is_wilders_average_and_the_mean_while_it_warms_up() {
        let bars = vec![bar(10.0, 12.0, 8.0, 10.0), bar(10.0, 13.0, 9.0, 11.0)];
        // True ranges 4 and 4.
        assert!((average_true_range(&bars, 14) - 4.0).abs() < 1e-12);
        let gap = vec![bar(10.0, 10.0, 10.0, 10.0), bar(20.0, 20.0, 20.0, 20.0)];
        // The second bar's true range is the 10 gap.
        assert!((average_true_range(&gap, 2) - 5.0).abs() < 1e-12);
        assert_eq!(average_true_range(&[], 14), 0.0);
    }

    #[test]
    fn size_rules_resolve_per_method_and_reject_nonsense() {
        let bars = vec![bar(100.0, 110.0, 90.0, 100.0)];
        let rule = |method, amount, percent| SizeRule {
            method,
            amount,
            atr_length: 14,
            percent,
        };
        assert_eq!(
            rule(SizeMethod::Traditional, 7.0, 1.0).resolve(&bars),
            Some(7.0)
        );
        assert_eq!(
            rule(SizeMethod::Percent, 1.0, 2.0).resolve(&bars),
            Some(2.0)
        );
        assert_eq!(rule(SizeMethod::Atr, 1.0, 1.0).resolve(&bars), Some(20.0));
        assert_eq!(rule(SizeMethod::Traditional, 0.0, 1.0).resolve(&bars), None);
        assert_eq!(
            rule(SizeMethod::Traditional, f64::NAN, 1.0).resolve(&bars),
            None
        );
        assert_eq!(SizeRule::default().resolve(&[]), None);
    }

    #[test]
    fn sanitizing_pulls_stale_values_into_range() {
        let mut style = ChartStyle::default();
        style.renko.size.amount = f64::NAN;
        style.line.width = 99;
        style.baseline.level_percent = 400.0;
        style.line_break.lines = 0;
        let healed = style.sanitized();
        assert_eq!(healed.renko.size.amount, 1.0);
        assert_eq!(healed.line.width, 4);
        assert_eq!(healed.baseline.level_percent, 100.0);
        assert_eq!(healed.line_break.lines, 1);
    }
}
