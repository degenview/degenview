import XCTest

@testable import DegenView

private final class QuoteURLProtocol: URLProtocol {
    static var handler: ((URLRequest) throws -> (Int, Data))?

    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        do {
            guard let handler = Self.handler, let url = request.url else { throw URLError(.badServerResponse) }
            let (status, data) = try handler(request)
            let response = try XCTUnwrap(
                HTTPURLResponse(url: url, statusCode: status, httpVersion: nil, headerFields: nil))
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: data)
            client?.urlProtocolDidFinishLoading(self)
        } catch {
            client?.urlProtocol(self, didFailWithError: error)
        }
    }

    override func stopLoading() {}
}

/// A source that answers batches (and candles), recording what it was asked.
private final class BatchSource: BatchQuoteDataSource, @unchecked Sendable {
    let type: DataSourceType
    var answers: [String: SourceQuote] = [:]
    var batchFails = false
    var klines: [String: [KlineData]] = [:]
    /// Held until opened, to stand in for a slow network.
    var gate: Gate?
    private let lock = NSLock()
    private var batches: [[String]] = []
    private var candleRequests: [String] = []

    init(_ type: DataSourceType) { self.type = type }

    var batchCalls: [[String]] { lock.withLock { batches } }
    var candleCalls: [String] { lock.withLock { candleRequests } }

    func fetchQuotes(_ requests: [QuoteRequest]) async throws -> [String: SourceQuote] {
        lock.withLock { batches.append(requests.map(\.symbol)) }
        await gate?.wait()
        if batchFails { throw URLError(.notConnectedToInternet) }
        return answers
    }

    func fetchKlines(symbol: String, interval: String, limit: Int) async throws -> [KlineData] {
        lock.withLock { candleRequests.append(symbol) }
        return klines[symbol] ?? []
    }

    func searchTickers(query: String) async throws -> [TickerSearchResult] { [] }
}

private final class CandleOnlySource: TickerDataSource, @unchecked Sendable {
    let type: DataSourceType
    var klines: [String: [KlineData]] = [:]

    init(_ type: DataSourceType) { self.type = type }

    func fetchKlines(symbol: String, interval: String, limit: Int) async throws -> [KlineData] {
        klines[symbol] ?? []
    }

    func searchTickers(query: String) async throws -> [TickerSearchResult] { [] }
}

private actor Gate {
    private var isOpen = false
    func open() { isOpen = true }
    func wait() async { while !isOpen { try? await Task.sleep(nanoseconds: 5_000_000) } }
}

private actor Counter {
    private(set) var value = 0
    func increment() { value += 1 }
}

final class PortfolioQuoteFetcherTests: XCTestCase {
    private func asset(_ source: DataSourceType, _ symbol: String, quote: PortfolioCurrency = .USD) -> PortfolioAsset {
        PortfolioAsset(
            key: "\(source.rawValue):\(symbol)", symbol: symbol, name: symbol, source: source, quoteCurrency: quote)
    }

    private func bars(_ closes: [Double], hoursApart: Double = 1) -> [KlineData] {
        let end = Date()
        return closes.enumerated().map { index, close in
            KlineData(
                openTime: end.addingTimeInterval(-Double(closes.count - 1 - index) * hoursApart * 3_600),
                openPrice: close, highPrice: close, lowPrice: close, closePrice: close, volume: 1)
        }
    }

    // MARK: - SourceQuote

    func testChangePercentDerivesThePreviousPrice() {
        XCTAssertEqual(SourceQuote(price: 110, changePercent24h: 10).previousDayPrice ?? 0, 100, accuracy: 1e-9)
        XCTAssertEqual(SourceQuote(price: 90, changePercent24h: -10).previousDayPrice ?? 0, 100, accuracy: 1e-9)
        XCTAssertNil(SourceQuote(price: 1, changePercent24h: nil).previousDayPrice)
        XCTAssertNil(SourceQuote(price: 1, changePercent24h: -100).previousDayPrice)
    }

    // MARK: - Fetcher

    func testEachSourceIsPricedWithOneBatchCall() async {
        let binance = BatchSource(.binance)
        binance.answers = [
            "BTCUSDT": SourceQuote(price: 100, previousDayPrice: 90),
            "ETHUSDT": SourceQuote(price: 10, previousDayPrice: nil),
        ]
        let coinbase = BatchSource(.coinbase)
        coinbase.answers = ["SOL-USD": SourceQuote(price: 5, previousDayPrice: 4)]
        let fetcher = PortfolioQuoteFetcher { $0 == .binance ? binance : coinbase }
        let btc = asset(.binance, "BTCUSDT")
        let eth = asset(.binance, "ETHUSDT")
        let sol = asset(.coinbase, "SOL-USD")

        let quotes = await fetcher.quotes(for: [btc, eth, sol])

        XCTAssertEqual(binance.batchCalls.count, 1)
        XCTAssertEqual(Set(binance.batchCalls[0]), ["BTCUSDT", "ETHUSDT"])
        XCTAssertEqual(coinbase.batchCalls, [["SOL-USD"]])
        XCTAssertTrue(binance.candleCalls.isEmpty && coinbase.candleCalls.isEmpty)
        XCTAssertEqual(quotes[btc.key]?.price, 100)
        XCTAssertEqual(quotes[btc.key]?.previousDayPrice, 90)
        XCTAssertNil(quotes[eth.key]?.previousDayPrice)
        XCTAssertEqual(quotes[sol.key]?.price, 5)
    }

