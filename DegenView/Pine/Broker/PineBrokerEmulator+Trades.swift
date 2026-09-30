import Foundation

/// Opening and closing trades, with slippage, sizing and commission.
extension PineBrokerEmulator {
    /// Adverse slippage on market and stop fills; limit fills never slip.
    func slipped(_ price: Double, buying: Bool, _ mintick: Double) -> Double {
        let amount = Double(settings.slippage) * mintick
        return buying ? price + amount : price - amount
    }

    private func defaultQuantity(at price: Double) -> Double {
        guard price > 0 else { return 0 }
        switch settings.quantityType {
        case .fixed: return settings.quantityValue
        case .cash: return settings.quantityValue / price
        case .percentOfEquity: return max(0, equity(at: price)) * settings.quantityValue / 100 / price
        }
    }

    private func commission(quantity: Double, price: Double, chargesOrder: Bool) -> Double {
        switch settings.commissionType {
        case .percent: quantity * price * settings.commissionValue / 100
        case .cashPerContract: quantity * settings.commissionValue
        case .cashPerOrder: chargesOrder ? settings.commissionValue : 0
        }
    }

    // MARK: - Orders that open

    mutating func executeEntry(
        _ order: Order, price rawPrice: Double, slipped useSlippage: Bool, _ context: BarContext
    ) {
        let price =
            useSlippage ? slipped(rawPrice, buying: order.isLong, context.mintick) : rawPrice
        let position = positionSize
        let opposesPosition = order.isLong ? position < -Self.epsilon : position > Self.epsilon

        switch order.kind {
        case .entry:
            if opposesPosition {
                // Reversal: flatten first, then open the new side at the same price.
                close(
                    matching: { $0.isLong != order.isLong }, quantity: nil, at: price, exitID: order.id,
                    context)
            } else {
                let sameSide = openTrades.filter { $0.isLong == order.isLong }.count
                // `pyramiding` counts extra entries; 0 and 1 both allow one.
                guard sameSide < max(1, settings.pyramiding) else { return }
            }
            open(order, quantity: order.quantity ?? defaultQuantity(at: price), at: price, context)
        case .order:
            var remaining = order.quantity ?? defaultQuantity(at: price)
            if opposesPosition {
                let reduce = min(remaining, abs(position))
                close(
                    matching: { $0.isLong != order.isLong }, quantity: reduce, at: price,
                    exitID: order.id, context)
                remaining -= reduce
            }
            open(order, quantity: remaining, at: price, context)
        case .exit, .close, .closeAll: break
        }
    }

    private mutating func open(
        _ order: Order, quantity: Double, at price: Double, _ context: BarContext
    ) {
        guard quantity.isFinite, quantity > Self.epsilon else { return }
        openTrades.append(
            .init(
                entryID: order.id, isLong: order.isLong, quantity: quantity,
                entryBar: context.barIndex, entryTime: context.bar.openTime, entryPrice: price,
                entryCommission: commission(quantity: quantity, price: price, chargesOrder: true)))
    }

    // MARK: - Orders that close

    /// `strategy.close(id)`: closing a long sells; closing a short buys.
    mutating func executeClose(_ order: Order, price: Double, _ context: BarContext) {
        let matching = openTrades.filter { $0.entryID == order.id }
        let total = matching.reduce(0) { $0 + $1.quantity }
        let quantity = order.quantity ?? order.quantityPercent.map { total * $0 / 100 }
        let isLong = matching.first?.isLong ?? true
        close(
            matching: { $0.entryID == order.id }, quantity: quantity,
            at: slipped(price, buying: !isLong, context.mintick),
            exitID: "Close entry(s) order \(order.id)", context)
    }

    mutating func executeCloseAll(_ order: Order, price: Double, _ context: BarContext) {
        let isLong = positionSize > 0
        close(
            matching: { _ in true }, quantity: nil,
            at: slipped(price, buying: !isLong, context.mintick), exitID: order.id, context)
    }

    /// Closes up to `quantity` (nil = everything) of the open trades `matching` selects,
    /// oldest first. A per-order commission is charged once however many trades close.
    mutating func close(
        matching selects: (PineOpenTrade) -> Bool, quantity: Double?, at price: Double,
        exitID: String, _ context: BarContext
    ) {
        var remaining = quantity ?? .infinity
        var chargedOrder = false
        var index = 0
        while index < openTrades.count, remaining > Self.epsilon {
            var trade = openTrades[index]
            guard selects(trade) else {
                index += 1
                continue
            }
            let closing = min(trade.quantity, remaining)
            let entryShare = trade.entryCommission * (closing / trade.quantity)
            let exitCommission = commission(quantity: closing, price: price, chargesOrder: !chargedOrder)
            chargedOrder = true
            let direction = trade.isLong ? 1.0 : -1.0
            let profit = (price - trade.entryPrice) * direction * closing - entryShare - exitCommission
            closedTrades.append(
                .init(
                    id: closedTrades.count + 1, entryID: trade.entryID, exitID: exitID,
                    isLong: trade.isLong, quantity: closing, entryBar: trade.entryBar,
                    entryTime: trade.entryTime, entryPrice: trade.entryPrice, exitBar: context.barIndex,
                    exitTime: context.bar.openTime, exitPrice: price,
                    commission: entryShare + exitCommission, profit: profit))
            netProfit += profit
            remaining -= closing
            if closing >= trade.quantity - Self.epsilon {
                openTrades.remove(at: index)
            } else {
                trade.quantity -= closing
                trade.entryCommission -= entryShare
                openTrades[index] = trade
                index += 1
            }
        }
    }
}
