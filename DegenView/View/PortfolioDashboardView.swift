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
    @State private var transactionFilter: PortfolioTransactionType?
    @State private var transactionSearch = ""
    @State private var selectedHolding: PortfolioHolding?
    @State private var remappingHolding: PortfolioHolding?
    @State private var historyRange: PortfolioHistoryRange = .oneMonth

    enum Tab: String, CaseIterable {
        case overview = "Overview"
        case holdings = "Holdings"
        case transactions = "Transactions"
        case statistics = "Statistics"
    }

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            Picker("Section", selection: $tab) { ForEach(Tab.allCases, id: \.self) { Text($0.rawValue).tag($0) } }
                .pickerStyle(.segmented)
                .labelsHidden()
                .padding(.horizontal, 16)
                .padding(.top, 10)
                .padding(.bottom, 10)
                .fixedSize(horizontal: false, vertical: true)
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
            await store.refresh()
            await store.refreshQuotes()
            await store.rebuildHistory()
            openInitialAssetIfNeeded()
        }
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
        .sheet(item: $selectedHolding) { holding in PortfolioAssetDetailView(store: store, holding: holding) }
        .sheet(item: $remappingHolding) { holding in
            AddTickerSheet(title: "Remap \(holding.asset.symbol)", actionLabel: "Use Asset") { result in
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
            CoinMarketCapImportSheet(store: store, preview: preview, portfolioID: destinationPortfolioID) { converted in
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
                Label(store.selectedPortfolio?.name ?? "All Portfolios", systemImage: "chevron.down")
            }
            .accessibilityLabel("Portfolio selector, \(store.selectedPortfolio?.name ?? "All Portfolios")")
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
                            store.transactions, portfolios: store.snapshot.portfolios))
                    showExporter = true
                }
                Button("Export Current Holdings…") {
                    exportDocument = .init(
                        text: PortfolioCSVService.exportHoldings(
                            store.holdings, portfolioName: store.selectedPortfolio?.name ?? "All Portfolios"))
                    showExporter = true
                }
                Button("Export Portfolio History…") {
                    exportDocument = .init(text: PortfolioCSVService.exportHistory(history))
                    showExporter = true
                }
            } label: {
                Label("Import or Export", systemImage: "arrow.up.arrow.down")
            }
            Button("Add Transaction", systemImage: "plus") { showAssetSearch = true }.buttonStyle(.borderedProminent)
            if !isTab { Button("Done") { dismiss() }.keyboardShortcut(.cancelAction) }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 16)
        .fixedSize(horizontal: false, vertical: true)
    }

    @ViewBuilder private var content: some View {
        switch tab {
        case .overview: overview
        case .holdings: holdingsList
        case .transactions: transactionsList
        case .statistics: statistics
        }
    }

    private var overview: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                VStack(alignment: .leading, spacing: 4) {
                    Text(store.selectedPortfolio?.name ?? "All Portfolios").font(.title2).bold()
                    ZStack(alignment: .leading) {
                        Text(privateMoney(store.totalValue))
                            .font(.system(size: 38, weight: .semibold, design: .rounded))
                            .opacity(store.isLoadingInitialValues ? 0 : 1)
                            .accessibilityHidden(store.isLoadingInitialValues)
                            .accessibilityLabel(
                                store.privacyMode
                                    ? "Portfolio balance hidden" : "Portfolio balance, \(money(store.totalValue))")
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
                    Text(store.marketValueCaption)
                        .font(.caption).foregroundStyle(.secondary)
                }
                HStack {
                    metric("All-time P&L", store.totalPnL)
                    metric("Realized P&L", store.totalRealizedPnL)
                    metric("Unrealized P&L", store.totalUnrealizedPnL)
                }
                GroupBox("Portfolio Value") {
                    VStack(spacing: 8) {
                        Picker("History range", selection: $historyRange) {
                            ForEach(PortfolioHistoryRange.allCases) { range in Text(range.rawValue).tag(range) }
                        }
                        .pickerStyle(.segmented)
                        .frame(maxWidth: 430)
                        PortfolioHistoryChart(
                            snapshots: history,
                            currentValue: store.totalValue,
                            currency: store.selectedPortfolio?.baseCurrency ?? .USD,
                            range: historyRange,
                            privacy: store.privacyMode,
                            isLoading: store.isLoadingInitialValues
                        )
                        .frame(height: 230)
                    }.padding(8)
                }
                HStack(alignment: .top) {
                    GroupBox("Allocation") {
                        PortfolioAllocationChart(holdings: store.holdings, privacy: store.privacyMode)
                            .frame(height: 220).padding(8)
                    }
                    GroupBox("Largest Holdings") {
                        holdingRows(sortedHoldings).frame(height: 220).padding(8)
                    }
                }
            }.padding()
        }
    }

    private var holdingsList: some View {
        VStack(spacing: 0) {
            HStack {
                HStack(spacing: 4) {
                    Text("Asset")
                    Menu {
                        ForEach(PortfolioHoldingsSort.allCases) { sort in
                            Button(sort.rawValue) {
                                guard var portfolio = store.selectedPortfolio else { return }
                                portfolio.sort = sort
                                store.update(portfolio)
                            }
                        }
                    } label: {
                        Image(systemName: "arrow.up.arrow.down")
                    }
                    .menuStyle(.borderlessButton)
                    .help(
                        "Sort by \(store.selectedPortfolio?.sort.rawValue ?? PortfolioHoldingsSort.currentValue.rawValue)"
                    )
                    Spacer()
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                Text("Price").frame(width: 110)
                Text("24h %").frame(width: 80)
                Text("Holdings").frame(width: 120)
                Text("Avg Cost").frame(width: 110)
                Text("Value").frame(width: 120)
                Text("Allocation").frame(width: 90)
                Text("P&L").frame(width: 120)
                Color.clear.frame(width: 24)
            }
            .font(.caption)
            .foregroundStyle(.secondary)
            .padding(.horizontal, 16)
            .padding(.vertical, 3)
            .fixedSize(horizontal: false, vertical: true)
            Divider()
            ScrollView {
                LazyVStack(spacing: 0) {
                    ForEach(sortedHoldings) { holding in
                        HStack {
                            VStack(alignment: .leading) {
                                Text(holding.asset.name).bold()
                                Text(holding.asset.symbol).font(.caption).foregroundStyle(.secondary)
                            }.frame(maxWidth: .infinity, alignment: .leading)
                            Text(holding.currentPrice.map(money) ?? "Unavailable").frame(width: 110)
                            Text(holding.dayChangePercent.map(percent) ?? "—").foregroundStyle(
                                (holding.dayChangePercent ?? 0) >= 0 ? .green : .red
                            ).frame(width: 80)
                            Text(verbatim: store.privacyMode ? "••••" : holding.quantity.description).frame(width: 120)
                            Text(privateMoney(holding.averageCost)).frame(width: 110)
                            Text(holding.currentValue.map(privateMoney) ?? "—").frame(width: 120)
                            Text(percent(holding.allocation)).frame(width: 90)
                            Text(holding.totalPnL.map(privateMoney) ?? "—").foregroundStyle(
                                (holding.totalPnL ?? 0) >= 0 ? .green : .red
                            ).frame(width: 120)
                            Menu {
                                Button("Remap Asset…") { remappingHolding = holding }
                            } label: {
                                Image(systemName: "ellipsis.circle")
                            }.menuStyle(.borderlessButton).frame(width: 24)
                        }
                        .padding(.horizontal, 16)
                        .padding(.vertical, 4)
                        .contentShape(Rectangle())
                        .onTapGesture { selectedHolding = holding }
                        .accessibilityElement(children: .combine)
                        .accessibilityLabel(
                            store.privacyMode
                                ? "\(holding.asset.name), financial values hidden"
                                : "\(holding.asset.name), allocation \(percent(holding.allocation)), value \(holding.currentValue.map(money) ?? "unavailable")"
                        )
                        Divider().padding(.leading, 16)
                    }
                }
            }
            .contentMargins(.vertical, 0, for: .scrollContent)
            .frame(maxHeight: .infinity, alignment: .top)
            .layoutPriority(1)
        }
        .frame(maxHeight: .infinity, alignment: .top)
    }

    private var transactionsList: some View {
        VStack {
            HStack {
                TextField("Search asset, notes, or source", text: $transactionSearch)
                Picker("Type", selection: $transactionFilter) {
                    Text("All").tag(nil as PortfolioTransactionType?)
                    ForEach(PortfolioTransactionType.allCases) { Text($0.rawValue).tag(Optional($0)) }
                }.frame(width: 180)
            }.padding(.horizontal)
            List(filteredTransactions) { tx in
                HStack {
                    Text(tx.asset.symbol).bold().frame(width: 100, alignment: .leading)
                    Text(tx.type.rawValue).frame(width: 110, alignment: .leading)
                    Text(store.privacyMode ? "••••" : tx.quantity.description).frame(width: 110)
                    Text(tx.price.map { store.privacyMode ? "••••" : money($0) } ?? "—").frame(width: 110)
                    Text(tx.timestamp.formatted(date: .abbreviated, time: .shortened)).frame(width: 170)
                    Text(tx.notes).frame(maxWidth: .infinity, alignment: .leading)
                    Text(tx.source.rawValue).foregroundStyle(.secondary)
                    Menu {
                        Button("Edit") { editor = .edit(tx) }
                        Button("Duplicate") { editor = .duplicate(tx) }
                        Button("Delete", role: .destructive) { deleteTransaction = tx }
                    } label: {
                        Image(systemName: "ellipsis.circle")
                    }
                }
            }
        }
    }

    private var statistics: AnyView {
        let valued = sortedHoldings.filter { $0.pnlPercent != nil }
        let best = valued.max { ($0.pnlPercent ?? 0) < ($1.pnlPercent ?? 0) }
        let worst = valued.min { ($0.pnlPercent ?? 0) < ($1.pnlPercent ?? 0) }
        return AnyView(
            ScrollView {
                LazyVGrid(columns: [.init(.flexible()), .init(.flexible()), .init(.flexible())], spacing: 16) {
                    metric("Total P&L", store.totalPnL)
                    metric("Realized P&L", store.totalRealizedPnL)
                    metric("Unrealized P&L", store.totalUnrealizedPnL)
                    textMetric(
                        "Best Performer", best.map { "\($0.asset.symbol) · \(percent($0.pnlPercent ?? 0))" } ?? "—")
                    textMetric(
                        "Worst Performer", worst.map { "\($0.asset.symbol) · \(percent($0.pnlPercent ?? 0))" } ?? "—")
                    textMetric("Number of Assets", "\(store.holdings.count)")
                    textMetric("Portfolio High", history.map(\.value).max().map(privateMoney) ?? "—")
                    textMetric("Portfolio Low", history.map(\.value).min().map(privateMoney) ?? "—")
                }.padding()
            })
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
    private var sortedHoldings: [PortfolioHolding] {
        store.holdings.sorted { lhs, rhs in
            switch store.selectedPortfolio?.sort ?? .currentValue {
            case .currentValue: return (lhs.currentValue ?? 0) > (rhs.currentValue ?? 0)
            case .allocation: return lhs.allocation > rhs.allocation
            case .dayChange: return (lhs.dayChangePercent ?? -999_999_999) > (rhs.dayChangePercent ?? -999_999_999)
            case .profitLoss: return (lhs.totalPnL ?? 0) > (rhs.totalPnL ?? 0)
            case .profitLossPercent: return (lhs.pnlPercent ?? -999_999_999) > (rhs.pnlPercent ?? -999_999_999)
            case .asset: return lhs.asset.name.localizedCaseInsensitiveCompare(rhs.asset.name) == .orderedAscending
            }
        }
    }
    private var history: [PortfolioSnapshot] {
        let values = store.snapshot.historicalSnapshots.filter { store.selectedPortfolioIDs.contains($0.portfolioID) }
        guard store.selectedPortfolio == nil else { return values.sorted { $0.timestamp < $1.timestamp } }
        let calendar = Calendar(identifier: .gregorian)
        return Dictionary(grouping: values, by: { calendar.startOfDay(for: $0.timestamp) }).map { date, points in
            PortfolioSnapshot(
                portfolioID: UUID(), timestamp: date, value: points.reduce(0) { $0 + $1.value },
                netContributions: points.reduce(0) { $0 + $1.netContributions },
                realizedPnL: points.reduce(0) { $0 + $1.realizedPnL },
                unrealizedPnL: points.reduce(0) { $0 + $1.unrealizedPnL }, isComplete: points.allSatisfy(\.isComplete))
        }.sorted { $0.timestamp < $1.timestamp }
    }
    private var filteredTransactions: [PortfolioTransaction] {
        store.transactions.filter { tx in
            (transactionFilter == nil || tx.type == transactionFilter)
                && (transactionSearch.isEmpty
                    || [tx.asset.name, tx.asset.symbol, tx.notes, tx.source.rawValue].contains {
                        $0.localizedCaseInsensitiveContains(transactionSearch)
                    })
        }.sorted { $0.timestamp > $1.timestamp }
    }
    private func openInitialAssetIfNeeded() { if let initialAsset { beginTransaction(for: initialAsset) } }
    private func beginTransaction(for asset: PortfolioAsset) {
        guard !store.activePortfolios.isEmpty else {
            store.lastError = "Create a portfolio before adding a transaction."
            return
        }
        editor = .new(asset: asset, portfolioID: destinationPortfolioID)
    }
    private func privateMoney(_ value: Decimal) -> String {
        PortfolioPrivacy.sensitive(money(value), enabled: store.privacyMode)
    }
    private func money(_ value: Decimal) -> String {
        let code = store.selectedPortfolio?.baseCurrency.rawValue ?? "USD"
        return value.formatted(.currency(code: code).precision(.fractionLength(2)))
    }
    private func percent(_ value: Decimal) -> String { value.formatted(.percent.precision(.fractionLength(2))) }
    private func metric(_ title: String, _ value: Decimal) -> some View {
        textMetric(title, privateMoney(value)).foregroundStyle(value >= 0 ? Color.primary : Color.red)
    }
    private func textMetric(_ title: String, _ value: String) -> some View {
        VStack(alignment: .leading) {
            Text(title).font(.caption).foregroundStyle(.secondary)
            Text(value).font(.title3).bold()
        }.padding().frame(maxWidth: .infinity, alignment: .leading).background(
            .quaternary.opacity(0.4), in: RoundedRectangle(cornerRadius: 10)
        ).eraseToAnyView()
    }
    private func holdingRows(_ holdings: [PortfolioHolding]) -> some View {
        ScrollView {
            LazyVStack(spacing: 8) {
                ForEach(holdings) { holding in
                    HStack {
                        Text(holding.asset.symbol).bold()
                        Spacer()
                        Text(percent(holding.allocation))
                        Text(holding.currentValue.map(privateMoney) ?? "—")
                            .frame(width: 110, alignment: .trailing)
                    }
                }
            }
        }
    }
}

private extension View { func eraseToAnyView() -> AnyView { AnyView(self) } }

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

private struct TransactionEditorSheet: View {
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
            Text("\(context.transaction.asset.name) · \(context.transaction.asset.source.rawValue)").foregroundStyle(
                .secondary)
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

private struct PortfolioCreateSheet: View {
    @ObservedObject var store: PortfolioStore
    @Environment(\.dismiss) var dismiss
    @State var name = ""
    @State var currency = PortfolioCurrency.USD
    var body: some View {
        VStack(spacing: 16) {
            Text("Create Portfolio").font(.headline)
            TextField("Name", text: $name)
            Picker("Base Currency", selection: $currency) {
                ForEach(PortfolioCurrency.allCases) { Text($0.rawValue).tag($0) }
            }
            HStack {
                Button("Cancel") { dismiss() }
                Button("Create") {
                    store.create(name: name, currency: currency)
                    dismiss()
                }.buttonStyle(.borderedProminent).disabled(name.trimmingCharacters(in: .whitespaces).isEmpty)
            }
        }.padding(24).frame(width: 360)
    }
}

private struct PortfolioManageSheet: View {
    @ObservedObject var store: PortfolioStore
    @Environment(\.dismiss) var dismiss
    @State var deleting: Portfolio?
    var body: some View {
        VStack {
            Text("Manage Portfolios").font(.headline)
            List {
                ForEach(store.snapshot.portfolios) { portfolio in
                    HStack {
                        TextField(
                            "Name",
                            text: Binding(
                                get: { portfolio.name },
                                set: {
                                    var copy = portfolio
                                    copy.name = $0
                                    store.update(copy)
                                }))
                        Spacer()
                        Button {
                            store.duplicate(portfolio.id)
                        } label: {
                            Image(systemName: "plus.square.on.square")
                        }
                        Button(role: .destructive) {
                            deleting = portfolio
                        } label: {
                            Image(systemName: "trash")
                        }
                    }
                }.onMove { store.reorder(from: $0, to: $1) }
            }
            Button("Done") { dismiss() }
        }.padding().frame(width: 520, height: 420).confirmationDialog(
            "Delete \(deleting?.name ?? "portfolio")?",
            isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } })
        ) {
            Button("Delete Portfolio and Transactions", role: .destructive) {
                if let id = deleting?.id { store.delete(id) }
                deleting = nil
            }
        }
    }
}

