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

final class BinanceAPIService: GranularReplayDataSource {
    let type: DataSourceType = .binance
    private let session: URLSession
    private let baseURL: String
    private let cache = KlineCache()

    init(session: URLSession = AppSupport.defaultSession, baseURL: String = "https://api.binance.com") {
        self.session = session
        self.baseURL = baseURL
    }

    /// Return cached klines regardless of freshness — instant first render.
    func getCachedKlines(symbol: String, interval: String, count: Int) async -> [KlineData]? {
        return await cache.getStale(symbol: symbol, interval: interval, days: 0, count: count)
    }

    /// Fetch candlestick data from Binance. Uses in-memory cache.
    func fetchKlines(symbol: String, interval: String, limit: Int) async throws -> [KlineData] {
        // Check cache first
        if let cached = await cache.get(
            symbol: symbol, interval: interval, days: 0, count: limit, ttl: Timeout.binanceCacheTTL)
        {
            return cached
        }

        // Cache miss — fetch from API. Binance has no quarterly or yearly klines: those are
        // folded from monthly ones, fetching one bucket more than shown for the partial oldest.
        let fold = KlineData.monthlyFold(for: interval)
        let requested = fold.map { min(1_000, (limit + 1) * $0.months) } ?? limit

        guard var components = URLComponents(string: "\(baseURL)/api/v3/klines") else {
            throw BinanceAPIError.invalidURL
        }

        components.queryItems = [
            URLQueryItem(name: "symbol", value: symbol.uppercased()),
            URLQueryItem(name: "interval", value: fold == nil ? interval : "1M"),
            URLQueryItem(name: "limit", value: String(requested)),
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

        var sorted = klines.sorted { $0.openTime < $1.openTime }
        if let fold {
            sorted = Array(sorted.folded(into: fold.seconds).suffix(limit))
        }

        #if DEBUG
            let formatter = DateFormatter()
            formatter.dateFormat = "HH:mm"
            formatter.timeZone = TimeZone(identifier: "UTC")!
            print("[API] \(symbol.uppercased()) interval=\(interval) limit=\(limit) candles=\(sorted.count)")
            for k in sorted.suffix(3) {
                let bullish = k.closePrice > k.openPrice ? "🟢" : "🔴"
                print(
                    "[API]   \(formatter.string(from: k.openTime)) O=\(k.openPrice) H=\(k.highPrice) L=\(k.lowPrice) C=\(k.closePrice) \(bullish)"
                )
            }
        #endif

        // Cache the result
        await cache.set(symbol: symbol, interval: interval, days: 0, data: sorted)

        return sorted
    }

    func supportedReplayIntervals(chartInterval: String) -> [ReplayInterval] {
        let chartSeconds = Self.intervalSeconds(chartInterval)
        return ReplayInterval.allCases.filter { interval in
            guard let seconds = interval.seconds else { return interval == .automatic || interval == .chartBar }
            return seconds <= chartSeconds
        }
    }

    func fetchReplayKlines(
        symbol: String,
        interval: ReplayInterval,
        start: Date,
        end: Date,
        maximumCount: Int = 100_000
    ) async throws -> [KlineData] {
        guard let token = interval.apiInterval, let seconds = interval.seconds, start <= end else { return [] }
        var cursor = start
        var result: [KlineData] = []
        result.reserveCapacity(min(maximumCount, 10_000))

        while cursor <= end, result.count < maximumCount {
            guard var components = URLComponents(string: "\(baseURL)/api/v3/klines") else {
                throw BinanceAPIError.invalidURL
            }
            let pageLimit = min(1_000, maximumCount - result.count)
            components.queryItems = [
                URLQueryItem(name: "symbol", value: symbol.uppercased()),
                URLQueryItem(name: "interval", value: token),
                URLQueryItem(name: "startTime", value: String(Int64(cursor.timeIntervalSince1970 * 1_000))),
                URLQueryItem(name: "endTime", value: String(Int64(end.timeIntervalSince1970 * 1_000))),
                URLQueryItem(name: "limit", value: String(pageLimit)),
            ]
            guard let url = components.url else { throw BinanceAPIError.invalidURL }
            let (data, response) = try await session.data(from: url)
            guard let http = response as? HTTPURLResponse else { throw BinanceAPIError.invalidResponse }
            guard http.statusCode == 200 else {
                if http.statusCode == 429 { throw BinanceAPIError.rateLimited }
                throw BinanceAPIError.httpError(http.statusCode)
            }
            guard let json = try JSONSerialization.jsonObject(with: data) as? [[Any]] else {
                throw BinanceAPIError.parseError("Expected replay kline array")
            }
            let page = json.compactMap(KlineData.init(raw:)).sorted { $0.openTime < $1.openTime }
            guard let last = page.last else { break }
            result.append(contentsOf: page)
            let next = last.openTime.addingTimeInterval(seconds)
            guard next > cursor else { break }
            cursor = next
            if page.count < pageLimit { break }
        }
        return Self.sanitized(result, maximumCount: maximumCount)
    }

    private static func intervalSeconds(_ interval: String) -> TimeInterval {
        switch interval {
        case "1m": return 60
        case "5m": return 300
        case "15m": return 900
        case "30m": return 1_800
        case "1h": return 3_600
        case "1d": return 86_400
        case "1w": return 604_800
        case "1M": return 2_592_000
        case "3M": return 7_776_000
        case "1Y": return 31_536_000
        default: return 0
        }
    }

    private static func sanitized(_ candles: [KlineData], maximumCount: Int) -> [KlineData] {
        var seen = Set<Date>()
        return
            candles
            .filter { candle in
                candle.openPrice.isFinite && candle.highPrice.isFinite && candle.lowPrice.isFinite
                    && candle.closePrice.isFinite && candle.volume.isFinite && candle.highPrice >= candle.lowPrice
                    && seen.insert(candle.openTime).inserted
            }
            .sorted { $0.openTime < $1.openTime }
            .prefix(maximumCount)
            .map { $0 }
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

        return Array(matching).map { info in
            TickerSearchResult(
                symbol: "\(info.baseAsset)/\(info.quoteAsset)",
                fullSymbol: info.symbol,
                source: .binance,
                price: nil
            )
        }.ranked(for: q).prefix(20).map { $0 }
    }

}

// MARK: - Batch quotes

extension BinanceAPIService: BatchQuoteDataSource {
    private struct Ticker: Decodable {
        let symbol: String
        let lastPrice: String
        let openPrice: String
        let volume: String?
        let quoteVolume: String?
        let closeTime: Double?
    }

