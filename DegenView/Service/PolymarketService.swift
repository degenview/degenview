import Foundation

/// Polymarket prediction markets — searched through the Gamma API, charted from the
/// CLOB price history endpoint. No auth, no key.
///
/// A "ticker" here is a CLOB token id for a market's YES outcome, and its price is a
/// probability in 0…1 rather than a currency amount.
final class PolymarketService: TickerDataSource {
    let type: DataSourceType = .polymarket

    /// Market metadata and search.
    static let gammaBase = "https://gamma-api.polymarket.com"
    /// Order book — the only source of historical prices.
    static let clobBase = "https://clob.polymarket.com"

    private let session: URLSession
    private let cache = KlineCache(persistenceKey: "polymarket")

    init(session: URLSession = AppSupport.defaultSession) {
        self.session = session
    }

    // MARK: - Search

    func searchTickers(query: String) async throws -> [TickerSearchResult] {
        let q = query.trimmingCharacters(in: .whitespaces)
        guard !q.isEmpty else { return [] }

        guard var components = URLComponents(string: "\(Self.gammaBase)/public-search") else {
            throw PolymarketError.invalidURL
        }
        components.queryItems = [
            URLQueryItem(name: "q", value: q),
            URLQueryItem(name: "limit_per_type", value: String(Polymarket.searchLimitPerType)),
            URLQueryItem(name: "events_status", value: "active"),
        ]

        guard let url = components.url else { throw PolymarketError.invalidURL }

        #if DEBUG
            print("[Polymarket] Search: \(q)")
        #endif

        let (data, response) = try await session.data(from: url)
        try Self.validate(response)

        let decoded = try JSONDecoder().decode(PolymarketSearchResponse.self, from: data)
        guard let events = decoded.events else { return [] }

        // Flatten events into their markets — one row per chartable bet, keeping the
        // API's relevance order so sections stay grouped by event.
        return events.flatMap { event -> [TickerSearchResult] in
            let eventTitle = event.title ?? ""

            // Collect all tradable choices upfront so multi-choice events can carry the
            // full list on every result row (selecting any row picks all choices).
            let tradable = (event.markets ?? []).compactMap {
                market -> (tokenID: String, label: String, market: PolymarketMarket)? in
                guard market.isTradable,
                    let tokenID = market.yesTokenID,
                    let label = market.shortTitle, !label.isEmpty
                else { return nil }
                return (tokenID, label, market)
            }

            let allSeries: [PmSeriesConfig]? =
                tradable.count > 1
                ? tradable.map { PmSeriesConfig(tokenID: $0.tokenID, label: $0.label, enabled: true) }
                : nil

            return tradable.compactMap { (tokenID, label, market) in
                let artwork = market.artworkURL ?? event.artworkURL
                var result = TickerSearchResult(
                    symbol: label,
                    fullSymbol: tokenID,
                    source: .polymarket,
                    price: market.displayedYesPrice,
                    metadata: [
                        "eventTitle": eventTitle,
                        "question": market.question ?? label,
                        "imageURL": artwork?.absoluteString ?? "",
                    ]
                )
                result.pmSeries = allSeries
                return result
            }
        }
    }

    // MARK: - Price history

    /// Fetch the YES price series for a market.
    ///
    /// Takes the full `TimeRange` rather than the protocol's interval token because
    /// that token is lossy — it maps 1D and 3M both onto `"1d"`. `ChartViewModel`
    /// calls this directly; `fetchKlines` below is the protocol fallback.
    func fetchPrices(tokenID: String, range: TimeRange, count: Int) async throws -> [KlineData] {
        let window = range.polymarketWindow

        if let cached = await cache.get(
            symbol: tokenID,
            interval: window.interval,
            days: window.fidelity,
            count: count,
            ttl: Polymarket.cacheTTL
        ) {
            return await applyingCurrentYesAsk(to: cached, tokenID: tokenID)
        }

        guard var components = URLComponents(string: "\(Self.clobBase)/prices-history") else {
            throw PolymarketError.invalidURL
        }
        components.queryItems = [
            URLQueryItem(name: "market", value: tokenID),
            URLQueryItem(name: "interval", value: window.interval),
            URLQueryItem(name: "fidelity", value: String(window.fidelity)),
        ]

        guard let url = components.url else { throw PolymarketError.invalidURL }

        let (data, response) = try await session.data(from: url)
        try Self.validate(response)

        let decoded = try JSONDecoder().decode(PolymarketPriceHistory.self, from: data)
        let points = (decoded.history ?? [])
            .sorted { $0.t < $1.t }
            .map(\.kline)

        // An empty history is the endpoint's answer for a market with no trades in
        // this window, not a transport failure.
        guard !points.isEmpty else { throw PolymarketError.noHistory }

        let thinned = points.downsampled(to: count)
        let current = await applyingCurrentYesAsk(to: thinned, tokenID: tokenID)
        await cache.set(symbol: tokenID, interval: window.interval, days: window.fidelity, data: current)
        return current
    }

