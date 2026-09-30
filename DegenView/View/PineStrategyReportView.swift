import SwiftUI

// MARK: - PineStrategyReportView

/// Backtest summary for a `strategy()` script: headline metrics, the equity curve, the
/// closed trades, and the alerts the script raised. Read-only — everything comes from the
/// `PineStrategyReport` the runtime produced. Indicators have no report but can still
/// raise alerts, so the report is optional.
struct PineStrategyReportView: View {
    var report: PineStrategyReport?
    var alerts: [PineAlertEvent] = []

    private static let listLimit = 100

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            if let report {
                Text("Strategy report").font(.caption.weight(.semibold))
                metricGrid(report)
                equityCurve(report)
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
                "Net profit", "\(signedMoney(report.netProfit))  \(percent(report.returnFraction, signed: true))",
                tint: report.netProfit)
            metric("Closed trades", "\(report.trades.count)")
            metric("Win rate", report.winRate.map { percent($0) } ?? "—")
            metric(
                "Profit factor",
                report.profitFactor.map { $0.formatted(.number.precision(.fractionLength(2))) } ?? "—")
            metric("Max drawdown", "\(money(drawdown.amount))  \(percent(drawdown.fraction))")
            metric("Avg trade", report.averageTrade.map(signedMoney) ?? "—")
            metric("Open P&L", signedMoney(report.openProfit), tint: report.openProfit)
            metric("Final equity", money(report.finalEquity))
        }
    }

    private func metric(_ title: String, _ value: String, tint: Double = 0) -> some View {
        VStack(alignment: .leading, spacing: 1) {
            Text(title).font(.caption2).foregroundStyle(.secondary)
            Text(value).font(.caption.monospacedDigit())
                .foregroundStyle(tint > 0 ? Color.green : tint < 0 ? Color.red : Color.primary)
        }
    }

    // MARK: Equity curve

    private func equityCurve(_ report: PineStrategyReport) -> some View {
        Canvas { context, size in
            let values = Self.downsample(report.equity, to: 400)
            guard values.count > 1 else { return }
            let capital = report.settings.initialCapital
            let low = min(values.min() ?? capital, capital)
            let high = max(values.max() ?? capital, capital)
            let span = max(high - low, 1e-9)
            func y(_ value: Double) -> CGFloat { size.height - CGFloat((value - low) / span) * (size.height - 4) - 2 }

            var baseline = Path()
            baseline.move(to: CGPoint(x: 0, y: y(capital)))
            baseline.addLine(to: CGPoint(x: size.width, y: y(capital)))
            context.stroke(
                baseline, with: .color(.secondary.opacity(0.5)),
                style: StrokeStyle(lineWidth: 1, dash: [3, 3]))

            var line = Path()
            for (i, value) in values.enumerated() {
                let point = CGPoint(x: size.width * CGFloat(i) / CGFloat(values.count - 1), y: y(value))
                if i == 0 { line.move(to: point) } else { line.addLine(to: point) }
            }
            context.stroke(line, with: .color((values.last ?? capital) >= capital ? .green : .red), lineWidth: 1.5)
        }
        .frame(height: 64)
        .overlay(alignment: .topLeading) {
            Text("Equity").font(.caption2).foregroundStyle(.secondary)
        }
    }

    /// Keeps the first and last points and evenly spaced ones between.
    static func downsample(_ values: [Double], to limit: Int) -> [Double] {
        guard values.count > limit, limit > 1 else { return values }
        return (0..<limit).map { values[$0 * (values.count - 1) / (limit - 1)] }
    }

    // MARK: Lists

    private func tradeList(_ report: PineStrategyReport) -> some View {
        DisclosureGroup("Trades (\(report.trades.count))") {
            ScrollView {
                VStack(alignment: .leading, spacing: 2) {
                    ForEach(report.trades.suffix(Self.listLimit).reversed()) { trade in
                        HStack(spacing: 8) {
                            Text("#\(trade.id)").frame(width: 34, alignment: .leading)
                            Text(trade.isLong ? "Long" : "Short").frame(width: 38, alignment: .leading)
                            Text("\(price(trade.entryPrice)) → \(price(trade.exitPrice))")
                            Spacer()
                            Text(signedMoney(trade.profit))
                                .foregroundStyle(trade.profit >= 0 ? Color.green : Color.red)
                        }
                        .font(.caption2.monospacedDigit())
                        .help("\(trade.entryID) → \(trade.exitID) · qty \(price(trade.quantity))")
                    }
                }
            }
            .frame(maxHeight: 150)
        }
        .font(.caption)
    }

    private var alertList: some View {
        DisclosureGroup("Alerts (\(alerts.count))") {
            ScrollView {
                VStack(alignment: .leading, spacing: 2) {
                    ForEach(alerts.suffix(Self.listLimit).reversed()) { alert in
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
            .frame(maxHeight: 120)
        }
        .font(.caption)
    }

    // MARK: Formatting

    private func money(_ value: Double) -> String {
        value.formatted(.number.precision(.fractionLength(2)).grouping(.automatic))
    }

    private func signedMoney(_ value: Double) -> String {
        (value >= 0 ? "+" : "−") + money(abs(value))
    }

    private func percent(_ fraction: Double, signed: Bool = false) -> String {
        let text = (abs(fraction) * 100).formatted(.number.precision(.fractionLength(2))) + "%"
        guard signed else { return text }
        return (fraction >= 0 ? "+" : "−") + text
    }

    private func price(_ value: Double) -> String {
        value.formatted(.number.precision(.significantDigits(1...7)))
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
