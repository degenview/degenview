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

    /// Debounced market search. Call on every keystroke.
    func scheduleSearch(query: String) {
        let text = query.trimmingCharacters(in: .whitespaces)
        guard !text.isEmpty else {
            debouncer.cancel()
            groups = []
            selectedResult = nil
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

    // MARK: - Search

    private func runSearch(_ query: String) async {
        isSearching = true
        defer { isSearching = false }

        let service = DataSourceFactory.shared.polymarket

        do {
            let results = try await service.searchTickers(query: query)
            guard !Task.isCancelled else { return }

            groups = Self.group(results)
            errorMessage = nil

            // The previous pick may not survive a new query.
            if let selected = selectedResult, !results.contains(selected) {
                selectedResult = nil
            }
        } catch {
            guard !Task.isCancelled else { return }
            #if DEBUG
            print("\(logPrefix) search failed: \(error.localizedDescription)")
            #endif
            groups = []
            errorMessage = error.localizedDescription
        }
    }

    /// Bucket results by event title, preserving the API's relevance order for both
    /// the events and the markets inside each one.
    private static func group(_ results: [TickerSearchResult]) -> [PolymarketResultGroup] {
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
            return PolymarketResultGroup(eventTitle: title, results: results)
        }
    }
}
