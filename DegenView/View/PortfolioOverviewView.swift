import SwiftUI

/// The portfolio's front page: what it is worth, how it has moved, and where the money sits.
struct PortfolioOverviewView: View {
    @ObservedObject var store: PortfolioStore
    @ObservedObject var info: PortfolioAssetInfoViewModel
    let onSelect: (PortfolioHolding) -> Void
    let onShowHoldings: () -> Void
    @State private var range: PortfolioHistoryRange = .oneMonth

    private var formatter: PortfolioValueFormatter {
        PortfolioValueFormatter(currency: store.reportingCurrency, privacy: store.privacyMode)
    }
    private var history: [PortfolioSnapshot] {
        let id = store.snapshot.selectedPortfolioID
        if range == .oneDay {
            let intraday = store.intradayHistory(for: id)
            if !intraday.isEmpty { return intraday }
        }
        return store.history(for: id)
    }
    private var isLoadingChart: Bool { store.isLoadingInitialValues || store.isChangingReportingCurrency }

    var body: some View {
        let stats = PortfolioStatistics(holdings: store.holdings)
        let periodChange = PortfolioPeriodChange(snapshots: range.filter(history), currentValue: store.totalValue)
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                hero(stats: stats, periodChange: periodChange)
                summaryCards(stats)
                HStack(spacing: 12) {
                    PortfolioPerformerCard(
                        title: "Best Performer", result: stats.best, formatter: formatter, info: info)
                    PortfolioPerformerCard(
                        title: "Worst Performer", result: stats.worst, formatter: formatter, info: info)
                }
                historyCard
                HStack(alignment: .top, spacing: 16) {
                    PortfolioCard(title: "Allocation") {
                        PortfolioAllocationChart(holdings: store.holdings, formatter: formatter)
                            .frame(height: Self.panelContentHeight)
                    }
                    .frame(maxHeight: .infinity, alignment: .top)
                    topHoldings
                        .frame(maxHeight: .infinity, alignment: .top)
                }
                .fixedSize(horizontal: false, vertical: true)
            }
            .padding(20)
        }
        .task(id: range) {
            guard range == .oneDay else { return }
            while !Task.isCancelled {
                await store.refreshIntraday(forPortfolioID: store.snapshot.selectedPortfolioID)
                try? await Task.sleep(for: .seconds(300))
            }
        }
    }

    // MARK: - Hero

    private func hero(stats: PortfolioStatistics, periodChange: PortfolioPeriodChange?) -> some View {
        HStack(alignment: .bottom, spacing: 24) {
            VStack(alignment: .leading, spacing: 6) {
                HStack(spacing: 6) {
                    Text(store.selectedPortfolio?.name ?? "All Portfolios").font(.title3.weight(.semibold))
                    Text("· \(store.holdings.count) \(store.holdings.count == 1 ? "asset" : "assets")")
                        .font(.title3)
                        .foregroundStyle(.secondary)
                }
                balance
                HStack(spacing: 6) {
                    Text(store.marketValueCaption)
                    if store.isUpdating && !store.isLoadingInitialValues {
                        ProgressView().controlSize(.mini)
                        Text("Updating…")
                    }
                }
                .font(.caption)
                .foregroundStyle(.secondary)
            }
            Spacer(minLength: 16)
            HStack(spacing: 28) {
                changeFigure(
                    "24h", amount: stats.dayChange?.amount, percentage: stats.dayChange?.percentage)
                changeFigure(
                    "P&L · \(range.periodName)", amount: periodChange?.profit,
                    percentage: periodChange?.percentage)
            }
            .opacity(store.isLoadingInitialValues ? 0 : 1)
        }
        .padding(.horizontal, 4)
    }

    private var balance: some View {
        ZStack(alignment: .leading) {
            Text(formatter.money(store.totalValue))
                .font(.system(size: 40, weight: .semibold, design: .rounded))
                .monospacedDigit()
                .opacity(store.isLoadingInitialValues ? 0 : 1)
                .accessibilityHidden(store.isLoadingInitialValues)
                .accessibilityLabel(
                    store.privacyMode
                        ? "Portfolio balance hidden" : "Portfolio balance, \(formatter.money(store.totalValue))")
            HStack(spacing: 8) {
                ProgressView().controlSize(.small)
                Text("Loading portfolio value…").font(.callout).foregroundStyle(.secondary)
            }
            .opacity(store.isLoadingInitialValues ? 1 : 0)
            .accessibilityElement(children: .combine)
            .accessibilityLabel("Loading portfolio value")
            .accessibilityHidden(!store.isLoadingInitialValues)
        }
        .animation(.easeOut(duration: 0.25), value: store.isLoadingInitialValues)
    }

    /// A signed amount and percentage under a period label: "+$1,204.10  ▲ 3.21%".
    private func changeFigure(_ label: String, amount: Decimal?, percentage: Decimal?) -> some View {
        let tone = formatter.tone(amount)
        return VStack(alignment: .trailing, spacing: 3) {
            Text(label).font(.caption).foregroundStyle(.secondary)
            if let amount {
                Text(formatter.signedMoney(amount))
                    .font(.title3.weight(.semibold))
                    .monospacedDigit()
                    .foregroundStyle(tone ?? .primary)
                if let percentage {
                    HStack(spacing: 3) {
                        if !store.privacyMode, amount != 0 {
                            Image(systemName: amount > 0 ? "arrowtriangle.up.fill" : "arrowtriangle.down.fill")
                                .font(.system(size: 8))
                        }
                        Text(formatter.percent(abs(percentage))).monospacedDigit()
                    }
                    .font(.caption.weight(.medium))
                    .foregroundStyle(tone ?? .secondary)
                }
            } else {
                Text("—").font(.title3.weight(.semibold)).foregroundStyle(.secondary)
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(label) change")
        .accessibilityValue(
            store.privacyMode
                ? "hidden"
                : amount.map { "\(formatter.signedMoney($0)), \(percentage.map { formatter.signedPercent($0) } ?? "")" }
                    ?? "unavailable")
    }

    // MARK: - Summary

    private func summaryCards(_ stats: PortfolioStatistics) -> some View {
        LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 12), count: 4), spacing: 12) {
            PortfolioStatCard(
                title: "Cost Basis", value: formatter.money(stats.costBasis), isMasked: store.privacyMode,
                systemImage: "banknote")
            PortfolioStatCard(
                title: "All-time P&L", value: formatter.signedMoney(stats.totalPnL),
                tone: formatter.tone(stats.totalPnL), isMasked: store.privacyMode,
                systemImage: "chart.line.uptrend.xyaxis")
            PortfolioStatCard(
                title: "Unrealized P&L", value: formatter.signedMoney(stats.unrealizedPnL),
                tone: formatter.tone(stats.unrealizedPnL),
                caption: stats.unrealizedPercent.map { formatter.signedPercent($0) }, isMasked: store.privacyMode,
                systemImage: "hourglass")
            PortfolioStatCard(
                title: "Realized P&L", value: formatter.signedMoney(stats.realizedPnL),
                tone: formatter.tone(stats.realizedPnL), isMasked: store.privacyMode,
                systemImage: "checkmark.seal")
        }
    }

    // MARK: - History

    private var historyCard: some View {
        PortfolioCard(title: "Portfolio Value") {
            Picker("History range", selection: $range) {
                ForEach(PortfolioHistoryRange.allCases) { Text($0.rawValue).tag($0) }
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .controlSize(.small)
            .fixedSize()
        } content: {
            PortfolioHistoryChart(
                snapshots: history, currentValue: store.totalValue, currency: store.reportingCurrency,
                range: range, privacy: store.privacyMode,
                isLoading: isLoadingChart || (store.isLoadingHistory && history.isEmpty),
                loadingMessage: isLoadingChart ? "Loading market data…" : "Building history…"
            )
            .frame(height: 250)
        }
    }

    // MARK: - Top holdings

    private static let topCount = 6
    /// Both lower panels size their content to this, so they always end level: six rows of
    /// Top Holdings (about 44 pt each) fill it exactly.
    private static let panelContentHeight: CGFloat = 270

    private var topHoldings: some View {
        let ranked = store.holdings.sorted { ($0.currentValue ?? 0) > ($1.currentValue ?? 0) }
        return PortfolioCard(title: "Top Holdings") {
            if store.holdings.count > Self.topCount {
                Button("View All", action: onShowHoldings)
                    .buttonStyle(.link)
                    .font(.subheadline)
            }
        } content: {
            VStack(spacing: 2) {
                ForEach(ranked.prefix(Self.topCount)) { holding in
                    TopHoldingRow(holding: holding, info: info, formatter: formatter) { onSelect(holding) }
                }
                Spacer(minLength: 0)
            }
            .frame(height: Self.panelContentHeight, alignment: .top)
        }
    }
}

