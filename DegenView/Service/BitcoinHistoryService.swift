import Foundation

actor BitcoinHistoryService {
    static let shared = BitcoinHistoryService()
    static let cacheLifetime: TimeInterval = 12 * 60 * 60

    struct Cache: Codable, Equatable {
        let fetchedAt: Date
        let closes: [BitcoinDailyClose]
    }

    enum ServiceError: LocalizedError {
        case invalidResponse
        case emptyHistory

        var errorDescription: String? {
            switch self {
            case .invalidResponse: "Bitstamp returned an invalid response."
            case .emptyHistory: "Bitstamp returned no BTC/USD history."
            }
        }
    }

    private let session: URLSession
    private let cacheURL: URL

    init(session: URLSession = AppSupport.defaultSession, cacheURL: URL? = nil) {
        self.session = session
        self.cacheURL = cacheURL ?? AppSupport.directory.appendingPathComponent("bitcoin_power_law_history.json")
    }

    func cachedHistory() -> Cache? {
        guard let data = try? Data(contentsOf: cacheURL) else { return nil }
        return try? JSONDecoder().decode(Cache.self, from: data)
    }

    func refresh(now: Date = Date(), force: Bool = false) async throws -> Cache {
        let cached = cachedHistory()
        if !force, let cached, now.timeIntervalSince(cached.fetchedAt) < Self.cacheLifetime {
            return cached
        }

        do {
            let closes: [BitcoinDailyClose]
            if let cached, !cached.closes.isEmpty {
                closes = Self.merge(cached.closes, with: try await fetchBatch(endingAt: now))
            } else {
                closes = try await fetchFullHistory(endingAt: now)
            }
            guard !closes.isEmpty else { throw ServiceError.emptyHistory }
            let result = Cache(fetchedAt: now, closes: closes)
            try write(result)
            return result
        } catch {
            if let cached, !cached.closes.isEmpty { return cached }
            throw error
        }
    }

    private func fetchFullHistory(endingAt date: Date) async throws -> [BitcoinDailyClose] {
        var result: [BitcoinDailyClose] = []
        var cursor = date
        while true {
            let batch = try await fetchBatch(endingAt: cursor)
            guard !batch.isEmpty else { break }
            result = Self.merge(result, with: batch)
            guard batch.count == 1_000, let earliest = batch.first?.date else { break }
            let next = earliest.addingTimeInterval(-1)
            guard next < cursor else { break }
            cursor = next
        }
        return result
    }

    private func fetchBatch(endingAt date: Date) async throws -> [BitcoinDailyClose] {
        var components = URLComponents(string: "https://www.bitstamp.net/api/v2/ohlc/btcusd/")!
        components.queryItems = [
            URLQueryItem(name: "step", value: "86400"),
            URLQueryItem(name: "limit", value: "1000"),
            URLQueryItem(name: "end", value: String(Int(date.timeIntervalSince1970))),
            URLQueryItem(name: "exclude_current_candle", value: "true"),
        ]
        let (data, response) = try await session.data(from: components.url!)
        guard let http = response as? HTTPURLResponse, (200..<300).contains(http.statusCode) else {
            throw ServiceError.invalidResponse
        }
        return try Self.parse(data)
    }

    static func parse(_ data: Data) throws -> [BitcoinDailyClose] {
        struct Response: Decodable {
            struct DataBody: Decodable {
                struct OHLC: Decodable {
                    let timestamp: String
                    let close: String
                }
                let ohlc: [OHLC]
            }
            let data: DataBody
        }
        let decoded = try JSONDecoder().decode(Response.self, from: data)
        let values = decoded.data.ohlc.compactMap { item -> BitcoinDailyClose? in
            guard let timestamp = TimeInterval(item.timestamp), let close = Double(item.close),
                timestamp.isFinite, close.isFinite, close > 0
            else { return nil }
            return BitcoinDailyClose(date: Date(timeIntervalSince1970: timestamp), close: close)
        }
        return merge([], with: values)
    }

    static func merge(_ old: [BitcoinDailyClose], with new: [BitcoinDailyClose]) -> [BitcoinDailyClose] {
        var byDate = Dictionary(uniqueKeysWithValues: old.map { ($0.date, $0) })
        new.forEach { byDate[$0.date] = $0 }
        return byDate.values.sorted { $0.date < $1.date }
    }

    private func write(_ cache: Cache) throws {
        let data = try JSONEncoder().encode(cache)
        try data.write(to: cacheURL, options: .atomic)
    }
}