    // MARK: - TickerDataSource

    func fetchKlines(symbol: String, interval: String, limit: Int) async throws -> [KlineData] {
        let range = TimeRange.allCases.first { $0.binanceInterval == interval } ?? .oneDay
        return try await fetchPrices(tokenID: symbol, range: range, count: limit)
    }

    func getCachedKlines(symbol: String, interval: String, count: Int) async -> [KlineData]? {
        await cache.getAnyStale(symbol: symbol, count: count)
    }

    /// Overlay the executable YES ask on the final history sample so the chart's
    /// endpoint, current-price line, and headline match Polymarket's market page.
    private func applyingCurrentYesAsk(to data: [KlineData], tokenID: String) async -> [KlineData] {
        guard let price = try? await fetchCurrentYesAsk(tokenID: tokenID) else { return data }
        return Self.replacingLastPrice(in: data, with: price)
    }

    private func fetchCurrentYesAsk(tokenID: String) async throws -> Double {
        guard var components = URLComponents(string: "\(Self.clobBase)/price") else {
            throw PolymarketError.invalidURL
        }
        components.queryItems = [
            URLQueryItem(name: "token_id", value: tokenID),
            // CLOB names the resting ask side SELL; this returns the price a YES
            // buyer can currently execute, matching Gamma's `bestAsk`.
            URLQueryItem(name: "side", value: "SELL"),
        ]
        guard let url = components.url else { throw PolymarketError.invalidURL }

        var request = URLRequest(url: url)
        request.cachePolicy = .reloadIgnoringLocalCacheData
        let (data, response) = try await session.data(for: request)
        try Self.validate(response)
        guard let price = try JSONDecoder().decode(PolymarketCurrentPrice.self, from: data).value else {
            throw PolymarketError.invalidResponse
        }
        return price
    }

    static func replacingLastPrice(in data: [KlineData], with price: Double) -> [KlineData] {
        guard !data.isEmpty, price.isFinite, (0...1).contains(price) else { return data }
        var result = data
        result[result.count - 1].closePrice = price
        result[result.count - 1].highPrice = price
        result[result.count - 1].lowPrice = price
        return result
    }

    // MARK: - Market metadata

    /// Title and artwork for a market, looked up by its YES token id.
    ///
    /// Static so `IconResolver` (an actor) can call it without sending a non-Sendable
    /// service instance across isolation — same reason as `DEXScreenerService.pairIcon`.
    static func marketInfo(tokenID: String) async -> (title: String, imageURL: URL?)? {
        let id = tokenID.trimmingCharacters(in: .whitespaces)
        guard !id.isEmpty,
            var components = URLComponents(string: "\(gammaBase)/markets")
        else { return nil }

        components.queryItems = [URLQueryItem(name: "clob_token_ids", value: id)]
        guard let url = components.url else { return nil }

        do {
            let (data, response) = try await AppSupport.defaultSession.data(from: url)
            guard let http = response as? HTTPURLResponse, http.statusCode == 200 else { return nil }

            let markets = try JSONDecoder().decode([PolymarketMarket].self, from: data)
            guard let market = markets.first, let title = market.shortTitle else { return nil }

            return (title: title, imageURL: market.artworkURL)
        } catch {
            #if DEBUG
                print("[Polymarket] Market lookup failed for \(id): \(error.localizedDescription)")
            #endif
            return nil
        }
    }

    // MARK: - Helpers

    private static func validate(_ response: URLResponse) throws {
        guard let http = response as? HTTPURLResponse else {
            throw PolymarketError.invalidResponse
        }
        switch http.statusCode {
        case 200: return
        case 429: throw PolymarketError.rateLimited
        default: throw PolymarketError.httpError(http.statusCode)
        }
    }
}

// MARK: - Errors

enum PolymarketError: LocalizedError {
    case invalidURL
    case invalidResponse
    case rateLimited
    case noHistory
    case httpError(Int)

    var errorDescription: String? {
        switch self {
        case .invalidURL: return "Invalid Polymarket URL"
        case .invalidResponse: return "Unexpected response from Polymarket"
        case .rateLimited: return "Polymarket rate limit reached — retrying shortly"
        case .noHistory: return "No price history for this market in this range"
        case .httpError(let code): return "Polymarket error (HTTP \(code))"
        }
    }
}
