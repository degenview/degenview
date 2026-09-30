import Foundation

/// Turns raw candle observations into a bar lifecycle: open, ticks, close, next bar.
///
/// Bar identity is the open time. The aggregator drops duplicates and late data, never lets a stale
/// message reopen a closed bar, and asks for a rebuild when the feed can no longer be reconciled with
/// what was already committed (a gap, a bar of the wrong length, a corrected closed bar).
struct PineCandleAggregator: Sendable {
    enum Output: Equatable, Sendable {
        /// The forming bar changed, or opened (`isNew`).
        case tick(KlineData, isNew: Bool)
        /// The bar's values are final.
        case close(KlineData)
        /// The committed history no longer matches the feed.
        case rebuild(RebuildReason)

        static func == (lhs: Output, rhs: Output) -> Bool {
            switch (lhs, rhs) {
            case (.tick(let a, let x), .tick(let b, let y)): a.openTime == b.openTime && x == y && a.sameValues(b)
            case (.close(let a), .close(let b)): a.openTime == b.openTime && a.sameValues(b)
            case (.rebuild(let a), .rebuild(let b)): a == b
            default: false
            }
        }
    }

    enum RebuildReason: Equatable, Sendable {
        /// One or more bars are missing between the last bar and the new one.
        case gap
        /// A bar arrived that cannot belong to this timeframe.
        case timeframeMismatch
        /// A closed bar's values changed after it was committed.
        case correction
    }

    /// Seconds per bar; zero until known, which disables the spacing checks.
    var barSeconds: Double
    private(set) var forming: KlineData?
    private var formingOrigin = PineMarketUpdate.Origin.poll
    private(set) var lastClosed: KlineData?

    init(barSeconds: Double = 0, lastClosed: KlineData? = nil, forming: KlineData? = nil) {
        self.barSeconds = barSeconds
        self.lastClosed = lastClosed
        self.forming = forming
    }

    mutating func ingest(_ update: PineMarketUpdate) -> [Output] {
        let bar = update.bar
        if let closed = lastClosed {
            if bar.openTime < closed.openTime { return [] }
            if bar.openTime == closed.openTime {
                // A stream repeat of a finished bar is noise. A REST copy that disagrees is a correction.
                guard update.origin == .poll, !bar.sameValues(closed) else { return [] }
                return [.rebuild(.correction)]
            }
        }
        if let current = forming {
            if bar.openTime < current.openTime { return [] }
            if bar.openTime == current.openTime { return apply(update, to: current) }
            if let reason = spacingProblem(from: current.openTime, to: bar.openTime) { return [.rebuild(reason)] }
            forming = nil
            lastClosed = current
            return [.close(current)] + open(bar, update.origin)
        }
        if let closed = lastClosed, let reason = spacingProblem(from: closed.openTime, to: bar.openTime) {
            return [.rebuild(reason)]
        }
        return open(bar, update.origin)
    }

    private mutating func apply(_ update: PineMarketUpdate, to current: KlineData) -> [Output] {
        let bar = update.bar
        if bar.isClosed {
            forming = nil
            lastClosed = bar
            return [.close(bar)]
        }
        // A REST refresh can be older than what the stream already delivered.
        if update.origin == .poll, formingOrigin == .stream { return [] }
        if bar.sameValues(current) { return [] }
        forming = bar
        formingOrigin = update.origin
        return [.tick(bar, isNew: false)]
    }

    private mutating func open(_ bar: KlineData, _ origin: PineMarketUpdate.Origin) -> [Output] {
        if bar.isClosed {
            lastClosed = bar
            return [.close(bar)]
        }
        forming = bar
        formingOrigin = origin
        return [.tick(bar, isNew: true)]
    }

    private func spacingProblem(from previous: Date, to next: Date) -> RebuildReason? {
        guard barSeconds > 0 else { return nil }
        let gap = next.timeIntervalSince(previous)
        if gap < barSeconds * 0.9 { return .timeframeMismatch }
        if gap > barSeconds * 1.5 { return .gap }
        return nil
    }
}

extension KlineData {
    /// Same prices and volume, ignoring identity and the close flag.
    func sameValues(_ other: KlineData) -> Bool {
        openPrice == other.openPrice && highPrice == other.highPrice && lowPrice == other.lowPrice
            && closePrice == other.closePrice && volume == other.volume
    }
}
