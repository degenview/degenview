import Foundation

extension PineBrokerEmulator {
    struct Order: Sendable, Equatable {
        enum Kind: Sendable {
            case entry, order, exit, close, closeAll

            /// `strategy.entry` and `strategy.order` both open or grow a position.
            var isEntryLike: Bool { self == .entry || self == .order }
        }

        var kind: Kind
        var id: String
        var isLong = true
        var quantity: Double?
        var quantityPercent: Double?
        var limit: Double?
        var stop: Double?
        /// `strategy.exit`: the entry this exit protects; nil protects every open trade.
        var fromEntry: String?
        var profitTicks: Double?
        var lossTicks: Double?
        var sequence = 0

        /// Fills at the next opportunity price instead of waiting for a level.
        var isMarket: Bool {
            switch kind {
            case .entry, .order: limit == nil && stop == nil
            case .close, .closeAll: true
            case .exit: false
            }
        }

        /// Whether this exit order covers the trade opened by `entryID`.
        func protects(entryID: String) -> Bool { fromEntry == nil || fromEntry == entryID }
    }

    /// The bar an order is being processed against.
    struct BarContext {
        let bar: KlineData
        let barIndex: Int
        let mintick: Double
    }
}
