import Foundation
import Security

extension Notification.Name {
    static let coinMarketCapCredentialsChanged = Notification.Name("CoinMarketCapCredentialsChanged")
}

enum CoinMarketCapCredentialStore {
    private static let service = "com.cryptocharts.coinmarketcap"
    private static let account = "api-key-v1"
    private static let lock = NSLock()
    private static var cached: String?
    private static var loaded = false

    static var apiKey: String? {
        lock.lock()
        defer { lock.unlock() }
        if loaded { return cached }
        loaded = true
        var item: CFTypeRef?
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        if SecItemCopyMatching(query as CFDictionary, &item) == errSecSuccess,
            let data = item as? Data, let value = String(data: data, encoding: .utf8), !value.isEmpty
        {
            cached = value
        }
        return cached
    }

    static var isConfigured: Bool { apiKey != nil }

    static func save(_ key: String) throws {
        let value = key.trimmingCharacters(in: .whitespacesAndNewlines)
        if value.isEmpty {
            try remove()
            return
        }
        let data = Data(value.utf8)
        let identity: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service, kSecAttrAccount as String: account,
        ]
        let status = SecItemUpdate(identity as CFDictionary, [kSecValueData as String: data] as CFDictionary)
        if status == errSecItemNotFound {
            var item = identity
            item[kSecValueData as String] = data
            let add = SecItemAdd(item as CFDictionary, nil)
            guard add == errSecSuccess else { throw NSError(domain: NSOSStatusErrorDomain, code: Int(add)) }
        } else if status != errSecSuccess {
            throw NSError(domain: NSOSStatusErrorDomain, code: Int(status))
        }
        lock.lock()
        cached = value
        loaded = true
        lock.unlock()
        NotificationCenter.default.post(name: .coinMarketCapCredentialsChanged, object: nil)
    }

    static func remove() throws {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service, kSecAttrAccount as String: account,
        ]
        let status = SecItemDelete(query as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw NSError(domain: NSOSStatusErrorDomain, code: Int(status))
        }
        lock.lock()
        cached = nil
        loaded = true
        lock.unlock()
        NotificationCenter.default.post(name: .coinMarketCapCredentialsChanged, object: nil)
    }
}

struct CMCStatus: Codable, Sendable {
    let timestamp: String?
    let errorCode: Int
    let errorMessage: String?
    let elapsed: Int?
    let creditCount: Int?
    let notice: String?
    enum CodingKeys: String, CodingKey {
        case timestamp, elapsed, notice
        case errorCode = "error_code"
        case errorMessage = "error_message"
        case creditCount = "credit_count"
    }
    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        timestamp = try c.decodeIfPresent(String.self, forKey: .timestamp)
        errorMessage = try c.decodeIfPresent(String.self, forKey: .errorMessage)
        elapsed = try c.decodeIfPresent(Int.self, forKey: .elapsed)
        creditCount = try c.decodeIfPresent(Int.self, forKey: .creditCount)
        notice = try c.decodeIfPresent(String.self, forKey: .notice)
        if let number = try? c.decode(Int.self, forKey: .errorCode) {
            errorCode = number
        } else if let text = try? c.decode(String.self, forKey: .errorCode), let number = Int(text) {
            errorCode = number
        } else {
            throw DecodingError.dataCorruptedError(
                forKey: .errorCode, in: c, debugDescription: "Expected numeric error_code")
        }
    }
}

struct CMCEnvelope<Value: Codable & Sendable>: Codable, Sendable {
    let data: Value
    let status: CMCStatus
}

enum CMCDateParser {
    private static let internet = ISO8601DateFormatter()
    private static let day: DateFormatter = {
        let f = DateFormatter()
        f.locale = Locale(identifier: "en_US_POSIX")
        f.timeZone = TimeZone(secondsFromGMT: 0)
        f.dateFormat = "yyyy-MM-dd"
        return f
    }()
    static func parse(_ value: String) -> Date? {
        if let seconds = TimeInterval(value) { return Date(timeIntervalSince1970: seconds) }
        return internet.date(from: value) ?? day.date(from: value)
    }
}

struct AltcoinSeasonLatest: Codable, Sendable {
    let altcoinIndex: Double
    let altcoinMarketcap: Double?
    let snapshotTime: String
    let yearlyHigh: Double
    let yearlyHighDate: String
    let yearlyLow: Double
    let yearlyLowDate: String
    enum CodingKeys: String, CodingKey {
        case altcoinIndex = "altcoin_index"
        case altcoinMarketcap = "altcoin_marketcap"
        case snapshotTime = "snapshot_time"
        case yearlyHigh = "yearly_high"
        case yearlyHighDate = "yearly_high_date"
        case yearlyLow = "yearly_low"
        case yearlyLowDate = "yearly_low_date"
    }
    var classification: String {
        altcoinIndex <= 25 ? "Bitcoin Season" : altcoinIndex >= 75 ? "Altcoin Season" : "Neutral"
    }
}

