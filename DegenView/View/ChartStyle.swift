import SwiftUI

/// Visual parameters shared by every chart renderer.
///
/// Grid, price axis, insets and the current-price marker are identical whether the
/// series is drawn as candles or as a line, so one style struct feeds both. The
/// candle- and line-specific sections apply only to their own renderer.
struct ChartStyle {
    // Candle colors
    var bullishColor: Color = .green
    var bearishColor: Color = .red
    var dojiColor: Color = .gray

    // Candle geometry — widths scale with slot (plot width ÷ candle count)
    var candleBodyFraction: CGFloat = 0.75  // body width as fraction of slot width
    var candleBodyMin: CGFloat = 1  // minimum body width in points
    var candleBodyMax: CGFloat = 15  // maximum body width in points
    var wickFraction: CGFloat = 0.12  // wick width as fraction of slot width
    var wickMin: CGFloat = 0.5  // minimum wick width in points
    var wickMax: CGFloat = 2  // maximum wick width in points
    var minBodyHeight: CGFloat = 1  // minimum body height in points (doji)
    var dojiThreshold: Double = 0.00001  // relative to price range; abs(close-open)/range < this → doji

    // Volume bars — off unless enabled per chart
    var volumePaneFraction: CGFloat = 0.22  // share of plot height the bars rise into
    var volumeOpacity: Double = 0.35  // kept low: candles draw on top of them

    // RSI — off unless enabled per chart
    var rsiPaneFraction: CGFloat = 0.28  // share of plot height the 0–100 scale maps to
    var rsiColor: Color = .purple
    var rsiLineWidth: CGFloat = 1.2
    var rsiGuideColor: Color = .purple.opacity(0.25)
    var rsiGuideDashPattern: [CGFloat] = [3, 3]

    // Price-scale overlays — off unless enabled per chart
    var emaColor: Color = .orange
    var emaLineWidth: CGFloat = 1.3
    var bollingerColor: Color = .cyan
    var bollingerLineWidth: CGFloat = 1
    /// Tint between the bands. Low enough to leave the candles legible through it.
    var bollingerFillOpacity: Double = 0.08

    // Confirmed Supertrend regime changes — colored with each chart's bull/bear colors.
    var trendFlipMarkerSize: CGFloat = 7
    var trendFlipMarkerGap: CGFloat = 4
    var trendFlipLabelFontSize: CGFloat = 8
    var trendFlipLabelHorizontalPadding: CGFloat = 4
    var trendFlipLabelVerticalPadding: CGFloat = 2
    var trendFlipLabelGap: CGFloat = 2

    // Line geometry
    var lineWidth: CGFloat = 1.5
    /// Opacity at the top of the gradient under the line; fades to zero at the bottom.
    var areaFillOpacity: Double = 0.18

    // Price axis
    var pricePadding: CGFloat = 0.05  // vertical padding fraction
    var priceLabelCount: Int = 5

    // Time axis
    var timeLabelCount: Int = 4

    // Grid
    var gridColor: Color = .secondary.opacity(0.15)
    var gridLineWidth: CGFloat = 0.5

    // Current price line
    var currentPriceLineWidth: CGFloat = 1
    var currentPriceDashPattern: [CGFloat] = [5, 3]

    // Hand-drawn trend lines. Blue and pink stay clear of every other overlay —
    // green/red candles, purple RSI, orange EMA, cyan Bollinger.
    var trendLineColor: Color = .blue
    var trendLineSelectedColor: Color = .pink
    var trendLineWidth: CGFloat = 1.5
    var trendLineSelectedWidth: CGFloat = 2
    var trendHandleRadius: CGFloat = 4
    /// Dash for the rubber band between the first click and the second.
    var trendDashPattern: [CGFloat] = [4, 3]

    // Ruler rectangles. No colour of their own — they take the chart's bull/bear colours,
    // since the whole point of the tool is which way the move went. The fill stays as low
    // as the Bollinger tint so the candles being measured still read through it.
    var rulerFillOpacity: Double = 0.12
    /// Faint on purpose: the border marks where the measurement ends, it isn't the
    /// measurement. A solid edge competes with the candles it brackets.
    var rulerBorderOpacity: Double = 0.4
    var rulerBorderWidth: CGFloat = 1
    /// Dash for the rectangle between the first click and the second.
    var rulerDashPattern: [CGFloat] = [4, 3]

    // Crosshair. Neutral on purpose — blue and pink belong to trend lines, and purple,
    // orange and cyan to the indicators, so a tinted crosshair would read as one of them.
    var crosshairColor: Color = .secondary
    var crosshairLineWidth: CGFloat = 1
    var crosshairDashPattern: [CGFloat] = [3, 3]
    /// Backing for the time and price read-outs, dark enough for white text either theme.
    var crosshairLabelColor: Color = .black.opacity(0.75)

    // Layout
    var chartInsets: EdgeInsets = EdgeInsets(top: 8, leading: 8, bottom: 4, trailing: 70)

    static let `default` = ChartStyle()
}
