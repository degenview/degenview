import Foundation

// MARK: - Errors

enum CoinbaseAPIError: LocalizedError {
    case invalidURL
    case invalidResponse
    case httpError(Int)
    case symbolNotFound(String)
    case rateLimited
    case unsupportedInterval(String)
    case parseError(String)

    var errorDescription: String? {
        switch self {
        case .invalidURL:
            return "Invalid URL"
        case .invalidResponse:
            return "Unexpected response from server"
        case .httpError(let code):
            return "Server error (HTTP \(code))"
        case .symbolNotFound(let symbol):
            return "Ticker \"\(symbol)\" not found"
        case .rateLimited:
            return "Too many requests. Wait a moment and try again."
        case .unsupportedInterval(let interval):
            return "Coinbase has no \(interval) candles"
        case .parseError(let detail):
            return "Data error: \(detail)"
        }
    }
}

// MARK: - Service

/// Coinbase Exchange public market data — no API key.
///
/// Differences from Binance that shape this file:
/// - Candles arrive newest-first as `[time, low, high, open, close, volume]`, with no
///   quote volume, and only in six sizes. Weekly and monthly charts are folded from daily.
/// - One request spans at most 300 candles, so deep history is paged backwards.
/// - Public REST is limited to about 10 requests a second per IP; requests are spaced out.
final class CoinbaseAPIService: GranularReplayDataSource {
    let type: DataSourceType = .coinbase

    private let session: URLSession
    private let baseURL: String
    /// Candles as displayed (after folding), keyed by the chart's interval token.
    private let cache = KlineCache()
    /// Candles as Coinbase sends them, merged across refreshes so a refresh costs one request.
    private let sourceCache = KlineCache()
    private let state = State()
    private let gate: RequestGate

    init(
        session: URLSession = AppSupport.defaultSession,
        baseURL: String = "https://api.exchange.coinbase.com",
        requestSpacing: TimeInterval = 0.12
    ) {
        self.session = session
        self.baseURL = baseURL
        self.gate = RequestGate(spacing: requestSpacing)
    }

    // MARK: - Klines

    func getCachedKlines(symbol: String, interval: String, count: Int) async -> [KlineData]? {
        await cache.getStale(symbol: Self.productID(symbol), interval: interval, days: 0, count: count)
    }

    func fetchKlines(symbol: String, interval: String, limit: Int) async throws -> [KlineData] {
        guard let plan = CoinbaseGranularity(interval: interval) else {
            throw CoinbaseAPIError.unsupportedInterval(interval)
        }
        let id = Self.productID(symbol)

        if let cached = await cache.get(
            symbol: id, interval: interval, days: 0, count: limit, ttl: Timeout.coinbaseCacheTTL)
        {
            return cached
        }

        // A folded candle is built from up to `sourceCandlesPerTarget` source candles, and the
        // oldest one is usually partial — fetch one bucket more than the chart will show.
        let needed = limit * plan.sourceCandlesPerTarget + (plan.needsAggregation ? plan.sourceCandlesPerTarget : 0)
        let source = try await sourceCandles(id: id, granularity: plan.source, needed: needed)
        let shown = plan.needsAggregation ? Self.fold(source, into: plan) : source
        let result = Array(Self.sanitized(shown).suffix(limit))

        await cache.set(symbol: id, interval: interval, days: 0, data: result)
        return result
    }

    /// Source candles, oldest first: what's held, brought up to date, then extended back to `needed`.
    private func sourceCandles(id: String, granularity: Int, needed: Int) async throws -> [KlineData] {
        let key = "\(granularity)s"
        let exhaustedKey = "\(id)-\(key)"
        var candles = await sourceCache.getStale(symbol: id, interval: key, days: 0, count: Int.max) ?? []

        // Newest page first. It also tells us whether what we hold still joins up with now.
        let newest = try await fetchPage(id: id, granularity: granularity, start: nil, end: nil)
        if let held = candles.last, let head = newest.first, held.openTime < head.openTime {
            // Left alone long enough that a hole opened between the held tail and the new page.
            candles = []
            await state.setExhausted(false, key: exhaustedKey)
        }
        candles = Self.merge(candles, newest)

        while candles.count < needed, let oldest = candles.first?.openTime {
            if await state.isExhausted(exhaustedKey) { break }
            let span = Double(granularity * CoinbaseGranularity.pageBuckets)
            let page = try await fetchPage(
                id: id, granularity: granularity, start: oldest.addingTimeInterval(-span), end: oldest)
            let older = page.filter { $0.openTime < oldest }
            guard !older.isEmpty else {
                // Nothing further back: the product's history starts here. Remember it, or every
                // refresh of a young listing would walk back to its first candle again.
                await state.setExhausted(true, key: exhaustedKey)
                break
            }
            candles = older + candles
        }

        await sourceCache.set(symbol: id, interval: key, days: 0, data: candles)
        return candles
    }

