import SwiftUI

struct CandleChartStyle {
    // Candle colors
    var bullishColor: Color = .green
    var bearishColor: Color = .red
    var dojiColor: Color = .gray

    // Candle geometry
    var wickWidth: CGFloat = 1
    var bodyWidthRatio: CGFloat = 0.6      // fraction of available slot per candle
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

    // Crosshair
    var crosshairColor: Color = .yellow.opacity(0.7)
    var crosshairLineWidth: CGFloat = 0.5

    // Background
    var backgroundColor: Color = .clear

    // Current price line
    var currentPriceLineWidth: CGFloat = 1
    var currentPriceDashPattern: [CGFloat] = [5, 3]

    // Layout
    var chartInsets: EdgeInsets = EdgeInsets(top: 8, leading: 70, bottom: 28, trailing: 60)

    static let `default` = CandleChartStyle()
}
