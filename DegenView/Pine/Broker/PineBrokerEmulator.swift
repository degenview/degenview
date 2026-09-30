import Foundation

/// Backtest broker for `strategy()` scripts.
///
/// A plain value type: the runtime keeps it inside its per-bar state, so realtime rollback
/// restores orders, positions, and trades together with every other variable.
///
/// Per bar the runtime calls `process(bar:…)` *before* the script runs — orders queued on
/// earlier bars fill against this bar — so `strategy.position_size` already reflects those
/// fills, and orders the script places now wait for the next bar (or fill at the close when
/// `process_orders_on_close` is set).
struct PineBrokerEmulator: Sendable {
    struct Order: Sendable, Equatable {
        enum Kind: Sendable { case entry, order, exit, close, closeAll }
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
    }

    private(set) var settings: PineStrategySettings
    private(set) var openTrades: [PineOpenTrade] = []
    private(set) var closedTrades: [PineTrade] = []
    private(set) var netProfit = 0.0
    private var equityCurve: [Double] = []
    private var lastClose = 0.0
    private var orders: [Order] = []
    private var nextSequence = 0

    private static let epsilon = 1e-12

    init(settings: PineStrategySettings = PineStrategySettings()) {
        self.settings = settings
    }

    // MARK: - Position and equity

    /// Signed contracts: positive long, negative short.
    var positionSize: Double {
        openTrades.reduce(0) { $0 + ($1.isLong ? $1.quantity : -$1.quantity) }
    }

    var averagePrice: Double {
        let total = openTrades.reduce(0) { $0 + $1.quantity }
        guard total > 0 else { return 0 }
        return openTrades.reduce(0) { $0 + $1.entryPrice * $1.quantity } / total
    }

    /// Gross unrealized P&L of the open trades at `price`.
    func openProfit(at price: Double) -> Double {
        openTrades.reduce(0) { $0 + ($1.isLong ? 1 : -1) * (price - $1.entryPrice) * $1.quantity }
    }

    /// Initial capital plus realized P&L plus open P&L, less entry commissions already paid.
    func equity(at price: Double) -> Double {
        settings.initialCapital + netProfit + openProfit(at: price)
            - openTrades.reduce(0) { $0 + $1.entryCommission }
    }

    mutating func recordEquity(close: Double) {
        lastClose = close
        equityCurve.append(equity(at: close))
    }

    func report() -> PineStrategyReport {
        .init(
            settings: settings, trades: closedTrades, openTrades: openTrades, equity: equityCurve,
            openProfit: openProfit(at: lastClose) - openTrades.reduce(0) { $0 + $1.entryCommission })
    }

    // MARK: - Order book

    /// Placing an order with the id of a pending one of the same family replaces it.
    mutating func place(_ order: Order) {
        var order = order
        order.sequence = nextSequence
        nextSequence += 1
        switch order.kind {
        case .entry, .order:
            orders.removeAll { ($0.kind == .entry || $0.kind == .order) && $0.id == order.id }
        case .exit: orders.removeAll { $0.kind == .exit && $0.id == order.id }
        case .close: orders.removeAll { $0.kind == .close && $0.id == order.id }
        case .closeAll: orders.removeAll { $0.kind == .closeAll }
        }
        // An exit with no level at all (every argument `na`) cancels the previous one.
        if order.kind == .exit, order.stop == nil, order.limit == nil, order.profitTicks == nil,
            order.lossTicks == nil
        {
            return
        }
        orders.append(order)
    }

    mutating func cancel(id: String) { orders.removeAll { $0.id == id } }
    mutating func cancelAll() { orders.removeAll() }

    // MARK: - Filling

    /// Fills queued orders against `bar`: market orders at the open, then stop/limit orders
    /// along the bar's price path.
    mutating func process(bar: KlineData, barIndex: Int, mintick: Double) {
        guard !orders.isEmpty else { return }
        fillMarketOrders(at: bar.openPrice, bar: bar, barIndex: barIndex, mintick: mintick)

        // TradingView's intrabar assumption: the path goes to the nearer extreme first.
        let towardHighFirst = abs(bar.openPrice - bar.highPrice) < abs(bar.openPrice - bar.lowPrice)
        let points =
            towardHighFirst
            ? [bar.openPrice, bar.highPrice, bar.lowPrice, bar.closePrice]
            : [bar.openPrice, bar.lowPrice, bar.highPrice, bar.closePrice]
        for index in 0..<3 {
            var start = points[index]
            let end = points[index + 1]
            var passes = 0
            // Each fill can arm new orders (an entry arms its exits) and moves the start of
            // the remaining segment to the fill price, so the path is never replayed.
            while passes < 64, let hit = nextTrigger(from: start, to: end, mintick: mintick) {
                passes += 1
                apply(hit, bar: bar, barIndex: barIndex, mintick: mintick)
                start = hit.price
            }
        }
        cullExits()
    }

