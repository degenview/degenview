import SwiftUI

struct CandleChartStyle {
    // Candle colors
    var bullishColor: Color = .green
    var bearishColor: Color = .red
    var dojiColor: Color = .gray

    // Candle geometry
    var wickWidth: CGFloat = 1
    var candleBodyWidth: CGFloat = 7        // fixed body width in points — constant regardless of interval/zoom
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