private enum PortfolioHistoryRange: String, CaseIterable, Identifiable {
    case oneDay = "1D"
    case oneWeek = "1W"
    case oneMonth = "1M"
    case oneYear = "1Y"
    case all = "ALL"
    var id: String { rawValue }
    var duration: TimeInterval? {
        switch self {
        case .oneDay: 86_400
        case .oneWeek: 7 * 86_400
        case .oneMonth: 31 * 86_400
        case .oneYear: 365 * 86_400
        case .all: nil
        }
    }
}

private struct PortfolioHistoryChart: View {
    let snapshots: [PortfolioSnapshot]
    let currentValue: Decimal
    let currency: PortfolioCurrency
    let range: PortfolioHistoryRange
    let privacy: Bool
    let isLoading: Bool
    @State private var hoveredIndex: Int?

    private let leftMargin: CGFloat = 12
    private let rightMargin: CGFloat = 76
    private let topMargin: CGFloat = 12
    private let bottomMargin: CGFloat = 28

    private func preparedPoints(now: Date = Date()) -> [PortfolioSnapshot] {
        let ranged = snapshots.filter { point in
            guard let duration = range.duration else { return true }
            return point.timestamp >= now.addingTimeInterval(-duration)
        }
        guard !isLoading, let latest = ranged.last ?? snapshots.last else { return ranged }
        let live = PortfolioSnapshot(
            portfolioID: latest.portfolioID, timestamp: now, value: currentValue,
            netContributions: latest.netContributions, realizedPnL: latest.realizedPnL,
            unrealizedPnL: currentValue - latest.netContributions, isComplete: latest.isComplete
        )
        return ranged + [live]
    }

