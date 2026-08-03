import Foundation

// MARK: - API Models

private struct CoinListItem: Codable {
    let id: String
    let symbol: String
    let name: String
}

private struct CoinSearchResult: Codable {
    let coins: [CoinSearchCoin]

    struct CoinSearchCoin: Codable {
        let id: String
        let symbol: String
        let name: String
        let marketCapRank: Int?
        let thumb: String?
        let large: String?

        enum CodingKeys: String, CodingKey {
            case id, symbol, name, thumb, large
            case marketCapRank = "market_cap_rank"
        }
    }
}

/// `/coins/markets` row, trimmed to the sparkline fields.
private struct MarketSparkline: Decodable {
    let id: String
    let lastUpdated: String?
    let sparklineIn7d: Sparkline?

    struct Sparkline: Decodable {
        let price: [Double]
    }

    enum CodingKeys: String, CodingKey {
        case id
        case lastUpdated = "last_updated"
        case sparklineIn7d = "sparkline_in_7d"
    }

    /// CoinGecko sends ISO-8601, sometimes with fractional seconds.
    var lastUpdatedDate: Date? {
        guard let lastUpdated else { return nil }
        let withFraction = ISO8601DateFormatter()
        withFraction.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let date = withFraction.date(from: lastUpdated) { return date }
        return ISO8601DateFormatter().date(from: lastUpdated)
    }
}

/// The coin IDs currently on screen, shared across concurrent fetches.
///
/// Keyed by tab, because every tab syncs its own list before each refetch. A
/// flat array here would make the last tab to sync evict the other tabs' coins
/// from the batched prime, dropping them back onto the rate-limited per-chart
/// queue.
private actor ActiveSymbols {
    private var byOwner: [UUID: [String]] = [:]

    /// Deduped union across every tab, order-stable so the request URL is cacheable.
    var current: [String] {
        var seen = Set<String>()
        return byOwner
            .sorted { $0.key.uuidString < $1.key.uuidString }
            .flatMap(\.value)
            .filter { seen.insert($0).inserted }
    }

    func set(_ symbols: [String], for owner: UUID) {
        byOwner[owner] = symbols
    }

    func remove(owner: UUID) {
        byOwner[owner] = nil
    }
}

// MARK: - Symbol → ID Cache

private struct CoinIDCache: Codable {
    var idMap: [String: String]   // "btc" → "bitcoin"
    var updatedAt: Date
}

// MARK: - Service

final class CoinGeckoAPIService: TickerDataSource {
    let type: DataSourceType = .coingecko

    /// Shared with the static icon lookup below, which runs without an instance.
    static let apiBase = "https://api.coingecko.com/api/v3"

    private let baseURL = CoinGeckoAPIService.apiBase
    private let session: URLSession
    private let cache: KlineCache
    private let rateLimiter = CGRateLimiter.shared
    private let provisional = CoinGeckoProvisionalStore()
    private let activeSymbols = ActiveSymbols()
    private var coinIDCache: CoinIDCache
    private let cacheURL: URL
    private var coinListRefreshTask: Task<Void, Never>?

    init() {
        self.session = AppSupport.defaultSession
        // Persisted: CoinGecko's rate limit makes a cold start expensive, so a
        // relaunch should paint from disk rather than from the request queue.
        //
        // The key carries a generation because entries hold candles as the source
        // built them. Stores written against `/ohlc` hold half-hour candles under
        // the same symbol-interval-days key, and a stale read of those is a first
        // paint that contradicts the timeframe.
        self.cache = KlineCache(persistenceKey: "coingecko2")
        cacheURL = AppSupport.directory.appendingPathComponent("coingecko_coin_ids.json")

        if let data = try? Data(contentsOf: cacheURL),
           let cached = try? JSONDecoder().decode(CoinIDCache.self, from: data) {
            coinIDCache = cached
        } else {
            coinIDCache = CoinIDCache(idMap: [:], updatedAt: .distantPast)
        }
    }

