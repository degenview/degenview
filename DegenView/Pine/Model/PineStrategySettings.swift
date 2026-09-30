import Foundation

/// The `strategy()` declaration arguments the broker emulator honours.
struct PineStrategySettings: Codable, Equatable, Sendable {
    enum QuantityType: String, Codable, Sendable { case fixed, cash, percentOfEquity }
    enum CommissionType: String, Codable, Sendable { case percent, cashPerOrder, cashPerContract }

    var initialCapital = 1_000_000.0
    var quantityType = QuantityType.fixed
    var quantityValue = 1.0
    var commissionType = CommissionType.percent
    var commissionValue = 0.0
    /// Adverse slippage, in ticks, on market and stop fills.
    var slippage = 0
    var pyramiding = 1
    var currency: String?
    var processOrdersOnClose = false
    /// `calc_on_every_tick`: recalculate on every realtime update instead of only at bar close.
    var calcOnEveryTick = false
}
