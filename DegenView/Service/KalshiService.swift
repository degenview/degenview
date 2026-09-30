import Foundation

/// Kalshi prediction markets — searched through a local series index plus the events
/// endpoint, charted from the candlestick endpoint. All public REST, no key.
///
/// A "ticker" here is `"SERIES/MARKET"` (see `KalshiMarketID`) for a market's YES side,
/// and its price is a probability in 0…1 — the dollar price of a $1 contract.
///
/// Kalshi also offers a WebSocket, but it needs a signed API key even for public
/// channels. Like Polymarket, charts here refresh over REST.
final class KalshiService: PredictionMarketDataSource {
    let type: DataSourceType = .kalshi

    private let session: URLSession
    private let cache: KlineCache
    private let index: KalshiSeriesIndex
    private let now: @Sendable () -> Date

    init(
        session: URLSession = AppSupport.defaultSession,
        cache: KlineCache = KlineCache(persistenceKey: "kalshi"),
        index: KalshiSeriesIndex? = nil,
        now: @escaping @Sendable () -> Date = { Date() }
    ) {
        self.session = session
        self.cache = cache
        self.index = index ?? KalshiSeriesIndex(session: session)
        self.now = now
    }

    // MARK: - Search

    func searchTickers(query: String) async throws -> [TickerSearchResult] {
        let q = query.trimmingCharacters(in: .whitespaces)
        guard !q.isEmpty else { return [] }

        #if DEBUG
            print("[Kalshi] Search: \(q)")
        #endif

        let matches = try await index.search(query: q, limit: Kalshi.maxSearchSeries)
        guard !matches.isEmpty else { return [] }

        // One events request per matched series, in parallel. Results are stitched back
        // in rank order so the best series' events stay on top.
        var fetched: [Int: [KalshiEvent]] = [:]
        var lastError: Error?
        await withTaskGroup(of: (Int, Result<[KalshiEvent], Error>).self) { group in
            for (offset, series) in matches.enumerated() {
                group.addTask { [self] in
                    do {
                        return (offset, .success(try await openEvents(series: series.ticker)))
                    } catch {
                        return (offset, .failure(error))
                    }
                }
            }
            for await (offset, result) in group {
                switch result {
                case .success(let events): fetched[offset] = events
                case .failure(let error): lastError = error
                }
            }
        }

        // A few failed series still leave a usable list; all of them failing is an error.
        if fetched.isEmpty, let lastError { throw lastError }

        return matches.indices.flatMap { offset -> [TickerSearchResult] in
            (fetched[offset] ?? []).flatMap { event in
                Self.results(for: event, series: matches[offset])
            }
        }
    }

    /// One row per chartable market. A multi-market event carries every choice on every
    /// row, so selecting any row charts them all — same contract as Polymarket.
    static func results(for event: KalshiEvent, series: KalshiSeries) -> [TickerSearchResult] {
        let seriesTicker = event.seriesTicker ?? series.ticker
        let eventTitle = event.title ?? series.title
        let tradable = (event.markets ?? []).filter(\.isTradable)
        let isSingle = tradable.count == 1

        let choices: [(market: KalshiMarket, id: String, label: String)] = tradable.compactMap { market in
            // A lone market's short label ("Yes", a bare name) means little on its own,
            // so it takes the full question instead.
            let label = isSingle ? (market.title ?? market.shortTitle) : market.shortTitle
            guard let label, !label.isEmpty else { return nil }
            return (market, KalshiMarketID(series: seriesTicker, market: market.ticker).raw, label)
        }

        let allSeries: [PmSeriesConfig]? =
            choices.count > 1
            ? choices.map { PmSeriesConfig(tokenID: $0.id, label: $0.label, enabled: true) }
            : nil

        return choices.map { choice in
            var result = TickerSearchResult(
                symbol: choice.label,
                fullSymbol: choice.id,
                source: .kalshi,
                price: choice.market.displayedYesPrice,
                metadata: [
                    "eventTitle": eventTitle,
                    "question": choice.market.title ?? choice.label,
                    "imageURL": "",
                ]
            )
            result.pmSeries = allSeries
            return result
        }
    }

    private func openEvents(series: String) async throws -> [KalshiEvent] {
        guard var components = URLComponents(string: "\(Kalshi.baseURL)/events") else {
            throw KalshiError.invalidURL
        }
        components.queryItems = [
            URLQueryItem(name: "series_ticker", value: series),
            URLQueryItem(name: "status", value: "open"),
            URLQueryItem(name: "with_nested_markets", value: "true"),
            URLQueryItem(name: "limit", value: String(Kalshi.maxEventsPerSeries)),
        ]
        guard let url = components.url else { throw KalshiError.invalidURL }

        let (data, response) = try await session.data(from: url)
        try Self.validate(response)
        return try KalshiJSON.decoder().decode(KalshiEventsResponse.self, from: data).events ?? []
    }

    // MARK: - Price history

