import Foundation

// MARK: - Errors

enum BinanceAPIError: LocalizedError {
    case invalidURL
    case invalidResponse
    case httpError(Int)
    case symbolNotFound(String)
    case rateLimited
    case networkError(Error)
    case parseError(String)

    var errorDescription: String? {
        switch self {
        case .invalidURL:
            return "Invalid URL"
        case .invalidResponse:
            return "Unexpected response from server"
        case .httpError(let code):
            return "Server error (HTTP \(code))"
        case .symbolNotFound(let symbol):
            return "Ticker \"\(symbol)\" not found"
        case .rateLimited:
            return "Too many requests. Wait a moment and try again."
        case .networkError(let error):
            return "Network error: \(error.localizedDescription)"
        case .parseError(let detail):
            return "Data error: \(detail)"
        }
    }
}

// MARK: - Service

final class BinanceAPIService: TickerDataSource {
    let type: DataSourceType = .binance
    private let session = AppSupport.defaultSession
    private let baseURL = "https://api.binance.com"
    private let cache = KlineCache()

    init() {}

    /// Fetch candlestick data from Binance. Uses in-memory cache.
    func fetchKlines(symbol: String, interval: String, limit: Int) async throws -> [KlineData] {
        // Check cache first
        if let cached = await cache.get(symbol: symbol, interval: interval, count: limit, ttl: Timeout.binanceCacheTTL) {
            return cached
        }

        // Cache miss — fetch from API
        guard var components = URLComponents(string: "\(baseURL)/api/v3/klines") else {
            throw BinanceAPIError.invalidURL
        }

        components.queryItems = [
            URLQueryItem(name: "symbol", value: symbol.uppercased()),
            URLQueryItem(name: "interval", value: interval),
            URLQueryItem(name: "limit", value: String(limit)),
        ]

        guard let url = components.url else {
            throw BinanceAPIError.invalidURL
        }

        let (data, response) = try await session.data(from: url)

        guard let httpResponse = response as? HTTPURLResponse else {
            throw BinanceAPIError.invalidResponse
        }

        switch httpResponse.statusCode {
        case 200:
            break
        case 429:
            throw BinanceAPIError.rateLimited
        case 400...499:
            throw BinanceAPIError.symbolNotFound(symbol)
        default:
            throw BinanceAPIError.httpError(httpResponse.statusCode)
        }

        guard let json = try JSONSerialization.jsonObject(with: data) as? [[Any]] else {
            throw BinanceAPIError.parseError("Expected array of arrays")
        }

        let klines = json.compactMap { KlineData(raw: $0) }

        if klines.isEmpty, !json.isEmpty {
            throw BinanceAPIError.parseError("Failed to parse kline data")
        }

        let sorted = klines.sorted { $0.openTime < $1.openTime }

#if DEBUG
        let formatter = DateFormatter()
        formatter.dateFormat = "HH:mm"
        formatter.timeZone = TimeZone(identifier: "UTC")!
        print("[API] \(symbol.uppercased()) interval=\(interval) limit=\(limit) candles=\(sorted.count)")
        for k in sorted.suffix(3) {
            let bullish = k.closePrice > k.openPrice ? "🟢" : "🔴"
            print("[API]   \(formatter.string(from: k.openTime)) O=\(k.openPrice) H=\(k.highPrice) L=\(k.lowPrice) C=\(k.closePrice) \(bullish)")
        }
#endif

        // Cache the result
        await cache.set(symbol: symbol, interval: interval, data: sorted)

        return sorted
    }

    /// Search for tickers matching a query. Returns up to 20 pairs sorted by volume.
    func searchTickers(query: String) async throws -> [TickerSearchResult] {
        let q = query.trimmingCharacters(in: .whitespaces).uppercased()
        guard !q.isEmpty else { return [] }

        guard let components = URLComponents(string: "\(baseURL)/api/v3/exchangeInfo") else {
            throw BinanceAPIError.invalidURL
        }
        // No symbol filter — get full exchange info. Small payload, cacheable.
        guard let url = components.url else {
            throw BinanceAPIError.invalidURL
        }

#if DEBUG
        print("[Binance] Searching exchangeInfo for: \(q)")
#endif

        let (data, response) = try await session.data(from: url)

        guard let httpResponse = response as? HTTPURLResponse, httpResponse.statusCode == 200 else {
            throw BinanceAPIError.invalidResponse
        }

        let decoder = JSONDecoder()
        let info = try decoder.decode(ExchangeInfoResponse.self, from: data)

        let matching = info.symbols
            .filter { $0.status == "TRADING" }
            .filter { $0.symbol.contains(q) }
            .prefix(20)

        return matching.map { info in
            TickerSearchResult(
                symbol: "\(info.baseAsset)/\(info.quoteAsset)",
                fullSymbol: info.symbol,
                source: .binance,
                price: nil
            )
        }
    }

}
