//! How a chart draws its price data, and the per-type settings that go with it.
//!
//! The chart type is a presentation choice kept apart from the market data: the provider's candles
//! stay canonical, and the types that construct their own bars (Heikin Ashi, Renko, Kagi, Line
//! Break, Point & Figure, Range) derive them in [`super::synthetic`]. [`ChartStyle`] rides inside
//! [`ChartAppearance`](super::ChartAppearance), so it is saved with the chart, the saved layouts
//! and the layout fingerprint without a second persistence path. Every type keeps its own
//! settings: switching away and back finds them as they were left.
//!
//! Pure: no egui types. Colours are `[r, g, b, a]` bytes; `None` follows the chart's up/down
//! palette (and, for borders and wicks, the body colour).

use super::{DataSource, Kline};
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
    VolumeFootprint,
    TimePriceOpportunity,
    SessionVolumeProfile,
}

/// Where a chart type sits in the selector.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChartGroup {
    Standard,
    LinesAndAreas,
    PriceBased,
    /// Types that draw what a candle cannot say: who traded at which price, how often a price was
    /// visited. They are built from finer data than the chart's own bars (see `market_profile`).
    OrderFlow,
}

impl ChartGroup {
    pub const ALL: [Self; 4] = [
        Self::Standard,
        Self::LinesAndAreas,
        Self::PriceBased,
        Self::OrderFlow,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Standard => "Standard",
            Self::LinesAndAreas => "Lines & Areas",
            Self::PriceBased => "Price-Based",
            Self::OrderFlow => "Order flow & profiles",
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
    pub const ALL: [Self; 21] = [
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
        Self::VolumeFootprint,
        Self::TimePriceOpportunity,
        Self::SessionVolumeProfile,
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
            Self::VolumeFootprint => "Volume footprint",
            Self::TimePriceOpportunity => "Time price opportunity",
            Self::SessionVolumeProfile => "Session volume profile",
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
            Self::VolumeFootprint => "volume_footprint",
            Self::TimePriceOpportunity => "time_price_opportunity",
            Self::SessionVolumeProfile => "session_volume_profile",
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
            Self::VolumeFootprint | Self::TimePriceOpportunity | Self::SessionVolumeProfile => {
                ChartGroup::OrderFlow
            }
        }
    }

    pub fn in_group(group: ChartGroup) -> impl Iterator<Item = Self> {
        Self::ALL
            .into_iter()
            .filter(move |kind| kind.group() == group)
    }

    pub const fn data_class(self) -> DataClass {
        match self.group() {
            // The order-flow types draw over the provider's own candles, which scripts and
            // indicators keep reading.
            ChartGroup::Standard | ChartGroup::OrderFlow => DataClass::StandardOhlc,
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
        if matches!(
            self,
            Self::VolumeFootprint | Self::TimePriceOpportunity | Self::SessionVolumeProfile
        ) {
            return match source {
                DataSource::Binance | DataSource::Coinbase | DataSource::DexScreener => None,
                DataSource::Alpaca => Some("No intraday candles for stocks yet"),
                _ => Some("This source has no volume or intraday candles"),
            };
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

/// How a footprint's rows are sized.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RowMode {
    /// A fifth of the average true range (TradingView's automatic row size).
    #[default]
    Auto,
    /// A fixed number of ticks per row.
    Manual,
}

/// Whether a footprint's cells are all one width or as wide as their volume.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FootprintDisplay {
    /// Equal cells.
    #[default]
    Cluster,
    /// Cells as wide as their share of the bar's busiest row.
    Profile,
}

/// What each footprint cell says.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FootprintKind {
    /// Sell volume on the left, buy volume on the right.
    #[default]
    BuyAndSell,
    /// Buy minus sell.
    Delta,
    /// Buy plus sell.
    Total,
    /// Buy and sell, with the busiest level of each row highlighted.
    Ladder,
}

/// What is written under or beside each footprint bar.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FootprintSummary {
    /// A box with the bar's total, buy and sell volume and delta.
    #[default]
    InfoBox,
    Off,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct FootprintSettings {
    /// Build a bar from the exchange's trades (each with its aggressor side) where the market has
    /// them and they cover the bar; off, or where there are none, the finer candles are used.
    pub tick_based: bool,
    pub row_mode: RowMode,
    /// The averaging length of the automatic row size.
    pub atr_length: u32,
    /// Ticks per row when the row size is manual.
    pub ticks_per_row: u32,
    pub display: FootprintDisplay,
    pub kind: FootprintKind,
    /// Colour each cell by its volume against the bar's busiest row, in four steps.
    pub gradient: bool,
    pub buy_color: Option<Rgba>,
    pub sell_color: Option<Rgba>,
    pub show_value_area: bool,
    /// The share of a bar's volume its value area holds, in percent.
    pub value_area_percent: u32,
    pub show_poc: bool,
    pub summary: FootprintSummary,
    /// A level is a buy imbalance when its buy volume is at least this percent of the sell volume
    /// of the level below (and the other way for sell), TradingView's 300.
    pub imbalance_percent: u32,
    pub highlight_imbalances: bool,
    pub buy_imbalance_color: Option<Rgba>,
    pub sell_imbalance_color: Option<Rgba>,
    pub highlight_stacked: bool,
    /// Consecutive imbalanced levels of one side that make a stack.
    pub stacked_levels: u32,
    /// The candle drawn beside the cells.
    pub candle: CandleSettings,
}

impl Default for FootprintSettings {
    fn default() -> Self {
        Self {
            tick_based: true,
            row_mode: RowMode::Auto,
            atr_length: 14,
            ticks_per_row: 10,
            display: FootprintDisplay::Cluster,
            kind: FootprintKind::BuyAndSell,
            gradient: true,
            buy_color: None,
            sell_color: None,
            show_value_area: true,
            value_area_percent: 70,
            show_poc: true,
            summary: FootprintSummary::InfoBox,
            imbalance_percent: 300,
            highlight_imbalances: true,
            buy_imbalance_color: None,
            sell_imbalance_color: None,
            highlight_stacked: true,
            stacked_levels: 3,
            candle: CandleSettings::default(),
        }
    }
}

/// The part of the footprint settings that changes what is built (rows, areas, imbalances), not how
/// it looks: restyling never rebuilds a footprint, and applying a template never changes these.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FootprintShape {
    pub tick_based: bool,
    pub row_mode: RowMode,
    pub atr_length: u32,
    pub ticks_per_row: u32,
    pub value_area_percent: u32,
    pub imbalance_percent: u32,
    pub stacked_levels: u32,
}

