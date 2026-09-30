import Foundation

/// Backtest broker for `strategy()` scripts.
///
/// A plain value type: the runtime keeps it inside its per-bar state, so realtime rollback
/// restores orders, positions, and trades together with every other variable.
///
/// Per bar the runtime calls `process(bar:…)` *before* the script runs — orders queued on
/// earlier bars fill against this bar — so `strategy.position_size` already reflects those
/// fills, and orders the script places now wait for the next bar (or fill at the close when
/// `process_orders_on_close` is set). Trigger search lives in `PineBrokerEmulator+Triggers`,
/// trade accounting in `PineBrokerEmulator+Trades`.
struct PineBrokerEmulator: Sendable {
    private(set) var settings: PineStrategySettings
    var openTrades: [PineOpenTrade] = []
    var closedTrades: [PineTrade] = []
    var netProfit = 0.0
    var orders: [Order] = []
    private var equityCurve: [Double] = []
    private var lastClose = 0.0
    private var nextSequence = 0

    static let epsilon = 1e-12
    /// Ceiling on fills within one price segment, so a pathological script cannot loop forever.
    private static let maxFillsPerSegment = 64

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

    /// Commission already paid to open the trades that are still open.
    private var openEntryCommission: Double { openTrades.reduce(0) { $0 + $1.entryCommission } }

    /// Gross unrealized P&L of the open trades at `price`.
    func openProfit(at price: Double) -> Double {
        openTrades.reduce(0) { $0 + ($1.isLong ? 1 : -1) * (price - $1.entryPrice) * $1.quantity }
    }

    /// Initial capital plus realized P&L plus open P&L, less entry commissions already paid.
    func equity(at price: Double) -> Double {
        settings.initialCapital + netProfit + openProfit(at: price) - openEntryCommission
    }

    mutating func recordEquity(close: Double) {
        lastClose = close
        equityCurve.append(equity(at: close))
    }

    func report() -> PineStrategyReport {
        .init(
            settings: settings, trades: closedTrades, openTrades: openTrades, equity: equityCurve,
            openProfit: openProfit(at: lastClose) - openEntryCommission)
    }

    // MARK: - Order book

    /// Placing an order with the id of a pending one of the same family replaces it.
    mutating func place(_ order: Order) {
        var order = order
        order.sequence = nextSequence
        nextSequence += 1
        switch order.kind {
        case .entry, .order: orders.removeAll { $0.kind.isEntryLike && $0.id == order.id }
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
        let context = BarContext(bar: bar, barIndex: barIndex, mintick: mintick)
        fillMarketOrders(at: bar.openPrice, context)
        let path = Self.pricePath(bar)
        for index in 0..<(path.count - 1) {
            var start = path[index]
            let end = path[index + 1]
            var fills = 0
            // Each fill can arm new orders (an entry arms its exits) and moves the start of
            // the remaining segment to the fill price, so the path is never replayed.
            while fills < Self.maxFillsPerSegment,
                let hit = nextTrigger(from: start, to: end, mintick: mintick)
            {
                fills += 1
                apply(hit, context)
                start = hit.price
            }
        }
        cullExits()
    }

    /// TradingView's intrabar assumption: price goes to the nearer extreme first.
    private static func pricePath(_ bar: KlineData) -> [Double] {
        let towardHighFirst = abs(bar.openPrice - bar.highPrice) < abs(bar.openPrice - bar.lowPrice)
        return towardHighFirst
            ? [bar.openPrice, bar.highPrice, bar.lowPrice, bar.closePrice]
            : [bar.openPrice, bar.lowPrice, bar.highPrice, bar.closePrice]
    }

    /// `process_orders_on_close`: market orders the script just placed fill at the close.
    mutating func fillMarketOrdersAtClose(bar: KlineData, barIndex: Int, mintick: Double) {
        fillMarketOrders(
            at: bar.closePrice, BarContext(bar: bar, barIndex: barIndex, mintick: mintick))
    }

    private mutating func fillMarketOrders(at price: Double, _ context: BarContext) {
        let market = orders.filter(\.isMarket).sorted { $0.sequence < $1.sequence }
        guard !market.isEmpty else { return }
        orders.removeAll(where: \.isMarket)
        for order in market {
            switch order.kind {
            case .entry, .order: executeEntry(order, price: price, slipped: true, context)
            case .close: executeClose(order, price: price, context)
            case .closeAll: executeCloseAll(order, price: price, context)
            case .exit: break
            }
        }
        cullExits()
    }

    /// Drops exit orders whose entry is gone and cannot come back: no open trade and no
    /// pending entry to attach to.
    private mutating func cullExits() {
        let pending = orders.filter { $0.kind.isEntryLike }
        let trades = openTrades
        orders.removeAll { order in
            guard order.kind == .exit else { return false }
            if trades.contains(where: { order.protects(entryID: $0.entryID) }) { return false }
            return !pending.contains { order.protects(entryID: $0.id) }
        }
    }
}