    func testSymbolTheBatchDidNotAnswerFallsBackToCandles() async {
        let binance = BatchSource(.binance)
        binance.answers = ["BTCUSDT": SourceQuote(price: 100, previousDayPrice: 90)]
        // 25 hourly closes. The last is 7; the newest bar at least 23h older is the 23h-old one (5).
        binance.klines["ETHUSDT"] = bars([2] + Array(repeating: 5, count: 23) + [7])
        let fetcher = PortfolioQuoteFetcher { _ in binance }
        let eth = asset(.binance, "ETHUSDT")

        let quotes = await fetcher.quotes(for: [asset(.binance, "BTCUSDT"), eth])

        XCTAssertEqual(binance.candleCalls, ["ETHUSDT"])
        XCTAssertEqual(quotes[eth.key]?.price, 7)
        XCTAssertEqual(quotes[eth.key]?.previousDayPrice, 5)
        XCTAssertEqual(quotes.count, 2)
    }

    func testFailedBatchFallsBackForEveryAsset() async {
        let binance = BatchSource(.binance)
        binance.batchFails = true
        binance.klines["BTCUSDT"] = bars([1, 2, 3])
        binance.klines["ETHUSDT"] = bars([4, 5, 6])
        let fetcher = PortfolioQuoteFetcher { _ in binance }
        let btc = asset(.binance, "BTCUSDT")
        let eth = asset(.binance, "ETHUSDT")

        let quotes = await fetcher.quotes(for: [btc, eth])

        XCTAssertEqual(Set(binance.candleCalls), ["BTCUSDT", "ETHUSDT"])
        XCTAssertEqual(quotes[btc.key]?.price, 3)
        XCTAssertEqual(quotes[eth.key]?.price, 6)
    }

    func testSourceWithoutABatchCallUsesCandles() async {
        let source = CandleOnlySource(.coinMarketCap)
        source.klines["X"] = bars([8, 9])
        let fetcher = PortfolioQuoteFetcher { _ in source }

        let quotes = await fetcher.quotes(for: [asset(.coinMarketCap, "X")])

        XCTAssertEqual(quotes["\(DataSourceType.coinMarketCap.rawValue):X"]?.price, 9)
    }

    func testAssetsNotQuotedInDollarsAreSkipped() async {
        let binance = BatchSource(.binance)
        let fetcher = PortfolioQuoteFetcher { _ in binance }

        let quotes = await fetcher.quotes(for: [asset(.binance, "BTCEUR", quote: .EUR)])

        XCTAssertTrue(quotes.isEmpty)
        XCTAssertTrue(binance.batchCalls.isEmpty)
        XCTAssertTrue(binance.candleCalls.isEmpty)
    }

    // MARK: - Binance and Coinbase responses

    private func session() -> URLSession {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [QuoteURLProtocol.self]
        return URLSession(configuration: configuration)
    }

    override func tearDown() {
        QuoteURLProtocol.handler = nil
        super.tearDown()
    }

    func testBinanceReadsLastAndOpenPriceFromOneTickerRequest() async throws {
        var requests: [URLRequest] = []
        QuoteURLProtocol.handler = { request in
            requests.append(request)
            let body = """
                [{"symbol":"BTCUSDT","openPrice":"90000.5","lastPrice":"99000.25","closeTime":1},
                 {"symbol":"ETHUSDT","openPrice":"3000","lastPrice":"3300","closeTime":1}]
                """
            return (200, Data(body.utf8))
        }
        let service = BinanceAPIService(session: session())

        let quotes = try await service.fetchQuotes([
            QuoteRequest(symbol: "btcusdt", metadata: [:]), QuoteRequest(symbol: "ETHUSDT", metadata: [:]),
        ])

        XCTAssertEqual(requests.count, 1)
        let items = URLComponents(url: try XCTUnwrap(requests[0].url), resolvingAgainstBaseURL: false)?.queryItems
        XCTAssertEqual(items?.first { $0.name == "symbols" }?.value, "[\"BTCUSDT\",\"ETHUSDT\"]")
        // Keyed by the symbol as it was passed in.
        XCTAssertEqual(quotes["btcusdt"]?.price, 99_000.25)
        XCTAssertEqual(quotes["btcusdt"]?.previousDayPrice, 90_000.5)
        XCTAssertEqual(quotes["ETHUSDT"]?.price, 3_300)
        XCTAssertEqual(quotes["ETHUSDT"]?.previousDayPrice, 3_000)
    }

    func testBinanceRejectionThrowsSoTheCallerCanFallBack() async {
        QuoteURLProtocol.handler = { _ in (400, Data("{\"code\":-1121}".utf8)) }
        let service = BinanceAPIService(session: session())

        do {
            _ = try await service.fetchQuotes([QuoteRequest(symbol: "NOPE", metadata: [:])])
            XCTFail("expected a throw")
        } catch {}
    }

