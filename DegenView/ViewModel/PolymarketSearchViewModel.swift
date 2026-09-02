import Foundation

/// Markets belonging to one Polymarket event, rendered as a titled section.
struct PolymarketResultGroup: Identifiable {
    let eventTitle: String
    let results: [TickerSearchResult]

    var id: String { eventTitle }
}

/// Debounced Polymarket market search.
///
/// Mirrors `TickerSearchViewModel`'s surface (`scheduleSearch` / `cancelSearch` /
/// `isSearching` / `selectedResult`) so the shared search-pane views bind to either,
/// but groups its results by parent event rather than by data source.
@MainActor
final class PolymarketSearchViewModel: ObservableObject {
    @Published var groups: [PolymarketResultGroup] = []
    @Published var isSearching = false
    @Published var selectedResult: TickerSearchResult?
    @Published var errorMessage: String?
    /// Per-tokenID checked state for multi-choice groups. False by default — user opts in.
    @Published var checkedChoices: [String: Bool] = [:]
    @Published private(set) var expandedGroupIDs: Set<String> = []
    private var resultSetID = ""

    private let debouncer = SearchDebouncer()
    private let logPrefix: String

    init(logPrefix: String = "[PolymarketSearch]") {
        self.logPrefix = logPrefix
    }

    var hasResults: Bool { !groups.isEmpty }

    /// First result across all groups (for Enter-key quick-select).
    var firstAvailableResult: TickerSearchResult? {
        groups.first?.results.first
    }

    func isExpanded(_ group: PolymarketResultGroup) -> Bool {
        expandedGroupIDs.contains(group.id)
    }

    func toggleExpansion(_ group: PolymarketResultGroup) {
        if isExpanded(group) {
            expandedGroupIDs.remove(group.id)
        } else {
            expandedGroupIDs.insert(group.id)
        }
    }

    // MARK: - Group-level selection

    /// Whether ALL choices in `group` are checked.
    func isGroupChecked(_ group: PolymarketResultGroup) -> Bool {
        group.results.allSatisfy { checkedChoices[$0.fullSymbol] == true }
    }

    /// Whether ANY choice in `group` is checked.
    func isGroupAnyChecked(_ group: PolymarketResultGroup) -> Bool {
        group.results.contains { checkedChoices[$0.fullSymbol] == true }
    }

    /// Whether SOME (but not all) choices in `group` are checked.
    func isGroupPartiallyChecked(_ group: PolymarketResultGroup) -> Bool {
        isGroupAnyChecked(group) && !isGroupChecked(group)
    }

    /// Toggle all choices in `group` on/off. Checking a group unchecks all other groups.
    func toggleGroup(_ group: PolymarketResultGroup) {
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
    func buildMultiChoiceResult(for group: PolymarketResultGroup) -> TickerSearchResult? {
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
            groups = []
            selectedResult = nil
            checkedChoices = [:]
            expandedGroupIDs = []
            resultSetID = ""
            errorMessage = nil
            return
        }
        debouncer.schedule { [weak self] in
            await self?.runSearch(text)
        }
    }

    /// Cancel any in-flight search.
    func cancelSearch() {
        debouncer.cancel()
    }

    private func runSearch(_ query: String) async {
        isSearching = true
        defer { isSearching = false }

        let service = DataSourceFactory.shared.polymarket

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
    static func group(_ results: [TickerSearchResult]) -> [PolymarketResultGroup] {
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
            return PolymarketResultGroup(eventTitle: title, results: normalized)
        }
    }

    /// Testable entry point for applying a completed provider result set.
    func applyResults(_ results: [TickerSearchResult]) {
        groups = Self.group(results)
        updateAfterSearch()
    }
}