private struct TopHoldingRow: View {
    let holding: PortfolioHolding
    @ObservedObject var info: PortfolioAssetInfoViewModel
    let formatter: PortfolioValueFormatter
    let action: () -> Void
    @State private var isHovered = false

    var body: some View {
        Button(action: action) {
            HStack(spacing: 10) {
                TickerIconView(symbol: holding.asset.displayTicker, url: info.iconURL(for: holding.asset), size: 26)
                VStack(alignment: .leading, spacing: 1) {
                    Text(holding.asset.displayTicker).font(.subheadline.weight(.semibold))
                    Text(info.subtitle(for: holding.asset))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
                Spacer(minLength: 8)
                VStack(alignment: .trailing, spacing: 3) {
                    Text(holding.currentValue.map(formatter.money) ?? "—")
                        .font(.subheadline)
                        .monospacedDigit()
                    HStack(spacing: 6) {
                        allocationBar
                        Text(formatter.percent(holding.allocation, digits: 1))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .monospacedDigit()
                            .frame(width: 44, alignment: .trailing)
                    }
                }
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 5)
            .background(
                isHovered ? Color.primary.opacity(0.07) : .clear,
                in: RoundedRectangle(cornerRadius: 8, style: .continuous)
            )
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { isHovered = $0 }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(holding.asset.displayTicker), \(info.subtitle(for: holding.asset))")
        .accessibilityValue(
            formatter.privacy
                ? "value hidden, \(formatter.percent(holding.allocation)) of portfolio"
                : "\(holding.currentValue.map(formatter.money) ?? "unavailable"), "
                    + "\(formatter.percent(holding.allocation)) of portfolio"
        )
        .accessibilityHint("Shows this asset's transactions")
    }

    private var allocationBar: some View {
        Capsule()
            .fill(.quaternary)
            .frame(width: 64, height: 4)
            .overlay(alignment: .leading) {
                Capsule()
                    .fill(Color.accentColor)
                    .frame(width: 64 * CGFloat(min(1, max(0, holding.allocation.doubleValue))), height: 4)
            }
    }
}
