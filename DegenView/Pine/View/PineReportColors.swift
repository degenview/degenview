import SwiftUI

/// Profit and loss colors, the same ones the chart draws trades with.
enum PineReportColors {
    static let win = Color(pineRGBA: PineChartLayer.winColor)
    static let loss = Color(pineRGBA: PineChartLayer.lossColor)

    /// Green above zero, red below, neutral at zero.
    static func tint(_ value: Double) -> Color { value > 0 ? win : value < 0 ? loss : .primary }
}