struct AltcoinSeasonHistoricalPoint: Codable, Sendable, Identifiable {
    let timestamp: String
    let altcoinIndex: Double
    let altcoinMarketcap: Double?
    var id: String { timestamp }
    enum CodingKeys: String, CodingKey {
        case timestamp
        case altcoinIndex = "altcoin_index"
        case altcoinMarketcap = "altcoin_marketcap"
    }
}
struct AltcoinSeasonHistorical: Codable, Sendable {
    let timeframe: String
    let points: [AltcoinSeasonHistoricalPoint]
}

struct FearAndGreedLatest: Codable, Sendable {
    let value: Double
    let valueClassification: String
    let updateTime: String
    enum CodingKeys: String, CodingKey {
        case value
        case valueClassification = "value_classification"
        case updateTime = "update_time"
    }
}
struct FearAndGreedHistoricalPoint: Codable, Sendable, Identifiable {
    let timestamp: String
    let value: Double
    let valueClassification: String
    var id: String { timestamp }
    enum CodingKeys: String, CodingKey {
        case timestamp, value
        case valueClassification = "value_classification"
    }
}

enum CoinMarketCapError: LocalizedError, Equatable {
    case badRequest, rejectedKey, forbidden, rateLimited
    case server(Int)
    case offline, timeout, malformed
    case api(Int, String)
    case invalidValue
    var errorDescription: String? {
        switch self {
        case .badRequest: "CoinMarketCap rejected the request."
        case .rejectedKey: "CoinMarketCap rejected the configured API key. Check the key in Settings."
        case .forbidden: "CoinMarketCap denied access to this endpoint."
        case .rateLimited: "CoinMarketCap rate limit reached."
        case .server: "CoinMarketCap is temporarily unavailable."
        case .offline: "CoinMarketCap data unavailable while offline."
        case .timeout: "The CoinMarketCap request timed out."
        case .malformed: "CoinMarketCap returned a malformed response."
        case .api(_, let message): message
        case .invalidValue: "CoinMarketCap returned an invalid index value."
        }
    }
}

actor CoinMarketCapClient {
    static let shared = CoinMarketCapClient()
    struct CacheEntry {
        let data: Data
        let expires: Date
    }
    private let session: URLSession
    private var cache: [String: CacheEntry] = [:]
    private var inFlight: [String: Task<Data, Error>] = [:]

    init(session: URLSession = .shared) { self.session = session }

    nonisolated static func makeRequest(path: String, query: [URLQueryItem] = [], apiKey: String?) -> URLRequest {
        var components = URLComponents(
            string: apiKey == nil ? "https://pro-api.coinmarketcap.com/public-api" : "https://pro-api.coinmarketcap.com"
        )!
        components.path += path
        components.queryItems = query.isEmpty ? nil : query
        var request = URLRequest(url: components.url!, cachePolicy: .useProtocolCachePolicy, timeoutInterval: 20)
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        if let apiKey { request.setValue(apiKey, forHTTPHeaderField: "X-CMC_PRO_API_KEY") }
        return request
    }

    func request<Value: Codable & Sendable>(
        _ path: String, query: [URLQueryItem] = [], ttl: TimeInterval,
        force: Bool = false
    ) async throws -> Value {
        let key = path + "?" + query.map { "\($0.name)=\($0.value ?? "")" }.joined(separator: "&")
        if !force, let hit = cache[key], hit.expires > Date() { return try decode(hit.data) }
        let data: Data
        if let task = inFlight[key] {
            data = try await task.value
        } else {
            let task = Task { try await self.load(path: path, query: query) }
            inFlight[key] = task
            do {
                data = try await task.value
                cache[key] = CacheEntry(data: data, expires: Date().addingTimeInterval(ttl))
            } catch {
                inFlight[key] = nil
                throw error
            }
            inFlight[key] = nil
        }
        return try decode(data)
    }

    private func decode<Value: Codable>(_ data: Data) throws -> Value {
        do {
            let envelope = try JSONDecoder().decode(CMCEnvelope<Value>.self, from: data)
            guard envelope.status.errorCode == 0 else {
                throw CoinMarketCapError.api(
                    envelope.status.errorCode,
                    envelope.status.errorMessage?.nilIfBlank ?? "CoinMarketCap API error \(envelope.status.errorCode).")
            }
            return envelope.data
        } catch let error as CoinMarketCapError { throw error } catch { throw CoinMarketCapError.malformed }
    }

    private func load(path: String, query: [URLQueryItem]) async throws -> Data {
        let key = CoinMarketCapCredentialStore.apiKey
        let request = Self.makeRequest(path: path, query: query, apiKey: key)
        for attempt in 0..<4 {
            do {
                let (data, response) = try await session.data(for: request)
                guard let http = response as? HTTPURLResponse else { throw CoinMarketCapError.malformed }
                switch http.statusCode {
                case 200..<300: return data
                case 400: throw CoinMarketCapError.badRequest
                case 401: throw key == nil ? CoinMarketCapError.forbidden : CoinMarketCapError.rejectedKey
                case 403: throw CoinMarketCapError.forbidden
                case 429: if attempt == 3 { throw CoinMarketCapError.rateLimited }
                case 500...599: if attempt == 3 { throw CoinMarketCapError.server(http.statusCode) }
                default: throw CoinMarketCapError.server(http.statusCode)
                }
                let base = UInt64(1 << attempt) * 1_000_000_000
                try await Task.sleep(nanoseconds: base + UInt64.random(in: 0...250_000_000))
            } catch let error as URLError {
                switch error.code {
                case .notConnectedToInternet, .networkConnectionLost: throw CoinMarketCapError.offline
                case .timedOut: throw CoinMarketCapError.timeout
                case .cancelled: throw CancellationError()
                default: throw error
                }
            }
        }
        throw CoinMarketCapError.rateLimited
    }
}

