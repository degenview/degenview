import Foundation

/// Shared search state and debounced multi-source ticker lookup.
/// Used by both AddTickerSheet and ChartSettingsSheet.
@MainActor
final class TickerSearchViewModel: ObservableObject {
    @Published var searchResults: [DataSourceType: [TickerSearchResult]] = [:]
    @Published var isSearching = false
    @Published var selectedResult: TickerSearchResult?
    @Published var errorMessage: String?

    private var searchTask: Task<Void, Never>?
    private var logPrefix: String

    init(logPrefix: String = "[Search]") {
        self.logPrefix = logPrefix
    }

    /// Data sources that have results sorted first, then by enum order.
    var orderedSources: [DataSourceType] {
        var sources = DataSourceType.allCases
        sources.sort { a, b in
            let aHas = !(searchResults[a]?.isEmpty ?? true)
            let bHas = !(searchResults[b]?.isEmpty ?? true)
            if aHas != bHas { return aHas }
            return a.rawValue < b.rawValue
        }
        return sources
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

    /// Debounced search across all data sources. Call on every keystroke.
    func scheduleSearch(query: String) {
        searchTask?.cancel()

        let text = query.trimmingCharacters(in: .whitespaces)
        guard !text.isEmpty else {
            searchResults = [:]
            selectedResult = nil
            return
        }

        let captured = text
        searchTask = Task { @MainActor in
            try? await Task.sleep(nanoseconds: Timeout.searchDebounceNS)
            guard !Task.isCancelled,
                  query.trimmingCharacters(in: .whitespaces) == captured
            else { return }

            isSearching = true
            defer { isSearching = false }

            let sources = DataSourceFactory.shared.allSources
            var newResults: [DataSourceType: [TickerSearchResult]] = [:]

            await withTaskGroup(of: (DataSourceType, [TickerSearchResult]?).self) { group in
                for source in sources {
                    group.addTask {
                        do {
                            let results = try await source.searchTickers(query: captured)
                            return (source.type, results)
                        } catch {
                            #if DEBUG
                            print("[Search] \(source.type.displayName) search failed: \(error.localizedDescription)")
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
               !newResults.values.flatMap({ $0 }).contains(selected) {
                selectedResult = nil
            }
        }
    }

    /// Cancel any in-flight search.
    func cancelSearch() {
        searchTask?.cancel()
    }
}
