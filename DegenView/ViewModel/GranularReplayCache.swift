import Foundation

/// Recently downloaded replay history, so choosing another start on the same market and
/// resolution slices what is already in memory instead of downloading it again.
struct GranularReplayCache {
    struct Key: Hashable {
        let symbol: String
        let interval: ReplayInterval
    }

    private struct Entry {
        let start: Date
        let end: Date
        let data: [KlineData]
        let storedAt: Date
    }

    /// The newest bars of a range may still have been forming when it was fetched.
    static let lifetime: TimeInterval = 300
    static let capacity = 3

    private var entries: [Key: Entry] = [:]

    /// The cached bars inside `start..<end`, when one entry covers all of it.
    func slice(_ key: Key, start: Date, end: Date, now: Date = Date()) -> [KlineData]? {
        guard let entry = entries[key], now.timeIntervalSince(entry.storedAt) < Self.lifetime,
            entry.start <= start, entry.end >= end
        else { return nil }
        return entry.data.filter { $0.openTime >= start && $0.openTime < end }
    }

    mutating func store(_ key: Key, start: Date, end: Date, data: [KlineData], now: Date = Date()) {
        entries[key] = Entry(start: start, end: end, data: data, storedAt: now)
        while entries.count > Self.capacity,
            let oldest = entries.min(by: { $0.value.storedAt < $1.value.storedAt })?.key
        {
            entries[oldest] = nil
        }
    }
}
