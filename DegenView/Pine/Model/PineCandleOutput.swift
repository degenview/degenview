import Foundation

struct PineCandleOutput: Sendable, Identifiable {
    let id: Int
    var title: String?
    /// Per-bar candles, parallel to the script's bars; nil where the inputs were `na`.
    var bars: [PineCandleBar?]
    var display = PineDisplay.all
}
