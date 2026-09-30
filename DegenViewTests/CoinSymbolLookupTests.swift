import XCTest

@testable import DegenView

private final class CoinMarketsURLProtocol: URLProtocol {
    static var handler: ((URLRequest) throws -> Data)?

    override class func canInit(with request: URLRequest) -> Bool { true }

    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        do {
            guard let handler = Self.handler, let url = request.url else { throw URLError(.badServerResponse) }
            let data = try handler(request)
            let response = try XCTUnwrap(
                HTTPURLResponse(url: url, statusCode: 200, httpVersion: nil, headerFields: nil))
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: data)
            client?.urlProtocolDidFinishLoading(self)
        } catch {
            client?.urlProtocol(self, didFailWithError: error)
        }
    }

    override func stopLoading() {}
}

final class CoinSymbolLookupTests: XCTestCase {
    private var directory: URL!

    override func setUpWithError() throws {
        directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    }

    override func tearDown() {
        CoinMarketsURLProtocol.handler = nil
        try? FileManager.default.removeItem(at: directory)
        super.tearDown()
    }

    private func makeResolver() -> IconResolver {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [CoinMarketsURLProtocol.self]
        return IconResolver(
            session: URLSession(configuration: configuration), baseURL: "https://coingecko.test/api/v3",
            directory: directory, rateLimiter: CGRateLimiter(gap: 0))
    }

    private func coin(_ id: String, _ symbol: String) -> [String: String] {
        ["id": id, "symbol": symbol, "image": "https://img.test/\(id).png"]
    }

    private func requestedIDs(_ request: URLRequest) -> [String] {
        let items = URLComponents(url: request.url!, resolvingAgainstBaseURL: false)?.queryItems ?? []
        return (items.first { $0.name == "ids" }?.value ?? "").split(separator: ",").map(String.init).sorted()
    }

    func testSymbolComesFromTheBatchedMarketsLookupAndIsUppercased() async throws {
        var requests: [URLRequest] = []
        CoinMarketsURLProtocol.handler = { request in
            requests.append(request)
            return try JSONSerialization.data(withJSONObject: [self.coin("bitcoin", "btc")])
        }

        let symbol = await makeResolver().symbol(forCoinID: "Bitcoin")

        XCTAssertEqual(symbol, "BTC")
        XCTAssertEqual(requests.count, 1)
        XCTAssertEqual(requestedIDs(requests[0]), ["bitcoin"])
        XCTAssertEqual(
            URLComponents(url: requests[0].url!, resolvingAgainstBaseURL: false)?.path, "/api/v3/coins/markets")
    }

    func testKnownSymbolIsServedWithoutAnotherRequestAndSurvivesRelaunch() async throws {
        var calls = 0
        CoinMarketsURLProtocol.handler = { _ in
            calls += 1
            return try JSONSerialization.data(withJSONObject: [self.coin("ethereum", "eth")])
        }
        let first = makeResolver()

        _ = await first.symbol(forCoinID: "ethereum")
        let again = await first.symbol(forCoinID: "ethereum")
        let relaunched = await makeResolver().symbol(forCoinID: "ethereum")

        XCTAssertEqual(again, "ETH")
        XCTAssertEqual(relaunched, "ETH", "The map is persisted with the icon cache")
        XCTAssertEqual(calls, 1)
    }

    func testCardsAppearingTogetherShareOneRequest() async throws {
        var requests: [URLRequest] = []
        CoinMarketsURLProtocol.handler = { request in
            requests.append(request)
            return try JSONSerialization.data(withJSONObject: [self.coin("bitcoin", "btc"), self.coin("solana", "sol")])
        }
        let resolver = makeResolver()

        async let btc = resolver.symbol(forCoinID: "bitcoin")
        async let sol = resolver.symbol(forCoinID: "solana")
        let (first, second) = await (btc, sol)

        XCTAssertEqual(first, "BTC")
        XCTAssertEqual(second, "SOL")
        XCTAssertEqual(requests.count, 1)
        XCTAssertEqual(requestedIDs(requests[0]), ["bitcoin", "solana"])
    }

    func testUnlistedCoinIsNotAskedAboutAgainImmediately() async throws {
        var calls = 0
        CoinMarketsURLProtocol.handler = { _ in
            calls += 1
            return try JSONSerialization.data(withJSONObject: [Any]())
        }
        let resolver = makeResolver()

        let first = await resolver.symbol(forCoinID: "no-such-coin")
        let second = await resolver.symbol(forCoinID: "no-such-coin")

        XCTAssertNil(first)
        XCTAssertNil(second)
        XCTAssertEqual(calls, 1)
    }

    func testFailedRequestIsRetriedNextTime() async throws {
        var calls = 0
        CoinMarketsURLProtocol.handler = { _ in
            calls += 1
            if calls == 1 { throw URLError(.notConnectedToInternet) }
            return try JSONSerialization.data(withJSONObject: [self.coin("bitcoin", "btc")])
        }
        let resolver = makeResolver()

        let offline = await resolver.symbol(forCoinID: "bitcoin")
        let online = await resolver.symbol(forCoinID: "bitcoin")

        XCTAssertNil(offline)
        XCTAssertEqual(online, "BTC", "Being offline is not the same as the coin being unlisted")
    }

    // MARK: - Chart label

    private final class StubSource: TickerDataSource {
        let type = DataSourceType.coingecko
        func fetchKlines(symbol: String, interval: String, limit: Int) async throws -> [KlineData] { [] }
        func searchTickers(query: String) async throws -> [TickerSearchResult] { [] }
    }

    @MainActor
    func testCoinGeckoChartTitlesAsSymbolSlashUSDOnceResolved() {
        let chart = ChartViewModel(ticker: "bitcoin", source: .coingecko, api: StubSource())
        XCTAssertEqual(chart.title, "BITCOIN", "Until the symbol is known the header keeps the id")

        chart.coinSymbol = "BTC"

        XCTAssertEqual(chart.title, "BTC/USD")
        XCTAssertEqual(chart.marketPair?.quote, "USD", "CoinGecko prices in USD, not USDT")
        XCTAssertEqual(chart.ticker, "bitcoin", "The coin id stays the identity")
    }

    @MainActor
    func testPointingTheChartAtAnotherCoinDropsTheOldSymbol() {
        let chart = ChartViewModel(ticker: "bitcoin", source: .coingecko, api: StubSource())
        chart.coinSymbol = "BTC"

        chart.updateTicker(symbol: "ethereum", source: .coingecko)

        XCTAssertNil(chart.coinSymbol)
        XCTAssertEqual(chart.title, "ETHEREUM")
    }

    @MainActor
    func testExplicitDisplayNameStillWinsForCoinGecko() {
        let chart = ChartViewModel(ticker: "bitcoin", source: .coingecko, displayName: "My coin", api: StubSource())
        chart.coinSymbol = "BTC"

        XCTAssertEqual(chart.title, "My coin")
    }
}