    /// `process_orders_on_close`: market orders the script just placed fill at the close.
    mutating func fillMarketOrdersAtClose(bar: KlineData, barIndex: Int, mintick: Double) {
        fillMarketOrders(at: bar.closePrice, bar: bar, barIndex: barIndex, mintick: mintick)
    }

    private mutating func fillMarketOrders(
        at price: Double, bar: KlineData, barIndex: Int, mintick: Double
    ) {
        let market = orders.filter(\.isMarket).sorted { $0.sequence < $1.sequence }
        guard !market.isEmpty else { return }
        orders.removeAll(where: \.isMarket)
        for order in market {
            switch order.kind {
            case .entry, .order:
                executeEntry(
                    order, price: price, slipped: true, bar: bar, barIndex: barIndex, mintick: mintick)
            case .close:
                let matching = openTrades.filter { $0.entryID == order.id }
                let total = matching.reduce(0) { $0 + $1.quantity }
                let quantity = order.quantity ?? order.quantityPercent.map { total * $0 / 100 }
                // Closing a long sells; closing a short buys.
                let isLong = matching.first?.isLong ?? true
                close(
                    matching: { $0.entryID == order.id }, quantity: quantity,
                    at: slipped(price, buying: !isLong, mintick: mintick),
                    exitID: "Close entry(s) order \(order.id)", barIndex: barIndex, time: bar.openTime)
            case .closeAll:
                let isLong = positionSize > 0
                close(
                    matching: { _ in true }, quantity: nil,
                    at: slipped(price, buying: !isLong, mintick: mintick), exitID: order.id,
                    barIndex: barIndex, time: bar.openTime)
            case .exit: break
            }
        }
        cullExits()
    }

    private struct Trigger {
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
    }

    private func nextTrigger(from p0: Double, to p1: Double, mintick: Double) -> Trigger? {
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
        func consider(_ trigger: Trigger) {
            if let current = best,
                (current.distance, current.sequence) <= (trigger.distance, trigger.sequence)
            {
                return
            }
            best = trigger
        }
        for order in orders {
            switch order.kind {
            case .entry, .order:
                if let stop = order.stop, let hit = reach(stop, atOrAbove: order.isLong) {
                    let action: Trigger.Action =
                        order.limit == nil ? .fillEntry(order, viaStop: true) : .activate(order)
                    consider(.init(action: action, price: hit.price, distance: hit.distance, sequence: order.sequence))
                } else if order.stop == nil, let limit = order.limit,
                    let hit = reach(limit, atOrAbove: !order.isLong)
                {
                    consider(
                        .init(
                            action: .fillEntry(order, viaStop: false), price: hit.price,
                            distance: hit.distance, sequence: order.sequence))
                }
            case .exit:
                for (index, trade) in openTrades.enumerated() where matches(order, trade) {
                    let levels = exitLevels(order, trade, mintick: mintick)
                    // A long exits by selling: its stop is below, its limit above.
                    if let stop = levels.stop, let hit = reach(stop, atOrAbove: !trade.isLong) {
                        consider(
                            .init(
                                action: .exit(order, tradeIndex: index, viaStop: true), price: hit.price,
                                distance: hit.distance, sequence: order.sequence))
                    }
                    if let limit = levels.limit, let hit = reach(limit, atOrAbove: trade.isLong) {
                        consider(
                            .init(
                                action: .exit(order, tradeIndex: index, viaStop: false), price: hit.price,
                                distance: hit.distance, sequence: order.sequence))
                    }
                }
            case .close, .closeAll: break
            }
        }
        return best
    }