impl FootprintSettings {
    pub fn shape(&self) -> FootprintShape {
        FootprintShape {
            tick_based: self.tick_based,
            row_mode: self.row_mode,
            atr_length: self.atr_length,
            ticks_per_row: self.ticks_per_row,
            value_area_percent: self.value_area_percent,
            imbalance_percent: self.imbalance_percent,
            stacked_levels: self.stacked_levels,
        }
    }

    /// Takes the shaping fields of `own`, keeping this one's look.
    pub fn keep_shape_of(&mut self, own: &Self) {
        self.tick_based = own.tick_based;
        self.row_mode = own.row_mode;
        self.atr_length = own.atr_length;
        self.ticks_per_row = own.ticks_per_row;
        self.value_area_percent = own.value_area_percent;
        self.imbalance_percent = own.imbalance_percent;
        self.stacked_levels = own.stacked_levels;
    }

    /// Heals values a hand-edited or older row could leave the builder unable to use.
    fn sanitized(mut self) -> Self {
        self.atr_length = self.atr_length.clamp(1, 500);
        self.ticks_per_row = self.ticks_per_row.clamp(1, 100_000);
        self.value_area_percent = self.value_area_percent.clamp(1, 100);
        self.imbalance_percent = self.imbalance_percent.clamp(100, 10_000);
        self.stacked_levels = self.stacked_levels.clamp(2, 20);
        self
    }
}

/// How long a TPO profile's period is.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PeriodKind {
    #[default]
    Day,
    Week,
    Month,
}

/// Which stretches of time get a profile.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionMode {
    /// One profile per period.
    #[default]
    All,
    /// One profile per custom window (up to five, in UTC).
    Custom,
}

/// Which side of the TPO the volume profile sits on.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VolumePlacement {
    Left,
    #[default]
    Right,
}

/// A level a TPO profile can mark: shown or not, and (for the ones that can) carried right until
/// price reaches it again.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct TpoLevel {
    pub show: bool,
    pub extend: bool,
    pub color: Option<Rgba>,
}

impl TpoLevel {
    const fn on() -> Self {
        Self {
            show: true,
            extend: false,
            color: None,
        }
    }

    const fn off() -> Self {
        Self {
            show: false,
            extend: false,
            color: None,
        }
    }
}

impl Default for TpoLevel {
    fn default() -> Self {
        Self::on()
    }
}

/// The volume profile beside the TPO.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct TpoVolume {
    pub show: bool,
    pub placement: VolumePlacement,
    /// Write each row's volume at the end of its bar.
    pub values: bool,
    pub show_poc: bool,
    pub show_vah: bool,
    pub show_val: bool,
    pub color: Option<Rgba>,
    pub value_area_color: Option<Rgba>,
}

impl Default for TpoVolume {
    fn default() -> Self {
        Self {
            show: true,
            placement: VolumePlacement::Right,
            values: false,
            show_poc: true,
            show_vah: false,
            show_val: false,
            color: None,
            value_area_color: None,
        }
    }
}

