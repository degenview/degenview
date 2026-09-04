import Foundation

protocol FXRateProviding: Sendable {
    func conversion(from: PortfolioCurrency, to: PortfolioCurrency, on date: Date?) async throws
        -> FXRateService.Conversion
    func conversions(from: PortfolioCurrency, to: PortfolioCurrency, on dates: [Date]) async
        -> [Date: Result<FXRateService.Conversion, Error>]
}

extension FXRateProviding {
    func conversion(from: PortfolioCurrency, to: PortfolioCurrency) async throws -> FXRateService.Conversion {
        try await conversion(from: from, to: to, on: nil)
    }
}

actor FXRateService: FXRateProviding {
    static let shared = FXRateService()
    static let historicalTolerance: TimeInterval = 7 * 86_400

    struct Conversion: Equatable, Sendable {
        let rate: Decimal
        let observationDate: Date
        let isCached: Bool
    }

    enum ServiceError: LocalizedError {
        case rateUnavailable(PortfolioCurrency, PortfolioCurrency, Date?)

        var errorDescription: String? {
            switch self {
            case .rateUnavailable(let from, let to, let date):
                let suffix = date.map { " for \($0.formatted(date: .abbreviated, time: .omitted))" } ?? ""
                return "Exchange rate from \(from.rawValue) to \(to.rawValue) is unavailable\(suffix)."
            }
        }
    }

    private struct CurrentCache: Codable {
        var fetchedAt: Date
        var rates: [String: Decimal]
    }

    private struct HistoricalCache: Codable {
        var ratesByDay: [String: [String: Decimal]] = [:]
    }

    private struct FiatRate: Decodable {
        let date: String?
        let quote: String
        let rate: Decimal
    }

    private struct HTTPError: Error {
        let statusCode: Int
    }

    private let session: URLSession
    private let currentStore: JSONStore<CurrentCache>
    private let historicalStore: JSONStore<HistoricalCache>
    private let bitcoinHistory: BitcoinHistoryService
    private let retrySleep: @Sendable (TimeInterval) async throws -> Void
    private var current: CurrentCache?
    private var historical: HistoricalCache
    private var currentBitcoin: (fetchedAt: Date, value: Decimal)?
    private let currentBitcoinFreshness: TimeInterval = 60

    init(
        session: URLSession = AppSupport.defaultSession, bitcoinHistory: BitcoinHistoryService = .shared,
        cacheDirectory: URL = AppSupport.directory,
        retrySleep: @escaping @Sendable (TimeInterval) async throws -> Void = { delay in
            try await Task.sleep(for: .seconds(delay))
        }
    ) {
        self.session = session
        self.bitcoinHistory = bitcoinHistory
        self.retrySleep = retrySleep
        currentStore = JSONStore(filename: "alert_fx_rates.json", directory: cacheDirectory)
        historicalStore = JSONStore(filename: "portfolio_fx_history.json", directory: cacheDirectory)
        current = currentStore.load()
        historical = historicalStore.load() ?? HistoricalCache()
    }

    /// Compatibility entry point used by alerts. A valid disk value may be used for five days.
    func rate(from: PortfolioCurrency, to: PortfolioCurrency) async -> Decimal? {
        try? await conversion(from: from, to: to).rate
    }

    func conversion(from: PortfolioCurrency, to: PortfolioCurrency, on date: Date? = nil) async throws -> Conversion {
        if from == to { return Conversion(rate: 1, observationDate: date ?? Date(), isCached: false) }
        if let date { return try await historicalConversion(from: from, to: to, on: date) }
        return try await currentConversion(from: from, to: to)
    }

    func conversions(from: PortfolioCurrency, to: PortfolioCurrency, on dates: [Date]) async
        -> [Date: Result<Conversion, Error>]
    {
        let uniqueDates = Dictionary(grouping: dates, by: { Self.dayKey($0) }).compactMap { _, values in values.first }
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(secondsFromGMT: 0)!
        let requestedDays = uniqueDates.map { calendar.startOfDay(for: $0) }
        var bitcoinUSD: [String: Decimal] = [:]
        if from == .BTC || to == .BTC {
            do {
                let history = try await bitcoinHistory.refresh()
                for day in requestedDays {
                    if let close = history.closes.last(where: {
                        $0.date <= day && day.timeIntervalSince($0.date) <= Self.historicalTolerance
                    }) {
                        bitcoinUSD[Self.dayKey(day)] = Decimal(close.close)
                    }
                }
            } catch {
                return Dictionary(uniqueKeysWithValues: dates.map { ($0, .failure(error)) })
            }
        }

        let needsFiat = ![from, to].allSatisfy { $0 == .BTC || $0 == .USD }
        if needsFiat {
            let missing = requestedDays.filter {
                cachedHistorical(from: from, to: to, requested: $0, btcUSD: bitcoinUSD[Self.dayKey($0)]) == nil
            }
            if let first = missing.min(), let last = missing.max() {
                do {
                    var start = calendar.date(byAdding: .day, value: -7, to: first)!
                    while start <= last {
                        let rangeEnd = min(calendar.date(byAdding: .day, value: 364, to: start)!, last)
                        try await fetchHistoricalFiat(from: start, through: rangeEnd)
                        start = calendar.date(byAdding: .day, value: 1, to: rangeEnd)!
                    }
                } catch {
                    // Preserve per-date fallback behavior: cached days may still succeed.
                }
            }
        }

        var result: [Date: Result<Conversion, Error>] = [:]
        for date in dates {
            let day = calendar.startOfDay(for: date)
            if from == to {
                result[date] = .success(Conversion(rate: 1, observationDate: day, isCached: true))
            } else if [from, to].allSatisfy({ $0 == .BTC || $0 == .USD }),
                let rate = Self.crossRate(
                    from: from, to: to, fiatUSD: [:], btcUSD: bitcoinUSD[Self.dayKey(day)])
            {
                result[date] = .success(Conversion(rate: rate, observationDate: day, isCached: true))
            } else if let conversion = cachedHistorical(
                from: from, to: to, requested: day, btcUSD: bitcoinUSD[Self.dayKey(day)])
            {
                result[date] = .success(conversion)
            } else {
                result[date] = .failure(ServiceError.rateUnavailable(from, to, date))
            }
        }
        return result
    }

    func refresh() async { _ = try? await refreshCurrentFiat() }

    private func currentConversion(from: PortfolioCurrency, to: PortfolioCurrency) async throws -> Conversion {
        let now = Date()
        if [from, to].allSatisfy({ $0 == .BTC || $0 == .USD }), let btcUSD = await currentBitcoinUSD(),
            let value = Self.crossRate(from: from, to: to, fiatUSD: [:], btcUSD: btcUSD)
        {
            return Conversion(rate: value, observationDate: now, isCached: false)
        }
        var isCached = true
        if current == nil || now.timeIntervalSince(current!.fetchedAt) > 86_400 {
            isCached = !(try await refreshCurrentFiat())
        }
        guard let current, now.timeIntervalSince(current.fetchedAt) <= 5 * 86_400 else {
            throw ServiceError.rateUnavailable(from, to, nil)
        }
        let btcUSD = from == .BTC || to == .BTC ? await currentBitcoinUSD() : nil
        guard let value = Self.crossRate(from: from, to: to, fiatUSD: current.rates, btcUSD: btcUSD) else {
            throw ServiceError.rateUnavailable(from, to, nil)
        }
        return Conversion(rate: value, observationDate: current.fetchedAt, isCached: isCached)
    }

    private func historicalConversion(
        from: PortfolioCurrency, to: PortfolioCurrency, on date: Date
    ) async throws -> Conversion {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(secondsFromGMT: 0)!
        let requested = calendar.startOfDay(for: date)
        let btcUSD: Decimal?
        if from == .BTC || to == .BTC {
            let history = try await bitcoinHistory.refresh()
            btcUSD = history.closes.last(where: {
                $0.date <= requested && requested.timeIntervalSince($0.date) <= Self.historicalTolerance
            }).map { Decimal($0.close) }
        } else {
            btcUSD = nil
        }
        if from == .BTC || to == .BTC,
            let value = Self.crossRate(from: from, to: to, fiatUSD: [:], btcUSD: btcUSD),
            [from, to].allSatisfy({ $0 == .BTC || $0 == .USD })
        {
            return Conversion(rate: value, observationDate: requested, isCached: true)
        }
        if let result = cachedHistorical(from: from, to: to, requested: requested, btcUSD: btcUSD) {
            return result
        }
        let start = calendar.date(byAdding: .day, value: -7, to: requested)!
        try await fetchHistoricalFiat(from: start, through: requested)
        if let result = cachedHistorical(from: from, to: to, requested: requested, btcUSD: btcUSD) {
            return Conversion(rate: result.rate, observationDate: result.observationDate, isCached: false)
        }
        throw ServiceError.rateUnavailable(from, to, date)
    }

    private func cachedHistorical(
        from: PortfolioCurrency, to: PortfolioCurrency, requested: Date, btcUSD: Decimal?
    ) -> Conversion? {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(secondsFromGMT: 0)!
        for offset in 0...7 {
            let candidate = calendar.date(byAdding: .day, value: -offset, to: requested)!
            if let rates = historical.ratesByDay[Self.dayKey(candidate)],
                let value = Self.crossRate(from: from, to: to, fiatUSD: rates, btcUSD: btcUSD)
            {
                return Conversion(rate: value, observationDate: candidate, isCached: true)
            }
        }
        return nil
    }

    @discardableResult private func refreshCurrentFiat() async throws -> Bool {
        let url = URL(string: "https://api.frankfurter.dev/v2/rates?base=USD&quotes=EUR,GBP,JPY,CHF")!
        do {
            let data = try await requestData(from: url)
            let values = try JSONDecoder().decode([FiatRate].self, from: data)
            let next = CurrentCache(
                fetchedAt: Date(), rates: Dictionary(uniqueKeysWithValues: values.map { ($0.quote, $0.rate) }))
            current = next
            currentStore.save(next)
            return true
        } catch {
            if current != nil { return false }
            throw error
        }
    }

    private func fetchHistoricalFiat(from start: Date, through date: Date) async throws {
        var components = URLComponents(string: "https://api.frankfurter.dev/v2/rates")!
        components.queryItems = [
            .init(name: "base", value: "USD"), .init(name: "quotes", value: "EUR,GBP,JPY,CHF"),
            .init(name: "from", value: Self.dayKey(start)), .init(name: "to", value: Self.dayKey(date)),
        ]
        let data = try await requestData(from: components.url!)
        for item in try JSONDecoder().decode([FiatRate].self, from: data) {
            guard let day = item.date else { continue }
            historical.ratesByDay[day, default: [:]][item.quote] = item.rate
        }
        historicalStore.save(historical)
    }

    private func requestData(from url: URL) async throws -> Data {
        let maximumAttempts = 4
        var attempt = 0
        while true {
            do {
                let (data, response) = try await session.data(from: url)
                guard let http = response as? HTTPURLResponse else { throw URLError(.badServerResponse) }
                guard (200..<300).contains(http.statusCode) else {
                    let error = HTTPError(statusCode: http.statusCode)
                    guard attempt + 1 < maximumAttempts, Self.isRetryable(statusCode: http.statusCode) else {
                        throw error
                    }
                    let delay = Self.retryDelay(attempt: attempt, response: http)
                    attempt += 1
                    try await retrySleep(delay)
                    continue
                }
                return data
            } catch is CancellationError {
                throw CancellationError()
            } catch {
                guard attempt + 1 < maximumAttempts, Self.isRetryable(error: error) else { throw error }
                let delay = Self.retryDelay(attempt: attempt, response: nil)
                attempt += 1
                try await retrySleep(delay)
            }
        }
    }

    private nonisolated static func isRetryable(statusCode: Int) -> Bool {
        statusCode == 408 || statusCode == 425 || statusCode == 429 || (500...599).contains(statusCode)
    }

    private nonisolated static func isRetryable(error: Error) -> Bool {
        guard let error = error as? URLError else { return false }
        return [
            .backgroundSessionInUseByAnotherProcess, .backgroundSessionWasDisconnected, .cannotConnectToHost,
            .cannotFindHost, .dataNotAllowed, .dnsLookupFailed, .internationalRoamingOff, .networkConnectionLost,
            .notConnectedToInternet, .resourceUnavailable, .secureConnectionFailed, .timedOut,
        ].contains(error.code)
    }

    private nonisolated static func retryDelay(attempt: Int, response: HTTPURLResponse?) -> TimeInterval {
        if let value = response?.value(forHTTPHeaderField: "Retry-After"), let seconds = TimeInterval(value) {
            return min(max(seconds, 0), 10)
        }
        return min(0.5 * pow(2, Double(attempt)), 4)
    }

    private func currentBitcoinUSD() async -> Decimal? {
        let now = Date()
        if let currentBitcoin, now.timeIntervalSince(currentBitcoin.fetchedAt) < currentBitcoinFreshness {
            return currentBitcoin.value
        }
        let service = DataSourceFactory.shared.service(for: .binance)
        guard let bars = try? await service.fetchKlines(symbol: "BTCUSDT", interval: "1h", limit: 1),
            let close = bars.last?.closePrice
        else { return nil }
        let value = Decimal(close)
        currentBitcoin = (now, value)
        return value
    }

    nonisolated static func crossRate(
        from: PortfolioCurrency, to: PortfolioCurrency, fiatUSD: [String: Decimal], btcUSD: Decimal?
    ) -> Decimal? {
        func unitsPerUSD(_ currency: PortfolioCurrency) -> Decimal? {
            switch currency {
            case .USD: 1
            case .BTC: btcUSD.map { 1 / $0 }
            default: fiatUSD[currency.rawValue]
            }
        }
        guard let source = unitsPerUSD(from), let destination = unitsPerUSD(to), source != 0 else { return nil }
        return destination / source
    }

    private static func dayKey(_ date: Date) -> String {
        let formatter = DateFormatter()
        formatter.calendar = Calendar(identifier: .gregorian)
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.timeZone = TimeZone(secondsFromGMT: 0)
        formatter.dateFormat = "yyyy-MM-dd"
        return formatter.string(from: date)
    }
}
