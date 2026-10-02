import SwiftUI

/// How the portfolio has performed and what it is made of, in sections: performance, value
/// range, the winners and losers, and the record of activity behind it.
struct PortfolioStatisticsView: View {
    @ObservedObject var store: PortfolioStore
    @ObservedObject var info: PortfolioAssetInfoViewModel

    private var formatter: PortfolioValueFormatter {
        PortfolioValueFormatter(currency: store.reportingCurrency, privacy: store.privacyMode)
    }
    private let columns = Array(repeating: GridItem(.flexible(), spacing: 12), count: 4)

    var body: some View {
        let stats = PortfolioStatistics(
            holdings: store.holdings, history: store.history(for: store.snapshot.selectedPortfolioID),
            transactions: store.transactions)
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                section("Performance") { performance(stats) }
                section("Value") { value(stats) }
                section("Activity") { activity(stats) }
                section("P&L by Asset", subtitle: "Total, in \(store.reportingCurrency.rawValue)") {
                    PortfolioCard {
                        PnLBars(results: stats.assetResults, formatter: formatter, info: info)
                    }
                }
            }
            .padding(20)
        }
    }

    private func section<Content: View>(
        _ title: String, subtitle: String? = nil, @ViewBuilder content: () -> Content
    ) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Text(title).font(.title3.weight(.semibold))
                if let subtitle {
                    Text(subtitle).font(.subheadline).foregroundStyle(.secondary)
                }
            }
            content()
        }
    }

    // MARK: - Performance and value

    private func performance(_ stats: PortfolioStatistics) -> some View {
        LazyVGrid(columns: columns, spacing: 12) {
            PortfolioStatCard(
                title: "Total P&L", value: formatter.signedMoney(stats.totalPnL),
                tone: formatter.tone(stats.totalPnL), isMasked: store.privacyMode)
            PortfolioStatCard(
                title: "Unrealized P&L", value: formatter.signedMoney(stats.unrealizedPnL),
                tone: formatter.tone(stats.unrealizedPnL),
                caption: stats.unrealizedPercent.map { formatter.signedPercent($0) }, isMasked: store.privacyMode)
            PortfolioStatCard(
                title: "Realized P&L", value: formatter.signedMoney(stats.realizedPnL),
                tone: formatter.tone(stats.realizedPnL), isMasked: store.privacyMode)
            PortfolioStatCard(
                title: "Last 24 Hours",
                value: stats.dayChange.map { formatter.signedMoney($0.amount) } ?? "—",
                tone: formatter.tone(stats.dayChange?.amount),
                caption: stats.dayChange?.percentage.map { formatter.signedPercent($0) },
                isPlaceholder: stats.dayChange == nil, isMasked: store.privacyMode)
        }
    }

    private func value(_ stats: PortfolioStatistics) -> some View {
        LazyVGrid(columns: columns, spacing: 12) {
            PortfolioStatCard(
                title: "Current Value", value: formatter.money(stats.totalValue), isMasked: store.privacyMode)
            PortfolioStatCard(
                title: "Cost Basis", value: formatter.money(stats.costBasis), isMasked: store.privacyMode)
            extreme("Portfolio High", stats.high)
            extreme("Portfolio Low", stats.low)
        }
    }

    private func extreme(_ title: String, _ extreme: PortfolioStatistics.Extreme?) -> some View {
        PortfolioStatCard(
            title: title, value: extreme.map { formatter.money($0.value) } ?? "—",
            caption: extreme.map { $0.date.formatted(date: .abbreviated, time: .omitted) },
            captionTone: .secondary, isPlaceholder: extreme == nil, isMasked: store.privacyMode)
    }

    // MARK: - Activity

    private func activity(_ stats: PortfolioStatistics) -> some View {
        LazyVGrid(columns: columns, spacing: 12) {
            PortfolioStatCard(
                title: "Transactions", value: "\(stats.transactionCount)",
                caption: "\(stats.buyCount) \(stats.buyCount == 1 ? "buy" : "buys") · "
                    + "\(stats.sellCount) \(stats.sellCount == 1 ? "sell" : "sells")", captionTone: .secondary)
            PortfolioStatCard(
                title: "Assets", value: "\(stats.assetCount)",
                caption: "\(stats.winners) in profit · \(stats.losers) at a loss", captionTone: .secondary)
            PortfolioStatCard(
                title: "Fees Paid", value: formatter.money(stats.feesPaid), isMasked: store.privacyMode)
            PortfolioStatCard(
                title: "Tracking Since",
                value: stats.firstTransaction.map { $0.formatted(date: .abbreviated, time: .omitted) } ?? "—",
                caption: stats.firstTransaction.map {
                    $0.formatted(.relative(presentation: .numeric, unitsStyle: .wide))
                },
                captionTone: .secondary, isPlaceholder: stats.firstTransaction == nil)
        }
    }
}

