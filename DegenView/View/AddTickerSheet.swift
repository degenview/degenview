import SwiftUI

struct AddTickerSheet: View {
    let title: String
    let actionLabel: String
    let onAdd: @MainActor (TickerSearchResult) async throws -> Void
    let onAddPortfolio: (@MainActor (PortfolioChartConfig) -> Void)?

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

    @Environment(\.dismiss) private var dismiss
    @Environment(\.openSettings) private var openSettings

    init(
        title: String = "Add Chart",
        actionLabel: String = "Add",
        onAddPortfolio: (@MainActor (PortfolioChartConfig) -> Void)? = nil,
        onAdd: @escaping @MainActor (TickerSearchResult) async throws -> Void
    ) {
        self.title = title
        self.actionLabel = actionLabel
        self.onAddPortfolio = onAddPortfolio
        self.onAdd = onAdd
    }

    private let suggestions = ["BTC", "ETH", "SOL", "BNB", "XRP", "ADA", "DOGE", "AVAX", "DOT", "LINK"]
    private let stockSuggestions = ["AAPL", "MSFT", "NVDA", "AMZN", "GOOGL", "META", "TSLA", "SPY", "QQQ", "AMD"]

    enum Tab: String, CaseIterable {
        case crypto = "Crypto"
        case stocks = "Stocks"
        case polymarket = "Polymarket"
        case portfolio = "Portfolio"
    }

    /// Whichever pane is showing owns the selection the Add button commits.
    private var activeSelection: TickerSearchResult? {
        switch selectedTab {
        case .crypto: return searchVM.selectedResult
        case .stocks: return stockVM.selectedResult
        case .polymarket: return polymarketVM.selectedResult
        case .portfolio: return nil
        }
    }

    var body: some View {
        VStack(spacing: 16) {
            // Header
            Text(title)
                .font(.headline)

            Picker("", selection: $selectedTab) {
                ForEach(Tab.allCases.filter { $0 != .portfolio || onAddPortfolio != nil }, id: \.self) { tab in
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
                    resultsMaxHeight: UI.addTickerResultsMaxHeight
                )
            case .portfolio:
                portfolioTab
            }

            // Error from add attempt
            if let error = addError {
                HStack {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .foregroundStyle(.orange)
                    Text(error)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }

            if needsAlpacaSetup {
                HStack {
                    Text("Set up Alpaca before adding a stock chart.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    Spacer()
                    Button("Open Settings") {
                        UserDefaults.standard.set(SettingsTab.alpaca.rawValue, forKey: "settingsTab")
                        openSettings()
                    }
                }
                .padding(10)
                .background(.orange.opacity(0.12), in: RoundedRectangle(cornerRadius: 8))
            }

            // Action buttons
            HStack(spacing: 12) {
                Button("Cancel") {
                    cancelSearches()
                    dismiss()
                }
                .buttonStyle(.plain)
                .keyboardShortcut(.escape)

                Button(actionLabel) {
                    if selectedTab == .portfolio { addPortfolio() } else { addTicker() }
                }
                .buttonStyle(.borderedProminent)
                .disabled(selectedTab == .portfolio ? portfolioStore.activePortfolios.isEmpty : activeSelection == nil)
                .keyboardShortcut(.return)
            }
            .padding(.top, 8)
        }
        .padding(24)
        .frame(width: UI.addTickerSheetWidth)
        .onChange(of: selectedTab) { addError = nil }
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
                .frame(maxWidth: .infinity, minHeight: 220)
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
                Spacer(minLength: 30)
            }
        }
        .frame(minHeight: 260)
        .task { await portfolioStore.refresh() }
    }

    private func addPortfolio() {
        guard let onAddPortfolio else { return }
        onAddPortfolio(.init(portfolioID: portfolioID, kind: portfolioKind))
        dismiss()
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
                List(results) { result in
                    SearchResultRow(result: result, isSelected: stockVM.selectedResult == result) {
                        stockVM.selectedResult = result
                    }
                }
                .listStyle(.inset)
                .frame(minHeight: UI.addTickerResultsMinHeight, maxHeight: UI.addTickerResultsMaxHeight)
            }

            if let selected = stockVM.selectedResult {
                SelectedResultBanner(prefix: "Selected", result: selected)
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
                                        onSelect: { searchVM.selectedResult = result }
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
                .frame(minHeight: UI.addTickerResultsMinHeight, maxHeight: UI.addTickerResultsMaxHeight)
            }

            // No results
            if !searchVM.isSearching && !inputText.trimmingCharacters(in: .whitespaces).isEmpty
                && searchVM.searchResults.isEmpty
            {
                Text("No results found")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            // Selected result
            if let selected = searchVM.selectedResult {
                SelectedResultBanner(prefix: "Selected", result: selected)
            }
        }
    }

    // MARK: - Add

    private func cancelSearches() {
        searchVM.cancelSearch()
        stockVM.cancelSearch()
        polymarketVM.cancelSearch()
    }

    private func addTicker() {
        guard let selected = activeSelection else { return }

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
