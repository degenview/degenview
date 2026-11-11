use crate::model::{ChartCanvas, ChartStyle, PriceFormat, SmallPrices};
use serde::{Deserialize, Serialize};

/// Per-chart colors, price-axis precision, chart type and canvas. The defaults match SwiftUI's
/// dark system colors.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct ChartAppearance {
    pub bullish: [u8; 3],
    pub bearish: [u8; 3],
    /// None chooses precision from the price magnitude; Some(0..=12) fixes the axis precision.
    pub axis_decimals: Option<u8>,
    /// How a price under 0.001 reads: its zeros counted (`0.0₅278`) or written out.
    pub axis_small_prices: SmallPrices,
    /// How the bars are drawn, and each chart type's own settings; old rows read as candles.
    pub style: ChartStyle,
    /// The chart's background, grid, crosshair, watermark, scales, buttons and margins; old rows
    /// read as the default canvas.
    pub canvas: ChartCanvas,
}

impl Default for ChartAppearance {
    fn default() -> Self {
        Self {
            bullish: [48, 209, 88],
            bearish: [255, 69, 58],
            axis_decimals: None,
            axis_small_prices: SmallPrices::default(),
            style: ChartStyle::default(),
            canvas: ChartCanvas::default(),
        }
    }
}

impl ChartAppearance {
    /// How this chart prints a price on its axis, crosshair, readouts and drawings.
    pub fn price_format(&self) -> PriceFormat {
        PriceFormat {
            decimals: self.axis_decimals,
            small: self.axis_small_prices,
        }
    }
}
