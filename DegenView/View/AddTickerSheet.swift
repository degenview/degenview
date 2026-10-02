import SwiftUI

struct AddTickerSheet: View {
    let title: String
    let actionLabel: String
    let subtitle: String
    let systemImage: String
    let onAdd: @MainActor (TickerSearchResult) async throws -> Void
    let onAddPortfolio: (@MainActor (PortfolioChartConfig) -> Void)?
    let onAddCoinMarketCap: (@MainActor (CoinMarketCapChartConfig) -> Void)?
    let onAddBitcoinPowerLaw: (@MainActor () -> Void)?

    @StateObject private var searchVM = TickerSearchViewModel(logPrefix: "[AddTicker]")
    @StateObject private var stockVM = TickerSearchViewModel(
        logPrefix: "[AddTicker/Stocks]",
        sources: { [DataSourceFactory.shared.alpaca] }
    )
    @StateObject private var polymarketVM = PredictionMarketSearchViewModel(
        provider: .polymarket, logPrefix: "[AddTicker/Polymarket]")
    @StateObject private var kalshiVM = PredictionMarketSearchViewModel(
        provider: .kalshi, logPrefix: "[AddTicker/Kalshi]")

    @State private var selectedTab: Tab = .crypto
    @State private var inputText = ""
    @State private var predictionMarketText = ""
    @State private var predictionProvider: DataSourceType = .polymarket
    @State private var stockText = ""
    @State private var addError: String?
    @State private var needsAlpacaSetup = false
    @StateObject private var portfolioStore = PortfolioStore.shared
    @ObservedObject private var recents = RecentMarketsStore.shared
    @State private var portfolioID: UUID?
    @State private var portfolioKind: PortfolioChartKind = .valueChart
    @State private var cmcType: CoinMarketCapChartType = .altcoinSeasonHistorical

    @Environment(\.dismiss) private var dismiss
    @Environment(\.openSettings) private var openSettings

    init(
        title: String = "Add Chart",
        actionLabel: String = "Add",
        subtitle: String = "Search a market, stock or index and add it as a live chart.",
        systemImage: String = "chart.xyaxis.line",
        initialTab: Tab = .crypto,
        onAddPortfolio: (@MainActor (PortfolioChartConfig) -> Void)? = nil,
        onAddCoinMarketCap: (@MainActor (CoinMarketCapChartConfig) -> Void)? = nil,
        onAddBitcoinPowerLaw: (@MainActor () -> Void)? = nil,
        onAdd: @escaping @MainActor (TickerSearchResult) async throws -> Void
    ) {
        self.title = title
        self.actionLabel = actionLabel
        self.subtitle = subtitle
        self.systemImage = systemImage
        _selectedTab = State(initialValue: initialTab)
        self.onAddPortfolio = onAddPortfolio
        self.onAddCoinMarketCap = onAddCoinMarketCap
        self.onAddBitcoinPowerLaw = onAddBitcoinPowerLaw
        self.onAdd = onAdd
    }

    /// Starting points for the Predictions tab; each searches the selected provider for `query`.
    private static let predictionTopics: [SuggestionChipGrid.Item] = [
        .init(title: "Fed decision", query: "fed", icon: .symbol("building.columns")),
        .init(title: "Bitcoin price", query: "bitcoin", icon: .symbol("bitcoinsign.circle")),
        .init(title: "Ethereum price", query: "ethereum", icon: .symbol("diamond")),
        .init(title: "Inflation", query: "inflation", icon: .symbol("chart.line.uptrend.xyaxis")),
        .init(title: "Recession", query: "recession", icon: .symbol("arrow.down.right.circle")),
        .init(title: "Election", query: "election", icon: .symbol("checkmark.seal")),
        .init(title: "Trump", query: "trump", icon: .symbol("person.crop.circle")),
        .init(title: "Oil & gold", query: "oil", icon: .symbol("flame")),
        .init(title: "Super Bowl", query: "super bowl", icon: .symbol("sportscourt")),
        .init(title: "Weather", query: "temperature", icon: .symbol("cloud.sun")),
    ]

    private let suggestions = MarketSuggestions.crypto
    private let stockSuggestions = MarketSuggestions.stocks

