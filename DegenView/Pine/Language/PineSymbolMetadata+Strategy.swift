import Foundation

extension PineSymbolMetadata {
    /// `strategy.*` order functions. The `strategy.risk.*` limits are accepted but ignored, so they
    /// are not offered.
    static let strategyEntries = """
        strategy.entry(id: series string, direction: strategy_direction, qty: series float = na, limit: series float = na, stop: series float = na, oca_name: series string = na, oca_type: input string = na, comment: series string = na, alert_message: series string = na) -> void :: Opens or adds to a position.
        strategy.order(id: series string, direction: strategy_direction, qty: series float = na, limit: series float = na, stop: series float = na, oca_name: series string = na, oca_type: input string = na, comment: series string = na, alert_message: series string = na) -> void :: Places an order without reversing a position.
        strategy.exit(id: series string, from_entry: series string = "", qty: series float = na, qty_percent: series float = 100, profit: series float = na, limit: series float = na, loss: series float = na, stop: series float = na, comment: series string = na, alert_message: series string = na) -> void :: Exits a position with a profit target or stop.
        strategy.close(id: series string, comment: series string = na, qty: series float = na, qty_percent: series float = 100) -> void :: Closes the position opened by an entry id.
        strategy.close_all(comment: series string = na) -> void :: Closes the whole position.
        strategy.cancel(id: series string) -> void :: Cancels a pending order by id.
        strategy.cancel_all() -> void :: Cancels every pending order.
        """
}
