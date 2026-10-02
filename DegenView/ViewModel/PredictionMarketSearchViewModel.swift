import Foundation

/// Markets belonging to one prediction-market event, rendered as a titled section.
struct PredictionMarketResultGroup: Identifiable {
    let eventTitle: String
    let results: [TickerSearchResult]

    var id: String { eventTitle }
}

/// Debounced prediction-market search, for one provider (Polymarket or Kalshi).
///
/// Mirrors `TickerSearchViewModel`'s surface (`scheduleSearch` / `cancelSearch` /
/// `isSearching` / `selectedResult`) so the shared search-pane views bind to either,
/// but groups its results by parent event rather than by data source.
@MainActor
final class PredictionMarketSearchViewModel: ObservableObject {
    @Published var groups: [PredictionMarketResultGroup] = []
    @Published var isSearching = false
    @Published var selectedResult: TickerSearchResult?
    @Published var errorMessage: String?
    /// Per-market-id checked state for multi-choice groups. False by default — user opts in.
    @Published var checkedChoices: [String: Bool] = [:]
    @Published private(set) var expandedGroupIDs: Set<String> = []
    /// True while `groups` holds the provider's trending events rather than search results.
    @Published private(set) var isShowingTrending = false
    private var resultSetID = ""

    private let debouncer = SearchDebouncer()
    private let logPrefix: String
    let provider: DataSourceType
    private let service: () -> TickerDataSource
    private let now: () -> Date
    private var trendingTask: Task<Void, Never>?
    private var cachedTrending: (results: [TickerSearchResult], fetched: Date)?

    init(
        provider: DataSourceType = .polymarket, logPrefix: String = "[PredictionMarketSearch]",
        service: (() -> TickerDataSource)? = nil, now: @escaping () -> Date = Date.init
    ) {
        self.provider = provider
        self.logPrefix = logPrefix
        self.service = service ?? { DataSourceFactory.shared.service(for: provider) }
        self.now = now
    }

    var hasResults: Bool { !groups.isEmpty }

    /// First result across all groups (for Enter-key quick-select).
    var firstAvailableResult: TickerSearchResult? {
        groups.first?.results.first
    }

    func isExpanded(_ group: PredictionMarketResultGroup) -> Bool {
        expandedGroupIDs.contains(group.id)
    }

    func toggleExpansion(_ group: PredictionMarketResultGroup) {
        if isExpanded(group) {
            expandedGroupIDs.remove(group.id)
        } else {
            expandedGroupIDs.insert(group.id)
        }
    }

    // MARK: - Group-level selection

    /// Whether ALL choices in `group` are checked.
    func isGroupChecked(_ group: PredictionMarketResultGroup) -> Bool {
        group.results.allSatisfy { checkedChoices[$0.fullSymbol] == true }
    }

    /// Whether ANY choice in `group` is checked.
    func isGroupAnyChecked(_ group: PredictionMarketResultGroup) -> Bool {
        group.results.contains { checkedChoices[$0.fullSymbol] == true }
    }

    /// Whether SOME (but not all) choices in `group` are checked.
    func isGroupPartiallyChecked(_ group: PredictionMarketResultGroup) -> Bool {
        isGroupAnyChecked(group) && !isGroupChecked(group)
    }

    /// Toggle all choices in `group` on/off. Checking a group unchecks all other groups.
    func toggleGroup(_ group: PredictionMarketResultGroup) {
        let allChecked = isGroupChecked(group)
        let newValue = !allChecked

        if newValue {
            // Uncheck every other multi-choice group so only one is active.
            for other in groups where other.id != group.id && other.results.count > 1 {
                for r in other.results { checkedChoices[r.fullSymbol] = false }
            }
        }

        for r in group.results { checkedChoices[r.fullSymbol] = newValue }
        selectedResult = newValue ? buildMultiChoiceResult(for: group) : nil
    }

    /// Toggle a single choice within a multi-choice group and rebuild `selectedResult`.
    func toggleChoice(_ tokenID: String) {
        let current = checkedChoices[tokenID] ?? false
        checkedChoices[tokenID] = !current

        for group in groups where group.results.count > 1 {
            if group.results.contains(where: { $0.fullSymbol == tokenID }) {
                // Uncheck every other group.
                for other in groups where other.id != group.id && other.results.count > 1 {
                    for r in other.results { checkedChoices[r.fullSymbol] = false }
                }
                let anyChecked = group.results.contains { checkedChoices[$0.fullSymbol] == true }
                selectedResult = anyChecked ? buildMultiChoiceResult(for: group) : nil
                return
            }
        }
    }

    /// Build a `TickerSearchResult` representing only the checked choices in `group`.
    func buildMultiChoiceResult(for group: PredictionMarketResultGroup) -> TickerSearchResult? {
        let checked = group.results.filter { checkedChoices[$0.fullSymbol] == true }
        guard let primary = checked.first else { return nil }
        var result = primary
        result.pmSeries = checked.map {
            PmSeriesConfig(tokenID: $0.fullSymbol, label: $0.symbol, enabled: true)
        }
        return result
    }

    // MARK: - Post-search init