/// The block sizes (minutes) a TPO offers.
pub const TPO_BLOCK_MINUTES: [u32; 7] = [5, 10, 15, 30, 60, 120, 240];

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct TpoSettings {
    pub period: PeriodKind,
    pub period_count: u32,
    pub block_minutes: u32,
    pub row_mode: RowMode,
    pub ticks_per_row: u32,
    pub value_area_percent: u32,
    pub sessions: SessionMode,
    pub custom: [Option<super::market_profile::sessions::SessionSpan>; 5],
    /// Blocks the initial balance covers.
    pub initial_balance_blocks: u32,
    /// The four gradient colours: the first two span the blocks A to Z, the last two a to z.
    pub gradient: [Option<Rgba>; 4],
    pub show_blocks: bool,
    pub show_letters: bool,
    /// How much of its colour a row outside the value area keeps, in percent.
    pub opacity_outside_value_area: u32,
    /// Put each letter in the column of its block (leaving gaps) instead of packing a row's letters
    /// together.
    pub expand_blocks: bool,
    pub poc: TpoLevel,
    pub vah: TpoLevel,
    pub val: TpoLevel,
    pub midpoint: TpoLevel,
    pub open: TpoLevel,
    pub close: TpoLevel,
    pub poor_high: TpoLevel,
    pub poor_low: TpoLevel,
    pub single_prints: TpoLevel,
    pub initial_balance: TpoLevel,
    pub volume: TpoVolume,
}

impl Default for TpoSettings {
    fn default() -> Self {
        Self {
            period: PeriodKind::Day,
            period_count: 1,
            block_minutes: 30,
            row_mode: RowMode::Auto,
            ticks_per_row: 10,
            value_area_percent: 70,
            sessions: SessionMode::All,
            custom: [None; 5],
            initial_balance_blocks: 2,
            gradient: [None; 4],
            show_blocks: true,
            show_letters: true,
            opacity_outside_value_area: 55,
            expand_blocks: false,
            poc: TpoLevel::on(),
            vah: TpoLevel::on(),
            val: TpoLevel::on(),
            midpoint: TpoLevel::off(),
            open: TpoLevel::off(),
            close: TpoLevel::off(),
            poor_high: TpoLevel::on(),
            poor_low: TpoLevel::on(),
            single_prints: TpoLevel::on(),
            initial_balance: TpoLevel::on(),
            volume: TpoVolume::default(),
        }
    }
}

/// The part of the TPO settings that changes what is built (periods, blocks, rows, the value-area
/// share): looks never rebuild, a template never changes these, and changing the period or the block
/// size drops a chart's splits and merges (TradingView's rule).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TpoShape {
    pub period: PeriodKind,
    pub period_count: u32,
    pub block_minutes: u32,
    pub row_mode: RowMode,
    pub ticks_per_row: u32,
    pub value_area_percent: u32,
    pub sessions: SessionMode,
    pub custom: [Option<super::market_profile::sessions::SessionSpan>; 5],
    pub initial_balance_blocks: u32,
}

impl TpoShape {
    pub fn period(&self) -> super::market_profile::sessions::Period {
        use super::market_profile::sessions::Period;
        match self.period {
            PeriodKind::Day => Period::Days(self.period_count),
            PeriodKind::Week => Period::Weeks(self.period_count),
            PeriodKind::Month => Period::Months(self.period_count),
        }
    }

    /// The custom windows, when the profiles are cut by them (at least one is valid).
    pub fn custom_sessions(&self) -> Option<Vec<super::market_profile::sessions::SessionSpan>> {
        if self.sessions != SessionMode::Custom {
            return None;
        }
        let spans: Vec<_> = self.custom.iter().flatten().copied().collect();
        (!spans.is_empty()).then_some(spans)
    }

    /// What decides which splits and merges still mean something: the period and the block size.
    pub fn edit_key(
        &self,
    ) -> (
        PeriodKind,
        u32,
        u32,
        SessionMode,
        [Option<super::market_profile::sessions::SessionSpan>; 5],
    ) {
        (
            self.period,
            self.period_count,
            self.block_minutes,
            self.sessions,
            self.custom,
        )
    }
}

impl TpoSettings {
    pub fn shape(&self) -> TpoShape {
        TpoShape {
            period: self.period,
            period_count: self.period_count,
            block_minutes: self.block_minutes,
            row_mode: self.row_mode,
            ticks_per_row: self.ticks_per_row,
            value_area_percent: self.value_area_percent,
            sessions: self.sessions,
            custom: self.custom,
            initial_balance_blocks: self.initial_balance_blocks,
        }
    }