    /// One `/candles` request, returned oldest first. Without `start`/`end` Coinbase sends its latest 350.
    private func fetchPage(id: String, granularity: Int, start: Date?, end: Date?) async throws -> [KlineData] {
        var query = [URLQueryItem(name: "granularity", value: String(granularity))]
        if let start, let end {
            query.append(URLQueryItem(name: "start", value: Self.timestamp(start)))
            query.append(URLQueryItem(name: "end", value: Self.timestamp(end)))
        }
        let data = try await get("/products/\(id)/candles", query: query, symbol: id)
        guard let rows = try JSONSerialization.jsonObject(with: data) as? [[Any]] else {
            throw CoinbaseAPIError.parseError("Expected array of candles")
        }
        let candles = rows.compactMap(Self.candle(from:))
        if candles.isEmpty, !rows.isEmpty {
            throw CoinbaseAPIError.parseError("Failed to parse candle data")
        }
        return candles.sorted { $0.openTime < $1.openTime }
    }

    // MARK: - Parsing and folding

    /// `[time s, low, high, open, close, volume]`. Coinbase reports no turnover, so
    /// `quoteVolume` — what the volume bars plot — is the volume priced at the candle's OHLC average.
    static func candle(from row: [Any]) -> KlineData? {
        guard row.count >= 6,
            let seconds = KlineData.extractTimestamp(row[0]),
            let low = KlineData.extractDouble(row[1]),
            let high = KlineData.extractDouble(row[2]),
            let open = KlineData.extractDouble(row[3]),
            let close = KlineData.extractDouble(row[4])
        else { return nil }

        let volume = KlineData.extractDouble(row[5]) ?? 0
        return KlineData(
            openTime: Date(timeIntervalSince1970: seconds),
            openPrice: open,
            highPrice: high,
            lowPrice: low,
            closePrice: close,
            volume: volume,
            quoteVolume: volume * (open + high + low + close) / 4
        )
    }

    /// Fold ascending daily candles into weekly or monthly ones on the same boundaries Binance uses.
    static func fold(_ candles: [KlineData], into plan: CoinbaseGranularity) -> [KlineData] {
        var folded: [KlineData] = []
        for candle in candles {
            let start = plan.bucketStart(of: candle.openTime)
            if let last = folded.last, last.openTime == start {
                let index = folded.count - 1
                folded[index].highPrice = max(last.highPrice, candle.highPrice)
                folded[index].lowPrice = min(last.lowPrice, candle.lowPrice)
                folded[index].closePrice = candle.closePrice
                folded[index].volume += candle.volume
                folded[index].quoteVolume += candle.quoteVolume
            } else {
                folded.append(
                    KlineData(
                        openTime: start, openPrice: candle.openPrice, highPrice: candle.highPrice,
                        lowPrice: candle.lowPrice, closePrice: candle.closePrice,
                        volume: candle.volume, quoteVolume: candle.quoteVolume))
            }
        }
        return folded
    }

    /// Union by open time, oldest first. `fresh` wins where both hold a candle — the newest one is still forming.
    static func merge(_ held: [KlineData], _ fresh: [KlineData]) -> [KlineData] {
        var byTime: [Date: KlineData] = [:]
        for candle in held { byTime[candle.openTime] = candle }
        for candle in fresh { byTime[candle.openTime] = candle }
        return byTime.values.sorted { $0.openTime < $1.openTime }
    }

    private static func sanitized(_ candles: [KlineData]) -> [KlineData] {
        var seen = Set<Date>()
        return
            candles
            .filter { candle in
                candle.openPrice.isFinite && candle.highPrice.isFinite && candle.lowPrice.isFinite
                    && candle.closePrice.isFinite && candle.volume.isFinite && candle.highPrice >= candle.lowPrice
                    && seen.insert(candle.openTime).inserted
            }
            .sorted { $0.openTime < $1.openTime }
    }

    /// Coinbase ids are `BASE-QUOTE`. Accept `BASE/QUOTE` and a bare `BASE` (which means USD).
    static func productID(_ symbol: String) -> String {
        let id = symbol.trimmingCharacters(in: .whitespaces).uppercased().replacingOccurrences(of: "/", with: "-")
        return id.contains("-") ? id : "\(id)-USD"
    }

    private static func timestamp(_ date: Date) -> String {
        ISO8601DateFormatter().string(from: date)
    }

    // MARK: - Replay

    func supportedReplayIntervals(chartInterval: String) -> [ReplayInterval] {
        let chartSeconds = CoinbaseGranularity(interval: chartInterval)?.target ?? 0
        return ReplayInterval.allCases.filter { interval in
            guard let seconds = interval.seconds else { return interval == .automatic || interval == .chartBar }
            // No 30m candles on Coinbase.
            guard let token = interval.apiInterval, CoinbaseGranularity(interval: token) != nil else { return false }
            return seconds <= chartSeconds
        }
    }