    // MARK: - Kline Fetching

    /// Return cached klines regardless of freshness — instant first render.
    /// Falls back to any window we hold for the coin so zoom and timeframe
    /// changes never blank the chart.
    func getCachedKlines(symbol: String, interval: String, count: Int) async -> [KlineData]? {
        let coinID = symbol.lowercased()
        let days = Self.marketChartDays(interval: interval)

        if let exact = await cache.getStale(symbol: coinID, interval: interval, days: days, count: count) {
            return exact
        }
        return await cache.getAnyStale(symbol: coinID, count: count)
    }

    func fetchKlines(symbol: String, interval: String, limit: Int) async throws -> [KlineData] {
        let coinID = symbol.lowercased()
        let days = Self.marketChartDays(interval: interval)

        if let cached = await cache.getFull(symbol: coinID, interval: interval, days: days,
                                            ttl: Timeout.coingeckoCacheTTL) {
            return Array(cached.suffix(limit))
        }

#if DEBUG
        print("[CoinGecko] Fetching market chart for \(coinID) days=\(days) limit=\(limit)")
#endif

        let candles = try await fetchCandles(coinID: coinID, interval: interval, days: days)
        let result = Array(candles.suffix(limit))

#if DEBUG
        print("[CoinGecko] \(coinID) candles=\(result.count) (built \(candles.count), limit \(limit))")
#endif

        await cache.set(symbol: coinID, interval: interval, days: days, data: candles)
        return result
    }

    /// One rate-limited price-series request, folded into candles of `interval`.
    private func fetchCandles(coinID: String, interval: String, days: Int,
                              attempts: Int = 2) async throws -> [KlineData] {
        guard let url = buildMarketChartURL(coinID: coinID, days: days) else {
            throw CoinGeckoError.invalidURL
        }

        for attempt in 1...attempts {
            await rateLimiter.waitForSlot()
            try Task.checkCancellation()

            do {
                let (data, response) = try await session.data(from: url)
                try await checkHTTPResponse(data: data, response: response, url: url)
                await rateLimiter.noteSuccess()
                return KlineData.candles(
                    from: try Self.parsePriceSeries(data),
                    interval: Self.intervalSeconds[interval] ?? 3_600
                )
            } catch CoinGeckoError.rateLimited where attempt < attempts {
                // `backoff` already pushed out the queue — the next
                // `waitForSlot()` sleeps through it.
                continue
            }
        }

        throw CoinGeckoError.rateLimited
    }

    /// Persist cached candles immediately — called on app termination.
    func flushCache() async {
        await cache.flush()
    }

    // MARK: - Provisional Candles (one request, every chart)

    /// The coins currently on screen in one tab. Set by that tab's view model so
    /// a single prime request can cover every tab's coins at once.
    func setActiveSymbols(_ symbols: [String], for owner: UUID) async {
        await activeSymbols.set(symbols.map { $0.lowercased() }, for: owner)
    }

    /// Drop a closed tab's coins from the prime.
    func clearActiveSymbols(for owner: UUID) async {
        await activeSymbols.remove(owner: owner)
    }

    /// Fetch 7-day hourly sparklines for every visible coin in one request.
    /// Deduplicated and TTL-guarded — concurrent charts share the one call.
    private func primeProvisionalCandles() async {
        let symbols = await activeSymbols.current

        // With a single chart there is nothing to amortise the extra request
        // over: its own price-series fetch is already first in the queue.
        guard symbols.count > 1 else { return }

        await provisional.ensurePrimed(ttl: Timeout.coingeckoProvisionalTTL) { [weak self] in
            guard let self else { return [:] }
            return await self.fetchSparklines(coinIDs: symbols)
        }
    }

