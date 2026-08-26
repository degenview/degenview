import Foundation

// MARK: - API Models

/// `ohlcv_list` rows are `[timestamp, open, high, low, close, volumeUSD]`, decoded
/// positionally rather than by key.
private struct GTOHLCVResponse: Decodable {
    let data: Payload

    struct Payload: Decodable {
        let attributes: Attributes
    }

    struct Attributes: Decodable {
        let ohlcv_list: [[Double?]]
    }
}

/// Pool search — used only to learn which network an address lives on.
private struct GTSearchResponse: Decodable {
    let data: [Pool]

    struct Pool: Decodable {
        /// `"{network}_{address}"`, e.g. `eth_0x88e6a0c2…`.
        let id: String
        let attributes: Attributes

        struct Attributes: Decodable {
            let address: String
        }
    }
}

// MARK: - Service

/// OHLCV for on-chain DEX pools, from GeckoTerminal — CoinGecko's DEX arm.
///
/// DEXScreener indexes the same pools but publishes no candles on its free tier, so
/// pairs are discovered there and charted here. Free and keyless, ~30 requests per
/// minute, and the volume it reports is already denominated in USD, which is what
/// the volume bars want.
///
/// An actor because the network-id lookups it memoizes are shared across every DEX
/// card fetching at once.
actor GeckoTerminalService {
    static let apiBase = "https://api.geckoterminal.com/api/v2"

    private let session = AppSupport.defaultSession
    private let cache = KlineCache(persistenceKey: "geckoterminal")
    /// Its own limiter: a separate host from CoinGecko, with a separate budget.
    private let rateLimiter = CGRateLimiter(gap: Timeout.geckoTerminalRateLimitGap)

    /// Pool address (lowercased) → GeckoTerminal network id. Addresses don't migrate
    /// between chains, so this never needs invalidating.
    private var networks: [String: String] = [:]

    // MARK: - Klines

    func fetchKlines(pairAddress: String, interval: String, limit: Int) async throws -> [KlineData] {
        let window = Self.window(interval: interval, limit: limit)

        if let cached = await cache.get(
            symbol: pairAddress,
            interval: interval,
            days: window.spanDays,
            count: limit,
            ttl: Timeout.geckoTerminalCacheTTL
        ) {
            return cached
        }

        let network = try await network(for: pairAddress)
        let raw = try await fetchOHLCV(network: network, pool: pairAddress, window: window)

        // GeckoTerminal returns newest first.
        let ascending = raw.sorted { $0.openTime < $1.openTime }
        // Only when the source has no bucket this coarse — 1:1 windows skip it.
        let candles = window.needsAggregation ? ascending.aggregated(into: limit) : ascending

        await cache.set(symbol: pairAddress, interval: interval, days: window.spanDays, data: candles)
        return Array(candles.suffix(limit))
    }

    func cachedKlines(pairAddress: String, interval: String, count: Int) async -> [KlineData]? {
        let window = Self.window(interval: interval, limit: count)
        return await cache.get(
            symbol: pairAddress,
            interval: interval,
            days: window.spanDays,
            count: count,
            ttl: Timeout.geckoTerminalCacheTTL
        )
    }

    // MARK: - OHLCV request

    private func fetchOHLCV(network: String, pool: String, window: Window) async throws -> [KlineData] {
        guard
            var components = URLComponents(
                string: "\(Self.apiBase)/networks/\(network)/pools/\(pool)/ohlcv/\(window.timeframe)"
            )
        else {
            throw GeckoTerminalError.invalidURL
        }
        components.queryItems = [
            URLQueryItem(name: "aggregate", value: String(window.aggregate)),
            URLQueryItem(name: "limit", value: String(window.fetchLimit)),
            URLQueryItem(name: "currency", value: "usd"),
        ]
        guard let url = components.url else { throw GeckoTerminalError.invalidURL }

        #if DEBUG
            print(
                "[GeckoTerminal] OHLCV \(network)/\(pool) \(window.timeframe)x\(window.aggregate) limit=\(window.fetchLimit)"
            )
        #endif

        await rateLimiter.waitForSlot()
        try Task.checkCancellation()

        let (data, response) = try await session.data(from: url)
        guard let http = response as? HTTPURLResponse else {
            throw GeckoTerminalError.invalidResponse
        }
        guard http.statusCode != 429 else {
            await rateLimiter.backoff(seconds: 60)
            throw GeckoTerminalError.rateLimited
        }
        guard http.statusCode == 200 else {
            throw GeckoTerminalError.invalidResponse
        }
        await rateLimiter.noteSuccess()

        let decoded = try JSONDecoder().decode(GTOHLCVResponse.self, from: data)
        let candles = decoded.data.attributes.ohlcv_list.compactMap(KlineData.init(rawGeckoTerminal:))
        guard !candles.isEmpty else { throw GeckoTerminalError.noChartData }
        return candles
    }

    // MARK: - Network resolution

    /// Which chain a pool address sits on. GeckoTerminal's per-pool endpoints are
    /// namespaced by network, and DEXScreener's chain names don't match its ids
    /// (`ethereum` vs `eth`, `avalanche` vs `avax`), so the address is looked up
    /// here instead of translated through a table that would rot.
    private func network(for pairAddress: String) async throws -> String {
        let key = pairAddress.lowercased()
        if let known = networks[key] { return known }

        guard var components = URLComponents(string: "\(Self.apiBase)/search/pools") else {
            throw GeckoTerminalError.invalidURL
        }
        components.queryItems = [URLQueryItem(name: "query", value: pairAddress)]
        guard let url = components.url else { throw GeckoTerminalError.invalidURL }

        await rateLimiter.waitForSlot()
        try Task.checkCancellation()

        let (data, response) = try await session.data(from: url)
        guard let http = response as? HTTPURLResponse, http.statusCode == 200 else {
            throw GeckoTerminalError.invalidResponse
        }
        await rateLimiter.noteSuccess()

        let decoded = try JSONDecoder().decode(GTSearchResponse.self, from: data)

        // Match on the address rather than taking the first row: a search can return
        // sibling pools of the same token pair on other chains.
        let suffix = "_\(key)"
        guard
            let pool = decoded.data.first(where: {
                $0.attributes.address.lowercased() == key || $0.id.lowercased().hasSuffix(suffix)
            })
        else {
            throw GeckoTerminalError.unknownPool
        }

        // Network ids themselves contain underscores (`polygon_pos`), so the address
        // is trimmed off the end rather than split on the first separator.
        let id = pool.id
        guard let range = id.lowercased().range(of: suffix, options: .backwards) else {
            throw GeckoTerminalError.unknownPool
        }
        let network = String(id[id.startIndex..<range.lowerBound])
        guard !network.isEmpty else { throw GeckoTerminalError.unknownPool }

        networks[key] = network
        return network
    }

    // MARK: - Window selection

    /// How one request should be shaped: which native bucket to ask for, how many of
    /// them, and whether they still need folding into the timeframe the chart wants.
    struct Window {
        let timeframe: String
        let aggregate: Int
        let fetchLimit: Int
        let spanDays: Int
        let needsAggregation: Bool
    }

    /// Buckets GeckoTerminal serves natively, in seconds.
    private static let buckets: [(timeframe: String, aggregate: Int, seconds: Int)] = [
        ("minute", 1, 60),
        ("minute", 5, 300),
        ("minute", 15, 900),
        ("hour", 1, 3_600),
        ("hour", 4, 14_400),
        ("hour", 12, 43_200),
        ("day", 1, 86_400),
    ]

    /// Pick the finest native bucket that still covers the requested span.
    ///
    /// The chart asks in Binance's vocabulary (`1h`, `1d`, `1w`, `1M`). Anything up to
    /// daily maps straight across; weekly and monthly have no GeckoTerminal
    /// equivalent, so daily candles are fetched and folded down afterwards — which is
    /// also why the fetch can ask for far more candles than the chart draws.
    static func window(interval: String, limit: Int) -> Window {
        let target = intervalSeconds(interval)
        let span = target * max(1, limit)

        // Largest bucket no coarser than what was asked for; the finest if none fits.
        let bucket = buckets.last(where: { $0.seconds <= target }) ?? buckets[0]

        let wanted = Int((Double(span) / Double(bucket.seconds)).rounded(.up))
        let fetchLimit = min(max(wanted, 1), Timeout.geckoTerminalMaxCandles)

        return Window(
            timeframe: bucket.timeframe,
            aggregate: bucket.aggregate,
            fetchLimit: fetchLimit,
            spanDays: max(1, span / 86_400),
            needsAggregation: bucket.seconds < target
        )
    }

    private static func intervalSeconds(_ interval: String) -> Int {
        switch interval {
        case "1m": return 60
        case "5m": return 300
        case "15m": return 900
        case "1h": return 3_600
        case "4h": return 14_400
        case "1d": return 86_400
        case "1w": return 604_800
        case "1M": return 2_592_000
        default: return 3_600
        }
    }
}

