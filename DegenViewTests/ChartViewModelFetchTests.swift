import XCTest

@testable import DegenView

private final class PolymarketURLProtocol: URLProtocol {
    static var handler: ((URLRequest) throws -> (HTTPURLResponse, Data))?

    override class func canInit(with request: URLRequest) -> Bool { true }

    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        do {
            guard let handler = Self.handler else {
                throw URLError(.badServerResponse)
            }
            let (response, data) = try handler(request)
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .allowed)
            client?.urlProtocol(self, didLoad: data)
            client?.urlProtocolDidFinishLoading(self)
        } catch {
            client?.urlProtocol(self, didFailWithError: error)
        }
    }

    override func stopLoading() {}
}

final class ChartViewModelFetchTests: XCTestCase {
    actor SequencedSource: TickerDataSource {
        nonisolated let type = DataSourceType.binance
        private var call = 0

        func fetchKlines(symbol: String, interval: String, limit: Int) async throws -> [KlineData] {
            call += 1
            let current = call
            try await Task.sleep(for: current == 1 ? .milliseconds(150) : .milliseconds(5))
            return [KlineData(time: Date(timeIntervalSince1970: Double(current)), price: Double(current))]
        }

        func searchTickers(query: String) async throws -> [TickerSearchResult] { [] }
    }

    @MainActor
    func testFetchDataWaitsForSourceCompletion() async {
        let source = SequencedSource()
        let viewModel = ChartViewModel(ticker: "BTC", api: source)
        let started = ContinuousClock.now

        await viewModel.fetchData(for: .oneDay, count: 1)

        XCTAssertGreaterThanOrEqual(started.duration(to: .now), .milliseconds(100))
        XCTAssertEqual(viewModel.currentPrice, 1)
    }

    @MainActor
    func testNewerFetchCancelsAndSupersedesOlderFetch() async {
        let source = SequencedSource()
        let viewModel = ChartViewModel(ticker: "BTC", api: source)
        let first = Task { @MainActor in await viewModel.fetchData(for: .oneDay, count: 1) }
        try? await Task.sleep(for: .milliseconds(20))

        await viewModel.fetchData(for: .oneWeek, count: 1)
        await first.value

        XCTAssertEqual(viewModel.currentPrice, 2)
        XCTAssertNil(viewModel.errorMessage)
    }

    @MainActor
    func testReplacingPolymarketTickerClearsOldSeriesAndPlottedData() {
        let viewModel = ChartViewModel(ticker: "old", source: .polymarket)
        viewModel.pmSeries = [
            PmSeriesConfig(tokenID: "old-a", label: "Old A", enabled: true),
            PmSeriesConfig(tokenID: "old-b", label: "Old B", enabled: true),
        ]
        viewModel.klineData = [KlineData(time: .now, price: 0.75)]
        viewModel.currentPrice = 0.75

        viewModel.updateTicker(
            symbol: "new", source: .polymarket, displayName: "New Market", pmSeries: nil)

        XCTAssertEqual(viewModel.ticker, "new")
        XCTAssertEqual(viewModel.title, "New Market")
        XCTAssertTrue(viewModel.pmSeries.isEmpty)
        XCTAssertTrue(viewModel.klineData.isEmpty)
        XCTAssertNil(viewModel.currentPrice)
    }

    @MainActor
    func testSinglePolymarketChoiceDrivesSubtitleWithoutReplacingEventTitle() {
        let viewModel = ChartViewModel(
            ticker: "bird", source: .polymarket, displayName: "AGT Winner")
        viewModel.pmSeries = [
            PmSeriesConfig(tokenID: "bird", label: "Bird & Byron", enabled: true)
        ]
        viewModel.currentPrice = 0.51

        XCTAssertEqual(viewModel.title, "AGT Winner")
        XCTAssertEqual(viewModel.leadingPolymarketChoice?.label, "Bird & Byron")
        XCTAssertEqual(viewModel.leadingPolymarketChoice?.price, 0.51)
    }

    @MainActor
    func testPolymarketRefreshUpdatesVisibleLeaderFromUncachedPriceSnapshot() async {
        let tokenA = "choice-a-\(UUID().uuidString)"
        let tokenB = "choice-b-\(UUID().uuidString)"
        let lock = NSLock()
        var priceCalls: [String: Int] = [:]
        var currentPricePolicies: [URLRequest.CachePolicy] = []

        PolymarketURLProtocol.handler = { request in
            let components = URLComponents(url: try XCTUnwrap(request.url), resolvingAgainstBaseURL: false)
            let query = Dictionary(
                uniqueKeysWithValues: (components?.queryItems ?? []).map {
                    ($0.name, $0.value ?? "")
                })
            let data: Data
            if components?.path == "/prices-history" {
                let initial = query["market"] == tokenA ? 0.55 : 0.45
                data = try JSONSerialization.data(withJSONObject: [
                    "history": [
                        ["t": 100, "p": initial],
                        ["t": 200, "p": initial],
                    ]
                ])
            } else {
                let token = try XCTUnwrap(query["token_id"])
                lock.lock()
                let call = priceCalls[token, default: 0]
                priceCalls[token] = call + 1
                currentPricePolicies.append(request.cachePolicy)
                lock.unlock()
                let prices = token == tokenA ? [0.60, 0.40] : [0.40, 0.70]
                data = try JSONSerialization.data(withJSONObject: ["price": String(prices[call])])
            }
            let response = try XCTUnwrap(
                HTTPURLResponse(
                    url: try XCTUnwrap(request.url),
                    statusCode: 200,
                    httpVersion: nil,
                    headerFields: ["Cache-Control": "max-age=3600"]
                ))
            return (response, data)
        }
        defer { PolymarketURLProtocol.handler = nil }

        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [PolymarketURLProtocol.self]
        let service = PolymarketService(session: URLSession(configuration: configuration))
        let viewModel = ChartViewModel(
            ticker: tokenA,
            source: .polymarket,
            displayName: "Event title",
            api: service
        )
        viewModel.pmSeries = [
            PmSeriesConfig(tokenID: tokenA, label: "Choice A", enabled: true),
            PmSeriesConfig(tokenID: tokenB, label: "Choice B", enabled: true),
        ]

        await viewModel.fetchData(for: .oneDay, count: 2)
        XCTAssertEqual(viewModel.leadingPolymarketChoice?.label, "Choice A")
        XCTAssertEqual(viewModel.leadingPolymarketChoice?.price, 0.60)

        await viewModel.fetchData(for: .oneDay, count: 2, silent: true)
        XCTAssertEqual(viewModel.title, "Event title")
        XCTAssertEqual(viewModel.leadingPolymarketChoice?.label, "Choice B")
        XCTAssertEqual(viewModel.leadingPolymarketChoice?.price, 0.70)
        XCTAssertEqual(currentPricePolicies.count, 4)
        XCTAssertTrue(currentPricePolicies.allSatisfy { $0 == .reloadIgnoringLocalCacheData })

        viewModel.togglePmSeries(tokenB)
        XCTAssertEqual(viewModel.leadingPolymarketChoice?.label, "Choice A")
        XCTAssertEqual(viewModel.leadingPolymarketChoice?.price, 0.40)

        viewModel.togglePmSeries(tokenB)
        viewModel.applyReplayTimestamp(Date(timeIntervalSince1970: 100))
        XCTAssertEqual(viewModel.leadingPolymarketChoice?.label, "Choice A")
        XCTAssertEqual(viewModel.leadingPolymarketChoice?.price, 0.55)
    }
}
