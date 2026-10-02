import SwiftUI
import UniformTypeIdentifiers

struct PortfolioDashboardView: View {
    @ObservedObject var store: PortfolioStore
    var initialAsset: PortfolioAsset?
    var isTab = false
    @Environment(\.dismiss) private var dismiss
    @State private var tab: Tab = .overview
    @State private var showCreate = false
    @State private var showManage = false
    @State private var showAssetSearch = false
    @State private var editor: TransactionEditorContext?
    @State private var deleteTransaction: PortfolioTransaction?
    @State private var showImporter = false
    @State private var showCoinMarketCapImporter = false
    @State private var importPreview: PortfolioCSVPreview?
    @State private var coinMarketCapPreview: CoinMarketCapCSVPreview?
    @State private var exportDocument = PortfolioCSVDocument(text: "")
    @State private var showExporter = false
    @State private var selectedHolding: PortfolioHolding?
    @StateObject private var assetInfo = PortfolioAssetInfoViewModel()
    @State private var remappingHolding: PortfolioHolding?

    enum Tab: String, CaseIterable {
        case overview = "Overview"
        case holdings = "Holdings"
        case transactions = "Transactions"
        case statistics = "Statistics"

        var systemImage: String {
            switch self {
            case .overview: "chart.pie"
            case .holdings: "square.stack.3d.up"
            case .transactions: "arrow.left.arrow.right"
            case .statistics: "chart.bar.xaxis"
            }
        }
    }

    var body: some View {
        VStack(spacing: 0) {
            header
            if store.isChangingReportingCurrency || store.isLoadingInitialValues { reportingProgress }
            Divider()
            PortfolioTabBar(
                items: Tab.allCases.map { .init(value: $0, title: $0.rawValue, systemImage: $0.systemImage) },
                selection: $tab
            )
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
            if store.activePortfolios.isEmpty {
                noPortfolios
            } else if store.transactions.isEmpty {
                emptyPortfolio
            } else {
                content
            }
        }
        .frame(minWidth: 980, idealWidth: 1180, minHeight: 680, idealHeight: 800)
        .task {
            await store.initialize()
            openInitialAssetIfNeeded()
        }
        .onChange(of: store.holdings.map(\.asset), initial: true) { _, assets in assetInfo.load(assets) }
        .onReceive(NotificationCenter.default.publisher(for: .portfolioAddTransaction)) { notification in
            guard isTab, let asset = notification.object as? PortfolioAsset else { return }
            beginTransaction(for: asset)
        }
        .sheet(isPresented: $showCreate) { PortfolioCreateSheet(store: store) }
        .sheet(isPresented: $showManage) { PortfolioManageSheet(store: store) }
        .sheet(isPresented: $showAssetSearch) {
            AddTickerSheet(title: "Add Asset", actionLabel: "Continue") { result in
                editor = .new(asset: PortfolioAsset(searchResult: result), portfolioID: destinationPortfolioID)
            }
        }
        .sheet(item: $editor) { context in TransactionEditorSheet(store: store, context: context) }
        .sheet(item: $selectedHolding) { holding in
            PortfolioAssetDetailView(store: store, assetKey: holding.asset.key, info: assetInfo)
        }
        .sheet(item: $remappingHolding) { holding in
            AddTickerSheet(title: "Remap \(holding.asset.displayTicker)", actionLabel: "Use Asset") { result in
                try await store.remapAsset(
                    from: holding.asset.key,
                    to: PortfolioAsset(searchResult: result),
                    portfolioIDs: store.selectedPortfolioIDs
                )
                remappingHolding = nil
                await store.refreshQuotes()
                await store.rebuildHistory()
            }
        }
        .sheet(item: $importPreview) { preview in PortfolioImportPreviewSheet(store: store, preview: preview) }
        .sheet(item: $coinMarketCapPreview) { preview in
            PortfolioCoinMarketCapImportSheet(store: store, preview: preview, portfolioID: destinationPortfolioID) { converted in
                coinMarketCapPreview = nil
                importPreview = converted
            }
        }
        .fileImporter(isPresented: $showImporter, allowedContentTypes: [.commaSeparatedText, .plainText]) { result in
            do {
                let url = try result.get()
                guard url.startAccessingSecurityScopedResource() else { return }
                defer { url.stopAccessingSecurityScopedResource() }
                importPreview = PortfolioCSVService.preview(
                    try String(contentsOf: url, encoding: .utf8), portfolios: store.snapshot.portfolios)
            } catch { store.lastError = error.localizedDescription }
        }
        .fileImporter(isPresented: $showCoinMarketCapImporter, allowedContentTypes: [.commaSeparatedText, .plainText]) {
            result in
            do {
                let url = try result.get()
                guard url.startAccessingSecurityScopedResource() else { return }
                defer { url.stopAccessingSecurityScopedResource() }
                coinMarketCapPreview = PortfolioCSVService.previewCoinMarketCap(
                    try String(contentsOf: url, encoding: .utf8))
            } catch { store.lastError = error.localizedDescription }
        }
        .fileExporter(
            isPresented: $showExporter, document: exportDocument, contentType: .commaSeparatedText,
            defaultFilename: "degenview-portfolio-transactions.csv"
        ) { _ in }
        .confirmationDialog(
            "Delete transaction?",
            isPresented: Binding(get: { deleteTransaction != nil }, set: { if !$0 { deleteTransaction = nil } })
        ) {
            Button("Delete", role: .destructive) {
                if let id = deleteTransaction?.id { store.deleteTransaction(id) }
                deleteTransaction = nil
            }
            Button("Cancel", role: .cancel) { deleteTransaction = nil }
        } message: {
            Text("Holdings, cost basis, P&L, allocation, and later history will be recalculated.")
        }
        .alert(
            "Portfolio",
            isPresented: Binding(get: { store.lastError != nil }, set: { if !$0 { store.lastError = nil } })
        ) {
            Button("OK") { store.lastError = nil }
        } message: {
            Text(store.lastError ?? "")
        }
    }

