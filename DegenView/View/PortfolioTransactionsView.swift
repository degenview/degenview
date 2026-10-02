import SwiftUI

/// Every transaction in the selected portfolio, searchable, filterable and sortable.
///
/// Rows come from the unprojected ledger (so Edit opens exactly what was entered); the price and
/// fee shown are the reporting-currency projection of the same transaction.
struct PortfolioTransactionsView: View {
    @ObservedObject var store: PortfolioStore
    @ObservedObject var info: PortfolioAssetInfoViewModel
    let onAdd: () -> Void
    let onEdit: (PortfolioTransaction) -> Void
    let onDuplicate: (PortfolioTransaction) -> Void
    let onDelete: (PortfolioTransaction) -> Void

    @State private var search = ""
    @State private var typeFilter: PortfolioTransactionType?
    @State private var assetFilter: String?
    @State private var selection: Set<UUID> = []
    @State private var sortOrder = [KeyPathComparator(\Row.date, order: .reverse)]

    private struct Row: Identifiable {
        let transaction: PortfolioTransaction
        let price: Decimal?
        let fee: Decimal

        var id: UUID { transaction.id }
        var date: Date { transaction.timestamp }
        var ticker: String { transaction.asset.displayTicker }
        var typeName: String { transaction.type.rawValue }
        var quantity: Decimal { transaction.quantity }
        var total: Decimal? { price.map { $0 * transaction.quantity } }
        var sortPrice: Decimal { price ?? 0 }
        var sortTotal: Decimal { total ?? 0 }
        var sourceName: String { transaction.source.rawValue }
        var notes: String { transaction.notes }
    }

    private var formatter: PortfolioValueFormatter {
        PortfolioValueFormatter(currency: store.reportingCurrency, privacy: store.privacyMode)
    }
    private var currencyCode: String { store.reportingCurrency.rawValue }