    private func updateAfterSearch() {
        let newResultSetID = groups.map { group in
            "\(group.id):\(group.results.map(\.fullSymbol).joined(separator: ","))"
        }.joined(separator: "|")
        if newResultSetID != resultSetID {
            resultSetID = newResultSetID
            expandedGroupIDs = groups.count == 1 ? Set(groups.map(\.id)) : []
        }
        let allMultiIDs = Set(
            groups.flatMap { g -> [String] in
                guard g.results.count > 1 else { return [] }
                return g.results.map { $0.fullSymbol }
            })
        // Remove stale entries; new IDs start unchecked.
        checkedChoices = checkedChoices.filter { allMultiIDs.contains($0.key) }
        for id in allMultiIDs where checkedChoices[id] == nil {
            checkedChoices[id] = false
        }
        // Clear selectedResult if it no longer exists in the new results.
        if let sel = selectedResult,
            !groups.flatMap(\.results).contains(where: { $0.fullSymbol == sel.fullSymbol })
        {
            selectedResult = nil
        }
    }

    // MARK: - Search

    /// Debounced market search. Call on every keystroke.
    func scheduleSearch(query: String) {
        let text = query.trimmingCharacters(in: .whitespaces)
        guard !text.isEmpty else {
            debouncer.cancel()
            selectedResult = nil
            checkedChoices = [:]
            expandedGroupIDs = []
            resultSetID = ""
            errorMessage = nil
            showTrending()
            return
        }
        trendingTask?.cancel()
        isShowingTrending = false
        debouncer.schedule { [weak self] in
            await self?.runSearch(text)
        }
    }

    /// Cancel any in-flight search.
    func cancelSearch() {
        debouncer.cancel()
        trendingTask?.cancel()
    }

    // MARK: - Trending

    /// With no query, list what the provider says is busiest — when it can say. Providers that
    /// can't (Kalshi) get an empty list. A fetch failure also leaves the list empty, quietly:
    /// trending is an extra, not something the user asked for.
    func showTrending() {
        trendingTask?.cancel()
        guard service() is TrendingMarketsDataSource else {
            isShowingTrending = false
            groups = []
            return
        }
        if let cached = cachedTrending, now().timeIntervalSince(cached.fetched) < Polymarket.trendingCacheTTL {
            applyTrending(cached.results)
            return
        }
        isShowingTrending = true
        groups = []
        isSearching = true
        trendingTask = Task { [weak self] in
            guard let self else { return }
            defer { self.isSearching = false }
            guard let source = self.service() as? TrendingMarketsDataSource else { return }
            do {
                let results = try await source.trendingMarkets(limit: Polymarket.trendingLimit)
                guard !Task.isCancelled else { return }
                self.cachedTrending = (results, self.now())
                self.applyTrending(results)
            } catch {
                guard !Task.isCancelled else { return }
                #if DEBUG
                    print("\(self.logPrefix) trending failed: \(error.localizedDescription)")
                #endif
                self.isShowingTrending = false
                self.groups = []
            }
        }
    }

    private func applyTrending(_ results: [TickerSearchResult]) {
        isShowingTrending = !results.isEmpty
        groups = Self.group(results)
        updateAfterSearch()
    }

    private func runSearch(_ query: String) async {
        isSearching = true
        defer { isSearching = false }

        let service = service()

        do {
            let results = try await service.searchTickers(query: query)
            guard !Task.isCancelled else { return }

            groups = Self.group(results)
            errorMessage = nil
            updateAfterSearch()
        } catch {
            guard !Task.isCancelled else { return }
            #if DEBUG
                print("\(logPrefix) search failed: \(error.localizedDescription)")
            #endif
            groups = []
            errorMessage = error.localizedDescription
        }
    }

    /// Bucket results without changing event relevance, then stably sort choices by
    /// descending current YES probability. Missing probabilities appear last.
    static func group(_ results: [TickerSearchResult]) -> [PredictionMarketResultGroup] {
        var order: [String] = []
        var buckets: [String: [TickerSearchResult]] = [:]

        for result in results {
            let title = result.eventTitle ?? result.question ?? result.symbol
            if buckets[title] == nil {
                order.append(title)
                buckets[title] = []
            }
            buckets[title]?.append(result)
        }

        return order.compactMap { title in
            guard let results = buckets[title], !results.isEmpty else { return nil }
            let sorted = results.enumerated().sorted { lhs, rhs in
                switch (lhs.element.price, rhs.element.price) {
                case let (left?, right?) where left != right: return left > right
                case (_?, nil): return true
                case (nil, _?): return false
                default: return lhs.offset < rhs.offset
                }
            }.map(\.element)
            let sortedSeries = sorted.map {
                PmSeriesConfig(tokenID: $0.fullSymbol, label: $0.symbol, enabled: true)
            }
            let normalized = sorted.map { item in
                var item = item
                item.pmSeries = sortedSeries
                return item
            }
            return PredictionMarketResultGroup(eventTitle: title, results: normalized)
        }
    }

    /// Testable entry point for applying a completed provider result set.
    func applyResults(_ results: [TickerSearchResult]) {
        groups = Self.group(results)
        updateAfterSearch()
    }
}