    var body: some View {
        let points = preparedPoints()
        GeometryReader { geometry in
            Canvas { context, size in draw(context: &context, size: size, points: points) }
                .contentShape(Rectangle())
                .onContinuousHover { phase in
                    guard !isLoading, !privacy, !points.isEmpty else {
                        hoveredIndex = nil
                        return
                    }
                    switch phase {
                    case .active(let location):
                        let width = max(1, geometry.size.width - leftMargin - rightMargin)
                        let fraction = ((location.x - leftMargin) / width).clamped(to: 0...1)
                        let index = points.count == 1 ? 0 : Int((fraction * CGFloat(points.count - 1)).rounded())
                        if hoveredIndex != index { hoveredIndex = index }
                    case .ended: hoveredIndex = nil
                    }
                }
        }
        .overlay {
            if isLoading {
                HStack(spacing: 8) {
                    ProgressView().controlSize(.small)
                    Text("Loading market data…").font(.callout)
                }
                .padding(.horizontal, 14)
                .padding(.vertical, 10)
                .background(.thinMaterial, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                .accessibilityElement(children: .combine)
                .accessibilityLabel("Loading market data")
            } else if privacy {
                Text("••••••••").font(.title)
            } else if points.isEmpty {
                Text("History builds from transaction dates and available market candles.").foregroundStyle(.secondary)
            }
        }
        .animation(.easeOut(duration: 0.25), value: isLoading)
        .accessibilityLabel(
            isLoading
                ? "Loading market data"
                : privacy ? "Portfolio history values hidden" : "Portfolio value history, \(points.count) observations")
    }

    private func draw(context: inout GraphicsContext, size: CGSize, points: [PortfolioSnapshot]) {
        guard !isLoading, !privacy, !points.isEmpty else { return }
        let plot = CGRect(
            x: leftMargin, y: topMargin,
            width: max(1, size.width - leftMargin - rightMargin),
            height: max(1, size.height - topMargin - bottomMargin))
        let values = points.map(\.value)
        guard var minimum = values.min(), var maximum = values.max() else { return }
        if minimum == maximum {
            minimum -= 1
            maximum += 1
        }
        let padding = (maximum - minimum) * Decimal(string: "0.08")!
        minimum -= padding
        maximum += padding
        let spread = maximum - minimum

        func position(_ index: Int) -> CGPoint {
            let x = points.count == 1 ? plot.midX : plot.minX + plot.width * CGFloat(index) / CGFloat(points.count - 1)
            let normalized = CGFloat(((points[index].value - minimum) / spread).doubleValue)
            return CGPoint(x: x, y: plot.maxY - plot.height * normalized)
        }

        for tick in 0...4 {
            let fraction = CGFloat(tick) / 4
            let y = plot.maxY - plot.height * fraction
            var grid = Path()
            grid.move(to: CGPoint(x: plot.minX, y: y))
            grid.addLine(to: CGPoint(x: plot.maxX, y: y))
            context.stroke(grid, with: .color(.secondary.opacity(0.18)), lineWidth: 1)
            let value = minimum + spread * Decimal(Double(fraction))
            context.draw(
                Text(shortMoney(value)).font(.caption2).foregroundStyle(.secondary),
                at: CGPoint(x: plot.maxX + 6, y: y), anchor: .leading)
        }

        if points.count > 1 {
            for index in 1..<points.count {
                var segment = Path()
                segment.move(to: position(index - 1))
                segment.addLine(to: position(index))
                let profit = points[index].value >= points[index].netContributions
                context.stroke(segment, with: .color(profit ? .green : .red), lineWidth: 2.25)
            }
        } else {
            let profit = points[0].value >= points[0].netContributions
            context.fill(
                Path(ellipseIn: CGRect(x: position(0).x - 3, y: position(0).y - 3, width: 6, height: 6)),
                with: .color(profit ? .green : .red))
        }

        let labelIndices = Array(Set([0, max(0, points.count / 2), max(0, points.count - 1)])).sorted()
        for index in labelIndices {
            context.draw(
                Text(dateLabel(points[index].timestamp)).font(.caption2).foregroundStyle(.secondary),
                at: CGPoint(x: position(index).x, y: plot.maxY + 8), anchor: .top)
        }

        guard let index = hoveredIndex, points.indices.contains(index) else { return }
        let selected = points[index]
        let selectedPosition = position(index)
        var vertical = Path()
        vertical.move(to: CGPoint(x: selectedPosition.x, y: plot.minY))
        vertical.addLine(to: CGPoint(x: selectedPosition.x, y: plot.maxY))
        var horizontal = Path()
        horizontal.move(to: CGPoint(x: plot.minX, y: selectedPosition.y))
        horizontal.addLine(to: CGPoint(x: plot.maxX, y: selectedPosition.y))
        context.stroke(
            vertical, with: .color(.secondary.opacity(0.65)), style: StrokeStyle(lineWidth: 1, dash: [4, 3]))
        context.stroke(
            horizontal, with: .color(.secondary.opacity(0.45)), style: StrokeStyle(lineWidth: 1, dash: [4, 3]))
        context.fill(
            Path(ellipseIn: CGRect(x: selectedPosition.x - 4, y: selectedPosition.y - 4, width: 8, height: 8)),
            with: .color(selected.value >= selected.netContributions ? .green : .red))
        let tooltip = "\(money(selected.value))\n\(selected.timestamp.formatted(date: .abbreviated, time: .shortened))"
        let tooltipX = min(plot.maxX - 70, max(plot.minX + 70, selectedPosition.x))
        context.draw(
            Text(tooltip).font(.caption).foregroundStyle(.primary),
            at: CGPoint(x: tooltipX, y: max(plot.minY + 20, selectedPosition.y - 24)), anchor: .bottom)
    }

    private func money(_ value: Decimal) -> String {
        value.formatted(.currency(code: currency.rawValue).precision(.fractionLength(2)))
    }
    private func shortMoney(_ value: Decimal) -> String {
        let absolute = abs(value)
        if absolute >= 1_000_000 { return "\(money(value / 1_000_000))M" }
        if absolute >= 1_000 { return "\(money(value / 1_000))K" }
        return money(value)
    }
    private func dateLabel(_ date: Date) -> String {
        switch range {
        case .oneDay: date.formatted(date: .omitted, time: .shortened)
        case .oneWeek, .oneMonth: date.formatted(.dateTime.month(.abbreviated).day())
        case .oneYear, .all: date.formatted(.dateTime.month(.abbreviated).year())
        }
    }
}

private struct PortfolioAllocationChart: View {
    let holdings: [PortfolioHolding]
    let privacy: Bool
    private var sortedHoldings: [PortfolioHolding] { holdings.sorted { $0.allocation > $1.allocation } }

