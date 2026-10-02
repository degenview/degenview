import XCTest

@testable import DegenView

private final class TrendingURLProtocol: URLProtocol {
    static var handler: ((URLRequest) throws -> Data)?

    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        do {
            guard let handler = Self.handler, let url = request.url else { throw URLError(.badServerResponse) }
            let data = try handler(request)
            let response = try XCTUnwrap(HTTPURLResponse(url: url, statusCode: 200, httpVersion: nil, headerFields: nil))
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: data)
            client?.urlProtocolDidFinishLoading(self)
        } catch {
            client?.urlProtocol(self, didFailWithError: error)
        }
    }

    override func stopLoading() {}
}

/// A scripted trending source that counts how often it is asked.
private final class StubTrendingSource: TrendingMarketsDataSource, @unchecked Sendable {
    let type: DataSourceType = .polymarket
    var results: [TickerSearchResult] = []
    var searchResults: [TickerSearchResult] = []
    var fails = false
    private(set) var trendingCalls = 0

    func trendingMarkets(limit: Int) async throws -> [TickerSearchResult] {
        trendingCalls += 1
        if fails { throw URLError(.notConnectedToInternet) }
        return results
    }

    func searchTickers(query: String) async throws -> [TickerSearchResult] { searchResults }
    func fetchKlines(symbol: String, interval: String, limit: Int) async throws -> [KlineData] { [] }
    func getCachedKlines(symbol: String, interval: String, count: Int) async -> [KlineData]? { nil }
    func fetchPrices(marketID: String, range: TimeRange, count: Int) async throws -> [KlineData] { [] }
}

/// A source with no trending endpoint, like Kalshi.
private final class StubPlainSource: PredictionMarketDataSource, @unchecked Sendable {
    let type: DataSourceType = .kalshi
    func searchTickers(query: String) async throws -> [TickerSearchResult] { [] }
    func fetchKlines(symbol: String, interval: String, limit: Int) async throws -> [KlineData] { [] }
    func getCachedKlines(symbol: String, interval: String, count: Int) async -> [KlineData]? { nil }
    func fetchPrices(marketID: String, range: TimeRange, count: Int) async throws -> [KlineData] { [] }
}

final class PredictionTrendingTests: XCTestCase {
    private func market(_ label: String, event: String, price: Double = 0.5) -> TickerSearchResult {
        TickerSearchResult(
            symbol: label, fullSymbol: "token-\(label)", source: .polymarket, price: price,
            metadata: ["eventTitle": event, "question": label, "imageURL": ""])
    }

    // MARK: - Service

    func testTrendingRequestsTheBusiestLiveEventsAndFlattensThemLikeSearch() async throws {
        var requested: URLRequest?
        TrendingURLProtocol.handler = { request in
            requested = request
            return Data(
                """
                [
                  {"id":"1","title":"Fed decision","markets":[
                    {"question":"Cut?","groupItemTitle":"Cut","active":true,"closed":false,
                     "outcomes":"[\\"Yes\\",\\"No\\"]","outcomePrices":"[\\"0.7\\",\\"0.3\\"]","clobTokenIds":"[\\"a\\",\\"b\\"]"},
                    {"question":"Hold?","groupItemTitle":"Hold","active":true,"closed":false,
                     "outcomes":"[\\"Yes\\",\\"No\\"]","outcomePrices":"[\\"0.3\\",\\"0.7\\"]","clobTokenIds":"[\\"c\\",\\"d\\"]"}]},
                  {"id":"2","title":"Single","markets":[
                    {"question":"Will it?","groupItemTitle":"Will it?","active":true,"closed":false,
                     "outcomes":"[\\"Yes\\",\\"No\\"]","outcomePrices":"[\\"0.5\\",\\"0.5\\"]","clobTokenIds":"[\\"e\\",\\"f\\"]"}]}
                ]
                """.utf8)
        }
        defer { TrendingURLProtocol.handler = nil }
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [TrendingURLProtocol.self]
        let service = PolymarketService(session: URLSession(configuration: configuration))

        let results = try await service.trendingMarkets(limit: 8)

        let query = Dictionary(
            uniqueKeysWithValues: URLComponents(url: try XCTUnwrap(requested?.url), resolvingAgainstBaseURL: false)?
                .queryItems?.map { ($0.name, $0.value ?? "") } ?? [])
        XCTAssertEqual(query["order"], "volume24hr")
        XCTAssertEqual(query["ascending"], "false")
        XCTAssertEqual(query["active"], "true")
        XCTAssertEqual(query["closed"], "false")
        XCTAssertEqual(query["limit"], "8")
        XCTAssertEqual(results.map(\.fullSymbol), ["a", "c", "e"])
        XCTAssertEqual(results.first?.eventTitle, "Fed decision")
        XCTAssertEqual(results.first?.pmSeries?.count, 2, "A multi-choice event carries every choice")
        XCTAssertNil(results.last?.pmSeries, "A single-market event is a plain selection")
    }

