import SwiftUI

/// The selected portfolio's positions as a sortable table, styled like the Transactions tab.
///
/// The sort column follows the portfolio's saved `PortfolioHoldingsSort`; clicking a header that
/// the saved sort has a case for writes it back. Double-click (or Return) opens the asset's sheet.
struct PortfolioHoldingsView: View {
    @ObservedObject var store: PortfolioStore
    @ObservedObject var info: PortfolioAssetInfoViewModel
    let onSelect: (PortfolioHolding) -> Void
    let onRemap: (PortfolioHolding) -> Void

    @State private var search = ""
    @State private var selection: Set<String> = []
    @State private var sortOrder = [KeyPathComparator(\Row.sortValue, order: .reverse)]

    private struct Row: Identifiable {
        let holding: PortfolioHolding
        let subtitle: String

        var id: String { holding.id }
        var ticker: String { holding.asset.displayTicker }
        var sortPrice: Decimal { holding.currentPrice ?? -1 }
        var sortDay: Decimal { holding.dayChangePercent ?? -999_999_999 }
        var quantity: Decimal { holding.quantity }
        var averageCost: Decimal { holding.averageCost }
        var sortValue: Decimal { holding.currentValue ?? -1 }
        var allocation: Decimal { holding.allocation }
        var sortPnL: Decimal { holding.totalPnL ?? -999_999_999 }
    }

    private var formatter: PortfolioValueFormatter {
        PortfolioValueFormatter(currency: store.reportingCurrency, privacy: store.privacyMode)
    }

    // MARK: - Saved sort ↔ table sort

    private static func keyPath(for sort: PortfolioHoldingsSort) -> PartialKeyPath<Row> {
        switch sort {
        case .asset: \Row.ticker
        case .currentValue: \Row.sortValue
        case .allocation: \Row.allocation
        case .dayChange: \Row.sortDay
        // The table has one P&L column; it sorts by amount.
        case .profitLoss, .profitLossPercent: \Row.sortPnL
        }
    }

    private static func sort(for keyPath: PartialKeyPath<Row>) -> PortfolioHoldingsSort? {
        PortfolioHoldingsSort.allCases.first { Self.keyPath(for: $0) == keyPath && $0 != .profitLossPercent }
    }

    private static func order(for sort: PortfolioHoldingsSort) -> [KeyPathComparator<Row>] {
        switch sort {
        case .asset: [KeyPathComparator(\Row.ticker)]
        case .currentValue: [KeyPathComparator(\Row.sortValue, order: .reverse)]
        case .allocation: [KeyPathComparator(\Row.allocation, order: .reverse)]
        case .dayChange: [KeyPathComparator(\Row.sortDay, order: .reverse)]
        case .profitLoss, .profitLossPercent: [KeyPathComparator(\Row.sortPnL, order: .reverse)]
        }
    }

    private func applySavedSort() {
        sortOrder = Self.order(for: store.selectedPortfolio?.sort ?? .currentValue)
    }

    private func persist(_ order: [KeyPathComparator<Row>]) {
        guard let first = order.first, var portfolio = store.selectedPortfolio,
            let sort = Self.sort(for: first.keyPath),
            first.keyPath != Self.keyPath(for: portfolio.sort)
        else { return }
        portfolio.sort = sort
        store.update(portfolio)
    }

    // MARK: - Body

    private func matches(_ row: Row) -> Bool {
        let needle = search.trimmingCharacters(in: .whitespaces)
        guard !needle.isEmpty else { return true }
        let asset = row.holding.asset
        return [asset.displayTicker, asset.symbol, asset.name, row.subtitle]
            .contains { $0.localizedCaseInsensitiveContains(needle) }
    }

    var body: some View {
        let all = store.holdings.map { Row(holding: $0, subtitle: info.subtitle(for: $0.asset)) }
        let rows = all.filter(matches).sorted(using: sortOrder)
        VStack(spacing: 0) {
            toolbar
            if rows.isEmpty {
                emptyState(hasHoldings: !all.isEmpty)
            } else {
                table(rows)
            }
            footer(shown: rows.count, total: all.count)
        }
        .onAppear(perform: applySavedSort)
        .onChange(of: store.snapshot.selectedPortfolioID) { applySavedSort() }
        .onChange(of: sortOrder) { _, order in persist(order) }
    }