    func testCoinbaseReadsStatsPerProductAndSkipsFailures() async throws {
        QuoteURLProtocol.handler = { request in
            let path = request.url?.path ?? ""
            if path.hasSuffix("/BTC-USD/stats") {
                return (200, Data("{\"open\":\"90\",\"high\":\"1\",\"low\":\"1\",\"last\":\"99\"}".utf8))
            }
            return (404, Data("{}".utf8))
        }
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [QuoteURLProtocol.self]
        let service = CoinbaseAPIService(session: URLSession(configuration: configuration), requestSpacing: 0)

        let quotes = try await service.fetchQuotes([
            QuoteRequest(symbol: "BTC-USD", metadata: [:]), QuoteRequest(symbol: "GONE-USD", metadata: [:]),
        ])

        XCTAssertEqual(Array(quotes.keys), ["BTC-USD"])
        XCTAssertEqual(quotes["BTC-USD"]?.price, 99)
        XCTAssertEqual(quotes["BTC-USD"]?.previousDayPrice, 90)
    }

    // MARK: - Startup: the value doesn't wait for the chart

    private struct Fixture {
        let store: PortfolioStore
        let quoteGate: Gate
        let candleGate: Gate
        let candleRequests: Counter
    }

    @MainActor
    private func startupFixture(persistedQuote: Bool) throws -> Fixture {
        let btc = asset(.binance, "BTCUSDT")
        let day = 86_400.0
        let start = Date(timeIntervalSince1970: 1_700_000_000 - 1_700_000_000.truncatingRemainder(dividingBy: day))
        let portfolio = Portfolio(id: UUID(), name: "Main", baseCurrency: .USD)
        let buy = PortfolioTransaction(
            portfolioID: portfolio.id, asset: btc, type: .buy, quantity: 1, price: 10, timestamp: start)
        let point = PortfolioSnapshot(
            portfolioID: portfolio.id, timestamp: start, value: 10, netContributions: 10,
            realizedPnL: 0, unrealizedPnL: 0, isComplete: true)
        let ledger = PortfolioLedgerSnapshot(
            portfolios: [portfolio], transactions: [buy], historicalSnapshots: [point],
            selectedPortfolioID: portfolio.id)
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)

        let quoteGate = Gate()
        let candleGate = Gate()
        let candleRequests = Counter()
        let database = try AppDatabase.makeInMemory()
        let candles = PortfolioCandleStore(database: database) { _, _ in
            await candleRequests.increment()
            await candleGate.wait()
            throw URLError(.cancelled)
        }
        let source = BatchSource(.binance)
        source.answers = ["BTCUSDT": SourceQuote(price: 12, previousDayPrice: 10)]
        source.gate = quoteGate
        let quotes = persistedQuote ? [btc.key: PortfolioQuote(price: 12, timestamp: Date())] : [:]
        let store = PortfolioStore(
            initialSnapshot: ledger, initialQuotes: quotes, database: database, storageDirectory: directory,
            candleStore: candles, quoteFetcher: PortfolioQuoteFetcher { _ in source })
        return Fixture(store: store, quoteGate: quoteGate, candleGate: candleGate, candleRequests: candleRequests)
    }

    @MainActor
    private func waitUntil(_ condition: () -> Bool) async throws {
        for _ in 0..<400 where !condition() { try await Task.sleep(nanoseconds: 5_000_000) }
    }

    @MainActor
    func testValueAppearsOnceQuotesArriveWhileHistoryIsStillLoading() async throws {
        let fixture = try startupFixture(persistedQuote: false)
        let store = fixture.store

        let running = Task { await store.initialize() }
        try await Task.sleep(nanoseconds: 100_000_000)

        // Quotes are still in flight: no value yet, and history has not been asked for anything.
        XCTAssertTrue(store.isLoadingInitialValues)
        let earlyRequests = await fixture.candleRequests.value
        XCTAssertEqual(earlyRequests, 0)

        await fixture.quoteGate.open()
        try await waitUntil { !store.isLoadingInitialValues }

        // The value is real while the chart's candles are still blocked.
        XCTAssertFalse(store.isLoadingInitialValues)
        XCTAssertEqual(store.totalValue, 12)
        try await waitUntil { store.isLoadingHistory }
        XCTAssertTrue(store.isLoadingHistory)

        await fixture.candleGate.open()
        await running.value
        XCTAssertFalse(store.isLoadingHistory)
        XCTAssertEqual(store.totalValue, 12)
    }

    @MainActor
    func testPersistedQuotesShowTheValueBeforeAnyNetworkAnswers() async throws {
        let fixture = try startupFixture(persistedQuote: true)
        let store = fixture.store
        XCTAssertTrue(store.isLoadingInitialValues)

        let running = Task { await store.initialize() }
        try await waitUntil { !store.isLoadingInitialValues }

        XCTAssertEqual(store.totalValue, 12)
        await fixture.quoteGate.open()
        await fixture.candleGate.open()
        await running.value
        XCTAssertFalse(store.isUpdating)
    }
}
