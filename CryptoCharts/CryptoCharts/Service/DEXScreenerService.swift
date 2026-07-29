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

    struct DEXToken: Codable {
        let symbol: String?
        let name: String?
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

    private let baseURL = "https://api.dexscreener.com/latest/dex"
    private let session: URLSession

    init() {
        let config = URLSessionConfiguration.default
        config.timeoutIntervalForRequest = 10
        config.timeoutIntervalForResource = 30
        self.session = URLSession(configuration: config)
    }

    // MARK: - Kline Fetching (unsupported on free tier)

    func fetchKlines(symbol: String, interval: String, limit: Int) async throws -> [KlineData] {
        throw DEXScreenerError.noChartData
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

        print("[DEXScreener] Search: \(q)")

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
        .prefix(20)

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
}

// MARK: - Errors

enum DEXScreenerError: LocalizedError {
    case invalidURL
    case invalidResponse
    case noChartData

    var errorDescription: String? {
        switch self {
        case .invalidURL:      return "Invalid URL"
        case .invalidResponse: return "Unexpected response from DEXScreener"
        case .noChartData:     return "Candlestick charts not available for DEX pairs on the free tier"
        }
    }
}