    private var header: some View {
        HStack(spacing: 12) {
            Menu {
                Button("All Portfolios") { store.select(.all) }
                Divider()
                ForEach(store.activePortfolios) { portfolio in
                    Button(portfolio.name) { store.select(.portfolio(portfolio.id)) }
                }
                Divider()
                Button("Create Portfolio…") { showCreate = true }
                Button("Manage Portfolios…") { showManage = true }
            } label: {
                HeaderMenuLabel(icon: "briefcase", title: store.selectedPortfolio?.name ?? "All Portfolios")
            }
            .menuStyle(.button)
            .menuIndicator(.hidden)
            .fixedSize()
            .accessibilityLabel("Portfolio selector, \(store.selectedPortfolio?.name ?? "All Portfolios")")
            Menu {
                ForEach(PortfolioCurrency.allCases) { currency in
                    Button {
                        Task { await store.selectReportingCurrency(currency) }
                    } label: {
                        if currency == store.reportingCurrency {
                            Label(currency.rawValue, systemImage: "checkmark")
                        } else {
                            Text(currency.rawValue)
                        }
                    }
                }
            } label: {
                HeaderMenuLabel(
                    icon: "banknote", title: store.reportingCurrency.rawValue,
                    isBusy: store.isChangingReportingCurrency)
            }
            .menuStyle(.button)
            .menuIndicator(.hidden)
            .fixedSize()
            .disabled(store.isChangingReportingCurrency)
            .accessibilityLabel("Reporting currency, \(store.reportingCurrency.rawValue)")
            Spacer()
            Button {
                store.privacyMode.toggle()
            } label: {
                Image(systemName: store.privacyMode ? "eye.slash" : "eye")
            }
            .help(store.privacyMode ? "Reveal portfolio values" : "Hide portfolio values")
            .accessibilityLabel(store.privacyMode ? "Privacy mode on" : "Privacy mode off")
            Menu {
                Button("Import Transactions…") { showImporter = true }
                Button("Import from CoinMarketCap…") {
                    guard store.selectedPortfolio != nil else {
                        store.lastError =
                            "Select the destination portfolio before importing CoinMarketCap transactions."
                        return
                    }
                    showCoinMarketCapImporter = true
                }
                Button("Export Transactions…") {
                    exportDocument = .init(
                        text: PortfolioCSVService.exportTransactions(
                            store.ledgerTransactions, portfolios: store.snapshot.portfolios))
                    showExporter = true
                }
                Button("Export Current Holdings…") {
                    exportDocument = .init(
                        text: PortfolioCSVService.exportHoldings(
                            store.ledgerHoldings, portfolioName: store.selectedPortfolio?.name ?? "All Portfolios"))
                    showExporter = true
                }
                Button("Export Portfolio History…") {
                    exportDocument = .init(text: PortfolioCSVService.exportHistory(store.ledgerHistory))
                    showExporter = true
                }
            } label: {
                HeaderMenuLabel(icon: "arrow.up.arrow.down", title: "Import or Export")
            }
            .menuStyle(.button)
            .menuIndicator(.hidden)
            .fixedSize()
            Button("Add Transaction", systemImage: "plus") { showAssetSearch = true }.buttonStyle(.borderedProminent)
            if !isTab { Button("Done") { dismiss() }.keyboardShortcut(.cancelAction) }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 16)
        .fixedSize(horizontal: false, vertical: true)
    }

