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
    }
}

// MARK: - Symbol → ID Cache

private struct CoinIDCache: Codable {
    var idMap: [String: String]   // "btc" → "bitcoin"
    var updatedAt: Date
}

// MARK: - Service

// MARK: - Rate Limiter

/// Enforces minimum interval between CoinGecko API calls to stay under ~25 req/min.
private actor CGRateLimiter {
    private var lastCallTime: Date = .distantPast

    /// ≤20 calls/min, safe under ~30/min limit.
    private let minGap = Timeout.coingeckoRateLimitGap

    /// Wait until we're clear to make another call. Returns immediately if gap satisfied.
    func waitForSlot() async {
        let now = Date()
        let nextSlot = lastCallTime.addingTimeInterval(minGap)
        if now < nextSlot {
            let wait = nextSlot.timeIntervalSince(now)
            try? await Task.sleep(nanoseconds: UInt64(wait * 1_000_000_000))
        }
        lastCallTime = Date()
    }

    /// Back off after a 429.
    func backoff(seconds: Int) async {
        try? await Task.sleep(nanoseconds: UInt64(seconds) * 1_000_000_000)
        lastCallTime = Date()  // reset — we've waited
    }
}

// MARK: - Service

final class CoinGeckoAPIService: TickerDataSource {
    let type: DataSourceType = .coingecko

    private let baseURL = "https://api.coingecko.com/api/v3"
    private let session: URLSession
    private let cache: KlineCache
    private let rateLimiter = CGRateLimiter()
    private var coinIDCache: CoinIDCache
    private let cacheURL: URL
    private var coinListRefreshTask: Task<Void, Never>?