    private var allRows: [Row] {
        let converted = Dictionary(store.transactions.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
        return store.ledgerTransactions.map { tx in
            let projected = converted[tx.id]
            return Row(transaction: tx, price: projected?.price, fee: projected?.fee ?? 0)
        }
    }

    private func matches(_ row: Row) -> Bool {
        let tx = row.transaction
        if let typeFilter, tx.type != typeFilter { return false }
        if let assetFilter, tx.asset.key != assetFilter { return false }
        let needle = search.trimmingCharacters(in: .whitespaces)
        guard !needle.isEmpty else { return true }
        return [
            tx.asset.displayTicker, tx.asset.symbol, tx.asset.name, info.subtitle(for: tx.asset),
            tx.notes, tx.source.rawValue, tx.type.rawValue,
        ].contains { $0.localizedCaseInsensitiveContains(needle) }
    }

    private var assetChoices: [(key: String, ticker: String)] {
        var seen = Set<String>()
        return store.ledgerTransactions.compactMap { tx -> (String, String)? in
            seen.insert(tx.asset.key).inserted ? (tx.asset.key, tx.asset.displayTicker) : nil
        }
        .sorted { $0.1.localizedCaseInsensitiveCompare($1.1) == .orderedAscending }
        .map { (key: $0.0, ticker: $0.1) }
    }

    private var hasFilters: Bool { typeFilter != nil || assetFilter != nil || !search.isEmpty }

    var body: some View {
        let all = allRows
        let rows = all.filter(matches).sorted(using: sortOrder)
        VStack(spacing: 0) {
            toolbar
            if rows.isEmpty {
                emptyState
            } else {
                table(rows)
            }
            footer(shown: rows.count, total: all.count)
        }
    }

    // MARK: - Toolbar

    private var toolbar: some View {
        HStack(spacing: 10) {
            PortfolioSearchField(text: $search, prompt: "Search asset, notes, or source")
            Menu {
                Picker("Type", selection: $typeFilter) {
                    Text("All Types").tag(nil as PortfolioTransactionType?)
                    Divider()
                    ForEach(PortfolioTransactionType.allCases) { Text($0.rawValue).tag(Optional($0)) }
                }
                .pickerStyle(.inline)
                .labelsHidden()
            } label: {
                filterLabel(typeFilter?.rawValue ?? "All Types", active: typeFilter != nil)
            }
            .menuStyle(.button)
            .fixedSize()
            Menu {
                Picker("Asset", selection: $assetFilter) {
                    Text("All Assets").tag(nil as String?)
                    Divider()
                    ForEach(assetChoices, id: \.key) { Text($0.ticker).tag(Optional($0.key)) }
                }
                .pickerStyle(.inline)
                .labelsHidden()
            } label: {
                filterLabel(
                    assetChoices.first { $0.key == assetFilter }?.ticker ?? "All Assets", active: assetFilter != nil)
            }
            .menuStyle(.button)
            .fixedSize()
            if hasFilters {
                Button("Clear", action: clearFilters)
                    .buttonStyle(.link)
                    .help("Clear search and filters")
            }
            Spacer(minLength: 8)
            Button("Add Transaction", systemImage: "plus", action: onAdd)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
    }

    private func filterLabel(_ title: String, active: Bool) -> some View {
        Text(title).foregroundStyle(active ? Color.accentColor : .primary)
    }

    private func clearFilters() {
        search = ""
        typeFilter = nil
        assetFilter = nil
    }

    // MARK: - Table

    private func table(_ rows: [Row]) -> some View {
        Table(rows, selection: $selection, sortOrder: $sortOrder) {
            TableColumn("Date", value: \.date) { row in
                Text(row.date.formatted(date: .abbreviated, time: .shortened)).lineLimit(1)
            }
            .width(min: 120, ideal: 145)
            TableColumn("Asset", value: \.ticker) { row in
                HStack(spacing: 8) {
                    TickerIconView(
                        symbol: row.ticker, url: info.iconURL(for: row.transaction.asset), size: 20)
                    Text(row.ticker).fontWeight(.semibold).lineLimit(1)
                }
            }
            .width(min: 70, ideal: 80)
            TableColumn("Type", value: \.typeName) { row in
                PortfolioTransactionTypeBadge(type: row.transaction.type)
                    .help("Source: \(row.sourceName)")
            }
            .width(min: 85, ideal: 95)
            TableColumn("Quantity", value: \.quantity) { row in
                PortfolioNumericCell(
                    text: formatter.quantity(row.quantity, sign: sign(of: row.transaction.type)),
                    color: quantityColor(row.transaction.type))
            }
            .width(min: 75, ideal: 90)
            TableColumn("Price (\(currencyCode))", value: \.sortPrice) { row in
                PortfolioNumericCell(text: row.price.map(formatter.price) ?? "—")
            }
            .width(min: 80, ideal: 95)
            TableColumn("Total (\(currencyCode))", value: \.sortTotal) { row in
                PortfolioNumericCell(text: row.total.map(formatter.money) ?? "—")
            }
            .width(min: 80, ideal: 100)
            TableColumn("Fee") { row in
                PortfolioNumericCell(text: row.fee == 0 ? "—" : formatter.money(row.fee), color: .secondary)
            }
            .width(min: 60, ideal: 75)
            // The row actions sit at the end of the last column: a column of their own was
            // pushed past the table's edge whenever the window was near its minimum width.
            TableColumn("Notes", value: \.notes) { row in
                HStack(spacing: 4) {
                    Text(row.notes).foregroundStyle(.secondary).lineLimit(1).help(row.notes)
                    Spacer(minLength: 0)
                    Menu {
                        actions(for: row.transaction)
                    } label: {
                        Image(systemName: "ellipsis.circle")
                    }
                    .menuStyle(.borderlessButton)
                    .menuIndicator(.hidden)
                    .fixedSize()
                    .accessibilityLabel("Actions for \(row.ticker) \(row.typeName)")
                }
            }
            .width(min: 90, ideal: 120)
        }
        .portfolioTableChrome()
        .contextMenu(forSelectionType: UUID.self) { ids in
            if let tx = transaction(ids.first, in: rows) { actions(for: tx) }
        } primaryAction: { ids in
            if let tx = transaction(ids.first, in: rows) { onEdit(tx) }
        }
        .onDeleteCommand {
            if let tx = transaction(selection.first, in: rows) { onDelete(tx) }
        }
    }

    private func transaction(_ id: UUID?, in rows: [Row]) -> PortfolioTransaction? {
        guard let id else { return nil }
        return rows.first { $0.id == id }?.transaction
    }

    @ViewBuilder private func actions(for tx: PortfolioTransaction) -> some View {
        Button("Edit") { onEdit(tx) }
        Button("Duplicate") { onDuplicate(tx) }
        Divider()
        Button("Delete", role: .destructive) { onDelete(tx) }
    }

    private func sign(of type: PortfolioTransactionType) -> PortfolioValueFormatter.QuantitySign {
        if type.addsQuantity { return .plus }
        if type.removesQuantity { return .minus }
        return .none
    }

    private func quantityColor(_ type: PortfolioTransactionType) -> Color {
        guard !store.privacyMode else { return .primary }
        if type.addsQuantity { return .green }
        if type.removesQuantity { return .red }
        return .primary
    }

    // MARK: - Empty and footer

    @ViewBuilder private var emptyState: some View {
        if search.trimmingCharacters(in: .whitespaces).isEmpty && !hasFilters {
            ContentUnavailableView(
                "No Transactions", systemImage: "list.bullet.rectangle",
                description: Text("Add a transaction or import a CSV to get started."))
        } else {
            ContentUnavailableView {
                Label("No Matching Transactions", systemImage: "magnifyingglass")
            } description: {
                Text("Nothing matches the current search and filters.")
            } actions: {
                Button("Clear Filters", action: clearFilters)
            }
        }
    }

    private func footer(shown: Int, total: Int) -> some View {
        HStack {
            Text(
                shown == total
                    ? "\(total) \(total == 1 ? "transaction" : "transactions")"
                    : "\(shown) of \(total) transactions"
            )
            Spacer()
            Text("Prices and fees in \(currencyCode)")
        }
        .font(.caption)
        .foregroundStyle(.secondary)
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
    }
}
