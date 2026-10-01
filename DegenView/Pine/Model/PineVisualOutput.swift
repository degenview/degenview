import Foundation

struct PineVisualOutput: Sendable {
    var overlay: Bool
    /// Number of bars the script ran over. Drawing objects use absolute `bar_index`
    /// coordinates; the renderer subtracts `barCount - visibleCandles` to map them.
    var barCount: Int = 0
    var plots: [PinePlotOutput] = []
    var hlines: [PineHorizontalLine] = []
    var markers: [PineMarkerOutput] = []
    var backgrounds: [PineColorOutput] = []
    var barColors: [PineColorOutput] = []
    var fills: [PineFillOutput] = []
    var lines: [PineLineOutput] = []
    var labels: [PineLabelOutput] = []
    var boxes: [PineBoxOutput] = []
    var linefills: [PineLinefillOutput] = []
    var tables: [PineTableOutput] = []
    var candles: [PineCandleOutput] = []
    var alerts: [PineAlertEvent] = []
    var strategy: PineStrategyReport?
    static let empty = PineVisualOutput(overlay: true)
}