    // MARK: - View model

    @MainActor
    func testEmptyQueryListsTrendingAndTypingReplacesIt() async throws {
        let source = StubTrendingSource()
        source.results = [market("Cut", event: "Fed"), market("Hold", event: "Fed")]
        source.searchResults = [market("BTC 100k", event: "Bitcoin")]
        let viewModel = PredictionMarketSearchViewModel(offersTrending: true, service: { source })

        viewModel.scheduleSearch(query: "")
        try await waitUntil { viewModel.hasResults }
        XCTAssertTrue(viewModel.isShowingTrending)
        XCTAssertEqual(viewModel.groups.map(\.eventTitle), ["Fed"])

        viewModel.scheduleSearch(query: "bitcoin")
        XCTAssertFalse(viewModel.isShowingTrending)
        try await waitUntil { viewModel.groups.first?.eventTitle == "Bitcoin" }
    }

    @MainActor
    func testTrendingIsCachedWithinItsLifetimeAndRefetchedAfter() async throws {
        let source = StubTrendingSource()
        source.results = [market("Cut", event: "Fed")]
        var clock = Date(timeIntervalSince1970: 1_000)
        let viewModel = PredictionMarketSearchViewModel(offersTrending: true, service: { source }, now: { clock })

        viewModel.scheduleSearch(query: "")
        try await waitUntil { viewModel.hasResults }
        viewModel.scheduleSearch(query: "fed")
        viewModel.scheduleSearch(query: "")
        XCTAssertTrue(viewModel.hasResults, "Served from the cache without waiting")
        XCTAssertEqual(source.trendingCalls, 1)

        clock = clock.addingTimeInterval(Polymarket.trendingCacheTTL + 1)
        viewModel.scheduleSearch(query: "")
        try await waitUntil { source.trendingCalls == 2 }
    }

    @MainActor
    func testProvidersWithoutTrendingStayEmpty() async throws {
        let viewModel = PredictionMarketSearchViewModel(
            provider: .kalshi, offersTrending: true, service: { StubPlainSource() })

        viewModel.scheduleSearch(query: "")

        XCTAssertTrue(viewModel.groups.isEmpty)
        XCTAssertFalse(viewModel.isShowingTrending)
    }

    @MainActor
    func testPanesThatDoNotOfferTrendingStayEmpty() async throws {
        let source = StubTrendingSource()
        source.results = [market("Cut", event: "Fed")]
        let viewModel = PredictionMarketSearchViewModel(service: { source })

        viewModel.scheduleSearch(query: "")

        XCTAssertTrue(viewModel.groups.isEmpty)
        XCTAssertEqual(source.trendingCalls, 0)
    }

    @MainActor
    func testAFailedFetchLeavesNothingAndNoError() async throws {
        let source = StubTrendingSource()
        source.fails = true
        let viewModel = PredictionMarketSearchViewModel(offersTrending: true, service: { source })

        viewModel.scheduleSearch(query: "")
        try await waitUntil { source.trendingCalls == 1 && !viewModel.isSearching }

        XCTAssertTrue(viewModel.groups.isEmpty)
        XCTAssertNil(viewModel.errorMessage)
        XCTAssertFalse(viewModel.isShowingTrending)
    }

    // MARK: - Chips

    func testStringChipsSearchForTheirOwnTitle() {
        var picked: [String] = []
        let grid = SuggestionChipGrid(caption: "Suggestions", items: ["BTC", "ETH"], iconSource: .binance) {
            picked.append($0)
        }

        XCTAssertEqual(grid.items.map(\.query), ["BTC", "ETH"])
        XCTAssertEqual(grid.items.map(\.title), ["BTC", "ETH"])
        XCTAssertEqual(grid.items.first?.icon, .logo(.binance))
        grid.onSelect(grid.items[1])
        XCTAssertEqual(picked, ["ETH"])
    }

    // MARK: - Helpers

    @MainActor
    private func waitUntil(timeout: TimeInterval = 2, _ condition: () -> Bool) async throws {
        let deadline = Date().addingTimeInterval(timeout)
        while !condition() {
            if Date() > deadline { return XCTFail("Timed out waiting for condition") }
            try await Task.sleep(nanoseconds: 10_000_000)
        }
    }
}
