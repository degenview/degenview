import Foundation

/// A round trip closed by the broker emulator. `profit` is net of both commissions.
struct PineTrade: Sendable, Identifiable, Equatable {
    let id: Int
    var entryID: String
    var exitID: String
    var isLong: Bool
    var quantity: Double
    var entryBar: Int
    var entryTime: Date
    var entryPrice: Double
    var exitBar: Int
    var exitTime: Date
    var exitPrice: Double
    var commission: Double
    var profit: Double
}
