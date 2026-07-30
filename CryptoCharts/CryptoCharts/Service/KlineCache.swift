import Foundation

/// Thread-safe in-memory cache for kline (OHLC) data with configurable TTL.
actor KlineCache {
    private var entries: [String: CachedEntry] = [:]

    private struct CachedEntry {
        let data: [KlineData]
        let fetchedAt: Date
    }

    private func key(symbol: String, interval: String) -> String {
        "\(symbol.uppercased())-\(interval)"
    }

    /// Return cached candles if fresh enough and we have at least `count` of them.
    func get(symbol: String, interval: String, count: Int, ttl: TimeInterval) -> [KlineData]? {
        guard let entry = entries[key(symbol: symbol, interval: interval)] else { return nil }
        guard Date().timeIntervalSince(entry.fetchedAt) < ttl else { return nil }
        guard entry.data.count >= count else { return nil }
        return Array(entry.data.suffix(count))
    }

    func set(symbol: String, interval: String, data: [KlineData]) {
        entries[key(symbol: symbol, interval: interval)] = CachedEntry(data: data, fetchedAt: Date())
    }

    func invalidate() {
        entries.removeAll()
    }
}