    /// Fetch the YES price series for a market.
    func fetchPrices(marketID: String, range: TimeRange, count: Int) async throws -> [KlineData] {
        guard let id = KalshiMarketID(marketID) else { throw KalshiError.invalidMarketID }

        let window = range.kalshiWindow
        let interval = "\(window.periodMinutes)m"
        let days = Int(window.spanSeconds / 86_400)

        if let cached = await cache.get(
            symbol: marketID, interval: interval, days: days, count: count, ttl: Kalshi.cacheTTL)
        {
            return await applyingCurrentYesAsk(to: cached, market: id.market)
        }

        let end = now()
        guard
            var components = URLComponents(
                string: "\(Kalshi.baseURL)/series/\(Self.pathSegment(id.series))"
                    + "/markets/\(Self.pathSegment(id.market))/candlesticks")
        else { throw KalshiError.invalidURL }
        components.queryItems = [
            URLQueryItem(name: "start_ts", value: String(Int(end.timeIntervalSince1970 - window.spanSeconds))),
            URLQueryItem(name: "end_ts", value: String(Int(end.timeIntervalSince1970))),
            URLQueryItem(name: "period_interval", value: String(window.periodMinutes)),
        ]
        guard let url = components.url else { throw KalshiError.invalidURL }

        let (data, response) = try await session.data(from: url)
        try Self.validate(response)

        let candles = try KalshiJSON.decoder().decode(KalshiCandlesResponse.self, from: data).candlesticks ?? []
        let points = Self.klines(from: candles)

        // No candles is the endpoint's answer for a market with no trades in this
        // window, not a transport failure.
        guard !points.isEmpty else { throw KalshiError.noHistory }

        let thinned = points.downsampled(to: count)
        let current = await applyingCurrentYesAsk(to: thinned, market: id.market)
        await cache.set(symbol: marketID, interval: interval, days: days, data: current)
        return current
    }

    /// Candles → one price per period, stamped at the period's end (when its close is
    /// known). A period with no trades reports a null close, so it carries the last
    /// known price forward; periods before any trade are dropped.
    static func klines(from candles: [KalshiCandle]) -> [KlineData] {
        var lastKnown: Double?
        var points: [KlineData] = []
        for candle in candles.sorted(by: { $0.endPeriodTs < $1.endPeriodTs }) {
            let price =
                KalshiJSON.probability(candle.price?.closeDollars)
                ?? KalshiJSON.probability(candle.price?.previousDollars)
                ?? lastKnown
            guard let price else { continue }
            lastKnown = price
            points.append(KlineData(time: Date(timeIntervalSince1970: candle.endPeriodTs), price: price))
        }
        return points
    }

    // MARK: - TickerDataSource

    func fetchKlines(symbol: String, interval: String, limit: Int) async throws -> [KlineData] {
        let range = TimeRange.allCases.first { $0.binanceInterval == interval } ?? .oneDay
        return try await fetchPrices(marketID: symbol, range: range, count: limit)
    }

    func getCachedKlines(symbol: String, interval: String, count: Int) async -> [KlineData]? {
        await cache.getAnyStale(symbol: symbol, count: count)
    }

    // MARK: - Current price

    /// Overlay the executable YES ask on the final candle so the chart's endpoint,
    /// current-price line, and headline match Kalshi's market page.
    private func applyingCurrentYesAsk(to data: [KlineData], market: String) async -> [KlineData] {
        guard let price = try? await fetchCurrentYesAsk(market: market) else { return data }
        return PredictionMarketPrice.replacingLastPrice(in: data, with: price)
    }

    private func fetchCurrentYesAsk(market: String) async throws -> Double {
        guard let url = URL(string: "\(Kalshi.baseURL)/markets/\(Self.pathSegment(market))") else {
            throw KalshiError.invalidURL
        }

        var request = URLRequest(url: url)
        request.cachePolicy = .reloadIgnoringLocalCacheData
        let (data, response) = try await session.data(for: request)
        try Self.validate(response)

        let decoded = try KalshiJSON.decoder().decode(KalshiMarketResponse.self, from: data)
        guard let price = decoded.market?.displayedYesPrice else { throw KalshiError.invalidResponse }
        return price
    }

    // MARK: - Helpers

    private static func pathSegment(_ value: String) -> String {
        value.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) ?? value
    }

    static func validate(_ response: URLResponse) throws {
        guard let http = response as? HTTPURLResponse else {
            throw KalshiError.invalidResponse
        }
        switch http.statusCode {
        case 200: return
        case 429: throw KalshiError.rateLimited
        default: throw KalshiError.httpError(http.statusCode)
        }
    }
}

// MARK: - Errors

enum KalshiError: LocalizedError {
    case invalidURL
    case invalidMarketID
    case invalidResponse
    case rateLimited
    case noHistory
    case httpError(Int)

    var errorDescription: String? {
        switch self {
        case .invalidURL: return "Invalid Kalshi URL"
        case .invalidMarketID: return "Unrecognized Kalshi market id"
        case .invalidResponse: return "Unexpected response from Kalshi"
        case .rateLimited: return "Kalshi rate limit reached — retrying shortly"
        case .noHistory: return "No price history for this market in this range"
        case .httpError(let code): return "Kalshi error (HTTP \(code))"
        }
    }
}