    var body: some View {
        HStack {
            Canvas { context, size in
                var start = Angle.degrees(-90)
                let colors: [Color] = [.blue, .orange, .green, .purple, .pink, .cyan]
                for (index, holding) in sortedHoldings.enumerated() {
                    let end = start + .degrees(holding.allocation.doubleValue * 360)
                    var path = Path()
                    let rect = CGRect(origin: .zero, size: size).insetBy(dx: 20, dy: 20)
                    path.addArc(
                        center: CGPoint(x: size.width / 2, y: size.height / 2),
                        radius: min(rect.width, rect.height) / 2,
                        startAngle: start, endAngle: end, clockwise: false)
                    context.stroke(path, with: .color(colors[index % colors.count]), lineWidth: 28)
                    start = end
                }
            }
            .frame(width: 200)

            ScrollView {
                LazyVStack(alignment: .leading, spacing: 8) {
                    ForEach(sortedHoldings) { holding in
                        HStack {
                            Text(holding.asset.symbol)
                            Spacer()
                            Text(
                                privacy ? "••••" : holding.allocation.formatted(.percent.precision(.fractionLength(1))))
                        }
                        .accessibilityLabel(
                            privacy
                                ? "\(holding.asset.name), allocation hidden"
                                : "\(holding.asset.name), \(holding.allocation.formatted(.percent)) of portfolio")
                    }
                }
            }
        }
    }
}

private struct PortfolioAssetDetailView: View {
    @ObservedObject var store: PortfolioStore
    let holding: PortfolioHolding
    @Environment(\.dismiss) private var dismiss
    @State private var editor: TransactionEditorContext?
    @State private var deleting: PortfolioTransaction?