// MARK: - Errors

enum GeckoTerminalError: LocalizedError {
    case invalidURL
    case invalidResponse
    case rateLimited
    case unknownPool
    case noChartData

    var errorDescription: String? {
        switch self {
        case .invalidURL: return "Invalid GeckoTerminal URL"
        case .invalidResponse: return "GeckoTerminal request failed"
        case .rateLimited: return "GeckoTerminal rate limit reached"
        case .unknownPool: return "Pool not indexed by GeckoTerminal"
        case .noChartData: return "No chart data for this pool"
        }
    }
}

// MARK: - Parsing

extension KlineData {
    /// Parse one GeckoTerminal OHLCV row.
    /// Array layout: `[timestamp_seconds, open, high, low, close, volumeUSD]`.
    ///
    /// The timestamp is in **seconds**, unlike Binance's milliseconds, and the volume
    /// is already USD — so it lands in `quoteVolume`, leaving base-asset `volume` at
    /// zero, which is the reverse of the Binance layout.
    init?(rawGeckoTerminal row: [Double?]) {
        guard row.count >= 6,
            let ts = row[0],
            let open = row[1],
            let high = row[2],
            let low = row[3],
            let close = row[4]
        else { return nil }

        self.init(
            openTime: Date(timeIntervalSince1970: ts),
            openPrice: open,
            highPrice: high,
            lowPrice: low,
            closePrice: close,
            volume: 0,
            quoteVolume: row[5] ?? 0
        )
    }
}
