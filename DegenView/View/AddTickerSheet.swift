import SwiftUI

struct AddTickerSheet: View {
    let title: String
    let actionLabel: String
    let onAdd: @MainActor (TickerSearchResult) async throws -> Void
    let onAddPortfolio: (@MainActor (PortfolioChartConfig) -> Void)?
    let onAddCoinMarketCap: (@MainActor (CoinMarketCapChartConfig) -> Void)?

    @StateObject private var searchVM = TickerSearchViewModel(logPrefix: "[AddTicker]")
    @StateObject private var stockVM = TickerSearchViewModel(
        logPrefix: "[AddTicker/Stocks]",
        sources: { [DataSourceFactory.shared.alpaca] }
    )
    @StateObject private var polymarketVM = PolymarketSearchViewModel(logPrefix: "[AddTicker/Polymarket]")

    @State private var selectedTab: Tab = .crypto
    @State private var inputText = ""
    @State private var polymarketText = ""
    @State private var stockText = ""
    @State private var addError: String?
    @State private var needsAlpacaSetup = false
    @StateObject private var portfolioStore = PortfolioStore.shared
    @State private var portfolioID: UUID?
    @State private var portfolioKind: PortfolioChartKind = .valueChart
    @State private var cmcType: CoinMarketCapChartType = .altcoinSeasonHistorical

    @Environment(\.dismiss) private var dismiss
    @Environment(\.openSettings) private var openSettings

    init(
        title: String = "Add Chart",
        actionLabel: String = "Add",
        onAddPortfolio: (@MainActor (PortfolioChartConfig) -> Void)? = nil,
        onAddCoinMarketCap: (@MainActor (CoinMarketCapChartConfig) -> Void)? = nil,
        onAdd: @escaping @MainActor (TickerSearchResult) async throws -> Void
    ) {
        self.title = title
        self.actionLabel = actionLabel
        self.onAddPortfolio = onAddPortfolio
        self.onAddCoinMarketCap = onAddCoinMarketCap
        self.onAdd = onAdd
    }

    private let suggestions = ["BTC", "ETH", "SOL", "BNB", "XRP", "ADA", "DOGE", "AVAX", "DOT", "LINK"]
    private let stockSuggestions = ["AAPL", "MSFT", "NVDA", "AMZN", "GOOGL", "META", "TSLA", "SPY", "QQQ", "AMD"]

    enum Tab: String, CaseIterable {
        case crypto = "Crypto"
        case stocks = "Stocks"
        case polymarket = "Polymarket"
        case coinMarketCap = "CoinMarketCap"
        case portfolio = "Portfolio"
    }

    /// Whichever pane is showing owns the selection the Add button commits.
    private var activeSelection: TickerSearchResult? {
        switch selectedTab {
        case .crypto: return searchVM.selectedResult
        case .stocks: return stockVM.selectedResult
        case .polymarket: return polymarketVM.selectedResult
        case .coinMarketCap, .portfolio: return nil
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(title)
                .font(.headline)

            Picker("", selection: $selectedTab) {
                ForEach(Tab.allCases.filter {
                    ($0 != .portfolio || onAddPortfolio != nil) && ($0 != .coinMarketCap || onAddCoinMarketCap != nil)
                }, id: \.self) { tab in
                    Text(tab.rawValue).tag(tab)
                }
            }
            .pickerStyle(.segmented)

            switch selectedTab {
            case .crypto:
                cryptoTab
            case .stocks:
                stockTab
            case .polymarket:
                PolymarketSearchPane(
                    searchVM: polymarketVM,
                    searchText: $polymarketText,
                    resultsMaxHeight: UI.addTickerResultsMaxHeight,
                    usesAdaptiveResultHeight: true,
                    showsStatus: false,
                    onCommitResult: { addTicker($0) }
                )
            case .coinMarketCap:
                coinMarketCapTab
            case .portfolio:
                portfolioTab
            }

            statusRows

            Divider()

            HStack(spacing: 12) {
                if let selected = activeSelection {
                    SelectedResultBanner(prefix: "Selected", result: selected)
                        .frame(maxWidth: 340)
                }
                Spacer(minLength: 0)
                Button("Cancel") {
                    cancelSearches()
                    dismiss()
                }
                .keyboardShortcut(.escape)

                Button(actionLabel) {
                    if selectedTab == .portfolio { addPortfolio() }
                    else if selectedTab == .coinMarketCap { addCoinMarketCap() }
                    else if let selected = activeSelection {
                        addTicker(selected)
                    }
                }
                .buttonStyle(.borderedProminent)
                .disabled(selectedTab == .portfolio ? portfolioStore.activePortfolios.isEmpty :
                    selectedTab == .coinMarketCap ? false : activeSelection == nil)
                .keyboardShortcut(.return)
            }
        }
        .padding(24)
        .frame(width: UI.addTickerSheetWidth)
        .fixedSize(horizontal: false, vertical: true)
        .animation(.easeInOut(duration: 0.18), value: selectedTab)
        .animation(.easeInOut(duration: 0.18), value: searchVM.searchResults.values.reduce(0) { $0 + $1.count })
        .animation(.easeInOut(duration: 0.18), value: stockVM.searchResults.values.reduce(0) { $0 + $1.count })
        .animation(.easeInOut(duration: 0.18), value: polymarketVM.groups.reduce(0) { $0 + $1.results.count })
        .onChange(of: selectedTab) {
            addError = nil
            needsAlpacaSetup = false
        }
        .onDisappear {
            cancelSearches()
        }
    }