    private var reportingProgress: some View {
        HStack(spacing: 10) {
            ProgressView(value: store.reportingConversionProgress ?? 0)
                .progressViewStyle(.linear)
                .frame(maxWidth: 260)
            Text(store.isLoadingInitialValues ? "Loading prices…" : "Updating values and charts…")
            Spacer()
            Text("\(Int(((store.reportingConversionProgress ?? 0) * 100).rounded()))%")
                .monospacedDigit()
        }
        .font(.caption)
        .foregroundStyle(.secondary)
        .padding(.horizontal, 16)
        .padding(.bottom, 8)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(
            "Updating portfolio values and charts, \(Int(((store.reportingConversionProgress ?? 0) * 100).rounded())) percent"
        )
    }

    @ViewBuilder private var content: some View {
        switch tab {
        case .overview:
            PortfolioOverviewView(
                store: store, info: assetInfo, onSelect: { selectedHolding = $0 },
                onShowHoldings: { tab = .holdings })
        case .holdings:
            PortfolioHoldingsView(
                store: store, info: assetInfo, onSelect: { selectedHolding = $0 }, onRemap: { remappingHolding = $0 })
        case .transactions:
            PortfolioTransactionsView(
                store: store, info: assetInfo, onAdd: { showAssetSearch = true },
                onEdit: { editor = .edit($0) }, onDuplicate: { editor = .duplicate($0) },
                onDelete: { deleteTransaction = $0 })
        case .statistics: PortfolioStatisticsView(store: store, info: assetInfo)
        }
    }

    private var noPortfolios: some View {
        VStack(spacing: 14) {
            ContentUnavailableView(
                "No Portfolios", systemImage: "briefcase",
                description: Text("Create a portfolio to start tracking investments."))
            Button("Create Portfolio") { showCreate = true }.buttonStyle(.borderedProminent)
        }.frame(maxWidth: .infinity, maxHeight: .infinity)
    }
    private var emptyPortfolio: some View {
        VStack(spacing: 14) {
            ContentUnavailableView(
                "Your portfolio is empty", systemImage: "chart.pie",
                description: Text("Track investments by adding your first transaction."))
            HStack {
                Button("Add Transaction") { showAssetSearch = true }.buttonStyle(.borderedProminent)
                Button("Import Transactions") { showImporter = true }
            }
        }.frame(maxWidth: .infinity, maxHeight: .infinity)
    }
    private var destinationPortfolioID: UUID { store.selectedPortfolio?.id ?? store.activePortfolios.first!.id }
    private func openInitialAssetIfNeeded() { if let initialAsset { beginTransaction(for: initialAsset) } }
    private func beginTransaction(for asset: PortfolioAsset) {
        guard !store.activePortfolios.isEmpty else {
            store.lastError = "Create a portfolio before adding a transaction."
            return
        }
        editor = .new(asset: asset, portfolioID: destinationPortfolioID)
    }
}

/// Label for the header's selector menus: icon, title, one chevron. The menus hide the system
/// indicator (`.menuIndicator(.hidden)`), so the chevron here is the only arrow.
private struct HeaderMenuLabel: View {
    let icon: String
    let title: String
    var isBusy = false

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: icon).foregroundStyle(.secondary)
            Text(title).lineLimit(1)
            if isBusy {
                ProgressView().controlSize(.mini)
            } else {
                Image(systemName: "chevron.down")
                    .font(.system(size: 9, weight: .semibold))
                    .foregroundStyle(.secondary)
            }
        }
    }
}

