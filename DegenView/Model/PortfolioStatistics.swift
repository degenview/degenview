import Foundation

/// Figures the Statistics and Overview tabs show, derived from the reporting-currency holdings,
/// the value history, and the (converted) transactions. Pure value type: no store, no formatting.
struct PortfolioStatistics: Equatable {
    struct AssetResult: Equatable, Identifiable {
        let asset: PortfolioAsset
        let totalPnL: Decimal
        let pnlPercent: Decimal?
        var id: String { asset.key }
    }

    struct Extreme: Equatable {
        let value: Decimal
        let date: Date
    }

    let totalValue: Decimal
    let costBasis: Decimal
    let realizedPnL: Decimal
    let unrealizedPnL: Decimal
    /// Unrealized P&L over the cost basis of the assets that have a price.
    let unrealizedPercent: Decimal?
    /// Value now against value 24 hours ago, at today's quantities. Nil when nothing has both prices.
    let dayChange: PortfolioValueChange?
    let best: AssetResult?
    let worst: AssetResult?
    /// Every priced asset, best total P&L first.
    let assetResults: [AssetResult]
    let winners: Int
    let losers: Int
    let high: Extreme?
    let low: Extreme?
    let assetCount: Int
    let transactionCount: Int
    let buyCount: Int
    let sellCount: Int
    let firstTransaction: Date?
    /// Fees paid, in the reporting currency (pass converted transactions).
    let feesPaid: Decimal

    var totalPnL: Decimal { realizedPnL + unrealizedPnL }

    init(
        holdings: [PortfolioHolding], history: [PortfolioSnapshot] = [],
        transactions: [PortfolioTransaction] = []
    ) {
        totalValue = holdings.compactMap(\.currentValue).reduce(0, +)
        costBasis = holdings.map(\.costBasis).reduce(0, +)
        realizedPnL = holdings.map(\.realizedPnL).reduce(0, +)
        unrealizedPnL = holdings.compactMap(\.unrealizedPnL).reduce(0, +)

        let pricedCost = holdings.filter { $0.unrealizedPnL != nil }.map(\.costBasis).reduce(0, +)
        unrealizedPercent = pricedCost == 0 ? nil : unrealizedPnL / pricedCost

        var previous: Decimal = 0
        var current: Decimal = 0
        for holding in holdings {
            guard let price = holding.currentPrice, let before = holding.previousDayPrice else { continue }
            previous += holding.quantity * before
            current += holding.quantity * price
        }
        dayChange = previous > 0 ? PortfolioValueChange(from: previous, to: current) : nil

        let results = holdings.compactMap { holding -> AssetResult? in
            guard let pnl = holding.totalPnL else { return nil }
            return AssetResult(asset: holding.asset, totalPnL: pnl, pnlPercent: holding.pnlPercent)
        }
        assetResults = results.sorted { $0.totalPnL > $1.totalPnL }
        let ranked = results.filter { $0.pnlPercent != nil }
        best = ranked.max { ($0.pnlPercent ?? 0) < ($1.pnlPercent ?? 0) }
        worst = ranked.min { ($0.pnlPercent ?? 0) < ($1.pnlPercent ?? 0) }
        winners = results.filter { $0.totalPnL > 0 }.count
        losers = results.filter { $0.totalPnL < 0 }.count

        high = history.max { $0.value < $1.value }.map { Extreme(value: $0.value, date: $0.timestamp) }
        low = history.min { $0.value < $1.value }.map { Extreme(value: $0.value, date: $0.timestamp) }

        assetCount = holdings.count
        transactionCount = transactions.count
        buyCount = transactions.filter { $0.type == .buy }.count
        sellCount = transactions.filter { $0.type == .sell }.count
        firstTransaction = transactions.map(\.timestamp).min()
        feesPaid = transactions.map(\.fee).reduce(0, +)
    }
}

/// Profit or loss over a stretch of history, separate from money moved in or out.
///
/// A value change alone would call a deposit a gain, so this follows `value − netContributions`:
/// the P&L at the end minus the P&L at the start. The percentage is against the capital in play —
/// the starting value plus anything contributed since.
struct PortfolioPeriodChange: Equatable {
    let profit: Decimal
    let percentage: Decimal?

    var direction: PortfolioValueChange.Direction {
        if profit > 0 { return .up }
        if profit < 0 { return .down }
        return .unchanged
    }

    /// - Parameters:
    ///   - snapshots: the history inside the range, oldest first.
    ///   - currentValue: today's value, which stands in for the end of the range.
    init?(snapshots: [PortfolioSnapshot], currentValue: Decimal) {
        guard let first = snapshots.first, let last = snapshots.last else { return nil }
        let startProfit = first.value - first.netContributions
        let endProfit = currentValue - last.netContributions
        profit = endProfit - startProfit
        let deposited = max(0, last.netContributions - first.netContributions)
        let capital = first.value + deposited
        percentage = capital > 0 ? profit / capital : nil
    }
}