/// Horizontal bars of each asset's total P&L around a zero line: gains grow right, losses left.
private struct PnLBars: View {
    let results: [PortfolioStatistics.AssetResult]
    let formatter: PortfolioValueFormatter
    @ObservedObject var info: PortfolioAssetInfoViewModel

    private static let rowHeight: CGFloat = 26

    var body: some View {
        if results.isEmpty {
            Text("Priced holdings appear here.")
                .font(.callout)
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, minHeight: 60)
        } else {
            let negativeReach = results.map { max(0, -$0.totalPnL) }.max() ?? 0
            let positiveReach = results.map { max(0, $0.totalPnL) }.max() ?? 0
            let span = max(negativeReach + positiveReach, Decimal(string: "0.00000001")!)
            VStack(spacing: 4) {
                ForEach(results) { result in
                    HStack(spacing: 10) {
                        HStack(spacing: 8) {
                            TickerIconView(
                                symbol: result.asset.displayTicker, url: info.iconURL(for: result.asset), size: 20)
                            Text(result.asset.displayTicker)
                                .font(.subheadline.weight(.medium))
                                .lineLimit(1)
                        }
                        .frame(width: 100, alignment: .leading)
                        GeometryReader { geometry in
                            bar(
                                for: result.totalPnL, span: span, negativeReach: negativeReach,
                                width: geometry.size.width)
                        }
                        .frame(height: Self.rowHeight - 8)
                        Text(formatter.signedMoney(result.totalPnL))
                            .font(.subheadline)
                            .monospacedDigit()
                            .foregroundStyle(formatter.tone(result.totalPnL) ?? .primary)
                            .frame(width: 120, alignment: .trailing)
                        Text(result.pnlPercent.map { formatter.signedPercent($0) } ?? "—")
                            .font(.subheadline)
                            .monospacedDigit()
                            .foregroundStyle(formatter.tone(result.pnlPercent) ?? .secondary)
                            .frame(width: 80, alignment: .trailing)
                    }
                    .frame(height: Self.rowHeight)
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel(result.asset.displayTicker)
                    .accessibilityValue(
                        [
                            formatter.privacy ? "amount hidden" : formatter.signedMoney(result.totalPnL),
                            result.pnlPercent.map { formatter.signedPercent($0) },
                        ].compactMap { $0 }.joined(separator: ", "))
                }
            }
        }
    }

    /// Neutral while values are hidden: red and green would give each sign away.
    private func barColor(for value: Decimal) -> Color {
        guard let tone = formatter.tone(value) else { return .secondary.opacity(0.5) }
        return tone.opacity(0.85)
    }

    private func bar(for value: Decimal, span: Decimal, negativeReach: Decimal, width: CGFloat) -> some View {
        let zero = width * CGFloat((negativeReach / span).doubleValue)
        let length = max(2, width * CGFloat((abs(value) / span).doubleValue))
        let isGain = value >= 0
        return ZStack(alignment: .topLeading) {
            Rectangle().fill(.separator).frame(width: 1).offset(x: zero)
            RoundedRectangle(cornerRadius: 3, style: .continuous)
                .fill(barColor(for: value))
                .frame(width: length)
                .offset(x: isGain ? zero : zero - length)
        }
    }
}