    private var transactions: [PortfolioTransaction] {
        store.transactions.filter { $0.asset.key == holding.asset.key }.sorted { $0.timestamp > $1.timestamp }
    }
    private func hidden(_ value: String) -> String { PortfolioPrivacy.sensitive(value, enabled: store.privacyMode) }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                VStack(alignment: .leading) {
                    Text(holding.asset.name).font(.title).bold()
                    Text("\(holding.asset.symbol) · \(holding.asset.source.rawValue)").foregroundStyle(.secondary)
                }
                Spacer()
                Button("Done") { dismiss() }
            }
            HStack {
                detail("Current Holdings", hidden("\(holding.quantity) \(holding.asset.symbol)"))
                detail("Current Value", hidden(holding.currentValue?.description ?? "Unavailable"))
                detail("Average Cost", hidden(holding.averageCost.description))
                detail("Current Price", holding.currentPrice.map { hidden($0.description) } ?? "Unavailable")
            }
            HStack {
                detail("Unrealized P&L", hidden(holding.unrealizedPnL?.description ?? "—"))
                detail("Realized P&L", hidden(holding.realizedPnL.description))
                detail("Total P&L", hidden(holding.totalPnL?.description ?? "—"))
                detail("Allocation", holding.allocation.formatted(.percent.precision(.fractionLength(2))))
            }
            Text("Transactions").font(.headline)
            List(transactions) { tx in
                HStack {
                    Text(tx.type.rawValue).frame(width: 110, alignment: .leading)
                    Text(hidden(tx.quantity.description)).frame(width: 110)
                    Text(tx.price.map { hidden($0.description) } ?? "—").frame(width: 100)
                    Text(tx.timestamp.formatted(date: .abbreviated, time: .shortened))
                    Text(tx.notes).frame(maxWidth: .infinity, alignment: .leading)
                    Menu {
                        Button("Edit") { editor = .edit(tx) }
                        Button("Duplicate") { editor = .duplicate(tx) }
                        Button("Delete", role: .destructive) { deleting = tx }
                    } label: {
                        Image(systemName: "ellipsis.circle")
                    }
                }
                .accessibilityElement(children: .combine)
                .accessibilityLabel(
                    Text(
                        verbatim: store.privacyMode
                            ? "\(holding.asset.name) transaction, financial values hidden"
                            : "\(tx.type.rawValue), \(tx.quantity) \(holding.asset.symbol)"
                    ))
            }
        }.padding(24).frame(width: 850, height: 600)
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

    private func detail(_ title: String, _ value: String) -> some View {
        VStack(alignment: .leading) {
            Text(title).font(.caption).foregroundStyle(.secondary)
            Text(value).font(.headline)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(store.privacyMode ? "\(title), hidden" : "\(title), \(value)")
    }
}