    /// One `/coins/markets` call returning 7 days of hourly prices per coin.
    private func fetchSparklines(coinIDs: [String]) async -> [String: [CoinGeckoProvisionalStore.PricePoint]] {
        guard !coinIDs.isEmpty,
              var components = URLComponents(string: "\(baseURL)/coins/markets")
        else { return [:] }

        components.queryItems = [
            URLQueryItem(name: "vs_currency", value: "usd"),
            URLQueryItem(name: "ids", value: coinIDs.joined(separator: ",")),
            URLQueryItem(name: "sparkline", value: "true"),
            URLQueryItem(name: "per_page", value: String(min(coinIDs.count, CoinGecko.pageLimit))),
        ]
        guard let url = components.url else { return [:] }

        await rateLimiter.waitForSlot()
        guard !Task.isCancelled else { return [:] }

        do {
#if DEBUG
            print("[CoinGecko] Priming sparklines for \(coinIDs.count) coins")
#endif
            let (data, response) = try await session.data(from: url)
            try await checkHTTPResponse(data: data, response: response, url: url)
            await rateLimiter.noteSuccess()

            let coins = try JSONDecoder().decode([MarketSparkline].self, from: data)
            return Self.pricePoints(from: coins)
        } catch {
#if DEBUG
            print("[CoinGecko] Sparkline prime failed: \(error.localizedDescription)")
#endif
            return [:]
        }
    }

    /// Sparkline prices are evenly spaced hourly samples ending at `last_updated`.
    private static func pricePoints(
        from coins: [MarketSparkline]
    ) -> [String: [CoinGeckoProvisionalStore.PricePoint]] {
        var result: [String: [CoinGeckoProvisionalStore.PricePoint]] = [:]

        for coin in coins {
            let prices = coin.sparklineIn7d?.price ?? []
            guard prices.count >= 2 else { continue }

            let end = coin.lastUpdatedDate ?? Date()
            let step: TimeInterval = 3_600
            let points = prices.enumerated().map { index, price in
                CoinGeckoProvisionalStore.PricePoint(
                    time: end.addingTimeInterval(-Double(prices.count - 1 - index) * step),
                    price: price
                )
            }
            result[coin.id.lowercased()] = points
        }
        return result
    }

    // MARK: - Staged Fetch (cached → provisional → real)

    /// Emits candles as they become available instead of only once the full
    /// window has loaded:
    ///
    /// 1. Cached candles — any window we hold for the coin — yielded immediately.
    /// 2. Provisional candles built from the shared 7-day sparkline prime, which
    ///    covers every visible coin in a single request.
    /// 3. The real target window, which supersedes the earlier batches.
    ///
    /// A per-chart "fetch a small window first" probe would not help here: every
    /// request costs the same slot in the shared queue, so a probe just pushes
    /// this chart's real data one slot later without arriving any sooner than it
    /// would have. Only the batched prime beats the queue, because it pays one
    /// request for all charts at once.
    func fetchKlinesStaged(
        symbol: String,
        interval: String,
        limit: Int,
        needsFirstPaint: Bool
    ) -> AsyncThrowingStream<[KlineData], Error> {
        AsyncThrowingStream { continuation in
            let coinID = symbol.lowercased()
            let fullDays = Self.marketChartDays(interval: interval)

            let task = Task {
                do {
                    // Fresh cache hit → yield all at once, skip fetching. This is the
                    // path a zoom takes: the window is the same whatever the count, so
                    // a wider or narrower view is a different slice of it, not another
                    // trip through the rate-limited queue.
                    if let cached = await cache.getFull(
                        symbol: coinID, interval: interval,
                        days: fullDays, ttl: Timeout.coingeckoCacheTTL
                    ) {
                        continuation.yield(Array(cached.suffix(limit)))
                        continuation.finish()
                        return
                    }

                    // Stale candles for this window, else any window we hold for
                    // this coin — either renders instantly.
                    var hasCandles = !needsFirstPaint
                    var stale = await cache.getStale(
                        symbol: coinID, interval: interval, days: fullDays, count: limit
                    )
                    if stale == nil {
                        stale = await cache.getAnyStale(symbol: coinID, count: limit)
                    }

                    if let stale, !stale.isEmpty {
                        continuation.yield(stale)
                        hasCandles = true
                    }

                    // Nothing cached: fall back to the shared sparkline prime.
                    // The first chart to ask triggers one request covering every
                    // visible coin, so all of them paint together — well before
                    // their own price-series requests come up in the queue.
                    if !hasCandles {
                        await primeProvisionalCandles()
                        try Task.checkCancellation()

                        let granularity = Self.intervalSeconds[interval] ?? 3_600
                        let approximate = await provisional.candles(
                            for: coinID, granularity: granularity, limit: limit
                        )
                        if let approximate, !approximate.isEmpty {
                        #if DEBUG
                            print("[CoinGecko] Provisional: \(approximate.count) candles for \(coinID)")
                        #endif
                            continuation.yield(approximate)
                        }
                    }

                    // The window actually requested.
                #if DEBUG
                    print("[CoinGecko] Fetching \(fullDays)d for \(coinID)")
                #endif
                    let candles = try await fetchCandles(
                        coinID: coinID, interval: interval, days: fullDays
                    )
                    let result = Array(candles.suffix(limit))

                    await cache.set(symbol: coinID, interval: interval, days: fullDays, data: candles)
                    continuation.yield(result)

                #if DEBUG
                    print("[CoinGecko] \(coinID) candles=\(result.count) (built \(candles.count), limit \(limit))")
                #endif

                    continuation.finish()
                } catch {
                    continuation.finish(throwing: error)
                }
            }

            // A dropped consumer (chart removed, zoom superseded) must release
            // its rate-limiter slots instead of holding up the other charts.
            continuation.onTermination = { _ in task.cancel() }
        }
    }

