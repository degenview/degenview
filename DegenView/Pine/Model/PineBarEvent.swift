import Foundation

struct PineBarEvent: Sendable {
    var candle: KlineData
    var phase: PineBarPhase
}
