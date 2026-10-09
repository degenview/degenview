import Foundation

/// The candles behind a script's `request.security` and `request.security_lower_tf` calls.
///
/// A lower-timeframe series is the chart's own symbol at a bar length shorter than the chart's, built from the
/// finest of the app's replay candle sizes that divides it (12 minutes from 1-minute candles, 4 hours from
/// 1-hour ones) and folded into that length. Unlike the `request.security` snapshot it can be topped up with
/// `refresh()`, so the forming chart bar keeps its intrabars while the market moves.
final class PineIntrabarSeries: PineSecurityDataProvider, @unchecked Sendable {
    /// Most candles fetched for one series; the newest are kept, so a long chart gets intrabars for its recent part.
    static let maximumCandles = 60_000
    /// A slow source delays the script, not the chart.
    static let timeout: TimeInterval = 30

    /// The replay candle sizes, coarsest first.
    private static let bases: [ReplayInterval] = [
        .oneDay, .oneHour, .thirtyMinutes, .fifteenMinutes, .fiveMinutes, .oneMinute,
    ]

    /// One lower-timeframe series: the base candles as fetched, and the same folded into `interval`.
    private struct Lower {
        let base: ReplayInterval
        var candles: [KlineData]
        var folded: [KlineData]
    }

    private let lock = NSLock()
    private var security: PineFetchedSecurityData
    private var lower: [PineSecurityKey: Lower] = [:]
    private let source: DataSourceType
    private let apiSymbol: String

    init(security: PineFetchedSecurityData, source: DataSourceType, apiSymbol: String) {
        self.security = security
        self.source = source
        self.apiSymbol = apiSymbol
    }

    func candles(for key: PineSecurityKey) -> [KlineData]? {
        lock.withLock { lower[key]?.folded ?? security.candles(for: key) }
    }

    var isEmpty: Bool { lock.withLock { lower.isEmpty && security.series.isEmpty } }

    /// The base candle size that builds `seconds` exactly: the coarsest that divides it. Nil when none does.
    static func base(forSeconds seconds: TimeInterval) -> ReplayInterval? {
        bases.first { base in
            guard let size = base.seconds, size <= seconds else { return false }
            return seconds.truncatingRemainder(dividingBy: size) == 0
        }
    }

    /// Fetches each key's candles over the chart's span, in parallel, giving up after `timeout`.
    func load(_ keys: [PineSecurityKey], bars: [KlineData], spacing: TimeInterval) async {
        guard let first = bars.first, let last = bars.last, spacing > 0,
            let service = DataSourceFactory.shared.service(for: source) as? GranularReplayDataSource
        else { return }
        let end = last.openTime.addingTimeInterval(spacing)
        let symbol = apiSymbol
        var wanted: [ReplayInterval: Date] = [:]
        for key in keys {
            guard let base = Self.base(forSeconds: key.interval), let size = base.seconds else { continue }
            let earliest = end.addingTimeInterval(-size * Double(Self.maximumCandles))
            var start = max(first.openTime, earliest)
            // `calc_bars_count`: nothing is read before the newest chart bars, so nothing is fetched for them.
            if key.recentBars > 0 {
                start = max(start, end.addingTimeInterval(-spacing * Double(key.recentBars)))
            }
            wanted[base] = wanted[base].map { min($0, start) } ?? start
        }
        let fetched = await withTaskGroup(of: Fetched.self) { group in
            for (base, start) in wanted {
                group.addTask {
                    guard
                        let candles = try? await service.fetchReplayKlines(
                            symbol: symbol, interval: base, start: start, end: end,
                            maximumCount: Self.maximumCandles), !candles.isEmpty
                    else { return .missing }
                    return .series(base, candles)
                }
            }
            group.addTask {
                try? await Task.sleep(for: .seconds(Self.timeout))
                return .timedOut
            }
            var result: [ReplayInterval: [KlineData]] = [:]
            var answered = 0
            collecting: for await item in group {
                switch item {
                case .series(let base, let candles):
                    result[base] = candles
                    answered += 1
                case .missing: answered += 1
                case .timedOut: break collecting
                }
                if answered >= wanted.count { break }
            }
            group.cancelAll()
            return result
        }
        lock.withLock {
            for key in keys {
                guard let base = Self.base(forSeconds: key.interval), let candles = fetched[base] else { continue }
                lower[key] = Lower(
                    base: base, candles: candles, folded: Self.fold(candles, into: key.interval, base: base))
            }
        }
    }

    /// Tops every series up with the candles since its last one, so the forming bar's intrabars stay current.
    func refresh() async {
        let held = lock.withLock { lower }
        guard !held.isEmpty,
            let service = DataSourceFactory.shared.service(for: source) as? GranularReplayDataSource
        else { return }
        let symbol = apiSymbol
        let now = Date()
        var fresh: [ReplayInterval: [KlineData]] = [:]
        for base in Set(held.values.map(\.base)) {
            guard let size = base.seconds, !Task.isCancelled else { continue }
            let latest = held.values.filter { $0.base == base }.compactMap { $0.candles.last?.openTime }.min()
            guard let start = latest else { continue }
            let tail = try? await service.fetchReplayKlines(
                symbol: symbol, interval: base, start: start, end: now.addingTimeInterval(size), maximumCount: 1_000)
            if let tail, !tail.isEmpty { fresh[base] = tail }
        }
        guard !fresh.isEmpty else { return }
        lock.withLock {
            for (key, series) in lower {
                guard let tail = fresh[series.base], let from = tail.first?.openTime else { continue }
                // Replace everything from the tail's first candle on: its newest candle was still forming.
                var merged = series.candles.filter { $0.openTime < from }
                merged.append(contentsOf: tail)
                lower[key] = Lower(
                    base: series.base, candles: merged,
                    folded: Self.fold(merged, into: key.interval, base: series.base))
            }
        }
    }

    private enum Fetched {
        case series(ReplayInterval, [KlineData])
        case missing
        case timedOut
    }

    private static func fold(_ candles: [KlineData], into interval: TimeInterval, base: ReplayInterval) -> [KlineData] {
        base.seconds == interval ? candles : candles.folded(into: interval)
    }
}
