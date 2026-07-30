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
        return await cache.getStale(symbol: symbol, interval: interval, count: count)
    }

    func fetchKlines(symbol: String, interval: String, limit: Int) async throws -> [KlineData] {
        let coinID = symbol.lowercased()

        if let cached = await cache.get(symbol: coinID, interval: interval, count: limit, ttl: Timeout.coingeckoCacheTTL) {
            return cached
        }

        await rateLimiter.waitForSlot()

        let days = daysForInterval(interval, limit: limit)

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

        await cache.set(symbol: coinID, interval: interval, data: sorted)
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
                        count: limit, ttl: Timeout.coingeckoCacheTTL
                    ) {
                        continuation.yield(cached)
                        continuation.finish()
                        return
                    }

                    // Yield stale cache first so chart renders instantly.
                    if let stale = await cache.getStale(
                        symbol: coinID, interval: interval, count: limit
                    ) {
                        continuation.yield(stale)
                    }

                    // Stage 1: fetch minimal day range for instant display.
                    // 1 day of data is a tiny response — arrives in ~500 ms.
                    let stage1Days = 1
                    let needStage1 = fullDays > stage1Days

                    if needStage1, let url1 = buildOHLCURL(coinID: coinID, days: stage1Days) {
                        do {
                            await rateLimiter.waitForSlot()

                        #if DEBUG
                            print("[CoinGecko] Stage 1: fetching \(stage1Days)d for \(coinID)")
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
                    if needStage1 {
                        await rateLimiter.waitForSlot()
                    #if DEBUG
                        print("[CoinGecko] Stage 2: fetching \(fullDays)d for \(coinID)")
                    #endif
                    } else {
                        await rateLimiter.waitForSlot()
                    }

                    guard let url = buildOHLCURL(coinID: coinID, days: fullDays) else {
                        throw CoinGeckoError.invalidURL
                    }
                    let (data, response) = try await session.data(from: url)
                    try await checkHTTPResponse(data: data, response: response, url: url)
                    let sorted = try parseOHLCResponse(data)
                    let result = Array(sorted.suffix(limit))

                    await cache.set(symbol: coinID, interval: interval, data: sorted)
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

    private func parseOHLCResponse(_ data: Data) throws -> [KlineData] {
        guard let json = try JSONSerialization.jsonObject(with: data) as? [[Any]] else {
            throw CoinGeckoError.parseError("Expected array of arrays")
        }
        let klines = json.compactMap { KlineData(rawCoinGecko: $0) }
        return klines.sorted { $0.openTime < $1.openTime }
    }

    /// CoinGecko OHLC only accepts these `days` values. Snap to nearest valid bucket.
    private static let validDays = [1, 7, 14, 30, 90, 180, 365, 9999]  // 9999 = "max"

    /// Map Binance-style interval + limit to the nearest valid CoinGecko `days` parameter.
    private func daysForInterval(_ interval: String, limit: Int) -> Int {
        let raw: Int
        switch interval {
        case "1m", "5m", "15m", "30m": raw = max(1, (limit * 5) / (24 * 60))
        case "1h":                      raw = max(1, limit / 24)
        case "4h":                      raw = max(1, limit / 6)
        case "1d":                      raw = max(1, limit)
        case "1w":                      raw = max(1, limit * 7)
        case "1M":                      raw = max(1, limit * 30)
        default:                        raw = max(1, limit)
        }
        return Self.validDays.first(where: { $0 >= raw }) ?? 365
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
