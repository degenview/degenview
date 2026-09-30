import Foundation

/// Shared search state and debounced multi-source ticker lookup.
/// Used by the Crypto tab of both AddTickerSheet and ChartSettingsSheet.
///
/// Prediction markets are not part of this fan-out — see `PredictionMarketSearchViewModel`.
@MainActor
final class TickerSearchViewModel: ObservableObject {
    @Published var searchResults: [DataSourceType: [TickerSearchResult]] = [:]
    @Published var isSearching = false
    @Published var selectedResult: TickerSearchResult?

    private let debouncer = SearchDebouncer()
    private let logPrefix: String
    private let sources: () -> [TickerDataSource]

    init(
        logPrefix: String = "[Search]",
        sources: @escaping () -> [TickerDataSource] = { DataSourceFactory.shared.allSources }
    ) {
        self.logPrefix = logPrefix
        self.sources = sources
    }

    /// Crypto sources, those with results first. Within each group the order is the
    /// factory's priority (Binance, then Coinbase, then the rest), not alphabetical.
    var orderedSources: [DataSourceType] {
        let sources = sources().map(\.type)
        let withResults = sources.filter { !(searchResults[$0]?.isEmpty ?? true) }
        let withoutResults = sources.filter { searchResults[$0]?.isEmpty ?? true }
        return withResults + withoutResults
    }

    /// First result across all sources (for Enter-key quick-select).
    var firstAvailableResult: TickerSearchResult? {
        for source in orderedSources {
            if let results = searchResults[source], let first = results.first {
                return first
            }
        }
        return nil
    }

    /// Debounced search across all crypto data sources. Call on every keystroke.
    func scheduleSearch(query: String) {
        let text = query.trimmingCharacters(in: .whitespaces)
        guard !text.isEmpty else {
            debouncer.cancel()
            searchResults = [:]
            selectedResult = nil
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

        let sources = sources()
        var newResults: [DataSourceType: [TickerSearchResult]] = [:]

        await withTaskGroup(of: (DataSourceType, [TickerSearchResult]?).self) { group in
            for source in sources {
                group.addTask { [logPrefix] in
                    do {
                        let results = try await source.searchTickers(query: query).ranked(for: query)
                        return (source.type, results)
                    } catch {
                        #if DEBUG
                            print(
                                "\(logPrefix) \(source.type.displayName) search failed: \(error.localizedDescription)")
                        #endif
                        return (source.type, nil)
                    }
                }
            }

            for await (type, results) in group {
                if let r = results, !r.isEmpty {
                    newResults[type] = r
                }
            }
        }

        guard !Task.isCancelled else { return }
        searchResults = newResults

        if let selected = selectedResult,
            !newResults.values.flatMap({ $0 }).contains(selected)
        {
            selectedResult = nil
        }
    }
}
