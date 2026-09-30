import Foundation

struct PineOpenTrade: Sendable, Identifiable, Equatable {
    var id: String { "\(entryID)@\(entryBar)" }
    var entryID: String
    var isLong: Bool
    var quantity: Double
    var entryBar: Int
    var entryTime: Date
    var entryPrice: Double
    var entryCommission: Double
}
