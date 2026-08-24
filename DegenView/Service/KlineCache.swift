import Foundation

/// Thread-safe cache for kline (OHLC) data with configurable TTL.
///
/// Optionally persists to disk. Sources behind a hard rate limit (CoinGecko)
/// use persistence so a relaunch renders candles immediately instead of
/// waiting out the request queue; fast sources keep it in memory only.
actor KlineCache {
    private var entries: [String: CachedEntry] = [:]
    private let storeURL: URL?
    private let now: @Sendable () -> Date
    private var saveTask: Task<Void, Never>?

    private struct CachedEntry: Codable {
        let symbol: String
        let interval: String
        let days: Int
        let data: [KlineData]
        let fetchedAt: Date
    }

    /// - Parameter persistenceKey: filename stem for the on-disk store.
    ///   `nil` keeps the cache in memory only.
    init(
        persistenceKey: String? = nil,
        directory: URL = AppSupport.directory,
        now: @escaping @Sendable () -> Date = Date.init
    ) {
        let url = persistenceKey.map {
            directory.appendingPathComponent("kline_cache_\($0).json")
        }
        storeURL = url
        self.now = now
        entries = url.map { Self.loadFromDisk($0, now: now()) } ?? [:]
    }

    private func key(symbol: String, interval: String, days: Int) -> String {
        "\(symbol.uppercased())-\(interval)-d\(days)"
    }

    // MARK: - Reads

    /// Return cached candles if fresh enough and we have at least `count` of them.
    func get(symbol: String, interval: String, days: Int, count: Int, ttl: TimeInterval) -> [KlineData]? {
        guard let entry = entries[key(symbol: symbol, interval: interval, days: days)] else { return nil }
        guard now().timeIntervalSince(entry.fetchedAt) < ttl else { return nil }
        guard entry.data.count >= count else { return nil }
        return Array(entry.data.suffix(count))
    }

    /// The whole entry, if fresh — however many candles it holds.
    ///
    /// For sources whose window is a fixed span rather than a candle count, "at least
    /// `count` candles" is the wrong question: CoinGecko's year of weekly candles is
    /// 52 whether the chart asked for 26 or 226, and judging that entry short would
    /// refetch it on every zoom step for data that doesn't exist. They take the whole
    /// window and slice the tail themselves.
    func getFull(symbol: String, interval: String, days: Int, ttl: TimeInterval) -> [KlineData]? {
        guard let entry = entries[key(symbol: symbol, interval: interval, days: days)] else { return nil }
        guard now().timeIntervalSince(entry.fetchedAt) < ttl, !entry.data.isEmpty else { return nil }
        return entry.data
    }

    /// Return cached candles regardless of staleness, and regardless of whether
    /// there are as many as requested. Used for instant-first-render: a short or
    /// stale chart beats an empty one while the real fetch is queued.
    func getStale(symbol: String, interval: String, days: Int, count: Int) -> [KlineData]? {
        guard let entry = entries[key(symbol: symbol, interval: interval, days: days)] else { return nil }
        guard !entry.data.isEmpty else { return nil }
        return Array(entry.data.suffix(count))
    }

    /// Best cached candles for a symbol from *any* window/interval.
    ///
    /// Zooming or switching timeframe changes the window key, so the exact-key
    /// lookup misses even though we hold perfectly good candles for that coin.
    /// This keeps the chart populated across those transitions.
    func getAnyStale(symbol: String, count: Int) -> [KlineData]? {
        let prefix = "\(symbol.uppercased())-"
        var candidates: [CachedEntry] = []
        for (key, entry) in entries where key.hasPrefix(prefix) && !entry.data.isEmpty {
            candidates.append(entry)
        }
        guard !candidates.isEmpty else { return nil }

        // Prefer the smallest window that still covers `count`; if none does,
        // take whichever holds the most candles.
        let covering = candidates.filter { $0.data.count >= count }
        let pool = covering.isEmpty ? candidates : covering
        let best = covering.isEmpty
            ? pool.max(by: { $0.data.count < $1.data.count })
            : pool.min(by: { $0.data.count < $1.data.count })

        guard let best else { return nil }
        return Array(best.data.suffix(count))
    }

    // MARK: - Writes

    func set(symbol: String, interval: String, days: Int, data: [KlineData]) {
        entries[key(symbol: symbol, interval: interval, days: days)] = CachedEntry(
            symbol: symbol.uppercased(),
            interval: interval,
            days: days,
            data: data,
            fetchedAt: now()
        )
        scheduleSave()
    }

    func invalidate() {
        entries.removeAll()
        scheduleSave()
    }

    /// Write pending changes now. Called on quit so the last refresh before the
    /// app closes still warms the next launch.
    func flush() {
        saveTask?.cancel()
        saveTask = nil
        writeToDisk()
    }

    // MARK: - Persistence

    private nonisolated static func loadFromDisk(_ url: URL, now: Date) -> [String: CachedEntry] {
        guard let raw = try? Data(contentsOf: url),
              let decoded = try? JSONDecoder().decode([String: CachedEntry].self, from: raw)
        else { return [:] }

        let cutoff = now.addingTimeInterval(-CacheLimit.diskStaleness)
        return decoded.filter { $0.value.fetchedAt > cutoff }
    }

    /// Coalesce writes — a full refresh touches every symbol at once.
    private func scheduleSave() {
        guard storeURL != nil else { return }
        saveTask?.cancel()
        saveTask = Task { [weak self] in
            try? await Task.sleep(nanoseconds: CacheLimit.saveDebounceNS)
            guard !Task.isCancelled else { return }
            await self?.writeToDisk()
        }
    }

    private func writeToDisk() {
        guard let storeURL else { return }

        // Keep the newest entries only — the store is a warm-start aid, not an archive.
        var kept = entries
        if kept.count > CacheLimit.maxDiskEntries {
            let newest = kept.sorted { $0.value.fetchedAt > $1.value.fetchedAt }
                .prefix(CacheLimit.maxDiskEntries)
            kept = Dictionary(uniqueKeysWithValues: newest.map { ($0.key, $0.value) })
        }

        guard let data = try? JSONEncoder().encode(kept) else { return }
        try? data.write(to: storeURL, options: .atomic)
    }
}