    private static let dollarQuotes: Set<String> = ["USDT", "USDC", "BUSD", "FDUSD", "TUSD", "USDP", "DAI", "USD"]

    /// One `/ticker/24hr` call for every symbol. `openPrice` is the rolling 24h open, a truer
    /// day-change reference than a candle picked from the last 25 hours. One unknown symbol
    /// fails the whole request; the caller then falls back to candles for each.
    func fetchQuotes(_ requests: [QuoteRequest]) async throws -> [String: SourceQuote] {
        let symbols = requests.map { $0.symbol.uppercased() }
        guard !symbols.isEmpty else { return [:] }
        guard var components = URLComponents(string: "\(baseURL)/api/v3/ticker/24hr") else {
            throw BinanceAPIError.invalidURL
        }
        let list = "[" + symbols.map { "\"\($0)\"" }.joined(separator: ",") + "]"
        components.queryItems = [
            URLQueryItem(name: "symbols", value: list),
            URLQueryItem(name: "type", value: "MINI"),
        ]
        guard let url = components.url else { throw BinanceAPIError.invalidURL }

        let (data, response) = try await session.data(from: url)
        guard let http = response as? HTTPURLResponse else { throw BinanceAPIError.invalidResponse }
        switch http.statusCode {
        case 200: break
        case 429: throw BinanceAPIError.rateLimited
        default: throw BinanceAPIError.httpError(http.statusCode)
        }

        let tickers = try JSONDecoder().decode([Ticker].self, from: data)
        let bySymbol = Dictionary(tickers.map { ($0.symbol, $0) }, uniquingKeysWith: { first, _ in first })
        var quotes: [String: SourceQuote] = [:]
        for request in requests {
            guard let ticker = bySymbol[request.symbol.uppercased()], let price = Double(ticker.lastPrice), price > 0
            else { continue }
            var quote = SourceQuote(
                price: price, previousDayPrice: Double(ticker.openPrice),
                timestamp: ticker.closeTime.map { Date(timeIntervalSince1970: $0 / 1000) })
            // Turnover is only a dollar figure on a dollar-quoted pair; otherwise report units traded.
            if Self.dollarQuotes.contains(where: ticker.symbol.uppercased().hasSuffix),
                let turnover = ticker.quoteVolume.flatMap(Double.init)
            {
                quote.volume24h = turnover
                quote.volumeKind = .quoteCurrency
            } else if let units = ticker.volume.flatMap(Double.init) {
                quote.volume24h = units
                quote.volumeKind = .base
            }
            quotes[request.symbol] = quote
        }
        return quotes
    }
}