    func fetchReplayKlines(
        symbol: String,
        interval: ReplayInterval,
        start: Date,
        end: Date,
        maximumCount: Int = 100_000
    ) async throws -> [KlineData] {
        guard let token = interval.apiInterval, let plan = CoinbaseGranularity(interval: token), start <= end
        else { return [] }

        let id = Self.productID(symbol)
        let span = Double(plan.source * CoinbaseGranularity.pageBuckets)
        var cursor = start
        var result: [KlineData] = []
        result.reserveCapacity(min(maximumCount, 10_000))

        while cursor <= end, result.count < maximumCount {
            let pageEnd = min(end, cursor.addingTimeInterval(span))
            result.append(
                contentsOf: try await fetchPage(id: id, granularity: plan.source, start: cursor, end: pageEnd))
            // An empty window is a quiet stretch, not the end of history — step over it.
            cursor = pageEnd.addingTimeInterval(TimeInterval(plan.source))
        }
        return Array(Self.sanitized(result).filter { $0.openTime >= start }.prefix(maximumCount))
    }

    // MARK: - Search

    /// Online, tradable pairs whose id contains the query, USD-quoted first. Up to 20.
    func searchTickers(query: String) async throws -> [TickerSearchResult] {
        let q = Self.compact(query)
        guard !q.isEmpty else { return [] }

        let products = try await listedProducts()
        return
            products
            .filter { $0.status == "online" && !$0.tradingDisabled && Self.compact($0.id).contains(q) }
            .sorted { lhs, rhs in
                // Ranking keeps equal scores in this order, so USD leads its USDT twin.
                let (l, r) = (Self.quotePriority(lhs.quoteCurrency), Self.quotePriority(rhs.quoteCurrency))
                return l == r ? lhs.id < rhs.id : l < r
            }
            .map {
                TickerSearchResult(
                    symbol: "\($0.baseCurrency)/\($0.quoteCurrency)",
                    fullSymbol: $0.id,
                    source: .coinbase,
                    price: nil
                )
            }
            .ranked(for: query)
            .prefix(20)
            .map { $0 }
    }

    private static func quotePriority(_ quote: String) -> Int {
        switch quote {
        case "USD": return 0
        case "USDT": return 1
        case "USDC": return 2
        default: return 3
        }
    }

    /// `BTC-USD`, `btc/usd` and `BTCUSD` all compare equal.
    private static func compact(_ text: String) -> String {
        text.uppercased().filter { $0.isLetter || $0.isNumber }
    }

    /// The whole catalogue is one ~840-row request with no server-side filter, so it's held for a while.
    private func listedProducts() async throws -> [CoinbaseProduct] {
        if let cached = await state.products() { return cached }
        let data = try await get("/products", query: [], symbol: "products")
        let products = try JSONDecoder().decode([CoinbaseProduct].self, from: data)
        await state.setProducts(products)
        return products
    }

    // MARK: - Transport

    private func get(_ path: String, query: [URLQueryItem], symbol: String) async throws -> Data {
        guard var components = URLComponents(string: baseURL + path) else { throw CoinbaseAPIError.invalidURL }
        components.queryItems = query.isEmpty ? nil : query
        guard let url = components.url else { throw CoinbaseAPIError.invalidURL }

        await gate.wait()
        let (data, response) = try await session.data(from: url)
        guard let http = response as? HTTPURLResponse else { throw CoinbaseAPIError.invalidResponse }

        switch http.statusCode {
        case 200: return data
        case 404: throw CoinbaseAPIError.symbolNotFound(symbol)
        case 429: throw CoinbaseAPIError.rateLimited
        default: throw CoinbaseAPIError.httpError(http.statusCode)
        }
    }
}

// MARK: - Support types

private struct CoinbaseProduct: Decodable {
    let id: String
    let baseCurrency: String
    let quoteCurrency: String
    let status: String
    let tradingDisabled: Bool

    private enum CodingKeys: String, CodingKey {
        case id, status
        case baseCurrency = "base_currency"
        case quoteCurrency = "quote_currency"
        case tradingDisabled = "trading_disabled"
    }
}

extension CoinbaseAPIService {
    /// Mutable bookkeeping shared by concurrent chart fetches.
    fileprivate actor State {
        private var catalogue: (products: [CoinbaseProduct], fetchedAt: Date)?
        private var exhausted: Set<String> = []

        func products() -> [CoinbaseProduct]? {
            guard let catalogue, Date().timeIntervalSince(catalogue.fetchedAt) < 600 else { return nil }
            return catalogue.products
        }

        func setProducts(_ products: [CoinbaseProduct]) {
            catalogue = (products, Date())
        }

        func isExhausted(_ key: String) -> Bool { exhausted.contains(key) }

        func setExhausted(_ value: Bool, key: String) {
            if value { exhausted.insert(key) } else { exhausted.remove(key) }
        }
    }

    /// Hands out request slots `spacing` seconds apart, keeping paged history under Coinbase's 10 requests/s.
    fileprivate actor RequestGate {
        private let spacing: TimeInterval
        private var nextSlot = Date.distantPast

        init(spacing: TimeInterval) { self.spacing = spacing }

        func wait() async {
            let now = Date()
            let slot = max(now, nextSlot)
            nextSlot = slot.addingTimeInterval(spacing)
            let delay = slot.timeIntervalSince(now)
            if delay > 0 { try? await Task.sleep(nanoseconds: UInt64(delay * 1_000_000_000)) }
        }
    }
}