    enum Tab: String, CaseIterable {
        case crypto = "Crypto"
        case stocks = "Stocks"
        case predictionMarkets = "Prediction Markets"
        case coinMarketCap = "CoinMarketCap"
        case portfolio = "Portfolio"
        case models = "Models"

        /// Shorter than the case name where six segments have to share one row.
        var title: String { self == .predictionMarkets ? "Predictions" : rawValue }

        var systemImage: String {
            switch self {
            case .crypto: "bitcoinsign.circle"
            case .stocks: "building.columns"
            case .predictionMarkets: "percent"
            case .coinMarketCap: "gauge.with.dots.needle.50percent"
            case .portfolio: "briefcase"
            case .models: "function"
            }
        }
    }

    /// View model behind whichever prediction-market provider is selected.
    private var predictionVM: PredictionMarketSearchViewModel {
        PredictionMarketPicker.viewModel(
            for: predictionProvider, polymarket: polymarketVM, kalshi: kalshiVM)
    }

    /// Whichever pane is showing owns the selection the Add button commits.
    private var activeSelection: TickerSearchResult? {
        switch selectedTab {
        case .crypto: return searchVM.selectedResult
        case .stocks: return stockVM.selectedResult
        case .predictionMarkets: return predictionVM.selectedResult
        case .coinMarketCap, .portfolio, .models: return nil
        }
    }

    private var visibleTabs: [Tab] {
        Tab.allCases.filter {
            ($0 != .portfolio || onAddPortfolio != nil)
                && ($0 != .coinMarketCap || onAddCoinMarketCap != nil)
                && ($0 != .models || onAddBitcoinPowerLaw != nil)
        }
    }

    private var canCommit: Bool {
        switch selectedTab {
        case .portfolio: !portfolioStore.activePortfolios.isEmpty
        case .coinMarketCap, .models: true
        case .crypto, .stocks, .predictionMarkets: activeSelection != nil
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            VStack(alignment: .leading, spacing: 16) {
                SheetHeader(systemImage: systemImage, title: title, subtitle: subtitle)

                IconTabBar(
                    items: visibleTabs.map { .init(value: $0, title: $0.title, systemImage: $0.systemImage) },
                    selection: $selectedTab, isCompact: true
                )
                .frame(maxWidth: .infinity)

                ZStack(alignment: .topLeading) {
                    switch selectedTab {
                    case .crypto:
                        cryptoTab
                    case .stocks:
                        stockTab
                    case .predictionMarkets:
                        PredictionMarketPicker(
                            provider: $predictionProvider,
                            polymarketVM: polymarketVM,
                            kalshiVM: kalshiVM,
                            searchText: $predictionMarketText,
                            sizing: .fillAvailable,
                            showsStatus: false,
                            suggestions: Self.predictionTopics,
                            onCommitResult: { addTicker($0) }
                        )
                    case .coinMarketCap:
                        coinMarketCapTab
                    case .portfolio:
                        portfolioTab
                    case .models:
                        modelsTab
                    }
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)

                statusRows
            }
            .padding(24)

            Divider()
            footer
        }
        // Fixed rather than content-hugging, so switching tabs never changes the sheet's size.
        .frame(width: UI.addTickerSheetWidth, height: UI.addTickerSheetHeight)
        .animation(.easeInOut(duration: 0.18), value: searchVM.searchResults.values.reduce(0) { $0 + $1.count })
        .animation(.easeInOut(duration: 0.18), value: stockVM.searchResults.values.reduce(0) { $0 + $1.count })
        .animation(.easeInOut(duration: 0.18), value: polymarketVM.groups.reduce(0) { $0 + $1.results.count })
        .animation(.easeInOut(duration: 0.18), value: kalshiVM.groups.reduce(0) { $0 + $1.results.count })
        .onChange(of: selectedTab) {
            addError = nil
            needsAlpacaSetup = false
        }
        .onDisappear {
            cancelSearches()
        }
    }

