import SwiftUI

struct CandleChartStyle {
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

    static let `default` = CandleChartStyle()
}