    // MARK: - URL Building

    private func buildMarketChartURL(coinID: String, days: Int) -> URL? {
        guard let encodedID = coinID.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) else {
            return nil
        }
        guard var components = URLComponents(string: "\(baseURL)/coins/\(encodedID)/market_chart") else {
            return nil
        }
        components.queryItems = [
            URLQueryItem(name: "vs_currency", value: "usd"),
            URLQueryItem(name: "days", value: String(days)),
        ]
        return components.url
    }

    // MARK: - HTTP Response Validation

    private func checkHTTPResponse(data: Data, response: URLResponse, url: URL) async throws {
        guard let httpResponse = response as? HTTPURLResponse else {
            throw CoinGeckoError.invalidResponse
        }
        switch httpResponse.statusCode {
        case 200:
            return
        case 429:
            let retryAfter = httpResponse.value(forHTTPHeaderField: "Retry-After")
            let waitSeconds = retryAfter.flatMap(Int.init) ?? 30
#if DEBUG
            print("[CoinGecko] 429 rate limited. Backing off \(waitSeconds)s…")
#endif
            await rateLimiter.backoff(seconds: waitSeconds)
            throw CoinGeckoError.rateLimited
        case 404:
            throw CoinGeckoError.coinNotFound(url.lastPathComponent)
        case 400:
            let body = String(data: data, encoding: .utf8) ?? ""
#if DEBUG
            print("[CoinGecko] 400 Bad Request for URL: \(url.absoluteString)")
            print("[CoinGecko] Response: \(body)")
#endif
            throw CoinGeckoError.httpError(400)
        default:
            throw CoinGeckoError.httpError(httpResponse.statusCode)
        }
    }

    // MARK: - JSON Parsing

    /// `/market_chart` returns parallel series; only `prices` is charted.
    /// Layout: `{"prices": [[timestamp_ms, price], …], …}`, oldest first.
    private static func parsePriceSeries(_ data: Data) throws -> [(time: Date, price: Double)] {
        guard let json = try JSONSerialization.jsonObject(with: data) as? [String: Any],
              let prices = json["prices"] as? [[Any]]
        else {
            throw CoinGeckoError.parseError("Expected a prices array")
        }

        let points: [(time: Date, price: Double)] = prices.compactMap { row in
            guard row.count >= 2,
                  let ts = KlineData.extractTimestamp(row[0]),
                  let price = KlineData.extractDouble(row[1])
            else { return nil }
            return (Date(timeIntervalSince1970: ts / 1000), price)
        }
        return points.sorted { $0.time < $1.time }
    }

    // MARK: - Window Selection

    /// Binance-style interval → seconds, the candle size a chart is asking for.
    private static let intervalSeconds: [String: TimeInterval] = [
        "1m": 60, "5m": 300, "15m": 900, "30m": 1_800,
        "1h": 3_600, "4h": 14_400,
        "1d": 86_400, "1w": 604_800, "1M": 2_592_000,
    ]

    /// Days of `/market_chart` history to request for candles of `interval`.
    ///
    /// This is the endpoint the charts are built from rather than `/ohlc`, because
    /// `/ohlc` takes its candle size from a fixed set of windows — 30 minutes for a
    /// 1-day window, 4 hours up to 30 days, 4 days beyond — and none of them is the
    /// size any timeframe here asks for. `/market_chart` takes a free-form `days` and
    /// picks its sample step from it:
    ///   1 day    → 5-minute samples
    ///   2–90     → hourly
    ///   90+      → daily
    ///
    /// Every step is fine enough to fold into the timeframe above it, so a window of
    /// `n` days yields the same candles a Binance chart draws for that span — same
    /// count, same width, same hours.
    ///
    /// Deliberately a function of `interval` alone, not of how many candles are on
    /// screen. `days` is part of the cache key, so a window that tracked the zoom
    /// level would change key on a zoom step, and the new key means a network fetch
    /// through a queue that clears a few requests a minute — during which the card
    /// keeps drawing the buffer it already had, which reads as a chart that ignores
    /// the zoom. One window per interval means every zoom level after the first is
    /// served by slicing a buffer already in hand.
    ///
    /// So the window is the deepest zoom the chart offers, bounded by the step the
    /// span comes back at: past 90 days CoinGecko samples daily, which can't build an
    /// hourly candle, and a year is as far back as the public tier goes.
    static func marketChartDays(interval: String) -> Int {
        let target = intervalSeconds[interval] ?? 3_600
        let deepest = (Double(Candle.maxCandles) * target / 86_400).rounded(.up)
        let finestEnoughStep = target < 3_600 ? 1 : (target < 86_400 ? 90 : 365)
        return Swift.min(Int(deepest), finestEnoughStep, 365).clamped(to: 1...365)
    }

    // MARK: - Search

    func searchTickers(query: String) async throws -> [TickerSearchResult] {
        let q = query.trimmingCharacters(in: .whitespaces).lowercased()
        guard !q.isEmpty else { return [] }

        // Refresh coin list cache in background if stale
        if coinIDCache.updatedAt.timeIntervalSinceNow < -Timeout.coinListStaleness {
            refreshCoinListInBackground()
        }

        return try await Self.searchCoins(query: q).map { coin in
            TickerSearchResult(
                symbol: coin.symbol.uppercased(),
                fullSymbol: coin.id,
                source: .coingecko,
                price: nil
            )
        }
    }

    /// The `/search` request itself — shared by ticker search and icon resolution.
    /// Static so `IconResolver` can reuse this decode path without reaching into
    /// (non-Sendable) instance state from another actor.
    private static func searchCoins(query: String) async throws -> [CoinSearchResult.CoinSearchCoin] {
        guard var components = URLComponents(string: "\(apiBase)/search") else {
            throw CoinGeckoError.invalidURL
        }
        components.queryItems = [URLQueryItem(name: "query", value: query)]

        guard let url = components.url else {
            throw CoinGeckoError.invalidURL
        }

#if DEBUG
        print("[CoinGecko] Search: \(query)")
#endif

        let (data, response) = try await AppSupport.defaultSession.data(from: url)

        guard let httpResponse = response as? HTTPURLResponse else {
            throw CoinGeckoError.invalidResponse
        }
        if httpResponse.statusCode == 429 {
            let waitSeconds = httpResponse.value(forHTTPHeaderField: "Retry-After").flatMap(Int.init) ?? 30
            await CGRateLimiter.shared.backoff(seconds: waitSeconds)
            throw CoinGeckoError.rateLimited
        }
        guard httpResponse.statusCode == 200 else {
            throw CoinGeckoError.httpError(httpResponse.statusCode)
        }

        return try JSONDecoder().decode(CoinSearchResult.self, from: data).coins
    }

    // MARK: - Icon Lookup

    /// Icon for a symbol the market snapshot didn't cover — `/search` reaches every
    /// coin CoinGecko knows, not just the top few hundred by market cap.
    ///
    /// Prefers an exact symbol match and, among those, the best-ranked coin, so "btc"
    /// resolves to bitcoin rather than to a same-ticker copycat.
    static func iconURL(searchingFor symbol: String) async -> URL? {
        let q = symbol.trimmingCharacters(in: .whitespaces).lowercased()
        guard !q.isEmpty else { return nil }

        await CGRateLimiter.shared.waitForSlot()
        guard !Task.isCancelled else { return nil }

        do {
            let coins = try await searchCoins(query: q)
            await CGRateLimiter.shared.noteSuccess()

            let exact = coins.filter { $0.symbol.lowercased() == q }
            let candidates = exact.isEmpty ? coins : exact
            let best = candidates.min { ($0.marketCapRank ?? .max) < ($1.marketCapRank ?? .max) }

            guard let image = best?.large ?? best?.thumb else { return nil }
            return URL(string: image)
        } catch {
#if DEBUG
            print("[CoinGecko] Icon search failed for \(q): \(error.localizedDescription)")
#endif
            return nil
        }
    }

    // MARK: - Coin List Cache

    private func refreshCoinListInBackground() {
        guard coinListRefreshTask == nil else { return }
        coinListRefreshTask = Task { [weak self] in
            defer { self?.coinListRefreshTask = nil }
            guard let self else { return }

            guard let url = URL(string: "\(self.baseURL)/coins/list") else { return }

            do {
                let (data, _) = try await self.session.data(from: url)
                let list = try JSONDecoder().decode([CoinListItem].self, from: data)

                var map: [String: String] = [:]
                for coin in list {
                    if map[coin.symbol] == nil {
                        map[coin.symbol] = coin.id
                    }
                }
                self.coinIDCache = CoinIDCache(idMap: map, updatedAt: Date())
                self.saveCoinIDCache()
#if DEBUG
                print("[CoinGecko] Coin list refreshed: \(map.count) symbols")
#endif
            } catch {
#if DEBUG
                print("[CoinGecko] Coin list refresh failed: \(error.localizedDescription)")
#endif
            }
        }
    }

    private func saveCoinIDCache() {
        guard let data = try? JSONEncoder().encode(coinIDCache) else { return }
        try? data.write(to: cacheURL, options: .atomic)
    }
}

// MARK: - Errors

enum CoinGeckoError: LocalizedError {
    case invalidURL
    case invalidResponse
    case httpError(Int)
    case coinNotFound(String)
    case rateLimited
    case parseError(String)

    var errorDescription: String? {
        switch self {
        case .invalidURL:           return "Invalid URL"
        case .invalidResponse:      return "Unexpected response from CoinGecko"
        case .httpError(let code):  return "CoinGecko server error (HTTP \(code))"
        case .coinNotFound(let id): return "Coin \"\(id)\" not found on CoinGecko"
        case .rateLimited:          return "CoinGecko rate limited. Wait and retry."
        case .parseError(let d):    return "CoinGecko data error: \(d)"
        }
    }
}