    private var footer: some View {
        HStack(spacing: 12) {
            if let selected = activeSelection {
                SelectedResultBanner(prefix: "Selected", result: selected)
                    .frame(maxWidth: 380)
            }
            Spacer(minLength: 0)
            Button("Cancel") {
                cancelSearches()
                dismiss()
            }
            .keyboardShortcut(.cancelAction)
            .controlSize(.large)

            Button(actionLabel) {
                switch selectedTab {
                case .portfolio: addPortfolio()
                case .coinMarketCap: addCoinMarketCap()
                case .models: addBitcoinPowerLaw()
                case .crypto, .stocks, .predictionMarkets:
                    if let selected = activeSelection { addTicker(selected) }
                }
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
            .disabled(!canCommit)
            .keyboardShortcut(.defaultAction)
        }
        .padding(.horizontal, 24)
        .padding(.vertical, 14)
    }

    private var modelsTab: some View {
        VStack(alignment: .leading, spacing: 12) {
            ChoiceCard(
                title: "Bitcoin Power Law",
                subtitle: "Bitstamp BTC/USD history on logarithmic time and price axes, with an editable "
                    + "power-law corridor and ten-year projection.",
                systemImage: "chart.xyaxis.line", isSelected: true, action: {})
        }
    }

    private func addBitcoinPowerLaw() {
        onAddBitcoinPowerLaw?()
        dismiss()
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
                VStack(alignment: .leading, spacing: 6) {
                    Text("Portfolio").font(.subheadline.weight(.medium))
                    Picker("Portfolio", selection: $portfolioID) {
                        Text("All Portfolios").tag(UUID?.none)
                        ForEach(portfolioStore.activePortfolios) { portfolio in
                            Text(portfolio.name).tag(Optional(portfolio.id))
                        }
                    }
                    .labelsHidden()
                    .fixedSize()
                }
                VStack(alignment: .leading, spacing: 8) {
                    Text("Show").font(.subheadline.weight(.medium))
                    ForEach(PortfolioChartKind.allCases) { kind in
                        ChoiceCard(
                            title: kind.rawValue, subtitle: portfolioKindDescription(kind),
                            systemImage: portfolioKindIcon(kind), isSelected: portfolioKind == kind
                        ) { portfolioKind = kind }
                    }
                }
                Text(
                    "Portfolio value charts include their own 1D, 1W, 1M, 1Y and all-time range control. "
                        + "Portfolio cards do not support market indicators."
                )
                .font(.caption)
                .foregroundStyle(.secondary)
            }
        }
        .task { await portfolioStore.refresh() }
    }

    private func portfolioKindIcon(_ kind: PortfolioChartKind) -> String {
        switch kind {
        case .valueChart: "chart.line.uptrend.xyaxis"
        case .value: "dollarsign.circle"
        case .allocation: "chart.pie"
        }
    }

    private func portfolioKindDescription(_ kind: PortfolioChartKind) -> String {
        switch kind {
        case .valueChart: "Value over time, with 1D to all-time ranges"
        case .value: "The current total as a big number"
        case .allocation: "How the portfolio splits across assets"
        }
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
            Text("Market-wide CoinMarketCap indices").font(.subheadline.weight(.medium))
            LazyVGrid(columns: [GridItem(.flexible(), spacing: 10), GridItem(.flexible(), spacing: 10)], spacing: 10) {
                ForEach(CoinMarketCapChartType.allCases) { type in
                    ChoiceCard(
                        title: type.title, subtitle: cmcDescription(type), systemImage: cmcIcon(type),
                        isSelected: cmcType == type
                    ) { cmcType = type }
                }
            }
        }
    }

    private func cmcIcon(_ type: CoinMarketCapChartType) -> String {
        switch type {
        case .altcoinSeasonHistorical: "chart.xyaxis.line"
        case .altcoinSeasonLatest: "gauge.with.dots.needle.67percent"
        case .fearAndGreedHistorical: "waveform.path.ecg"
        case .fearAndGreedLatest: "gauge.with.needle"
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

            if isQueryEmpty(stockText) {
                idleContent {
                    SuggestionChipGrid(
                        caption: "Popular US stocks and ETFs · free IEX feed",
                        items: stockSuggestions,
                        iconSource: .alpaca
                    ) { symbol in
                        stockVM.selectedResult = TickerSearchResult(
                            symbol: symbol, fullSymbol: symbol, source: .alpaca, price: nil
                        )
                        // Only search when there is a feed to ask: the text would hide the chips,
                        // leaving an empty pane.
                        if AlpacaCredentialsStore.isConfigured {
                            stockText = symbol
                            stockVM.scheduleSearch(query: symbol)
                        }
                    }
                    recentMarkets
                }
            }

            if let results = stockVM.searchResults[.alpaca], !results.isEmpty {
                TickerSearchResultList(
                    searchVM: stockVM,
                    sources: [.alpaca],
                    sizing: .fillAvailable,
                    onCommitResult: { addTicker($0) }
                )
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

            // Suggestions and recents give way to results the moment something is typed.
            if isQueryEmpty(inputText) {
                idleContent {
                    SuggestionChipGrid(caption: "Suggestions", items: suggestions, iconSource: .binance) { ticker in
                        inputText = ticker
                        searchVM.scheduleSearch(query: ticker)
                    }
                    recentMarkets
                }
            }

            // Search results grouped by source
            if !searchVM.searchResults.isEmpty {
                TickerSearchResultList(
                    searchVM: searchVM,
                    sources: searchVM.orderedSources,
                    sizing: .fillAvailable,
                    onCommitResult: { addTicker($0) }
                )
            }

            // No results
            if !searchVM.isSearching && !inputText.trimmingCharacters(in: .whitespaces).isEmpty
                && searchVM.searchResults.isEmpty
            {
                ContentUnavailableView.search(text: inputText)
                    .frame(maxHeight: .infinity)
            }

        }
    }

    @ViewBuilder
    private var statusRows: some View {
        if let error = addError ?? (selectedTab == .predictionMarkets ? predictionVM.errorMessage : nil) {
            NoticeCard(
                systemImage: "exclamationmark.triangle.fill", tint: addError == nil ? .orange : .red, title: error)
        } else if selectedTab == .predictionMarkets, !predictionVM.isSearching,
            !predictionMarketText.trimmingCharacters(in: .whitespaces).isEmpty,
            !predictionVM.hasResults
        {
            Text("No markets found")
                .font(.caption)
                .foregroundStyle(.secondary)
        }

        if needsAlpacaSetup {
            NoticeCard(
                systemImage: "exclamationmark.triangle.fill", tint: .orange,
                title: "Set up Alpaca before adding a stock chart.",
                actionTitle: "Open Settings",
                action: {
                    UserDefaults.standard.set(SettingsTab.alpaca.rawValue, forKey: "settingsTab")
                    openSettings()
                })
        }
    }

    // MARK: - Recents

    /// Suggestions plus ten recents can outgrow the pane; they scroll rather than push the sheet.
    private func idleContent<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
        ScrollView {
            VStack(spacing: 16) { content() }
        }
        .scrollBounceBehavior(.basedOnSize)
    }

    private func isQueryEmpty(_ text: String) -> Bool { text.trimmingCharacters(in: .whitespaces).isEmpty }

    @ViewBuilder private var recentMarkets: some View {
        if !recents.items.isEmpty {
            RecentMarketsCard(
                markets: recents.items,
                selected: selectedTab == .stocks ? stockVM.selectedResult : searchVM.selectedResult,
                onSelect: pickRecent, onCommit: { addTicker($0) },
                onRemove: { recents.remove($0) }, onClear: { recents.clear() })
        }
    }

    /// Selects the market on the tab that owns its source, switching to it when needed.
    private func pickRecent(_ result: TickerSearchResult) {
        if result.source == .alpaca {
            selectedTab = .stocks
            stockVM.selectedResult = result
        } else {
            selectedTab = .crypto
            searchVM.selectedResult = result
        }
    }

    // MARK: - Add

    private func cancelSearches() {
        searchVM.cancelSearch()
        stockVM.cancelSearch()
        polymarketVM.cancelSearch()
        kalshiVM.cancelSearch()
    }

    private func addTicker(_ selected: TickerSearchResult) {
        if selected.source == .alpaca, !AlpacaCredentialsStore.isConfigured {
            needsAlpacaSetup = true
            return
        }

        Task { @MainActor in
            do {
                // Prediction-market search already handed us the market artwork (when the
                // provider has any); seed the resolver so the new card paints it without
                // another round trip.
                if selected.source.isPredictionMarket {
                    await IconResolver.shared.remember(
                        ticker: selected.fullSymbol,
                        source: selected.source,
                        url: selected.imageURL
                    )
                }

                try await onAdd(selected)
                recents.record(selected)
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
