import Foundation

extension PortfolioAsset {
    /// The symbol a data source understands: the key minus its `source:` prefix.
    var marketSymbol: String {
        key.components(separatedBy: ":").dropFirst().joined(separator: ":")
    }
}

/// Daily candles for the portfolio's history rebuild.
///
/// A closed daily candle never changes, so each one is fetched once and kept in SQLite.
/// A rebuild then asks the network only for what the database can't answer: the days since
/// the newest stored candle, plus any stretch older than what has been fetched before.
/// Without this every rebuild re-requested up to 2000 bars per asset to add one new day.
actor PortfolioCandleStore {
    typealias Fetch = @Sendable (_ asset: PortfolioAsset, _ limit: Int) async throws -> [KlineData]

    static let shared = PortfolioCandleStore()
    static let interval = "1d"
    static let day: TimeInterval = 86_400
    /// A day's price is the nearest prior bar within this span, so that many earlier bars matter.
    static let lookback: TimeInterval = 3 * day
    static let maximumLimit = 2_000
    private static let minimumTail = 3
    /// Portfolios rebuilt together share one fetch per asset instead of one each.
    private static let recentWindow: TimeInterval = 60

    private let database: AppDatabase
    private let fetch: Fetch
    private var inFlight: [String: Task<[KlineData], Never>] = [:]
    private var recent: [String: (at: Date, bars: [KlineData])] = [:]

    init(
        database: AppDatabase = .shared,
        fetch: @escaping Fetch = { asset, limit in
            try await DataSourceFactory.shared.service(for: asset.source).fetchKlines(
                symbol: asset.marketSymbol, interval: PortfolioCandleStore.interval, limit: limit)
        }
    ) {
        self.database = database
        self.fetch = fetch
    }

    /// Ascending daily bars covering `start` (and the days just before it) through today.
    /// Today's still-open bar is included but never stored. Failures fall back to whatever
    /// the database holds.
    func dailyBars(for asset: PortfolioAsset, from start: Date, now: Date = Date()) async -> [KlineData] {
        let key = asset.key
        while let running = inFlight[key] { _ = await running.value }
        let task = Task { [self] () -> [KlineData] in
            let bars = await load(asset, from: start, now: now)
            inFlight[key] = nil
            return bars
        }
        inFlight[key] = task
        return await task.value
    }

    private func load(_ asset: PortfolioAsset, from start: Date, now: Date) async -> [KlineData] {
        let source = asset.source.rawValue
        let symbol = asset.marketSymbol
        let needStart = start.timeIntervalSince1970 - Self.lookback
        let nowSeconds = now.timeIntervalSince1970
        var coverage = database.candleCoverage(source: source, symbol: symbol, interval: Self.interval)
        // A tail fetch can't reach back this far, so the stored candles would leave a hole.
        if let known = coverage, Self.days(nowSeconds - known.newest) > Self.maximumLimit { coverage = nil }
        let needsOlder = coverage.map { needStart < $0.coveredFrom } ?? true
        let stored = database.candles(source: source, symbol: symbol, interval: Self.interval, from: needStart)
            .map(Self.bar)

        if !needsOlder, let latest = recent[asset.key], now.timeIntervalSince(latest.at) < Self.recentWindow {
            return Self.merged(stored, latest.bars)
        }

        let limit =
            needsOlder
            ? min(Self.maximumLimit, Self.days(nowSeconds - needStart))
            : max(Self.minimumTail, Self.days(nowSeconds - (coverage?.newest ?? needStart)))
        let fetched: [KlineData]
        do {
            fetched = try await fetch(asset, limit)
        } catch {
            return stored
        }
        guard !fetched.isEmpty else { return stored }
        recent[asset.key] = (now, fetched)

        // The oldest returned bar can be partial (sources that fold a price series into
        // days start mid-day), and the +2 in `days` leaves room to drop it. Open bars stay out.
        let ascending = fetched.sorted { $0.openTime < $1.openTime }
        let kept = ascending.dropFirst().filter { $0.openTime.timeIntervalSince1970 + Self.day <= nowSeconds }
        if let first = kept.first, let last = kept.last {
            var coveredFrom = coverage?.coveredFrom ?? needStart
            if needsOlder {
                // A short answer means the source has nothing older, as good as reaching it.
                let reached = first.openTime.timeIntervalSince1970 <= needStart || fetched.count < limit
                let fetchedFrom = reached ? needStart : first.openTime.timeIntervalSince1970
                coveredFrom = min(coverage?.coveredFrom ?? fetchedFrom, fetchedFrom)
            }
            let updated = CandleCoverage(
                coveredFrom: coveredFrom, newest: max(coverage?.newest ?? 0, last.openTime.timeIntervalSince1970))
            database.storeCandles(
                kept.map(Self.record), coverage: updated, source: source, symbol: symbol, interval: Self.interval)
        }
        return Self.merged(stored, ascending)
    }

    /// Whole days spanned, plus slack for the dropped oldest bar and a partial current day.
    private static func days(_ seconds: TimeInterval) -> Int {
        Int((max(0, seconds) / day).rounded(.up)) + 2
    }

    private static func merged(_ stored: [KlineData], _ fresh: [KlineData]) -> [KlineData] {
        var byTime: [Double: KlineData] = [:]
        for bar in stored { byTime[bar.openTime.timeIntervalSince1970] = bar }
        for bar in fresh { byTime[bar.openTime.timeIntervalSince1970] = bar }
        return byTime.values.sorted { $0.openTime < $1.openTime }
    }

    private static func bar(_ record: CandleRecord) -> KlineData {
        KlineData(
            openTime: Date(timeIntervalSince1970: record.openTime), openPrice: record.open, highPrice: record.high,
            lowPrice: record.low, closePrice: record.close, volume: record.volume, quoteVolume: record.quoteVolume,
            isClosed: true)
    }

    private static func record(_ bar: KlineData) -> CandleRecord {
        CandleRecord(
            openTime: bar.openTime.timeIntervalSince1970, open: bar.openPrice, high: bar.highPrice,
            low: bar.lowPrice, close: bar.closePrice, volume: bar.volume, quoteVolume: bar.quoteVolume)
    }
}