actor CoinMarketCapDataProvider {
    static let shared = CoinMarketCapDataProvider()
    private let client: CoinMarketCapClient
    init(client: CoinMarketCapClient = .shared) { self.client = client }
    func altcoinLatest(force: Bool = false) async throws -> AltcoinSeasonLatest {
        let value: AltcoinSeasonLatest = try await client.request(
            "/v1/altcoin-season-index/latest", ttl: 900, force: force)
        try validate(value.altcoinIndex)
        try validate(value.yearlyHigh)
        try validate(value.yearlyLow)
        return value
    }
    func altcoinHistorical(_ range: CMCAltcoinRange, force: Bool = false) async throws -> AltcoinSeasonHistorical {
        let value: AltcoinSeasonHistorical = try await client.request(
            "/v1/altcoin-season-index/historical",
            query: [URLQueryItem(name: "timeframe", value: range.rawValue)], ttl: 900, force: force)
        try value.points.forEach { try validate($0.altcoinIndex) }
        return value
    }
    func fearGreedLatest(force: Bool = false) async throws -> FearAndGreedLatest {
        let value: FearAndGreedLatest = try await client.request("/v3/fear-and-greed/latest", ttl: 900, force: force)
        try validate(value.value)
        return value
    }
    func fearGreedHistorical(_ range: CMCFearGreedRange, force: Bool = false) async throws
        -> [FearAndGreedHistoricalPoint]
    {
        var points: [FearAndGreedHistoricalPoint] = []
        var start = 1
        let target = range.dayCount
        repeat {
            let limit = min(500, target.map { max(1, $0 - points.count) } ?? 500)
            let page: [FearAndGreedHistoricalPoint] = try await client.request(
                "/v3/fear-and-greed/historical",
                query: [
                    URLQueryItem(name: "start", value: String(start)),
                    URLQueryItem(name: "limit", value: String(limit)),
                ],
                ttl: 6 * 3600, force: force)
            try page.forEach { try validate($0.value) }
            points.append(contentsOf: page)
            if page.count < limit || (target != nil && points.count >= target!) { break }
            start += page.count
        } while !Task.isCancelled
        let sorted = points.sorted {
            (CMCDateParser.parse($0.timestamp) ?? .distantPast) < (CMCDateParser.parse($1.timestamp) ?? .distantPast)
        }
        guard let days = range.dayCount, let newest = sorted.last.flatMap({ CMCDateParser.parse($0.timestamp) }) else {
            return sorted
        }
        let cutoff = Calendar(identifier: .gregorian).date(byAdding: .day, value: -(days - 1), to: newest)!
        return sorted.filter { (CMCDateParser.parse($0.timestamp) ?? .distantPast) >= cutoff }
    }
    private func validate(_ value: Double) throws {
        guard value.isFinite && (0...100).contains(value) else { throw CoinMarketCapError.invalidValue }
    }
}

private extension String {
    var nilIfBlank: String? { trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? nil : self }
}