    /// Takes the shaping fields of `own`, keeping this one's look.
    pub fn keep_shape_of(&mut self, own: &Self) {
        self.period = own.period;
        self.period_count = own.period_count;
        self.block_minutes = own.block_minutes;
        self.row_mode = own.row_mode;
        self.ticks_per_row = own.ticks_per_row;
        self.value_area_percent = own.value_area_percent;
        self.sessions = own.sessions;
        self.custom = own.custom;
        self.initial_balance_blocks = own.initial_balance_blocks;
    }

    fn sanitized(mut self) -> Self {
        self.period_count = self.period_count.clamp(1, 60);
        // Only the sizes TradingView offers: anything else becomes the nearest one.
        self.block_minutes = TPO_BLOCK_MINUTES
            .into_iter()
            .min_by_key(|size| size.abs_diff(self.block_minutes))
            .unwrap_or(30);
        self.ticks_per_row = self.ticks_per_row.clamp(1, 100_000);
        self.value_area_percent = self.value_area_percent.clamp(1, 100);
        self.initial_balance_blocks = self.initial_balance_blocks.clamp(1, 20);
        self.opacity_outside_value_area = self.opacity_outside_value_area.min(100);
        self
    }
}

/// What the session volume profile's bars show.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SvpVolume {
    /// Up volume and down volume side by side in each row.
    #[default]
    UpDown,
    Total,
    /// Up minus down, in the colour of whichever is larger.
    Delta,
}

/// How the rows of a session volume profile are sized.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RowsLayout {
    /// About this many rows between the session's low and high (TradingView's default, 24).
    #[default]
    NumberOfRows,
    /// Rows this many price steps tall.
    TicksPerRow,
}

/// A line the session volume profile marks: shown or not, and carried right until a bar crosses it.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct SvpLine {
    pub show: bool,
    pub extend: bool,
    pub color: Option<Rgba>,
}