    private mutating func apply(_ trigger: Trigger, bar: KlineData, barIndex: Int, mintick: Double) {
        switch trigger.action {
        case .activate(let order):
            if let index = orders.firstIndex(where: { $0.sequence == order.sequence }) {
                orders[index].stop = nil
            }
        case .fillEntry(let order, let viaStop):
            orders.removeAll { $0.sequence == order.sequence }
            executeEntry(
                order, price: trigger.price, slipped: viaStop, bar: bar, barIndex: barIndex, mintick: mintick)
        case .exit(let order, let tradeIndex, let viaStop):
            guard openTrades.indices.contains(tradeIndex) else { return }
            let trade = openTrades[tradeIndex]
            var quantity = order.quantity ?? order.quantityPercent.map { trade.quantity * $0 / 100 }
            quantity = min(quantity ?? trade.quantity, trade.quantity)
            let price =
                viaStop
                ? slipped(trigger.price, buying: !trade.isLong, mintick: mintick) : trigger.price
            let target = trade
            close(
                matching: { $0.entryID == target.entryID && $0.entryBar == target.entryBar },
                quantity: quantity, at: price, exitID: order.id, barIndex: barIndex, time: bar.openTime)
            if !openTrades.contains(where: { matches(order, $0) }) {
                orders.removeAll { $0.sequence == order.sequence }
            }
        }
    }

    // MARK: - Trades

    private func matches(_ order: Order, _ trade: PineOpenTrade) -> Bool {
        order.fromEntry == nil || order.fromEntry == trade.entryID
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

    /// Adverse slippage on market and stop fills; limit fills never slip.
    private func slipped(_ price: Double, buying: Bool, mintick: Double) -> Double {
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

    private mutating func executeEntry(
        _ order: Order, price rawPrice: Double, slipped useSlippage: Bool, bar: KlineData, barIndex: Int,
        mintick: Double
    ) {
        let price =
            useSlippage ? slipped(rawPrice, buying: order.isLong, mintick: mintick) : rawPrice
        let position = positionSize
        let opposesPosition = order.isLong ? position < -Self.epsilon : position > Self.epsilon

        switch order.kind {
        case .entry:
            if opposesPosition {
                // Reversal: flatten first, then open the new side at the same price.
                close(
                    matching: { $0.isLong != order.isLong }, quantity: nil, at: price, exitID: order.id,
                    barIndex: barIndex, time: bar.openTime)
            } else {
                let sameSide = openTrades.filter { $0.isLong == order.isLong }.count
                // `pyramiding` counts extra entries; 0 and 1 both allow one.
                guard sameSide < max(1, settings.pyramiding) else { return }
            }
            open(
                order, quantity: order.quantity ?? defaultQuantity(at: price), at: price, bar: bar,
                barIndex: barIndex)
        case .order:
            var remaining = order.quantity ?? defaultQuantity(at: price)
            if opposesPosition {
                let reduce = min(remaining, abs(position))
                close(
                    matching: { $0.isLong != order.isLong }, quantity: reduce, at: price, exitID: order.id,
                    barIndex: barIndex, time: bar.openTime)
                remaining -= reduce
            }
            open(order, quantity: remaining, at: price, bar: bar, barIndex: barIndex)
        default: break
        }
    }

    private mutating func open(_ order: Order, quantity: Double, at price: Double, bar: KlineData, barIndex: Int) {
        guard quantity.isFinite, quantity > Self.epsilon else { return }
        openTrades.append(
            .init(
                entryID: order.id, isLong: order.isLong, quantity: quantity, entryBar: barIndex,
                entryTime: bar.openTime, entryPrice: price,
                entryCommission: commission(quantity: quantity, price: price, chargesOrder: true)))
    }

    /// Closes up to `quantity` (nil = everything) of the open trades `matching` selects,
    /// oldest first. A per-order commission is charged once however many trades close.
    private mutating func close(
        matching selects: (PineOpenTrade) -> Bool, quantity: Double?, at price: Double, exitID: String,
        barIndex: Int, time: Date
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
                    entryTime: trade.entryTime, entryPrice: trade.entryPrice, exitBar: barIndex,
                    exitTime: time, exitPrice: price, commission: entryShare + exitCommission,
                    profit: profit))
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

    /// Drops exit orders whose entry is gone and cannot come back: no open trade and no
    /// pending entry to attach to.
    private mutating func cullExits() {
        let pending = orders.filter { $0.kind == .entry || $0.kind == .order }
        let trades = openTrades
        orders.removeAll { order in
            guard order.kind == .exit else { return false }
            if trades.contains(where: { order.fromEntry == nil || order.fromEntry == $0.entryID }) {
                return false
            }
            return !pending.contains { order.fromEntry == nil || order.fromEntry == $0.id }
        }
    }
}
