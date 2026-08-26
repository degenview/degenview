import Foundation

// MARK: - API Models

private struct DEXPairsResponse: Codable {
    let pairs: [DEXPair]?
}

private struct DEXPair: Codable {
    let chainId: String?
    let dexId: String?
    let pairAddress: String?
    let baseToken: DEXToken?
    let quoteToken: DEXToken?
    let priceUsd: String?
    let volume: DEXVolume?
    let liquidity: DEXLiquidity?
    let pairCreatedAt: Int64?
    let info: DEXInfo?

    struct DEXToken: Codable {
        let symbol: String?
        let name: String?
    }

    /// Present only for pairs whose token has been listed with artwork — most
    /// long-tail pairs send `"info": null`.
    struct DEXInfo: Codable {
        let imageUrl: String?
    }

    struct DEXVolume: Codable {
        let h24: Double?
    }

    struct DEXLiquidity: Codable {
        let usd: Double?
    }
}

// MARK: - Service

final class DEXScreenerService: TickerDataSource {
    let type: DataSourceType = .dexscreener

    /// Shared with the static icon lookup below, which runs without an instance.
    static let apiBase = "https://api.dexscreener.com/latest/dex"

    private let baseURL = DEXScreenerService.apiBase
    private let session = AppSupport.defaultSession

    /// DEXScreener publishes no candles on its free tier, so charts for the pools it
    /// finds come from GeckoTerminal, which indexes the same pools and does.
    private let charts = GeckoTerminalService()

    init() {}

    // MARK: - Kline Fetching

    /// `symbol` is the pair contract address — what `searchTickers` puts in
    /// `fullSymbol` and what gets persisted as the ticker.
    func fetchKlines(symbol: String, interval: String, limit: Int) async throws -> [KlineData] {
        try await charts.fetchKlines(pairAddress: symbol, interval: interval, limit: limit)
    }

    func getCachedKlines(symbol: String, interval: String, count: Int) async -> [KlineData]? {
        await charts.cachedKlines(pairAddress: symbol, interval: interval, count: count)
    }

    // MARK: - Search

    func searchTickers(query: String) async throws -> [TickerSearchResult] {
        let q = query.trimmingCharacters(in: .whitespaces).lowercased()
        guard !q.isEmpty else { return [] }

        guard var components = URLComponents(string: "\(baseURL)/search") else {
            throw DEXScreenerError.invalidURL
        }
        components.queryItems = [URLQueryItem(name: "q", value: q)]

        guard let url = components.url else {
            throw DEXScreenerError.invalidURL
        }

        #if DEBUG
            print("[DEXScreener] Search: \(q)")
        #endif

        let (data, response) = try await session.data(from: url)

        guard let httpResponse = response as? HTTPURLResponse, httpResponse.statusCode == 200 else {
            throw DEXScreenerError.invalidResponse
        }

        let decoder = JSONDecoder()
        let result = try decoder.decode(DEXPairsResponse.self, from: data)

        guard let pairs = result.pairs, !pairs.isEmpty else { return [] }

        // Deduplicate by base+quote symbol combo, keep highest liquidity
        let filtered = Dictionary(grouping: pairs) { pair in
            let base = pair.baseToken?.symbol ?? "?"
            let quote = pair.quoteToken?.symbol ?? "?"
            return "\(base.uppercased())/\(quote.uppercased())"
        }
        .compactMapValues { pairs in
            pairs.max(by: { ($0.liquidity?.usd ?? 0) < ($1.liquidity?.usd ?? 0) })
        }
        .values
        .sorted { ($0.liquidity?.usd ?? 0) > ($1.liquidity?.usd ?? 0) }
        .prefix(Timeout.dexSearchLimit)

        return filtered.compactMap { pair in
            guard let baseSymbol = pair.baseToken?.symbol,
                let quoteSymbol = pair.quoteToken?.symbol
            else { return nil }

            let price = Double(pair.priceUsd ?? "")

            return TickerSearchResult(
                symbol: "\(baseSymbol.uppercased())/\(quoteSymbol.uppercased())",
                fullSymbol: pair.pairAddress ?? "\(baseSymbol)/\(quoteSymbol)",
                source: .dexscreener,
                price: price,
                metadata: [
                    "chain": pair.chainId ?? "unknown",
                    "dex": pair.dexId ?? "unknown",
                    "pairAddress": pair.pairAddress ?? "",
                ]
            )
        }
    }

    // MARK: - Icon Lookup

    /// What a pair-address lookup can contribute to icon resolution.
    struct PairIcon: Sendable {
        /// Token artwork, when DEXScreener has any for this pair.
        let imageURL: URL?
        /// The pair's base token symbol — lets the icon chain keep going with a real
        /// ticker once the address itself has been resolved.
        let baseSymbol: String?
    }

    /// Look up a pair by its address. Searching by address returns that one pair, so
    /// the chain doesn't need the chain id — the address alone identifies it.
    ///
    /// Static so `IconResolver` can call it without sending a (non-Sendable) service
    /// instance across actor isolation.
    static func pairIcon(forPair address: String) async -> PairIcon? {
        let q = address.trimmingCharacters(in: .whitespaces)
        guard !q.isEmpty,
            var components = URLComponents(string: "\(apiBase)/search")
        else { return nil }

        components.queryItems = [URLQueryItem(name: "q", value: q)]
        guard let url = components.url else { return nil }

        do {
            let (data, response) = try await AppSupport.defaultSession.data(from: url)
            guard let httpResponse = response as? HTTPURLResponse, httpResponse.statusCode == 200 else {
                return nil
            }

            let result = try JSONDecoder().decode(DEXPairsResponse.self, from: data)
            guard let pairs = result.pairs, !pairs.isEmpty else { return nil }

            let image = pairs.compactMap { $0.info?.imageUrl }.first
            return PairIcon(
                imageURL: image.flatMap { URL(string: $0) },
                baseSymbol: pairs.first?.baseToken?.symbol
            )
        } catch {
            #if DEBUG
                print("[DEXScreener] Icon lookup failed for \(q): \(error.localizedDescription)")
            #endif
            return nil
        }
    }
}

// MARK: - Errors

enum DEXScreenerError: LocalizedError {
    case invalidURL
    case invalidResponse

    var errorDescription: String? {
        switch self {
        case .invalidURL: return "Invalid URL"
        case .invalidResponse: return "Unexpected response from DEXScreener"
        }
    }
}