private struct PortfolioImportPreviewSheet: View {
    @ObservedObject var store: PortfolioStore
    let preview: PortfolioCSVPreview
    @Environment(\.dismiss) private var dismiss
    @State private var isImporting = false
    @State private var importFailure: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Import Preview").font(.headline)
            Text("\(preview.transactions.count) transactions ready to import")

            if !preview.errors.isEmpty {
                Text("\(preview.errors.count) require attention").foregroundStyle(.red)
                List(preview.errors, id: \.self) { Text($0) }
            }
            if !preview.warnings.isEmpty {
                DisclosureGroup(
                    "\(preview.warnings.count) warning\(preview.warnings.count == 1 ? "" : "s") — warnings do not block import"
                ) {
                    ForEach(preview.warnings, id: \.self) { Text($0).font(.caption) }
                }
            }
            if let importFailure {
                VStack(alignment: .leading, spacing: 5) {
                    Label("Import failed", systemImage: "exclamationmark.triangle.fill").font(.headline)
                    Text(importFailure).textSelection(.enabled)
                    Text(
                        "No transactions were imported. Correct the source data, or restart the CoinMarketCap import and skip the affected ticker or item."
                    )
                    .font(.caption).foregroundStyle(.secondary)
                }
                .foregroundStyle(.red)
                .padding(10)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(.red.opacity(0.08), in: RoundedRectangle(cornerRadius: 8))
            }