impl Default for SvpLine {
    fn default() -> Self {
        Self {
            show: true,
            extend: false,
            color: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct SvpSettings {
    pub sessions: SessionMode,
    /// Up to five `HHMM-HHMM[:days]` windows (UTC), one profile each.
    pub custom: [Option<super::market_profile::sessions::SessionSpan>; 5],
    pub volume: SvpVolume,
    /// The share of a session's volume its value area holds, in percent.
    pub value_area_percent: u32,
    pub layout: RowsLayout,
    pub row_size: u32,
    pub poc: SvpLine,
    pub vah: SvpLine,
    pub val: SvpLine,
    /// How the point of control moved through the session, built only from what had traded by each
    /// moment.
    pub developing_poc: bool,
    /// How the value area's edges moved through the session.
    pub developing_value_area: bool,
    /// Write each row's volume at the end of its bar.
    pub values: bool,
    /// How much of its session's width the histogram takes, in percent.
    pub width_percent: u32,
    pub placement: VolumePlacement,
    pub up_color: Option<Rgba>,
    pub down_color: Option<Rgba>,
    pub value_area_up_color: Option<Rgba>,
    pub value_area_down_color: Option<Rgba>,
    /// A translucent box behind the histogram, over the session's range.
    pub box_color: Option<Rgba>,
    pub box_opacity: u32,
}

impl Default for SvpSettings {
    fn default() -> Self {
        Self {
            sessions: SessionMode::All,
            custom: [None; 5],
            volume: SvpVolume::UpDown,
            value_area_percent: 70,
            layout: RowsLayout::NumberOfRows,
            row_size: 24,
            poc: SvpLine::default(),
            vah: SvpLine::default(),
            val: SvpLine::default(),
            developing_poc: false,
            developing_value_area: false,
            values: false,
            width_percent: 30,
            placement: VolumePlacement::Left,
            up_color: None,
            down_color: None,
            value_area_up_color: None,
            value_area_down_color: None,
            box_color: None,
            box_opacity: 12,
        }
    }
}

/// What decides which profiles are built and how: looks never rebuild, a template never changes it.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SvpShape {
    pub sessions: SessionMode,
    pub custom: [Option<super::market_profile::sessions::SessionSpan>; 5],
    pub value_area_percent: u32,
    pub layout: RowsLayout,
    pub row_size: u32,
    /// Whether the developing levels are worked out (they cost a value area per step).
    pub developing: bool,
}

impl SvpShape {
    /// The custom windows, when the profiles are cut by them (at least one is valid).
    pub fn custom_sessions(&self) -> Option<Vec<super::market_profile::sessions::SessionSpan>> {
        if self.sessions != SessionMode::Custom {
            return None;
        }
        let spans: Vec<_> = self.custom.iter().flatten().copied().collect();
        (!spans.is_empty()).then_some(spans)
    }
}

impl SvpSettings {
    pub fn shape(&self) -> SvpShape {
        SvpShape {
            sessions: self.sessions,
            custom: self.custom,
            value_area_percent: self.value_area_percent,
            layout: self.layout,
            row_size: self.row_size,
            developing: self.developing_poc || self.developing_value_area,
        }
    }

    /// Takes the shaping fields of `own`, keeping this one's look.
    pub fn keep_shape_of(&mut self, own: &Self) {
        self.sessions = own.sessions;
        self.custom = own.custom;
        self.value_area_percent = own.value_area_percent;
        self.layout = own.layout;
        self.row_size = own.row_size;
    }

    fn sanitized(mut self) -> Self {
        self.value_area_percent = self.value_area_percent.clamp(1, 100);
        self.row_size = self.row_size.clamp(1, 1_000);
        self.width_percent = self.width_percent.clamp(5, 100);
        self.box_opacity = self.box_opacity.min(100);
        self
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
    pub footprint: FootprintSettings,
    pub tpo: TpoSettings,
    pub svp: SvpSettings,
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
            ChartType::VolumeFootprint => &self.footprint.candle,
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
            ChartType::VolumeFootprint => &mut self.footprint.candle,
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
            | ChartType::LineBreak
            | ChartType::VolumeFootprint => {
                self.candle_settings().map_or((None, None), |settings| {
                    (settings.up_body, settings.down_body)
                })
            }
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
            | ChartType::Area
            | ChartType::TimePriceOpportunity => (None, None),
            ChartType::SessionVolumeProfile => (self.svp.up_color, self.svp.down_color),
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
            ChartType::VolumeFootprint => self.footprint = fresh.footprint,
            ChartType::TimePriceOpportunity => self.tpo = fresh.tpo,
            ChartType::SessionVolumeProfile => self.svp = fresh.svp,
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
        self.footprint = self.footprint.sanitized();
        self.tpo = self.tpo.sanitized();
        self.svp = self.svp.sanitized();
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

    // MARK: Volume footprint

    #[test]
    fn the_footprint_is_an_order_flow_type_drawn_over_the_real_candles() {
        let kind = ChartType::VolumeFootprint;
        assert_eq!(kind.label(), "Volume footprint");
        assert_eq!(kind.slug(), "volume_footprint");
        assert_eq!(kind.group(), ChartGroup::OrderFlow);
        // Scripts and indicators keep reading the provider's own candles.
        assert_eq!(kind.data_class(), DataClass::StandardOhlc);
        assert!(!kind.is_synthetic());
        assert_eq!(kind.kind(), ChartKind::Standard);
        assert!(ChartType::ALL.contains(&kind));
    }

    #[test]
    fn only_markets_with_intraday_candles_and_volume_can_show_a_footprint() {
        let kind = ChartType::VolumeFootprint;
        for source in [
            DataSource::Binance,
            DataSource::Coinbase,
            DataSource::DexScreener,
        ] {
            assert!(kind.supported_by(source), "{source:?}");
        }
        for source in [
            DataSource::Alpaca,
            DataSource::CoinGecko,
            DataSource::Polymarket,
            DataSource::Kalshi,
            DataSource::CoinMarketCap,
        ] {
            assert!(!kind.supported_by(source), "{source:?}");
            assert!(kind.unsupported_reason(source).is_some());
        }
        // The choice is kept; the chart draws candles where it cannot.
        let style = ChartStyle {
            chart_type: kind,
            ..ChartStyle::default()
        };
        assert_eq!(style.effective_type(DataSource::Alpaca), ChartType::Candles);
        assert_eq!(style.effective_type(DataSource::Binance), kind);
    }

    #[test]
    fn an_old_row_has_the_default_footprint_settings_and_a_new_one_round_trips() {
        let old: ChartStyle = serde_json::from_str(r#"{"chart_type":"candles"}"#).expect("old row");
        assert_eq!(old.footprint, FootprintSettings::default());
        let mut style = ChartStyle {
            chart_type: ChartType::VolumeFootprint,
            ..ChartStyle::default()
        };
        style.footprint.imbalance_percent = 450;
        style.footprint.kind = FootprintKind::Delta;
        style.footprint.buy_color = Some([1, 2, 3, 255]);
        let back: ChartStyle =
            serde_json::from_str(&serde_json::to_string(&style).unwrap()).expect("round trip");
        assert_eq!(back, style);
        // A newer build's extra field is ignored, a missing one defaults.
        let partial: ChartStyle = serde_json::from_str(
            r#"{"chart_type":"volume_footprint","footprint":{"imbalance_percent":500,"future_field":1}}"#,
        )
        .expect("lenient");
        assert_eq!(partial.footprint.imbalance_percent, 500);
        assert_eq!(partial.footprint.value_area_percent, 70);
    }

    #[test]
    fn the_defaults_are_tradingviews_where_it_says() {
        let footprint = FootprintSettings::default();
        assert_eq!(footprint.imbalance_percent, 300);
        assert_eq!(footprint.value_area_percent, 70);
        assert_eq!(footprint.kind, FootprintKind::BuyAndSell);
        assert_eq!(footprint.summary, FootprintSummary::InfoBox);
        assert_eq!(footprint.row_mode, RowMode::Auto);
    }

    #[test]
    fn impossible_footprint_values_are_healed() {
        let mut style = ChartStyle::default();
        style.footprint.atr_length = 0;
        style.footprint.ticks_per_row = 0;
        style.footprint.value_area_percent = 0;
        style.footprint.imbalance_percent = 5;
        style.footprint.stacked_levels = 0;
        let healed = style.sanitized().footprint;
        assert_eq!(healed.atr_length, 1);
        assert_eq!(healed.ticks_per_row, 1);
        assert_eq!(healed.value_area_percent, 1);
        assert_eq!(healed.imbalance_percent, 100);
        assert_eq!(healed.stacked_levels, 2);
        style.footprint.value_area_percent = 5_000;
        style.footprint.ticks_per_row = u32::MAX;
        let healed = style.sanitized().footprint;
        assert_eq!(healed.value_area_percent, 100);
        assert_eq!(healed.ticks_per_row, 100_000);
    }

    #[test]
    fn switching_away_and_back_keeps_the_footprint_settings_and_reset_touches_only_it() {
        let mut style = ChartStyle {
            chart_type: ChartType::VolumeFootprint,
            ..ChartStyle::default()
        };
        style.footprint.stacked_levels = 5;
        style.candles.wick = false;
        style.chart_type = ChartType::Candles;
        style.chart_type = ChartType::VolumeFootprint;
        assert_eq!(style.footprint.stacked_levels, 5);
        style.reset_selected();
        assert_eq!(style.footprint, FootprintSettings::default());
        assert!(
            !style.candles.wick,
            "another type's settings are left alone"
        );
    }

    #[test]
    fn the_footprints_candle_is_its_own_and_the_up_down_colours_follow_it() {
        let mut style = ChartStyle {
            chart_type: ChartType::VolumeFootprint,
            ..ChartStyle::default()
        };
        style.footprint.candle.up_body = Some([9, 9, 9, 255]);
        assert_eq!(
            style.candle_settings().unwrap().up_body,
            Some([9, 9, 9, 255])
        );
        let (up, down) = style.up_down(([1, 1, 1, 255], [2, 2, 2, 255]));
        assert_eq!((up, down), ([9, 9, 9, 255], [2, 2, 2, 255]));
    }

    #[test]
    fn the_shaping_fields_are_the_ones_that_change_what_is_built() {
        let base = FootprintSettings::default();
        let mut restyled = base;
        restyled.buy_color = Some([1, 1, 1, 255]);
        restyled.display = FootprintDisplay::Profile;
        restyled.gradient = false;
        restyled.show_poc = false;
        assert_eq!(
            base.shape(),
            restyled.shape(),
            "looks never rebuild a footprint"
        );
        let mut reshaped = base;
        reshaped.ticks_per_row = 20;
        assert_ne!(base.shape(), reshaped.shape());
        // A template applied over a chart keeps that chart's shaping and takes the look.
        let mut theirs = restyled;
        theirs.ticks_per_row = 99;
        theirs.keep_shape_of(&base);
        assert_eq!(theirs.ticks_per_row, base.ticks_per_row);
        assert_eq!(theirs.buy_color, Some([1, 1, 1, 255]));
    }

    // MARK: Time price opportunity

    #[test]
    fn the_tpo_is_an_order_flow_type_over_the_real_candles_for_the_same_sources_as_the_footprint() {
        let kind = ChartType::TimePriceOpportunity;
        assert_eq!(kind.label(), "Time price opportunity");
        assert_eq!(kind.slug(), "time_price_opportunity");
        assert_eq!(kind.group(), ChartGroup::OrderFlow);
        assert_eq!(kind.data_class(), DataClass::StandardOhlc);
        assert!(!kind.is_synthetic());
        assert_eq!(kind.kind(), ChartKind::Standard);
        for source in [
            DataSource::Binance,
            DataSource::Coinbase,
            DataSource::DexScreener,
        ] {
            assert!(kind.supported_by(source), "{source:?}");
        }
        for source in [
            DataSource::Alpaca,
            DataSource::CoinGecko,
            DataSource::Kalshi,
        ] {
            assert!(!kind.supported_by(source), "{source:?}");
        }
    }

    #[test]
    fn the_tpo_defaults_follow_tradingview_where_it_says_and_an_old_row_has_them() {
        let tpo = TpoSettings::default();
        assert_eq!((tpo.period, tpo.period_count), (PeriodKind::Day, 1));
        assert_eq!(tpo.block_minutes, 30);
        assert_eq!(tpo.row_mode, RowMode::Auto);
        assert_eq!(tpo.value_area_percent, 70);
        let old: ChartStyle = serde_json::from_str(r#"{"chart_type":"volume_footprint"}"#).unwrap();
        assert_eq!(old.tpo, tpo);
        let mut style = ChartStyle {
            chart_type: ChartType::TimePriceOpportunity,
            ..ChartStyle::default()
        };
        style.tpo.block_minutes = 60;
        style.tpo.sessions = SessionMode::Custom;
        style.tpo.custom[1] =
            crate::model::market_profile::sessions::SessionSpan::parse("0930-1600:23456");
        style.tpo.poc.extend = true;
        style.tpo.volume.placement = VolumePlacement::Left;
        let back: ChartStyle =
            serde_json::from_str(&serde_json::to_string(&style).unwrap()).unwrap();
        assert_eq!(back, style);
    }

    #[test]
    fn impossible_tpo_values_are_healed_and_a_block_size_becomes_one_tradingview_offers() {
        let mut style = ChartStyle::default();
        style.tpo.period_count = 0;
        style.tpo.block_minutes = 40;
        style.tpo.ticks_per_row = 0;
        style.tpo.value_area_percent = 500;
        style.tpo.initial_balance_blocks = 0;
        style.tpo.opacity_outside_value_area = 900;
        let healed = style.sanitized().tpo;
        assert_eq!(healed.period_count, 1);
        assert_eq!(healed.block_minutes, 30, "40 is nearer 30 than 60");
        assert_eq!(healed.ticks_per_row, 1);
        assert_eq!(healed.value_area_percent, 100);
        assert_eq!(healed.initial_balance_blocks, 1);
        assert_eq!(healed.opacity_outside_value_area, 100);
        style.tpo.block_minutes = 1_000;
        assert_eq!(style.sanitized().tpo.block_minutes, 240);
    }

    #[test]
    fn the_tpo_shape_is_what_decides_the_profiles_and_a_look_never_rebuilds() {
        let base = TpoSettings::default();
        let mut restyled = base;
        restyled.gradient[0] = Some([1, 2, 3, 255]);
        restyled.show_letters = false;
        restyled.poc.extend = true;
        restyled.volume.show = false;
        restyled.opacity_outside_value_area = 10;
        assert_eq!(base.shape(), restyled.shape());
        assert_eq!(base.shape().edit_key(), restyled.shape().edit_key());
        let mut reshaped = base;
        reshaped.block_minutes = 60;
        assert_ne!(base.shape(), reshaped.shape());
        assert_ne!(
            base.shape().edit_key(),
            reshaped.shape().edit_key(),
            "splits mean nothing then"
        );
        // Row size and the value area change what is built but not where a split is.
        let mut rows = base;
        rows.ticks_per_row = 77;
        assert_ne!(base.shape(), rows.shape());
        assert_eq!(base.shape().edit_key(), rows.shape().edit_key());
        // A template over a chart keeps that chart's shape and takes the look.
        let mut theirs = restyled;
        theirs.period = PeriodKind::Week;
        theirs.keep_shape_of(&base);
        assert_eq!(theirs.period, PeriodKind::Day);
        assert!(!theirs.show_letters);
    }

    #[test]
    fn custom_sessions_are_only_used_when_chosen_and_one_is_valid() {
        let mut tpo = TpoSettings::default();
        tpo.custom[0] = crate::model::market_profile::sessions::SessionSpan::parse("0900-1700");
        assert!(
            tpo.shape().custom_sessions().is_none(),
            "All ignores the windows"
        );
        tpo.sessions = SessionMode::Custom;
        assert_eq!(tpo.shape().custom_sessions().unwrap().len(), 1);
        tpo.custom[0] = None;
        assert!(tpo.shape().custom_sessions().is_none(), "no valid window");
    }

    #[test]
    fn switching_away_from_the_tpo_keeps_its_settings_and_reset_touches_only_it() {
        let mut style = ChartStyle {
            chart_type: ChartType::TimePriceOpportunity,
            ..ChartStyle::default()
        };
        style.tpo.value_area_percent = 55;
        style.footprint.stacked_levels = 7;
        style.chart_type = ChartType::VolumeFootprint;
        style.chart_type = ChartType::TimePriceOpportunity;
        assert_eq!(style.tpo.value_area_percent, 55);
        style.reset_selected();
        assert_eq!(style.tpo, TpoSettings::default());
        assert_eq!(style.footprint.stacked_levels, 7);
    }

    // MARK: Session volume profile

    #[test]
    fn the_session_profile_is_an_order_flow_type_over_the_real_candles() {
        let kind = ChartType::SessionVolumeProfile;
        assert_eq!(kind.label(), "Session volume profile");
        assert_eq!(kind.slug(), "session_volume_profile");
        assert_eq!(kind.group(), ChartGroup::OrderFlow);
        assert_eq!(kind.data_class(), DataClass::StandardOhlc);
        assert!(!kind.is_synthetic());
        assert_eq!(kind.kind(), ChartKind::Standard);
        for source in [
            DataSource::Binance,
            DataSource::Coinbase,
            DataSource::DexScreener,
        ] {
            assert!(kind.supported_by(source), "{source:?}");
        }
        for source in [
            DataSource::Alpaca,
            DataSource::CoinGecko,
            DataSource::Kalshi,
        ] {
            assert!(!kind.supported_by(source), "{source:?}");
        }
        // Its colours stand for the chart's up and down where the chart borrows them.
        let mut style = ChartStyle {
            chart_type: kind,
            ..ChartStyle::default()
        };
        style.svp.up_color = Some([1, 2, 3, 255]);
        assert_eq!(style.up_down(([9; 4], [8; 4])), ([1, 2, 3, 255], [8; 4]));
    }

    #[test]
    fn the_session_profile_defaults_follow_tradingviews_inputs_and_round_trip() {
        let svp = SvpSettings::default();
        assert_eq!(svp.sessions, SessionMode::All);
        assert_eq!(svp.volume, SvpVolume::UpDown);
        assert_eq!(svp.value_area_percent, 70);
        assert_eq!((svp.layout, svp.row_size), (RowsLayout::NumberOfRows, 24));
        assert!(
            !svp.poc.extend && !svp.vah.extend && !svp.val.extend,
            "extends default off"
        );
        let old: ChartStyle =
            serde_json::from_str(r#"{"chart_type":"time_price_opportunity"}"#).unwrap();
        assert_eq!(old.svp, svp);
        let mut style = ChartStyle {
            chart_type: ChartType::SessionVolumeProfile,
            ..ChartStyle::default()
        };
        style.svp.volume = SvpVolume::Delta;
        style.svp.placement = VolumePlacement::Right;
        style.svp.developing_poc = true;
        style.svp.box_color = Some([1, 2, 3, 255]);
        style.svp.custom[0] =
            crate::model::market_profile::sessions::SessionSpan::parse("0900-1700");
        let back: ChartStyle =
            serde_json::from_str(&serde_json::to_string(&style).unwrap()).unwrap();
        assert_eq!(back, style);
    }

    #[test]
    fn impossible_session_profile_values_are_healed() {
        let mut style = ChartStyle::default();
        style.svp.value_area_percent = 0;
        style.svp.row_size = 0;
        style.svp.width_percent = 0;
        style.svp.box_opacity = 900;
        let healed = style.sanitized().svp;
        assert_eq!((healed.value_area_percent, healed.row_size), (1, 1));
        assert_eq!(healed.width_percent, 5);
        assert_eq!(healed.box_opacity, 100);
        style.svp.row_size = 1_000_000;
        assert_eq!(style.sanitized().svp.row_size, 1_000);
    }

    #[test]
    fn the_session_shape_is_what_decides_the_profiles_and_a_look_never_rebuilds() {
        let base = SvpSettings::default();
        let mut restyled = base;
        restyled.up_color = Some([1, 2, 3, 255]);
        restyled.volume = SvpVolume::Total;
        restyled.values = true;
        restyled.width_percent = 80;
        restyled.placement = VolumePlacement::Right;
        restyled.poc.extend = true;
        assert_eq!(base.shape(), restyled.shape());
        let mut reshaped = base;
        reshaped.row_size = 30;
        assert_ne!(base.shape(), reshaped.shape());
        // Developing levels cost a value area per step: asking for one changes the build.
        let mut developing = base;
        developing.developing_value_area = true;
        assert_ne!(base.shape(), developing.shape());
        assert!(developing.shape().developing && !base.shape().developing);
        // A template over a chart keeps that chart's shape and takes the look.
        let mut theirs = restyled;
        theirs.row_size = 99;
        theirs.sessions = SessionMode::Custom;
        theirs.keep_shape_of(&base);
        assert_eq!((theirs.row_size, theirs.sessions), (24, SessionMode::All));
        assert_eq!(theirs.up_color, Some([1, 2, 3, 255]));
    }

    #[test]
    fn switching_away_from_the_session_profile_keeps_its_settings_and_reset_touches_only_it() {
        let mut style = ChartStyle {
            chart_type: ChartType::SessionVolumeProfile,
            ..ChartStyle::default()
        };
        style.svp.row_size = 48;
        style.tpo.block_minutes = 60;
        style.chart_type = ChartType::TimePriceOpportunity;
        style.chart_type = ChartType::SessionVolumeProfile;
        assert_eq!(style.svp.row_size, 48);
        style.reset_selected();
        assert_eq!(style.svp, SvpSettings::default());
        assert_eq!(style.tpo.block_minutes, 60);
    }
}
