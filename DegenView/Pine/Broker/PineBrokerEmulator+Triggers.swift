import Foundation

/// Finding and applying the next stop/limit trigger along a price segment.
extension PineBrokerEmulator {
    struct Trigger {
        enum Action {
            case fillEntry(Order, viaStop: Bool)
            /// A stop-limit entry's stop was crossed; it becomes a plain limit order.
            case activate(Order)
            case exit(Order, tradeIndex: Int, viaStop: Bool)
        }

        var action: Action
        var price: Double
        var distance: Double
        var sequence: Int

        init(_ action: Action, _ hit: (price: Double, distance: Double), order: Order) {
            self.action = action
            self.price = hit.price
            self.distance = hit.distance
            self.sequence = order.sequence
        }
    }

    /// The earliest trigger on the move from `p0` to `p1`; ties go to the older order.
    func nextTrigger(from p0: Double, to p1: Double, mintick: Double) -> Trigger? {
        /// `atOrAbove`: the level triggers once price is at or above it, else at or below it.
        func reach(_ level: Double, atOrAbove: Bool) -> (price: Double, distance: Double)? {
            if atOrAbove {
                if p0 >= level { return (p0, 0) }
                if p1 >= level { return (level, level - p0) }
            } else {
                if p0 <= level { return (p0, 0) }
                if p1 <= level { return (level, p0 - level) }
            }
            return nil
        }
        var best: Trigger?
        func consider(_ trigger: Trigger?) {
            guard let trigger else { return }
            if let current = best,
                (current.distance, current.sequence) <= (trigger.distance, trigger.sequence)
            {
                return
            }
            best = trigger
        }
        for order in orders {
            switch order.kind {
            case .entry, .order: consider(entryTrigger(order, reach))
            case .exit:
                for (index, trade) in openTrades.enumerated() where order.protects(entryID: trade.entryID) {
                    let levels = exitLevels(order, trade, mintick: mintick)
                    // A long exits by selling: its stop is below, its limit above.
                    if let stop = levels.stop, let hit = reach(stop, atOrAbove: !trade.isLong) {
                        consider(.init(.exit(order, tradeIndex: index, viaStop: true), hit, order: order))
                    }
                    if let limit = levels.limit, let hit = reach(limit, atOrAbove: trade.isLong) {
                        consider(.init(.exit(order, tradeIndex: index, viaStop: false), hit, order: order))
                    }
                }
            case .close, .closeAll: break
            }
        }
        return best
    }

    private func entryTrigger(
        _ order: Order, _ reach: (Double, Bool) -> (price: Double, distance: Double)?
    ) -> Trigger? {
        if let stop = order.stop, let hit = reach(stop, order.isLong) {
            let action: Trigger.Action =
                order.limit == nil ? .fillEntry(order, viaStop: true) : .activate(order)
            return .init(action, hit, order: order)
        }
        if order.stop == nil, let limit = order.limit, let hit = reach(limit, !order.isLong) {
            return .init(.fillEntry(order, viaStop: false), hit, order: order)
        }
        return nil
    }

    mutating func apply(_ trigger: Trigger, _ context: BarContext) {
        switch trigger.action {
        case .activate(let order):
            if let index = orders.firstIndex(where: { $0.sequence == order.sequence }) {
                orders[index].stop = nil
            }
        case .fillEntry(let order, let viaStop):
            orders.removeAll { $0.sequence == order.sequence }
            executeEntry(order, price: trigger.price, slipped: viaStop, context)
        case .exit(let order, let tradeIndex, let viaStop):
            applyExit(order, tradeIndex: tradeIndex, viaStop: viaStop, at: trigger.price, context)
        }
    }

    private mutating func applyExit(
        _ order: Order, tradeIndex: Int, viaStop: Bool, at triggerPrice: Double, _ context: BarContext
    ) {
        guard openTrades.indices.contains(tradeIndex) else { return }
        let trade = openTrades[tradeIndex]
        let requested = order.quantity ?? order.quantityPercent.map { trade.quantity * $0 / 100 }
        let price =
            viaStop
            ? slipped(triggerPrice, buying: !trade.isLong, context.mintick) : triggerPrice
        close(
            matching: { $0.entryID == trade.entryID && $0.entryBar == trade.entryBar },
            quantity: min(requested ?? trade.quantity, trade.quantity), at: price, exitID: order.id,
            context)
        if !openTrades.contains(where: { order.protects(entryID: $0.entryID) }) {
            orders.removeAll { $0.sequence == order.sequence }
        }
    }

    private func exitLevels(_ order: Order, _ trade: PineOpenTrade, mintick: Double) -> (
        stop: Double?, limit: Double?
    ) {
        let direction = trade.isLong ? 1.0 : -1.0
        let limit =
            order.limit ?? order.profitTicks.map { trade.entryPrice + direction * $0 * mintick }
        let stop = order.stop ?? order.lossTicks.map { trade.entryPrice - direction * $0 * mintick }
        return (stop, limit)
    }
}