    private var portfolioTab: some View {
        VStack(alignment: .leading, spacing: 16) {
            if portfolioStore.activePortfolios.isEmpty {
                ContentUnavailableView(
                    "No Portfolios", systemImage: "briefcase",
                    description: Text("Create a portfolio in the Portfolio Tracker first.")
                )
                .frame(maxWidth: .infinity, minHeight: 150)
            } else {
                Picker("Portfolio", selection: $portfolioID) {
                    Text("All Portfolios").tag(UUID?.none)
                    ForEach(portfolioStore.activePortfolios) { portfolio in
                        Text(portfolio.name).tag(Optional(portfolio.id))
                    }
                }
                Picker("Show", selection: $portfolioKind) {
                    ForEach(PortfolioChartKind.allCases) { kind in Text(kind.rawValue).tag(kind) }
                }
                .pickerStyle(.radioGroup)
                Text(
                    "Portfolio value charts include their own 1D, 1W, 1M, 1Y, and all-time range control. Portfolio cards do not support market indicators."
                )
                .font(.caption).foregroundStyle(.secondary)
            }
        }
        .task { await portfolioStore.refresh() }
    }

    private func addPortfolio() {
        guard let onAddPortfolio else { return }
        onAddPortfolio(.init(portfolioID: portfolioID, kind: portfolioKind))
        dismiss()
    }

    private func addCoinMarketCap() {
        onAddCoinMarketCap?(CoinMarketCapChartConfig(type: cmcType))
        dismiss()
    }