struct TransactionEditorContext: Identifiable {
    let id = UUID()
    var transaction: PortfolioTransaction
    var mode: Mode
    enum Mode { case new, edit, duplicate }
    static func new(asset: PortfolioAsset, portfolioID: UUID) -> Self {
        .init(transaction: .init(portfolioID: portfolioID, asset: asset, type: .buy, quantity: 0), mode: .new)
    }
    static func edit(_ value: PortfolioTransaction) -> Self { .init(transaction: value, mode: .edit) }
    static func duplicate(_ value: PortfolioTransaction) -> Self {
        var copy = value
        copy = .init(
            portfolioID: copy.portfolioID, asset: copy.asset, type: copy.type, quantity: copy.quantity,
            price: copy.price, priceCurrency: copy.priceCurrency, fee: copy.fee, feeCurrency: copy.feeCurrency,
            timestamp: copy.timestamp, notes: copy.notes)
        return .init(transaction: copy, mode: .duplicate)
    }
}

struct TransactionEditorSheet: View {
    @ObservedObject var store: PortfolioStore
    let context: TransactionEditorContext
    @Environment(\.dismiss) private var dismiss
    @State private var type: PortfolioTransactionType
    @State private var quantity: String
    @State private var price: String
    @State private var total: String
    @State private var fee: String
    @State private var date: Date
    @State private var notes: String
    @State private var editingTotal = false
    init(store: PortfolioStore, context: TransactionEditorContext) {
        self.store = store
        self.context = context
        let tx = context.transaction
        _type = State(initialValue: tx.type)
        _quantity = State(initialValue: tx.quantity == 0 ? "" : tx.quantity.description)
        _price = State(initialValue: tx.price?.description ?? "")
        _total = State(initialValue: tx.price.map { ($0 * tx.quantity).description } ?? "")
        _fee = State(initialValue: tx.fee == 0 ? "" : tx.fee.description)
        _date = State(initialValue: tx.timestamp)
        _notes = State(initialValue: tx.notes)
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("\(context.mode == .edit ? "Edit" : "Add") Transaction").font(.headline)
            Text("\(context.transaction.asset.displayTicker) · \(context.transaction.asset.source.rawValue)")
                .foregroundStyle(.secondary)
            Picker("Type", selection: $type) {
                ForEach(PortfolioTransactionType.allCases) { Text($0.rawValue).tag($0) }
            }
            TextField("Quantity", text: $quantity).onChange(of: quantity) { recalculateTotal() }
            if [.buy, .sell, .transferIn, .reward, .stakingReward, .airdrop, .mining, .interest].contains(type) {
                TextField(
                    type == .sell ? "Sale price" : "Price per asset (optional for transfers/rewards)", text: $price
                ).onChange(of: price) { recalculateTotal() }
                TextField("Total", text: $total).onChange(of: total) { _, _ in
                    if editingTotal, let q = Decimal(string: quantity), q != 0, let t = Decimal(string: total) {
                        price = (t / q).description
                    }
                }.onTapGesture { editingTotal = true }
            }
            DatePicker("Date and time", selection: $date)
            TextField("Fee", text: $fee)
            TextField("Notes", text: $notes, axis: .vertical).lineLimit(2...4)
            HStack {
                Spacer()
                Button("Cancel") { dismiss() }
                Button("Save") { save() }.buttonStyle(.borderedProminent).disabled(
                    Decimal(string: quantity).map { $0 <= 0 } ?? true)
            }
        }.padding(24).frame(width: 430)
    }
    private func recalculateTotal() {
        guard !editingTotal, let q = Decimal(string: quantity), let p = Decimal(string: price) else { return }
        total = (q * p).description
    }
    private func save() {
        guard let q = Decimal(string: quantity) else { return }
        var tx = context.transaction
        tx.type = type
        tx.quantity = q
        tx.price = Decimal(string: price)
        tx.fee = Decimal(string: fee) ?? 0
        tx.timestamp = date
        tx.notes = notes
        tx.updatedAt = Date()
        Task {
            let ok = context.mode == .edit ? await store.update(tx) : await store.add(tx)
            if ok {
                dismiss()
                await store.refreshQuotes()
                await store.rebuildHistory()
            }
        }
    }
}

struct PortfolioCSVDocument: FileDocument {
    static var readableContentTypes: [UTType] { [.commaSeparatedText] }
    var text: String
    init(text: String) { self.text = text }
    init(configuration: ReadConfiguration) throws {
        text = configuration.file.regularFileContents.flatMap { String(data: $0, encoding: .utf8) } ?? ""
    }
    func fileWrapper(configuration: WriteConfiguration) throws -> FileWrapper {
        .init(regularFileWithContents: Data(text.utf8))
    }
}
