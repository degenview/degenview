import SwiftUI

/// Backtest summary for a `strategy()` script: headline metrics, the equity curve, the
/// closed trades, and the alerts the script raised. Read-only — everything comes from the
/// `PineStrategyReport` the runtime produced. Indicators have no report but can still
/// raise alerts, so the report is optional.
struct PineStrategyReportView: View {
    var report: PineStrategyReport?
    var alerts: [PineAlertEvent] = []

    private typealias Format = PineReportFormat

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            if let report {
                Text("Strategy report").font(.caption.weight(.semibold))
                metricGrid(report)
                PineEquityCurveView(equity: report.equity, initialCapital: report.settings.initialCapital)
                if !report.trades.isEmpty { tradeList(report) }
            }
            if !alerts.isEmpty { alertList }
        }
        .padding(10)
        .background(.quaternary.opacity(0.4), in: RoundedRectangle(cornerRadius: 8))
    }

    // MARK: Metrics

    private func metricGrid(_ report: PineStrategyReport) -> some View {
        let drawdown = report.maxDrawdown
        return LazyVGrid(
            columns: [GridItem(.adaptive(minimum: 130), spacing: 8, alignment: .leading)],
            alignment: .leading, spacing: 8
        ) {
            metric(
                "Net profit",
                "\(Format.signedMoney(report.netProfit))  \(Format.percent(report.returnFraction, signed: true))",
                tint: report.netProfit)
            metric("Closed trades", "\(report.trades.count)")
            metric("Win rate", report.winRate.map { Format.percent($0) } ?? "—")
            metric("Profit factor", report.profitFactor.map(Format.ratio) ?? "—")
            metric(
                "Max drawdown", "\(Format.money(drawdown.amount))  \(Format.percent(drawdown.fraction))")
            metric("Avg trade", report.averageTrade.map(Format.signedMoney) ?? "—")
            metric("Open P&L", Format.signedMoney(report.openProfit), tint: report.openProfit)
            metric("Final equity", Format.money(report.finalEquity))
        }
    }

    private func metric(_ title: String, _ value: String, tint: Double = 0) -> some View {
        VStack(alignment: .leading, spacing: 1) {
            Text(title).font(.caption2).foregroundStyle(.secondary)
            Text(value).font(.caption.monospacedDigit()).foregroundStyle(PineReportColors.tint(tint))
        }
    }

    // MARK: Lists

    private func tradeList(_ report: PineStrategyReport) -> some View {
        PineReportListSection(title: "Trades", items: report.trades, maxHeight: 150) { trade in
            HStack(spacing: 8) {
                Text("#\(trade.id)").frame(width: 34, alignment: .leading)
                Text(trade.isLong ? "Long" : "Short").frame(width: 38, alignment: .leading)
                Text("\(Format.price(trade.entryPrice)) → \(Format.price(trade.exitPrice))")
                Spacer()
                Text(Format.signedMoney(trade.profit)).foregroundStyle(PineReportColors.tint(trade.profit))
            }
            .font(.caption2.monospacedDigit())
            .help("\(trade.entryID) → \(trade.exitID) · qty \(Format.price(trade.quantity))")
        }
    }

    private var alertList: some View {
        PineReportListSection(title: "Alerts", items: alerts, maxHeight: 120) { alert in
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Text(alert.time.formatted(date: .abbreviated, time: .shortened))
                    .foregroundStyle(.secondary)
                Text(alert.message)
                Spacer(minLength: 0)
            }
            .font(.caption2)
            .textSelection(.enabled)
        }
    }
}

#Preview {
    PineStrategyReportView(
        report: PineStrategyReport(
            settings: PineStrategySettings(initialCapital: 10_000), trades: [], openTrades: [],
            equity: [10_000, 10_200, 10_100, 10_500], openProfit: 0)
    )
    .frame(width: 420)
    .padding()
}
