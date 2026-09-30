import Foundation

/// Backtest result of a `strategy()` script.
struct PineStrategyReport: Sendable {
    var settings: PineStrategySettings
    var trades: [PineTrade]
    var openTrades: [PineOpenTrade]
    /// Mark-to-market equity after each bar, parallel to the script's bars.
    var equity: [Double]
    /// Unrealized P&L of the open position at the last bar, net of entry commissions.
    var openProfit: Double

    var netProfit: Double { trades.reduce(0) { $0 + $1.profit } }
    var grossProfit: Double { trades.filter { $0.profit > 0 }.reduce(0) { $0 + $1.profit } }
    var grossLoss: Double { -trades.filter { $0.profit < 0 }.reduce(0) { $0 + $1.profit } }
    var winCount: Int { trades.filter { $0.profit > 0 }.count }
    var winRate: Double? { trades.isEmpty ? nil : Double(winCount) / Double(trades.count) }
    var profitFactor: Double? { grossLoss > 0 ? grossProfit / grossLoss : nil }
    var averageTrade: Double? { trades.isEmpty ? nil : netProfit / Double(trades.count) }
    var finalEquity: Double { equity.last ?? settings.initialCapital }
    var returnFraction: Double {
        settings.initialCapital > 0 ? (finalEquity - settings.initialCapital) / settings.initialCapital : 0
    }

    /// Largest peak-to-trough fall of the equity curve, in currency and as a fraction of the peak.
    var maxDrawdown: (amount: Double, fraction: Double) {
        var peak = settings.initialCapital
        var worst = (amount: 0.0, fraction: 0.0)
        for value in equity {
            peak = max(peak, value)
            let drop = peak - value
            if drop > worst.amount { worst = (drop, peak > 0 ? drop / peak : 0) }
        }
        return worst
    }
}
