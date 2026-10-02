import SwiftUI

/// One asset's position and its transactions, presented as a sheet from the Holdings tab.
///
/// The holding is resolved from the store by key rather than captured, so editing or deleting a
/// transaction refreshes the figures in place. Every amount is in the store's reporting currency
/// — the same one the Holdings table shows — and says so.
struct PortfolioAssetDetailView: View {
    @ObservedObject var store: PortfolioStore
    let assetKey: String
    @ObservedObject var info: PortfolioAssetInfoViewModel
    @Environment(\.dismiss) private var dismiss
    @State private var editor: TransactionEditorContext?
    @State private var deleting: PortfolioTransaction?
    @State private var selection: Set<PortfolioTransaction.ID> = []

    private var holding: PortfolioHolding? { store.holdings.first { $0.asset.key == assetKey } }
    private var transactions: [PortfolioTransaction] {
        store.ledgerTransactions.filter { $0.asset.key == assetKey }.sorted { $0.timestamp > $1.timestamp }
    }
    private var asset: PortfolioAsset? { holding?.asset ?? transactions.first?.asset }
    private var currency: PortfolioCurrency { store.reportingCurrency }

    private var formatter: PortfolioValueFormatter {
        PortfolioValueFormatter(currency: currency, privacy: store.privacyMode)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            VStack(alignment: .leading, spacing: 18) {
                header
                if let holding {
                    stats(for: holding)
                } else {
                    noPosition
                }
                transactionsSection
            }
            .padding(24)
            Divider()
            footer
        }
        .frame(width: 880, height: 660)
        .sheet(item: $editor) { TransactionEditorSheet(store: store, context: $0) }
        .confirmationDialog(
            "Delete transaction?",
            isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } })
        ) {
            Button("Delete", role: .destructive) {
                if let id = deleting?.id { store.deleteTransaction(id) }
                deleting = nil
            }
        }
    }

    // MARK: - Header

    @ViewBuilder private var header: some View {
        if let asset {
            HStack(spacing: 14) {
                TickerIconView(symbol: asset.displayTicker, url: info.iconURL(for: asset), size: 40)
                VStack(alignment: .leading, spacing: 2) {
                    Text(asset.displayTicker).font(.title.bold())
                    Text("\(info.subtitle(for: asset)) · \(asset.source.rawValue)")
                        .foregroundStyle(.secondary)
                }
                Spacer()
            }
            .accessibilityElement(children: .combine)
        }
    }

    // MARK: - Position

    private func stats(for holding: PortfolioHolding) -> some View {
        let masked = store.privacyMode
        return VStack(alignment: .leading, spacing: 8) {
            Text("Values in \(currency.rawValue)")
                .font(.caption)
                .foregroundStyle(.secondary)
            LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 12), count: 4), spacing: 12) {
                PortfolioStatCard(
                    title: "Holdings",
                    value: masked
                        ? PortfolioValueFormatter.mask
                        : "\(holding.quantity.portfolioQuantity) \(holding.asset.displayTicker)",
                    isMasked: masked)
                PortfolioStatCard(
                    title: "Current Value", value: holding.currentValue.map(formatter.money) ?? "Unavailable",
                    isPlaceholder: holding.currentValue == nil, isMasked: masked)
                PortfolioStatCard(
                    title: "Average Cost", value: formatter.price(holding.averageCost), isMasked: masked)
                PortfolioStatCard(
                    title: "Current Price", value: holding.currentPrice.map(formatter.price) ?? "Unavailable",
                    isPlaceholder: holding.currentPrice == nil, isMasked: masked)
                PortfolioStatCard(
                    title: "Unrealized P&L", value: holding.unrealizedPnL.map(formatter.signedMoney) ?? "—",
                    tone: formatter.tone(holding.unrealizedPnL), isPlaceholder: holding.unrealizedPnL == nil,
                    isMasked: masked)
                PortfolioStatCard(
                    title: "Realized P&L", value: formatter.signedMoney(holding.realizedPnL),
                    tone: formatter.tone(holding.realizedPnL), isMasked: masked)
                PortfolioStatCard(
                    title: "Total P&L", value: holding.totalPnL.map(formatter.signedMoney) ?? "—",
                    tone: formatter.tone(holding.totalPnL), caption: holding.pnlPercent.map { formatter.signedPercent($0) },
                    isPlaceholder: holding.totalPnL == nil, isMasked: masked)
                PortfolioStatCard(title: "Allocation", value: formatter.percent(holding.allocation))
            }
        }
    }

    private var noPosition: some View {
        HStack(spacing: 8) {
            Image(systemName: "tray")
            Text("No open position — the transactions below net to zero.")
        }
        .font(.callout)
        .foregroundStyle(.secondary)
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(12)
        .background(.quaternary.opacity(0.4), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
    }

    // MARK: - Transactions

    private var transactionsSection: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .firstTextBaseline) {
                Text("Transactions").font(.headline)
                Text("\(transactions.count)").font(.subheadline).foregroundStyle(.secondary)
            }
            if transactions.isEmpty {
                ContentUnavailableView("No Transactions", systemImage: "list.bullet.rectangle")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                table
            }
        }
        .frame(maxHeight: .infinity)
    }

    private var table: some View {
        Table(transactions, selection: $selection) {
            TableColumn("Date") { tx in
                Text(tx.timestamp.formatted(date: .abbreviated, time: .shortened)).lineLimit(1)
            }
            .width(min: 110, ideal: 140)
            TableColumn("Type") { tx in
                PortfolioTransactionTypeBadge(type: tx.type)
            }
            .width(min: 90, ideal: 100)
            TableColumn("Quantity") { tx in
                Text(formatter.quantity(tx.quantity, sign: sign(of: tx.type))).monospacedDigit()
                    .foregroundStyle(quantityColor(tx.type))
                    .frame(maxWidth: .infinity, alignment: .trailing)
            }
            .width(min: 80, ideal: 100)
            TableColumn("Price (\(currency.rawValue))") { tx in
                Text(price(of: tx).map(formatter.price) ?? "—").monospacedDigit()
                    .frame(maxWidth: .infinity, alignment: .trailing)
            }
            .width(min: 80, ideal: 105)
            TableColumn("Total (\(currency.rawValue))") { tx in
                Text(price(of: tx).map { formatter.money($0 * tx.quantity) } ?? "—").monospacedDigit()
                    .frame(maxWidth: .infinity, alignment: .trailing)
            }
            .width(min: 80, ideal: 110)
            // Row actions live at the end of the last column; see PortfolioTransactionsView.
            TableColumn("Notes") { tx in
                HStack(spacing: 4) {
                    Text(tx.notes).foregroundStyle(.secondary).lineLimit(1)
                    Spacer(minLength: 0)
                    Menu {
                        actions(for: tx)
                    } label: {
                        Image(systemName: "ellipsis.circle")
                    }
                    .menuStyle(.borderlessButton)
                    .menuIndicator(.hidden)
                    .fixedSize()
                }
            }
            .width(min: 90, ideal: 130)
        }
        .portfolioTableChrome(inset: 0)
        .contextMenu(forSelectionType: PortfolioTransaction.ID.self) { ids in
            if let id = ids.first, let tx = transactions.first(where: { $0.id == id }) {
                actions(for: tx)
            }
        } primaryAction: { ids in
            if let id = ids.first, let tx = transactions.first(where: { $0.id == id }) { editor = .edit(tx) }
        }
    }

    @ViewBuilder private func actions(for tx: PortfolioTransaction) -> some View {
        Button("Edit") { editor = .edit(tx) }
        Button("Duplicate") { editor = .duplicate(tx) }
        Divider()
        Button("Delete", role: .destructive) { deleting = tx }
    }

    /// The price in the reporting currency; the ledger's own `price` may be in another one.
    private func price(of tx: PortfolioTransaction) -> Decimal? { store.reportingPrice(for: tx) }

    private func sign(of type: PortfolioTransactionType) -> PortfolioValueFormatter.QuantitySign {
        if type.addsQuantity { return .plus }
        if type.removesQuantity { return .minus }
        return .none
    }

    private func quantityColor(_ type: PortfolioTransactionType) -> Color {
        if store.privacyMode { return .primary }
        if type.addsQuantity { return .green }
        if type.removesQuantity { return .red }
        return .primary
    }

    // MARK: - Footer

    private var footer: some View {
        HStack {
            Spacer()
            Button("Done") { dismiss() }
                .keyboardShortcut(.defaultAction)
                .controlSize(.large)
        }
        .padding(.horizontal, 24)
        .padding(.vertical, 14)
    }
}
