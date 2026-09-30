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
    let priceChange: DEXPriceChange?
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

    struct DEXPriceChange: Codable {
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
        }.ranked(for: q).prefix(Timeout.dexSearchLimit).map { $0 }
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

// MARK: - Batch quotes

extension DEXScreenerService: BatchQuoteDataSource {
    /// DEXScreener takes at most this many pair addresses per call.
    private static let pairsPerRequest = 30

    /// One request per chain for up to 30 pools each, priced straight from DEXScreener. Candles
    /// come from GeckoTerminal, whose limiter allows a call every 2.2 seconds. Pairs without a
    /// known chain are left out and fall back to candles.
    func fetchQuotes(_ requests: [QuoteRequest]) async throws -> [String: SourceQuote] {
        let byChain = Dictionary(grouping: requests.filter { $0.metadata["chain"].map { $0 != "unknown" } ?? false }) {
            $0.metadata["chain"] ?? ""
        }
        return await withTaskGroup(of: [String: SourceQuote].self) { group in
            for (chain, pairs) in byChain {
                for start in stride(from: 0, to: pairs.count, by: Self.pairsPerRequest) {
                    let page = Array(pairs[start..<min(start + Self.pairsPerRequest, pairs.count)])
                    group.addTask { [self] in (try? await fetchPairs(chain: chain, requests: page)) ?? [:] }
                }
            }
            var quotes: [String: SourceQuote] = [:]
            for await partial in group { quotes.merge(partial) { first, _ in first } }
            return quotes
        }
    }

    private func fetchPairs(chain: String, requests: [QuoteRequest]) async throws -> [String: SourceQuote] {
        let addresses = requests.map(\.symbol).joined(separator: ",")
        guard let url = URL(string: "\(baseURL)/pairs/\(chain)/\(addresses)") else { return [:] }
        let (data, response) = try await session.data(from: url)
        guard (response as? HTTPURLResponse)?.statusCode == 200 else { return [:] }
        let pairs = try JSONDecoder().decode(DEXPairsResponse.self, from: data).pairs ?? []
        let byAddress = Dictionary(
            pairs.compactMap { pair in pair.pairAddress.map { ($0.lowercased(), pair) } },
            uniquingKeysWith: { first, _ in first })
        var quotes: [String: SourceQuote] = [:]
        for request in requests {
            guard let pair = byAddress[request.symbol.lowercased()],
                let price = Double(pair.priceUsd ?? ""), price > 0
            else { continue }
            quotes[request.symbol] = SourceQuote(price: price, changePercent24h: pair.priceChange?.h24)
        }
        return quotes
    }
}
