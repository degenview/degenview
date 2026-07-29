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
    private let minGap: TimeInterval = 3.0  // ≤20 calls/min, safe under ~30/min limit

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
    /// CoinGecko free tier is ~30 req/min. Long cache — OHLC candles close slowly, no WebSocket.
    private let cacheTTL: TimeInterval = 120

    private let rateLimiter = CGRateLimiter()
    private var coinIDCache: CoinIDCache
    private let cacheURL: URL
    private var coinListRefreshTask: Task<Void, Never>?

    init() {
        let config = URLSessionConfiguration.default
        config.timeoutIntervalForRequest = 10
        config.timeoutIntervalForResource = 30
        self.session = URLSession(configuration: config)
        self.cache = KlineCache()

        let appSupport = FileManager.default.urls(
            for: .applicationSupportDirectory, in: .userDomainMask
        ).first!
        let dir = appSupport.appendingPathComponent("CryptoCharts", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        cacheURL = dir.appendingPathComponent("coingecko_coin_ids.json")

        if let data = try? Data(contentsOf: cacheURL),
           let cached = try? JSONDecoder().decode(CoinIDCache.self, from: data) {
            coinIDCache = cached
        } else {
            coinIDCache = CoinIDCache(idMap: [:], updatedAt: .distantPast)
        }
    }

    // MARK: - Kline Fetching

    func fetchKlines(symbol: String, interval: String, limit: Int) async throws -> [KlineData] {
        // symbol here is the CoinGecko coin ID (e.g. "bitcoin", "ethereum")
        let coinID = symbol.lowercased()

        // Check cache
        if let cached = await cache.get(symbol: coinID, interval: interval, count: limit, ttl: cacheTTL) {
            return cached
        }

        // Wait for rate-limit slot
        await rateLimiter.waitForSlot()

        // Map our interval to CoinGecko days param
        let days = daysForInterval(interval, limit: limit)

        // CoinGecko requires percent-encoded coin ID in path
        guard let encodedID = coinID.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) else {
            throw CoinGeckoError.invalidURL
        }

        guard var components = URLComponents(string: "\(baseURL)/coins/\(encodedID)/ohlc") else {
            throw CoinGeckoError.invalidURL
        }

        components.queryItems = [
            URLQueryItem(name: "vs_currency", value: "usd"),
            URLQueryItem(name: "days", value: String(days)),
        ]

        guard let url = components.url else {
            throw CoinGeckoError.invalidURL
        }

        print("[CoinGecko] Fetching OHLC for \(coinID) days=\(days) limit=\(limit)")

        let (data, response) = try await session.data(from: url)

        guard let httpResponse = response as? HTTPURLResponse else {
            throw CoinGeckoError.invalidResponse
        }

        switch httpResponse.statusCode {
        case 200:
            break
        case 429:
            // Extract Retry-After header, back off, then re-throw
            let retryAfter = httpResponse.value(forHTTPHeaderField: "Retry-After")
            let waitSeconds = retryAfter.flatMap(Int.init) ?? 30
            print("[CoinGecko] 429 rate limited. Backing off \(waitSeconds)s…")
            await rateLimiter.backoff(seconds: waitSeconds)
            throw CoinGeckoError.rateLimited
        case 404:
            throw CoinGeckoError.coinNotFound(coinID)
        case 400:
            let body = String(data: data, encoding: .utf8) ?? ""
            print("[CoinGecko] 400 Bad Request for URL: \(url.absoluteString)")
            print("[CoinGecko] Response: \(body)")
            throw CoinGeckoError.httpError(400)
        default:
            throw CoinGeckoError.httpError(httpResponse.statusCode)
        }

        // CoinGecko OHLC format: [[timestamp_ms, open, high, low, close], ...]
        guard let json = try JSONSerialization.jsonObject(with: data) as? [[Any]] else {
            throw CoinGeckoError.parseError("Expected array of arrays")
        }

        let klines = json.compactMap { KlineData(rawCoinGecko: $0) }
        let sorted = klines.sorted { $0.openTime < $1.openTime }
        let result = Array(sorted.suffix(limit))

        print("[CoinGecko] \(coinID) candles=\(result.count) (fetched \(sorted.count), limit \(limit))")

        await cache.set(symbol: coinID, interval: interval, data: result)
        return result
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
        // Snap to nearest valid CoinGecko bucket (round up so we always have enough data)
        return Self.validDays.first(where: { $0 >= raw }) ?? 365
    }

    // MARK: - Search

    func searchTickers(query: String) async throws -> [TickerSearchResult] {
        let q = query.trimmingCharacters(in: .whitespaces).lowercased()
        guard !q.isEmpty else { return [] }

        // Refresh coin list cache in background if stale
        if coinIDCache.updatedAt.timeIntervalSinceNow < -86400 * 7 {
            refreshCoinListInBackground()
        }

        guard var components = URLComponents(string: "\(baseURL)/search") else {
            throw CoinGeckoError.invalidURL
        }
        components.queryItems = [URLQueryItem(name: "query", value: q)]

        guard let url = components.url else {
            throw CoinGeckoError.invalidURL
        }

        print("[CoinGecko] Search: \(q)")

        let (data, response) = try await session.data(from: url)

        guard let httpResponse = response as? HTTPURLResponse, httpResponse.statusCode == 200 else {
            throw CoinGeckoError.invalidResponse
        }

        let decoder = JSONDecoder()
        let result = try decoder.decode(CoinSearchResult.self, from: data)

        return result.coins.map { coin in
            TickerSearchResult(
                symbol: coin.symbol.uppercased(),
                fullSymbol: coin.id,           // coin ID used for OHLC fetch
                source: .coingecko,
                price: nil                    // search doesn't include price
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
                print("[CoinGecko] Coin list refreshed: \(map.count) symbols")
            } catch {
                print("[CoinGecko] Coin list refresh failed: \(error.localizedDescription)")
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
