import Foundation

enum AlpacaError: LocalizedError {
    case credentialsMissing
    case invalidResponse
    case noData(String)
    case api(String)

    var errorDescription: String? {
        switch self {
        case .credentialsMissing: return "Set up your Alpaca API key in Settings before using stock charts."
        case .invalidResponse: return "Alpaca returned an invalid response."
        case .noData(let symbol): return "No data is available for \(symbol) on Alpaca's free IEX feed."
        case .api(let message): return message
        }
    }
}

final class AlpacaAPIService: TickerDataSource {
    let type: DataSourceType = .alpaca
    private let session: URLSession
    private var assetCache: [Asset]?

    init(session: URLSession = .shared) { self.session = session }

    func fetchKlines(symbol: String, interval: String, limit: Int) async throws -> [KlineData] {
        let credentials = try configuredCredentials()
        var components = URLComponents(string: "https://data.alpaca.markets/v2/stocks/\(symbol.uppercased())/bars")!
        components.queryItems = [
            URLQueryItem(name: "timeframe", value: Self.timeframe(for: interval)),
            URLQueryItem(name: "start", value: Self.startDate(interval: interval, limit: limit)),
            URLQueryItem(name: "limit", value: String(min(limit, 10_000))),
            URLQueryItem(name: "adjustment", value: "all"),
            URLQueryItem(name: "feed", value: "iex"),
            URLQueryItem(name: "sort", value: "desc")
        ]
        let data = try await request(components.url!, credentials: credentials)
        let response = try JSONDecoder.alpaca.decode(BarsResponse.self, from: data)
        guard !response.bars.isEmpty else { throw AlpacaError.noData(symbol.uppercased()) }
        return response.bars.reversed().map { bar in
            KlineData(openTime: bar.t, openPrice: bar.o, highPrice: bar.h, lowPrice: bar.l,
                      closePrice: bar.c, volume: bar.v, quoteVolume: bar.vw.map { $0 * bar.v } ?? 0)
        }
    }

    func searchTickers(query: String) async throws -> [TickerSearchResult] {
        let credentials = try configuredCredentials()
        let assets: [Asset]
        if let assetCache { assets = assetCache } else {
            // The app asks users for Paper Trading credentials. Alpaca's trading
            // endpoints (including the asset directory) require those keys to use
            // the paper host; the same keys still authenticate against Market Data.
            var components = URLComponents(string: "https://paper-api.alpaca.markets/v2/assets")!
            components.queryItems = [
                URLQueryItem(name: "status", value: "active"),
                URLQueryItem(name: "asset_class", value: "us_equity")
            ]
            let data = try await request(components.url!, credentials: credentials)
            assets = try JSONDecoder().decode([Asset].self, from: data)
            assetCache = assets
        }
        let needle = query.trimmingCharacters(in: .whitespacesAndNewlines).uppercased()
        return assets.lazy.filter {
            $0.tradable && ($0.symbol.uppercased().contains(needle) || $0.name.uppercased().contains(needle))
        }.prefix(30).map {
            TickerSearchResult(symbol: "\($0.symbol) — \($0.name)", fullSymbol: $0.symbol, source: .alpaca, price: nil)
        }
    }

    private func configuredCredentials() throws -> AlpacaCredentials {
        let value = AlpacaCredentialsStore.credentials
        guard value.isConfigured else { throw AlpacaError.credentialsMissing }
        return value
    }

    private func request(_ url: URL, credentials: AlpacaCredentials) async throws -> Data {
        var request = URLRequest(url: url)
        request.setValue(credentials.keyID, forHTTPHeaderField: "APCA-API-KEY-ID")
        request.setValue(credentials.secretKey, forHTTPHeaderField: "APCA-API-SECRET-KEY")
        let (data, response) = try await session.data(for: request)
        guard let http = response as? HTTPURLResponse else { throw AlpacaError.invalidResponse }
        guard (200..<300).contains(http.statusCode) else {
            let message = (try? JSONDecoder().decode(APIError.self, from: data).message)
                ?? HTTPURLResponse.localizedString(forStatusCode: http.statusCode)
            throw AlpacaError.api(message)
        }
        return data
    }

    private static func timeframe(for interval: String) -> String {
        switch interval {
        case "1h": return "1Hour"
        case "1d": return "1Day"
        case "1w": return "1Week"
        case "1M": return "1Month"
        default: return "1Day"
        }
    }

    /// Alpaca otherwise defaults historical bars to the current trading day. That
    /// produces an empty response before the open and throughout weekends/holidays.
    /// Include ample non-trading time, then request descending bars so `limit` keeps
    /// the newest observations rather than the oldest part of the wider window.
    private static func startDate(interval: String, limit: Int) -> String {
        let secondsPerCandle: TimeInterval
        let marketGapFactor: Double
        switch interval {
        case "1h": secondsPerCandle = 3_600; marketGapFactor = 6
        case "1d": secondsPerCandle = 86_400; marketGapFactor = 2
        case "1w": secondsPerCandle = 604_800; marketGapFactor = 1.6
        case "1M": secondsPerCandle = 2_592_000; marketGapFactor = 1.5
        default: secondsPerCandle = 86_400; marketGapFactor = 2
        }
        let lookback = secondsPerCandle * Double(max(limit, 1)) * marketGapFactor
        return ISO8601DateFormatter().string(from: Date().addingTimeInterval(-lookback))
    }

    private struct BarsResponse: Decodable {
        let bars: [Bar]

        private enum CodingKeys: String, CodingKey { case bars }

        init(from decoder: Decoder) throws {
            let container = try decoder.container(keyedBy: CodingKeys.self)
            // Alpaca sometimes omits `bars` instead of returning an empty array
            // when IEX has no observations for the requested symbol.
            bars = try container.decodeIfPresent([Bar].self, forKey: .bars) ?? []
        }
    }
    private struct Bar: Decodable {
        let t: Date; let o: Double; let h: Double; let l: Double; let c: Double
        let v: Double; let vw: Double?
    }
    private struct Asset: Decodable { let symbol: String; let name: String; let tradable: Bool }
    private struct APIError: Decodable { let message: String }
}

private extension JSONDecoder {
    static var alpaca: JSONDecoder {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        return decoder
    }
}