    private var toolbar: some View {
        HStack(spacing: 10) {
            PortfolioSearchField(text: $search, prompt: "Search asset or name")
            if !search.isEmpty {
                Button("Clear") { search = "" }
                    .buttonStyle(.link)
            }
            Spacer(minLength: 8)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
    }

    private func table(_ rows: [Row]) -> some View {
        Table(rows, selection: $selection, sortOrder: $sortOrder) {
            TableColumn("Asset", value: \.ticker) { row in
                HStack(spacing: 10) {
                    TickerIconView(
                        symbol: row.ticker, url: info.iconURL(for: row.holding.asset), size: 24)
                    VStack(alignment: .leading, spacing: 1) {
                        Text(row.ticker).fontWeight(.semibold).lineLimit(1)
                        Text(row.subtitle).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                    }
                }
            }
            .width(min: 110, ideal: 125)
            TableColumn("Price", value: \.sortPrice) { row in
                PortfolioNumericCell(text: row.holding.currentPrice.map(formatter.price) ?? "—")
            }
            .width(min: 80, ideal: 100)
            TableColumn("24h %", value: \.sortDay) { row in
                PortfolioNumericCell(
                    text: row.holding.dayChangePercent.map { formatter.signedPercent($0) } ?? "—",
                    color: formatter.tone(row.holding.dayChangePercent) ?? .secondary)
            }
            .width(min: 55, ideal: 65)
            TableColumn("Quantity", value: \.quantity) { row in
                PortfolioNumericCell(text: formatter.quantity(row.holding.quantity))
            }
            .width(min: 65, ideal: 80)
            TableColumn("Avg Cost", value: \.averageCost) { row in
                PortfolioNumericCell(text: formatter.price(row.holding.averageCost))
            }
            .width(min: 80, ideal: 96)
            TableColumn("Value", value: \.sortValue) { row in
                PortfolioNumericCell(text: row.holding.currentValue.map(formatter.money) ?? "—")
            }
            .width(min: 80, ideal: 100)
            TableColumn("Allocation", value: \.allocation) { row in
                PortfolioNumericCell(text: formatter.percent(row.holding.allocation, digits: 1))
            }
            .width(min: 50, ideal: 58)
            // Row actions sit at the end of the last column, as in the Transactions table.
            TableColumn("P&L", value: \.sortPnL) { row in
                HStack(spacing: 4) {
                    PortfolioNumericCell(
                        text: row.holding.totalPnL.map(formatter.signedMoney) ?? "—",
                        color: formatter.tone(row.holding.totalPnL) ?? .primary)
                    Menu {
                        actions(for: row.holding)
                    } label: {
                        Image(systemName: "ellipsis.circle")
                    }
                    .menuStyle(.borderlessButton)
                    .menuIndicator(.hidden)
                    .fixedSize()
                    .accessibilityLabel("Actions for \(row.ticker)")
                }
            }
            .width(min: 120, ideal: 140)
        }
        .portfolioTableChrome()
        .contextMenu(forSelectionType: String.self) { ids in
            if let holding = holding(ids.first, in: rows) { actions(for: holding) }
        } primaryAction: { ids in
            if let holding = holding(ids.first, in: rows) { onSelect(holding) }
        }
    }

    private func holding(_ id: String?, in rows: [Row]) -> PortfolioHolding? {
        guard let id else { return nil }
        return rows.first { $0.id == id }?.holding
    }

    @ViewBuilder private func actions(for holding: PortfolioHolding) -> some View {
        Button("View Transactions") { onSelect(holding) }
        Button("Remap Asset…") { onRemap(holding) }
    }

    // MARK: - Empty and footer

    @ViewBuilder private func emptyState(hasHoldings: Bool) -> some View {
        if hasHoldings {
            ContentUnavailableView.search(text: search)
        } else {
            ContentUnavailableView(
                "No Holdings", systemImage: "briefcase",
                description: Text("Positions appear once a transaction leaves you holding an asset."))
        }
    }

    private func footer(shown: Int, total: Int) -> some View {
        HStack {
            Text(
                shown == total
                    ? "\(total) \(total == 1 ? "asset" : "assets")" : "\(shown) of \(total) assets")
            Spacer()
            Text("Values in \(store.reportingCurrency.rawValue) · Double-click a row for its transactions")
        }
        .font(.caption)
        .foregroundStyle(.secondary)
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
    }
}
