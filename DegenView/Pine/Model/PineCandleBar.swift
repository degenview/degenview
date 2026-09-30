import Foundation

/// One bar of `plotcandle()`. A nil color hides that part.
struct PineCandleBar: Sendable, Equatable {
    var open: Double
    var high: Double
    var low: Double
    var close: Double
    var color: UInt32?
    var wickColor: UInt32?
    var borderColor: UInt32?
}
