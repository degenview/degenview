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
    var candleBodyMin: CGFloat = 1          // minimum body width in points
    var candleBodyMax: CGFloat = 15         // maximum body width in points
    var wickFraction: CGFloat = 0.12        // wick width as fraction of slot width
    var wickMin: CGFloat = 0.5              // minimum wick width in points
    var wickMax: CGFloat = 2                // maximum wick width in points
    var minBodyHeight: CGFloat = 1          // minimum body height in points (doji)
    var dojiThreshold: Double = 0.00001     // relative to price range; abs(close-open)/range < this → doji

    // Volume bars — off unless enabled per chart
    var volumePaneFraction: CGFloat = 0.22  // share of plot height the bars rise into
    var volumeOpacity: Double = 0.35        // kept low: candles draw on top of them

    // RSI — off unless enabled per chart
    var rsiPaneFraction: CGFloat = 0.28     // share of plot height the 0–100 scale maps to
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

    // Line geometry
    var lineWidth: CGFloat = 1.5
    /// Opacity at the top of the gradient under the line; fades to zero at the bottom.
    var areaFillOpacity: Double = 0.18

    // Price axis
    var pricePadding: CGFloat = 0.05        // vertical padding fraction
    var priceLabelCount: Int = 5

    // Time axis
    var timeLabelCount: Int = 4

    // Grid
    var gridColor: Color = .secondary.opacity(0.15)
    var gridLineWidth: CGFloat = 0.5

    // Current price line
    var currentPriceLineWidth: CGFloat = 1
    var currentPriceDashPattern: [CGFloat] = [5, 3]

    // Layout
    var chartInsets: EdgeInsets = EdgeInsets(top: 8, leading: 8, bottom: 4, trailing: 70)

    static let `default` = ChartStyle()
}