    private var coinMarketCapTab: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Market-wide CoinMarketCap indices").font(.caption).foregroundStyle(.secondary)
            ForEach(CoinMarketCapChartType.allCases) { type in
                Button { cmcType = type } label: {
                    HStack {
                        VStack(alignment: .leading, spacing: 3) {
                            Text(type.title).fontWeight(.semibold)
                            Text(cmcDescription(type)).font(.caption).foregroundStyle(.secondary)
                        }
                        Spacer()
                        Image(systemName: cmcType == type ? "checkmark.circle.fill" : "circle")
                            .foregroundStyle(cmcType == type ? .blue : .secondary)
                    }.padding(10).background(cmcType == type ? Color.accentColor.opacity(0.1) : Color.secondary.opacity(0.06), in: RoundedRectangle(cornerRadius: 8))
                }.buttonStyle(.plain)
            }
        }
    }

    private func cmcDescription(_ type: CoinMarketCapChartType) -> String {
        switch type {
        case .altcoinSeasonHistorical: "Historical 0–100 index chart"
        case .altcoinSeasonLatest: "Current Bitcoin/Altcoin season reading"
        case .fearAndGreedHistorical: "Historical market-sentiment chart"
        case .fearAndGreedLatest: "Current sentiment gauge"
        }
    }

    // MARK: - Stocks Tab

    private var stockTab: some View {
        VStack(spacing: 16) {
            SearchFieldRow(
                placeholder: "US stock symbol or company name (e.g. AAPL)",
                text: $stockText,
                isSearching: stockVM.isSearching,
                onChange: { stockVM.scheduleSearch(query: $0) },
                onSubmit: { stockVM.selectedResult = stockVM.firstAvailableResult }
            )

            if stockVM.searchResults.isEmpty && !stockVM.isSearching {
                VStack(alignment: .leading, spacing: 8) {
                    Text("Popular US stocks and ETFs · free IEX feed")
                        .font(.caption).foregroundStyle(.secondary)
                    LazyVGrid(
                        columns: Array(repeating: .init(.flexible()), count: UI.suggestionGridColumns), spacing: 8
                    ) {
                        ForEach(stockSuggestions, id: \.self) { symbol in
                            Button(symbol) {
                                stockText = symbol
                                stockVM.selectedResult = TickerSearchResult(
                                    symbol: symbol, fullSymbol: symbol, source: .alpaca, price: nil
                                )
                                if AlpacaCredentialsStore.isConfigured { stockVM.scheduleSearch(query: symbol) }
                            }
                            .buttonStyle(.bordered).controlSize(.small)
                        }
                    }
                }
            }

            if let results = stockVM.searchResults[.alpaca], !results.isEmpty {
                List {
                    Section {
                        ForEach(results) { result in
                            SearchResultRow(
                                result: result,
                                isSelected: stockVM.selectedResult == result,
                                onSelect: { stockVM.selectedResult = result },
                                onCommit: { addTicker(result) }
                            )
                        }
                    } header: {
                        Label(DataSourceType.alpaca.displayName, systemImage: DataSourceType.alpaca.icon)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
                .listStyle(.inset)
                .frame(height: UI.searchResultsHeight(rowCount: results.count, sectionCount: 1))
            }
        }
    }

    // MARK: - Crypto Tab

    private var cryptoTab: some View {
        VStack(spacing: 16) {
            SearchFieldRow(
                placeholder: "Ticker symbol (e.g. BTC or PEPE)",
                text: $inputText,
                isSearching: searchVM.isSearching,
                onChange: { searchVM.scheduleSearch(query: $0) },
                onSubmit: {
                    if let first = searchVM.firstAvailableResult {
                        searchVM.selectedResult = first
                    }
                }
            )

            // Suggestions
            if searchVM.searchResults.isEmpty && !searchVM.isSearching {
                VStack(alignment: .leading, spacing: 8) {
                    Text("Suggestions")
                        .font(.caption)
                        .foregroundStyle(.secondary)

                    LazyVGrid(
                        columns: Array(repeating: .init(.flexible()), count: UI.suggestionGridColumns), spacing: 8
                    ) {
                        ForEach(suggestions, id: \.self) { ticker in
                            Button(ticker) {
                                inputText = ticker
                                searchVM.scheduleSearch(query: ticker)
                            }
                            .buttonStyle(.bordered)
                            .controlSize(.small)
                        }
                    }
                }
            }

            // Search results grouped by source
            if !searchVM.searchResults.isEmpty {
                List {
                    ForEach(searchVM.orderedSources, id: \.self) { source in
                        if let results = searchVM.searchResults[source], !results.isEmpty {
                            Section {
                                ForEach(results) { result in
                                    SearchResultRow(
                                        result: result,
                                        isSelected: searchVM.selectedResult == result,
                                        onSelect: { searchVM.selectedResult = result },
                                        onCommit: { addTicker(result) }
                                    )
                                }
                            } header: {
                                Label(source.displayName, systemImage: source.icon)
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                        }
                    }
                }
                .listStyle(.inset)
                .frame(height: UI.searchResultsHeight(
                    rowCount: searchVM.searchResults.values.reduce(0) { $0 + $1.count },
                    sectionCount: searchVM.searchResults.values.filter { !$0.isEmpty }.count
                ))
            }

            // No results
            if !searchVM.isSearching && !inputText.trimmingCharacters(in: .whitespaces).isEmpty
                && searchVM.searchResults.isEmpty
            {
                Text("No results found")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

        }
    }

    @ViewBuilder
    private var statusRows: some View {
        if let error = addError ?? (selectedTab == .polymarket ? polymarketVM.errorMessage : nil) {
            Label(error, systemImage: "exclamationmark.triangle.fill")
                .font(.caption)
                .foregroundStyle(.secondary)
        } else if selectedTab == .polymarket, !polymarketVM.isSearching,
                  !polymarketText.trimmingCharacters(in: .whitespaces).isEmpty,
                  !polymarketVM.hasResults {
            Text("No markets found")
                .font(.caption)
                .foregroundStyle(.secondary)
        }

        if needsAlpacaSetup {
            HStack {
                Label("Set up Alpaca before adding a stock chart.", systemImage: "exclamationmark.triangle.fill")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Spacer()
                Button("Open Settings") {
                    UserDefaults.standard.set(SettingsTab.alpaca.rawValue, forKey: "settingsTab")
                    openSettings()
                }
                .controlSize(.small)
            }
        }
    }

    // MARK: - Add

    private func cancelSearches() {
        searchVM.cancelSearch()
        stockVM.cancelSearch()
        polymarketVM.cancelSearch()
    }

    private func addTicker(_ selected: TickerSearchResult) {
        if selected.source == .alpaca, !AlpacaCredentialsStore.isConfigured {
            needsAlpacaSetup = true
            return
        }

        Task { @MainActor in
            do {
                // Polymarket search already handed us the market artwork; seed the
                // resolver so the new card paints it without another round trip.
                if selected.source == .polymarket {
                    await IconResolver.shared.remember(
                        ticker: selected.fullSymbol,
                        source: .polymarket,
                        url: selected.imageURL
                    )
                }

                try await onAdd(selected)
                dismiss()
            } catch {
                addError = error.localizedDescription
            }
        }
    }
}

#Preview {
    AddTickerSheet { _ in }
}
