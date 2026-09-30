import Foundation

/// Local search over Kalshi's series list.
///
/// Kalshi has no keyword search, and the open-event listing is far too large to mirror
/// (well over 10,000 events, 180 MB with their markets). The series list is one
/// unpaginated request, so it is fetched once, trimmed to ticker/title/category/tags
/// and cached on disk. A query ranks series here; only the best few then get their open
/// events fetched.
actor KalshiSeriesIndex {
    private struct Stored: Codable {
        let series: [KalshiSeries]
        let fetchedAt: Date
    }

    private let session: URLSession
    private let store: JSONStore<Stored>
    private let now: @Sendable () -> Date
    private var loaded: Stored?
    private var inFlight: Task<Stored, Error>?

    init(
        session: URLSession = AppSupport.defaultSession,
        directory: URL = AppSupport.directory,
        now: @escaping @Sendable () -> Date = { Date() }
    ) {
        self.session = session
        self.store = JSONStore(filename: "kalshi_series_index.json", directory: directory)
        self.now = now
    }

    /// Best-matching series for `query`, most relevant first.
    func search(query: String, limit: Int) async throws -> [KalshiSeries] {
        let index = try await load()
        return Self.rank(index.series, query: query, limit: limit)
    }

    // MARK: - Loading

    private func load() async throws -> Stored {
        if let loaded, isFresh(loaded) { return loaded }

        if loaded == nil, let onDisk = store.load() {
            loaded = onDisk
            if isFresh(onDisk) { return onDisk }
        }

        if let inFlight { return try await inFlight.value }

        let task = Task { try await self.download() }
        inFlight = task
        defer { inFlight = nil }

        do {
            let fresh = try await task.value
            loaded = fresh
            store.save(fresh)
            return fresh
        } catch {
            // A stale list still searches fine; only a cold cache is a hard failure.
            if let loaded { return loaded }
            throw error
        }
    }

    private func isFresh(_ stored: Stored) -> Bool {
        now().timeIntervalSince(stored.fetchedAt) < Kalshi.seriesIndexTTL
    }

    private func download() async throws -> Stored {
        guard let url = URL(string: "\(Kalshi.baseURL)/series") else { throw KalshiError.invalidURL }

        #if DEBUG
            print("[Kalshi] Downloading series list")
        #endif

        let (data, response) = try await session.data(from: url)
        try KalshiService.validate(response)

        let list = try KalshiJSON.decoder().decode(KalshiSeriesList.self, from: data)
        let series = (list.series ?? []).filter { !$0.title.isEmpty }
        return Stored(series: series, fetchedAt: now())
    }

    // MARK: - Ranking

    /// Every whitespace-separated word of the query must appear somewhere in the
    /// series' title, ticker, category or tags. Matches at the start of the title or of
    /// a title word outrank mid-word hits, and ties favor shorter titles, which are the
    /// general ones ("Fed meeting") rather than the narrow ones.
    nonisolated static func rank(_ series: [KalshiSeries], query: String, limit: Int) -> [KalshiSeries] {
        let words = query.lowercased().split(whereSeparator: \.isWhitespace).map(String.init)
        guard !words.isEmpty else { return [] }
        let phrase = words.joined(separator: " ")

        var scored: [(series: KalshiSeries, score: Int)] = []
        for item in series {
            let title = item.title.lowercased()
            let ticker = item.ticker.lowercased()
            let haystack = ([title, ticker, item.category?.lowercased() ?? ""] + (item.tags ?? []).map { $0.lowercased() })
                .joined(separator: " ")
            guard words.allSatisfy({ haystack.contains($0) }) else { continue }

            let score: Int
            if title.hasPrefix(phrase) {
                score = 0
            } else if title.contains(" " + phrase) {
                score = 1
            } else if ticker.hasPrefix(phrase) || ticker.hasPrefix("kx" + phrase) {
                score = 2
            } else if title.contains(phrase) {
                score = 3
            } else {
                score = 4
            }
            scored.append((item, score))
        }

        return
            scored
            .sorted { lhs, rhs in
                if lhs.score != rhs.score { return lhs.score < rhs.score }
                if lhs.series.title.count != rhs.series.title.count {
                    return lhs.series.title.count < rhs.series.title.count
                }
                return lhs.series.ticker < rhs.series.ticker
            }
            .prefix(limit)
            .map(\.series)
    }
}
