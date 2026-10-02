import SwiftUI

/// The last step of any CSV import: what is about to be written, with the problems found.
struct PortfolioImportPreviewSheet: View {
    @ObservedObject var store: PortfolioStore
    let preview: PortfolioCSVPreview
    @Environment(\.dismiss) private var dismiss
    @State private var isImporting = false
    @State private var importFailure: String?

    private var rows: [PortfolioTransaction] { preview.transactions.sorted { $0.timestamp > $1.timestamp } }
    private var assetCount: Int { PortfolioAccountingEngine.uniqueAssets(in: preview.transactions).count }
    private var canImport: Bool { preview.isValid && !preview.transactions.isEmpty && !isImporting }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            VStack(alignment: .leading, spacing: 16) {
                PortfolioSheetHeader(
                    systemImage: "doc.text.magnifyingglass", title: "Import Preview",
                    subtitle: preview.transactions.isEmpty
                        ? "Nothing to import yet." : "Review what will be added to your portfolio.")
                summary
                notices
                content
            }
            .padding(24)
            Divider()
            footer
        }
        .frame(width: 680, height: 600)
    }

    // MARK: - Summary and notices

    private var summary: some View {
        HStack(spacing: 12) {
            PortfolioStatCard(
                title: "Transactions", value: "\(preview.transactions.count)", systemImage: "list.bullet.rectangle")
            PortfolioStatCard(title: "Assets", value: "\(assetCount)", systemImage: "square.stack.3d.up")
            PortfolioStatCard(
                title: preview.errors.isEmpty ? "Warnings" : "Errors",
                value: "\(preview.errors.isEmpty ? preview.warnings.count : preview.errors.count)",
                tone: preview.errors.isEmpty
                    ? (preview.warnings.isEmpty ? nil : .orange) : .red,
                systemImage: preview.errors.isEmpty ? "exclamationmark.triangle" : "xmark.octagon")
        }
    }

    @ViewBuilder private var notices: some View {
        if !preview.errors.isEmpty {
            PortfolioNoticeCard(
                systemImage: "xmark.octagon.fill", tint: .red,
                title: "\(preview.errors.count) \(preview.errors.count == 1 ? "problem requires" : "problems require") attention",
                detail: "Fix the source data and import again.", lines: preview.errors, maxListHeight: 90)
        }
        if !preview.warnings.isEmpty {
            PortfolioNoticeCard(
                systemImage: "exclamationmark.triangle.fill", tint: .orange,
                title: "\(preview.warnings.count) \(preview.warnings.count == 1 ? "warning" : "warnings")",
                detail: "Warnings don't block the import.", lines: preview.warnings, maxListHeight: 70)
        }
        if let importFailure {
            PortfolioNoticeCard(
                systemImage: "exclamationmark.triangle.fill", tint: .red, title: "Import failed",
                detail: importFailure + "\n\nNo transactions were imported. Correct the source data, "
                    + "or restart the CoinMarketCap import and skip the affected ticker or item.")
        }
    }

    // MARK: - Transactions

    @ViewBuilder private var content: some View {
        if rows.isEmpty {
            ContentUnavailableView(
                "No Transactions", systemImage: "tray",
                description: Text("The file didn't contain any transactions that can be imported.")
            )
            .frame(maxHeight: .infinity)
        } else {
            Table(rows) {
                TableColumn("Date") { tx in
                    Text(tx.timestamp.formatted(date: .abbreviated, time: .shortened)).lineLimit(1)
                }
                .width(min: 120, ideal: 140)
                TableColumn("Asset") { tx in
                    Text(tx.asset.displayTicker).fontWeight(.semibold).lineLimit(1)
                }
                .width(min: 55, ideal: 65)
                TableColumn("Type") { tx in
                    PortfolioTransactionTypeBadge(type: tx.type)
                }
                .width(min: 85, ideal: 95)
                TableColumn("Quantity") { tx in
                    PortfolioNumericCell(text: tx.quantity.portfolioQuantity)
                }
                .width(min: 70, ideal: 90)
                TableColumn("Price") { tx in
                    PortfolioNumericCell(text: tx.price.map(tx.priceCurrency.formatPrice) ?? "—")
                }
                .width(min: 85, ideal: 100)
            }
            .portfolioTableChrome(inset: 0)
        }
    }

    // MARK: - Footer

    private var footer: some View {
        HStack(spacing: 10) {
            if isImporting {
                ProgressView().controlSize(.small)
                Text("Validating and importing…").font(.callout).foregroundStyle(.secondary)
            }
            Spacer()
            Button("Cancel") { dismiss() }
                .keyboardShortcut(.cancelAction)
                .controlSize(.large)
                .disabled(isImporting)
            Button("Import \(preview.transactions.count) \(preview.transactions.count == 1 ? "Transaction" : "Transactions")") {
                beginImport()
            }
            .keyboardShortcut(.defaultAction)
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
            .disabled(!canImport)
        }
        .padding(.horizontal, 24)
        .padding(.vertical, 14)
    }

    private func beginImport() {
        isImporting = true
        importFailure = nil
        store.lastError = nil
        Task {
            let succeeded = await store.importTransactions(preview.transactions)
            guard succeeded else {
                let message = store.lastError ?? "The transaction ledger rejected this import for an unknown reason."
                store.lastError = nil
                importFailure = message
                isImporting = false
                return
            }
            dismiss()
            await store.refreshQuotes()
            await store.rebuildHistory()
        }
    }
}