    init() {
        self.session = AppSupport.defaultSession
        self.cache = KlineCache()
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
    func getCachedKlines(symbol: String, interval: String, count: Int) async -> [KlineData]? {
        let days = daysForInterval(interval, limit: count)
        return await cache.getStale(symbol: symbol, interval: interval, days: days, count: count)
    }

    func fetchKlines(symbol: String, interval: String, limit: Int) async throws -> [KlineData] {
        let coinID = symbol.lowercased()
        let days = daysForInterval(interval, limit: limit)

        if let cached = await cache.get(symbol: coinID, interval: interval, days: days, count: limit, ttl: Timeout.coingeckoCacheTTL) {
            return cached
        }

        await rateLimiter.waitForSlot()

        guard let url = buildOHLCURL(coinID: coinID, days: days) else {
            throw CoinGeckoError.invalidURL
        }

#if DEBUG
        print("[CoinGecko] Fetching OHLC for \(coinID) days=\(days) limit=\(limit)")
#endif

        let (data, response) = try await session.data(from: url)
        try await checkHTTPResponse(data: data, response: response, url: url)

        let sorted = try parseOHLCResponse(data)
        let result = Array(sorted.suffix(limit))

#if DEBUG
        print("[CoinGecko] \(coinID) candles=\(result.count) (fetched \(sorted.count), limit \(limit))")
#endif

        await cache.set(symbol: coinID, interval: interval, days: days, data: sorted)
        return result
    }

    // MARK: - Staged Fetch (fast-first, then full)

    /// Fetches klines in two stages so the chart renders almost immediately:
    /// 1. Fetch 1 day of data → tiny response, ~500 ms → candles appear fast.
    /// 2. Fetch the full day range → chart fills in completely.
    ///
    /// Between stages the rate limiter enforces a 3 s gap, but the user sees
    /// candles from stage 1 the whole time — no spinner after ~500 ms.
    func fetchKlinesStaged(
        symbol: String,
        interval: String,
        limit: Int
    ) -> AsyncThrowingStream<[KlineData], Error> {
        AsyncThrowingStream { continuation in
            let coinID = symbol.lowercased()
            let fullDays = daysForInterval(interval, limit: limit)

            Task {
                do {
                    // Cache hit → yield all at once, skip fetching.
                    if let cached = await cache.get(
                        symbol: coinID, interval: interval,
                        days: fullDays, count: limit, ttl: Timeout.coingeckoCacheTTL
                    ) {
                        continuation.yield(cached)
                        continuation.finish()
                        return
                    }

                    // Yield stale cache first so chart renders instantly.
                    if let stale = await cache.getStale(
                        symbol: coinID, interval: interval, days: fullDays, count: limit
                    ) {
                        continuation.yield(stale)
                    }

                    // Stage 1: fetch the smallest window for instant display.
                    // 1 day of data is a tiny response — arrives in ~500 ms.
                    // CoinGecko returns 30-minute candles for a 1-day window,
                    // so it supplies ~48. Only run stage 1 when that satisfies
                    // the full limit; otherwise skip straight to stage 2 so
                    // rapid zooming never leaves the chart stuck with a wrong
                    // candle count.
                    let stage1 = Self.ohlcWindows[0]
                    let needStage1 = fullDays > stage1.days && limit <= stage1.supply

                    if needStage1, let url1 = buildOHLCURL(coinID: coinID, days: stage1.days) {
                        do {
                            await rateLimiter.waitForSlot()

                        #if DEBUG
                            print("[CoinGecko] Stage 1: fetching \(stage1.days)d for \(coinID)")
                        #endif

                            let (data1, response1) = try await session.data(from: url1)
                            try await checkHTTPResponse(data: data1, response: response1, url: url1)
                            let sorted1 = try parseOHLCResponse(data1)
                            let batch1 = Array(sorted1.suffix(limit))

                            if !batch1.isEmpty {
                                continuation.yield(batch1)
                            }
                        } catch {
                            // Stage 1 is a best-effort fast path. On failure,
                            // fall through to stage 2 — don't break the stream.
                        #if DEBUG
                            print("[CoinGecko] Stage 1 failed, proceeding to stage 2: \(error)")
                        #endif
                        }
                    }

                    // Stage 2: always run the full fetch.
                    await rateLimiter.waitForSlot()
                #if DEBUG
                    print("[CoinGecko] Stage 2: fetching \(fullDays)d for \(coinID)")
                #endif

                    guard let url = buildOHLCURL(coinID: coinID, days: fullDays) else {
                        throw CoinGeckoError.invalidURL
                    }
                    let (data, response) = try await session.data(from: url)
                    try await checkHTTPResponse(data: data, response: response, url: url)
                    let sorted = try parseOHLCResponse(data)
                    let result = Array(sorted.suffix(limit))

                    await cache.set(symbol: coinID, interval: interval, days: fullDays, data: sorted)
                    continuation.yield(result)

                #if DEBUG
                    print("[CoinGecko] Staged fetch complete for \(coinID)")
                #endif

                    continuation.finish()
                } catch {
                    continuation.finish(throwing: error)
                }
            }
        }
    }

    // MARK: - URL Building

    private func buildOHLCURL(coinID: String, days: Int) -> URL? {
        guard let encodedID = coinID.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) else {
            return nil
        }
        guard var components = URLComponents(string: "\(baseURL)/coins/\(encodedID)/ohlc") else {
            return nil
        }
        components.queryItems = [
            URLQueryItem(name: "vs_currency", value: "usd"),
            URLQueryItem(name: "days", value: Self.daysParameter(days)),
        ]
        return components.url
    }