            Spacer()
            HStack {
                if isImporting {
                    ProgressView()
                    Text("Validating and importing…").foregroundStyle(.secondary)
                }
                Spacer()
                Button("Cancel") { dismiss() }.disabled(isImporting)
                Button("Import") { beginImport() }
                    .buttonStyle(.borderedProminent)
                    .disabled(!preview.isValid || preview.transactions.isEmpty || isImporting)
            }
        }
        .padding(24)
        .frame(width: 560, height: 430)
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

private struct CoinMarketCapImportSheet: View {
    @ObservedObject var store: PortfolioStore
    let preview: CoinMarketCapCSVPreview
    let portfolioID: UUID
    let onReady: (PortfolioCSVPreview) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var mappings: [String: PortfolioAsset] = [:]
    @State private var skippedSymbols: Set<String> = []
    @State private var mappingSymbol: String?
    @State private var resolvingSymbols: Set<String> = []
    @State private var autoMappedSymbols: Set<String> = []
    @State private var didAutoMap = false
    @State private var feeFXRateText: [String: String] = [:]
    @State private var skippedRowIDs: Set<String> = []

    private var mappedRows: Int {
        preview.rows.filter { mappings[$0.token] != nil && !skippedRowIDs.contains($0.id) }.count
    }
    private var skippedRows: Int { preview.rows.count - mappedRows }
    private var portfolioCurrency: PortfolioCurrency {
        store.snapshot.portfolios.first { $0.id == portfolioID }?.baseCurrency ?? .USD
    }
    private var foreignFeeRows: [CoinMarketCapCSVRow] {
        preview.rows.filter { row in
            mappings[row.token] != nil && !skippedRowIDs.contains(row.id) && row.fee != 0
                && (row.feeCurrency ?? .USD) != .USD
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Import from CoinMarketCap").font(.headline)
            Text("\(preview.rows.count) transactions · \(preview.symbols.count) assets")
            if !preview.errors.isEmpty {
                Label(
                    "\(preview.errors.count) CSV errors require attention", systemImage: "exclamationmark.triangle.fill"
                ).foregroundStyle(.red)
                List(preview.errors, id: \.self) { Text($0).font(.caption) }.frame(maxHeight: 100)
            }
            Text("Map CoinMarketCap tokens").font(.subheadline).bold()
            Text(
                "Only transactions for mapped tickers will be imported. Leave a ticker skipped to exclude all of its transactions."
            )
            .font(.caption).foregroundStyle(.secondary)
            if !didAutoMap {
                HStack {
                    ProgressView().controlSize(.small)
                    Text(
                        "Finding \(portfolioCurrency.rawValue) markets — Binance first, then CoinGecko and DEXScreener…"
                    ).font(.caption).foregroundStyle(.secondary)
                }
            }
            List(preview.symbols, id: \.self) { symbol in
                HStack {
                    Text(symbol).bold().frame(width: 90, alignment: .leading)
                    if resolvingSymbols.contains(symbol) {
                        ProgressView().controlSize(.small)
                        Text("Searching Binance…").foregroundStyle(.secondary)
                    } else if let asset = mappings[symbol] {
                        Text("\(asset.name) · \(asset.source.rawValue)").foregroundStyle(.secondary)
                        if autoMappedSymbols.contains(symbol) {
                            Text("Auto-mapped").font(.caption2).padding(.horizontal, 6).padding(.vertical, 2)
                                .background(.blue.opacity(0.12), in: Capsule())
                        }
                    } else if skippedSymbols.contains(symbol) {
                        Text("Skipped — will not be imported").foregroundStyle(.secondary)
                    } else {
                        Text("Not mapped — will be skipped").foregroundStyle(.orange)
                    }
                    Spacer()
                    if mappings[symbol] != nil || skippedSymbols.contains(symbol) {
                        Button("Reset") {
                            mappings[symbol] = nil
                            skippedSymbols.remove(symbol)
                        }
                    }
                    Button(mappings[symbol] == nil ? "Map…" : "Change…") {
                        skippedSymbols.remove(symbol)
                        autoMappedSymbols.remove(symbol)
                        mappingSymbol = symbol
                    }
                    if mappings[symbol] == nil && !skippedSymbols.contains(symbol) {
                        Button("Skip") { skippedSymbols.insert(symbol) }
                    }
                }
            }
            if !foreignFeeRows.isEmpty {
                GroupBox("Missing Historical FX") {
                    VStack(alignment: .leading, spacing: 8) {
                        Text(
                            "Enter the fee-currency value in USD at the transaction time. For example, if €1 equaled $1.08, enter 1.08. You can skip an individual transaction instead."
                        )
                        .font(.caption).foregroundStyle(.secondary)
                        ScrollView {
                            VStack(spacing: 8) {
                                ForEach(foreignFeeRows) { row in
                                    HStack {
                                        Text(
                                            verbatim:
                                                "Line \(row.line) · \(row.token) · \(row.fee) \(row.feeCurrency?.rawValue ?? "")"
                                        )
                                        .frame(maxWidth: .infinity, alignment: .leading)
                                        TextField(
                                            "USD per \(row.feeCurrency?.rawValue ?? "unit")",
                                            text: Binding(
                                                get: { feeFXRateText[row.id] ?? "" },
                                                set: { feeFXRateText[row.id] = $0 }
                                            )
                                        ).frame(width: 145)
                                        Button("Skip Item") { skippedRowIDs.insert(row.id) }
                                    }
                                }
                            }
                        }.frame(maxHeight: 140)
                    }.padding(6)
                }
            }
            if !skippedRowIDs.isEmpty {
                Button(
                    "Restore \(skippedRowIDs.count) individually skipped transaction\(skippedRowIDs.count == 1 ? "" : "s")"
                ) { skippedRowIDs.removeAll() }
                .font(.caption)
            }
            HStack {
                Label("\(mappedRows) transactions will be imported", systemImage: "checkmark.circle.fill")
                    .foregroundStyle(.green)
                Spacer()
                Label("\(skippedRows) transactions will be skipped", systemImage: "minus.circle").foregroundStyle(
                    .secondary)
            }.font(.callout)
            if !preview.warnings.isEmpty {
                DisclosureGroup("\(preview.warnings.count) warnings") {
                    ScrollView {
                        VStack(alignment: .leading) {
                            ForEach(preview.warnings, id: \.self) { Text($0).font(.caption) }
                        }
                    }.frame(maxHeight: 90)
                }
            }
            HStack {
                Spacer()
                Button("Cancel") { dismiss() }
                Button("Continue to Preview") {
                    let rates = feeFXRateText.reduce(into: [String: Decimal]()) { result, item in
                        if let rate = Decimal(string: item.value), rate > 0 { result[item.key] = rate }
                    }
                    onReady(
                        preview.transactions(
                            portfolioID: portfolioID, mappings: mappings,
                            feeFXRates: rates, skippedRowIDs: skippedRowIDs))
                }
                .buttonStyle(.borderedProminent)
                .disabled(!preview.isValid || mappedRows == 0 || !didAutoMap)
            }
        }
        .padding(24).frame(width: 650, height: 560)
        .sheet(isPresented: Binding(get: { mappingSymbol != nil }, set: { if !$0 { mappingSymbol = nil } })) {
            AddTickerSheet(title: "Map \(mappingSymbol ?? "Token")", actionLabel: "Use Asset") { result in
                guard let symbol = mappingSymbol else { return }
                mappings[symbol] = PortfolioAsset(searchResult: result)
                skippedSymbols.remove(symbol)
                autoMappedSymbols.remove(symbol)
                mappingSymbol = nil
            }
        }
        .task {
            applyExistingMappings()
            await autoMapRemainingSymbols()
        }
    }

    private func applyExistingMappings() {
        for symbol in preview.symbols {
            let assets = Set(
                store.snapshot.transactions.filter { $0.asset.symbol.caseInsensitiveCompare(symbol) == .orderedSame }
                    .map(\.asset))
            if assets.count == 1 { mappings[symbol] = assets.first }
        }
    }

    private func autoMapRemainingSymbols() async {
        guard !didAutoMap else { return }
        for symbol in preview.symbols where mappings[symbol] == nil && !skippedSymbols.contains(symbol) {
            resolvingSymbols.insert(symbol)
            if let asset = await PortfolioAssetAutoMapper.resolve(symbol: symbol, baseCurrency: portfolioCurrency),
                mappings[symbol] == nil, !skippedSymbols.contains(symbol)
            {
                mappings[symbol] = asset
                autoMappedSymbols.insert(symbol)
            }
            resolvingSymbols.remove(symbol)
        }
        didAutoMap = true
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