    /// CoinGecko spells the full-history window "max", not a day count.
    private static func daysParameter(_ days: Int) -> String {
        days >= maxDays ? "max" : String(days)
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

    private func parseOHLCResponse(_ data: Data) throws -> [KlineData] {
        guard let json = try JSONSerialization.jsonObject(with: data) as? [[Any]] else {
            throw CoinGeckoError.parseError("Expected array of arrays")
        }
        let klines = json.compactMap { KlineData(rawCoinGecko: $0) }
        return klines.sorted { $0.openTime < $1.openTime }
    }

    // MARK: - OHLC Window Selection

    /// One selectable CoinGecko OHLC window.
    ///
    /// The public OHLC endpoint has no `limit` parameter and no granularity
    /// parameter — it derives candle size from the `days` window alone:
    ///   1–2 days   → 30-minute candles
    ///   3–30 days  → 4-hour candles
    ///   31+ days   → 4-day candles
    private struct OHLCWindow {
        let days: Int
        let granularity: TimeInterval

        /// Candles CoinGecko returns for this window.
        var supply: Int { Int(Double(days) * 86_400 / granularity) }
    }

    /// Candle supply is *non-monotonic* in `days` — a 30-day window yields ~180
    /// candles but a 90-day window only ~22. Scaling `days` with the requested
    /// count (the way a Binance `limit` scales) therefore returns fewer candles
    /// than asked for, and leaves the chart frozen across zoom steps that don't
    /// happen to cross a window boundary. We select by supply instead.
    private static let ohlcWindows: [OHLCWindow] = [
        OHLCWindow(days: 1,   granularity: 1_800),    //  48 candles
        OHLCWindow(days: 7,   granularity: 14_400),   //  42
        OHLCWindow(days: 14,  granularity: 14_400),   //  84
        OHLCWindow(days: 30,  granularity: 14_400),   // 180
        OHLCWindow(days: 90,  granularity: 345_600),  //  22
        OHLCWindow(days: 180, granularity: 345_600),  //  45
        OHLCWindow(days: 365, granularity: 345_600),  //  91
        // Full history — the only window that can exceed 180 candles. Its real
        // supply depends on how long the coin has traded; the nominal figure
        // just marks it as the deepest option.
        OHLCWindow(days: maxDays, granularity: 345_600),
    ]

    /// Sentinel `days` value standing in for CoinGecko's "max" (full history)
    /// window.
    private static let maxDays = 9999

    /// Binance-style interval → seconds, for matching against CG granularity.
    private static let intervalSeconds: [String: TimeInterval] = [
        "1m": 60, "5m": 300, "15m": 900, "30m": 1_800,
        "1h": 3_600, "4h": 14_400,
        "1d": 86_400, "1w": 604_800, "1M": 2_592_000,
    ]

    /// Pick the `days` window that can actually supply `limit` candles, at the
    /// granularity closest to `interval`.
    ///
    /// Matching `limit` is what keeps a CoinGecko chart zooming in step with a
    /// Binance one. The trade-off is span: CoinGecko offers no 1h/1d/1w candles
    /// on the public tier, so the same candle count can cover a longer period.
    private func daysForInterval(_ interval: String, limit: Int) -> Int {
        let target = Self.intervalSeconds[interval] ?? 3_600

        let usable = Self.ohlcWindows.filter { $0.supply >= limit }
        guard !usable.isEmpty else { return Self.maxDays }

        return usable.min { a, b in
            // Compare granularity on a log scale so "twice as coarse" and
            // "twice as fine" count equally against a window.
            let distanceA = abs(log(a.granularity / target))
            let distanceB = abs(log(b.granularity / target))
            // Closest granularity wins; on a tie take the smaller window so we
            // don't fetch history we'd immediately discard.
            return (distanceA, a.supply) < (distanceB, b.supply)
        }!.days
    }

    // MARK: - Search

    func searchTickers(query: String) async throws -> [TickerSearchResult] {
        let q = query.trimmingCharacters(in: .whitespaces).lowercased()
        guard !q.isEmpty else { return [] }

        // Refresh coin list cache in background if stale
        if coinIDCache.updatedAt.timeIntervalSinceNow < -Timeout.coinListStaleness {
            refreshCoinListInBackground()
        }

        guard var components = URLComponents(string: "\(baseURL)/search") else {
            throw CoinGeckoError.invalidURL
        }
        components.queryItems = [URLQueryItem(name: "query", value: q)]

        guard let url = components.url else {
            throw CoinGeckoError.invalidURL
        }

#if DEBUG
        print("[CoinGecko] Search: \(q)")
#endif

        let (data, response) = try await session.data(from: url)

        guard let httpResponse = response as? HTTPURLResponse, httpResponse.statusCode == 200 else {
            throw CoinGeckoError.invalidResponse
        }

        let decoder = JSONDecoder()
        let result = try decoder.decode(CoinSearchResult.self, from: data)

        return result.coins.map { coin in
            TickerSearchResult(
                symbol: coin.symbol.uppercased(),
                fullSymbol: coin.id,
                source: .coingecko,
                price: nil
            )
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
