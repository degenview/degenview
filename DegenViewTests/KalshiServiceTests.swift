import XCTest

@testable import DegenView

private final class KalshiURLProtocol: URLProtocol {
    static var handler: ((URLRequest) throws -> (HTTPURLResponse, Data))?

    override class func canInit(with request: URLRequest) -> Bool { true }

    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        do {
            guard let handler = Self.handler else { throw URLError(.badServerResponse) }
            let (response, data) = try handler(request)
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: data)
            client?.urlProtocolDidFinishLoading(self)
        } catch {
            client?.urlProtocol(self, didFailWithError: error)
        }
    }

    override func stopLoading() {}
}

final class KalshiServiceTests: XCTestCase {
    private let fixedNow = Date(timeIntervalSince1970: 1_800_000_000)
    private var directory: URL!

    override func setUpWithError() throws {
        directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("kalshi-tests-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    }

    override func tearDown() {
        KalshiURLProtocol.handler = nil
        try? FileManager.default.removeItem(at: directory)
    }

    // MARK: - Helpers

    private func makeService() -> KalshiService {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [KalshiURLProtocol.self]
        let session = URLSession(configuration: configuration)
        let fixedNow = fixedNow
        return KalshiService(
            session: session,
            cache: KlineCache(),
            index: KalshiSeriesIndex(session: session, directory: directory, now: { fixedNow }),
            now: { fixedNow }
        )
    }

    private func respond(_ request: URLRequest, status: Int = 200, json: Any) throws -> (HTTPURLResponse, Data) {
        let url = try XCTUnwrap(request.url)
        let response = try XCTUnwrap(
            HTTPURLResponse(url: url, statusCode: status, httpVersion: nil, headerFields: nil))
        return (response, try JSONSerialization.data(withJSONObject: json))
    }

    private func query(_ request: URLRequest) -> [String: String] {
        let components = URLComponents(url: request.url!, resolvingAgainstBaseURL: false)
        return Dictionary(
            uniqueKeysWithValues: (components?.queryItems ?? []).map { ($0.name, $0.value ?? "") })
    }

    private func market(
        _ ticker: String, label: String, title: String? = nil, ask: String = "0.5000",
        last: String = "0.4800", status: String = "active"
    ) -> [String: Any] {
        [
            "ticker": ticker, "title": title ?? "Will \(label) happen?", "yes_sub_title": label,
            "status": status, "yes_ask_dollars": ask, "yes_bid_dollars": "0.4000",
            "last_price_dollars": last,
        ]
    }

    // MARK: - Models

    func testMarketIDRoundTripsAndRejectsMalformed() {
        let id = KalshiMarketID(series: "KXFED", market: "KXFED-26OCT-H0")
        XCTAssertEqual(id.raw, "KXFED/KXFED-26OCT-H0")
        XCTAssertEqual(KalshiMarketID(id.raw), id)
        XCTAssertNil(KalshiMarketID("NOSLASH"))
        XCTAssertNil(KalshiMarketID("/MARKET"))
        XCTAssertNil(KalshiMarketID("SERIES/"))
    }

    func testDisplayedPricePrefersAskAndIgnoresEmptyBook() throws {
        let priced = try KalshiJSON.decoder().decode(
            KalshiMarket.self,
            from: JSONSerialization.data(withJSONObject: market("A", label: "A", ask: "0.6100", last: "0.5000")))
        XCTAssertEqual(priced.displayedYesPrice, 0.61)

        // An empty book reports an ask of 0.0000 — that must not chart as 0%.
        let empty = try KalshiJSON.decoder().decode(
            KalshiMarket.self,
            from: JSONSerialization.data(withJSONObject: market("B", label: "B", ask: "0.0000", last: "0.5000")))
        XCTAssertEqual(empty.displayedYesPrice, 0.5)
    }

    func testProbabilityRejectsOutOfRangeAndGarbage() {
        XCTAssertEqual(KalshiJSON.probability("0.5600"), 0.56)
        XCTAssertNil(KalshiJSON.probability("1.5"))
        XCTAssertNil(KalshiJSON.probability("abc"))
        XCTAssertNil(KalshiJSON.probability(nil))
    }

    func testSeriesDecodeToleratesMissingTitle() throws {
        let data = try JSONSerialization.data(withJSONObject: [
            "series": [["ticker": "A", "title": "Alpha", "tags": NSNull()], ["ticker": "B"]]
        ])
        let list = try KalshiJSON.decoder().decode(KalshiSeriesList.self, from: data)
        XCTAssertEqual(list.series?.map(\.ticker), ["A", "B"])
        XCTAssertEqual(list.series?[1].title, "")
    }

    // MARK: - Candles

    func testKlinesForwardFillNullClosesAndDropLeadingGaps() {
        func candle(_ ts: Double, close: String?, previous: String?) -> KalshiCandle {
            KalshiCandle(
                endPeriodTs: ts, price: KalshiCandlePrice(closeDollars: close, previousDollars: previous))
        }
        let points = KalshiService.klines(from: [
            candle(300, close: "0.40", previous: "0.30"),
            candle(100, close: nil, previous: nil),  // before any trade — dropped
            candle(200, close: "0.30", previous: nil),
            candle(400, close: nil, previous: "0.40"),  // no trades — carries previous
            candle(500, close: nil, previous: nil),  // no info at all — carries last known
        ])

        XCTAssertEqual(points.map(\.closePrice), [0.30, 0.40, 0.40, 0.40])
        XCTAssertEqual(points.map { $0.openTime.timeIntervalSince1970 }, [200, 300, 400, 500])
    }

    func testTimeRangeWindowsStayUnderTheCandleCap() {
        for range in TimeRange.allCases {
            let window = range.kalshiWindow
            XCTAssertTrue([1, 60, 1_440].contains(window.periodMinutes))
            let candles = window.spanSeconds / 60 / Double(window.periodMinutes)
            XCTAssertLessThan(candles, 5_000, "\(range) would exceed Kalshi's candle cap")
        }
    }

    // MARK: - Series ranking

    func testRankRequiresEveryWordAndPrefersTitleStarts() {
        let series = [
            KalshiSeries(ticker: "KXLONG", title: "Will the federal gas tax be suspended?"),
            KalshiSeries(ticker: "KXFED", title: "Fed meeting"),
            KalshiSeries(ticker: "KXFEDCHAIR", title: "Fed Chair pick", category: "Politics"),
            KalshiSeries(ticker: "KXBTC", title: "Bitcoin price"),
        ]

        let ranked = KalshiSeriesIndex.rank(series, query: "fed", limit: 10)
        XCTAssertEqual(ranked.map(\.ticker), ["KXFED", "KXFEDCHAIR", "KXLONG"])

        XCTAssertEqual(KalshiSeriesIndex.rank(series, query: "fed chair", limit: 10).map(\.ticker), ["KXFEDCHAIR"])
        XCTAssertEqual(KalshiSeriesIndex.rank(series, query: "fed", limit: 1).count, 1)
        XCTAssertTrue(KalshiSeriesIndex.rank(series, query: "   ", limit: 10).isEmpty)
        XCTAssertEqual(KalshiSeriesIndex.rank(series, query: "politics", limit: 10).map(\.ticker), ["KXFEDCHAIR"])
    }

    // MARK: - Search results

    func testMultiMarketEventCarriesEveryChoiceOnEveryRow() throws {
        let event = try decodeEvent(
            title: "Fed decision in Oct 2026?",
            markets: [
                market("KXFED-26OCT-H0", label: "Hold", ask: "0.6100"),
                market("KXFED-26OCT-C25", label: "Cut 25", ask: "0.3000"),
                market("KXFED-26OCT-X", label: "Closed", status: "closed"),
            ])

        let results = KalshiService.results(
            for: event, series: KalshiSeries(ticker: "KXFED", title: "Fed meeting"))

        XCTAssertEqual(results.map(\.symbol), ["Hold", "Cut 25"])
        XCTAssertEqual(results.map(\.fullSymbol), ["KXFED/KXFED-26OCT-H0", "KXFED/KXFED-26OCT-C25"])
        XCTAssertEqual(results.map(\.price), [0.61, 0.30])
        XCTAssertTrue(results.allSatisfy { $0.source == .kalshi })
        XCTAssertEqual(results[0].eventTitle, "Fed decision in Oct 2026?")
        XCTAssertNil(results[0].imageURL)
        XCTAssertEqual(results[1].pmSeries?.map(\.label), ["Hold", "Cut 25"])
        XCTAssertEqual(results[1].pmSeries?.map(\.tokenID), results.map(\.fullSymbol))
    }

    func testSingleMarketEventLabelsItselfWithTheFullQuestion() throws {
        let event = try decodeEvent(
            title: "Mars?",
            markets: [market("KXMARS-99", label: "Mars", title: "Will Elon visit Mars?")])

        let results = KalshiService.results(for: event, series: KalshiSeries(ticker: "KXMARS", title: "Mars"))

        XCTAssertEqual(results.map(\.symbol), ["Will Elon visit Mars?"])
        XCTAssertNil(results[0].pmSeries)
    }

    func testSearchFansOutToMatchedSeriesAndDownloadsTheListOnce() async throws {
        let lock = NSLock()
        var seriesCalls = 0
        var eventQueries: [[String: String]] = []

        KalshiURLProtocol.handler = { [self] request in
            let path = request.url?.path ?? ""
            if path.hasSuffix("/series") {
                lock.lock()
                seriesCalls += 1
                lock.unlock()
                return try respond(
                    request,
                    json: [
                        "series": [
                            ["ticker": "KXFED", "title": "Fed meeting"],
                            ["ticker": "KXBTC", "title": "Bitcoin price"],
                        ]
                    ])
            }
            XCTAssertTrue(path.hasSuffix("/events"))
            lock.lock()
            eventQueries.append(query(request))
            lock.unlock()
            return try respond(
                request,
                json: [
                    "events": [
                        [
                            "event_ticker": "KXFED-26OCT", "series_ticker": "KXFED",
                            "title": "Fed decision in Oct 2026?",
                            "markets": [
                                market("KXFED-26OCT-H0", label: "Hold"),
                                market("KXFED-26OCT-C25", label: "Cut 25"),
                            ],
                        ]
                    ]
                ])
        }

        let service = makeService()
        let first = try await service.searchTickers(query: "fed")
        _ = try await service.searchTickers(query: "fed")

        XCTAssertEqual(first.map(\.symbol), ["Hold", "Cut 25"])
        XCTAssertEqual(seriesCalls, 1)
        XCTAssertEqual(eventQueries.count, 2)
        XCTAssertEqual(eventQueries[0]["series_ticker"], "KXFED")
        XCTAssertEqual(eventQueries[0]["status"], "open")
        XCTAssertEqual(eventQueries[0]["with_nested_markets"], "true")
    }

    func testSearchWithNoMatchingSeriesMakesNoEventRequests() async throws {
        KalshiURLProtocol.handler = { [self] request in
            XCTAssertTrue(request.url?.path.hasSuffix("/series") == true)
            return try respond(request, json: ["series": [["ticker": "KXBTC", "title": "Bitcoin price"]]])
        }

        let results = try await makeService().searchTickers(query: "zzz")
        XCTAssertTrue(results.isEmpty)
    }

    func testRateLimitedSeriesDownloadSurfacesAsAnError() async {
        KalshiURLProtocol.handler = { [self] request in
            try respond(request, status: 429, json: [:])
        }

        do {
            _ = try await makeService().searchTickers(query: "fed")
            XCTFail("expected a rate-limit error")
        } catch let error as KalshiError {
            guard case .rateLimited = error else { return XCTFail("wrong error: \(error)") }
        } catch {
            XCTFail("wrong error: \(error)")
        }
    }

    // MARK: - Price history

    func testFetchPricesRequestsCandlesAndOverlaysTheLiveAsk() async throws {
        let lock = NSLock()
        var candleRequest: URLRequest?
        var askPolicy: URLRequest.CachePolicy?

        KalshiURLProtocol.handler = { [self] request in
            let path = request.url?.path ?? ""
            if path.hasSuffix("/candlesticks") {
                lock.lock()
                candleRequest = request
                lock.unlock()
                return try respond(
                    request,
                    json: [
                        "candlesticks": [
                            ["end_period_ts": 1_799_990_000, "price": ["close_dollars": "0.4000"]],
                            ["end_period_ts": 1_799_993_600, "price": ["close_dollars": "0.4500"]],
                        ]
                    ])
            }
            lock.lock()
            askPolicy = request.cachePolicy
            lock.unlock()
            return try respond(request, json: ["market": market("KXFED-26OCT-H0", label: "Hold", ask: "0.6100")])
        }

        let data = try await makeService().fetchPrices(
            marketID: "KXFED/KXFED-26OCT-H0", range: .oneDay, count: 10)

        XCTAssertEqual(data.map(\.closePrice), [0.40, 0.61])
        XCTAssertEqual(askPolicy, .reloadIgnoringLocalCacheData)

        let request = try XCTUnwrap(candleRequest)
        XCTAssertEqual(request.url?.path, "/trade-api/v2/series/KXFED/markets/KXFED-26OCT-H0/candlesticks")
        let window = TimeRange.oneDay.kalshiWindow
        XCTAssertEqual(query(request)["period_interval"], String(window.periodMinutes))
        XCTAssertEqual(query(request)["end_ts"], "1800000000")
        XCTAssertEqual(query(request)["start_ts"], String(1_800_000_000 - Int(window.spanSeconds)))
    }

    func testFetchPricesKeepsHistoryWhenTheAskLookupFails() async throws {
        KalshiURLProtocol.handler = { [self] request in
            if request.url?.path.hasSuffix("/candlesticks") == true {
                return try respond(
                    request,
                    json: ["candlesticks": [["end_period_ts": 100, "price": ["close_dollars": "0.3000"]]]])
            }
            return try respond(request, status: 500, json: [:])
        }

        let data = try await makeService().fetchPrices(marketID: "S/M", range: .oneDay, count: 10)
        XCTAssertEqual(data.map(\.closePrice), [0.30])
    }

    func testEmptyCandlesThrowNoHistory() async {
        KalshiURLProtocol.handler = { [self] request in
            try respond(request, json: ["candlesticks": []])
        }

        do {
            _ = try await makeService().fetchPrices(marketID: "S/M", range: .oneDay, count: 10)
            XCTFail("expected noHistory")
        } catch let error as KalshiError {
            guard case .noHistory = error else { return XCTFail("wrong error: \(error)") }
        } catch {
            XCTFail("wrong error: \(error)")
        }
    }

    func testMalformedMarketIDThrowsBeforeAnyRequest() async {
        KalshiURLProtocol.handler = { _ in
            XCTFail("no request expected")
            throw URLError(.badURL)
        }

        do {
            _ = try await makeService().fetchPrices(marketID: "not-a-pair", range: .oneDay, count: 10)
            XCTFail("expected invalidMarketID")
        } catch let error as KalshiError {
            guard case .invalidMarketID = error else { return XCTFail("wrong error: \(error)") }
        } catch {
            XCTFail("wrong error: \(error)")
        }
    }

    // MARK: - Chart integration

    @MainActor
    func testChartViewModelRefreshPlotsKalshiSeriesAsALine() async throws {
        KalshiURLProtocol.handler = { [self] request in
            let path = request.url?.path ?? ""
            if path.hasSuffix("/candlesticks") {
                let isHold = path.contains("H0")
                return try respond(
                    request,
                    json: [
                        "candlesticks": [
                            ["end_period_ts": 100, "price": ["close_dollars": isHold ? "0.5500" : "0.3500"]],
                            ["end_period_ts": 200, "price": ["close_dollars": isHold ? "0.5500" : "0.3500"]],
                        ]
                    ])
            }
            let isHold = path.contains("H0")
            return try respond(
                request,
                json: [
                    "market": market(
                        isHold ? "KXFED-H0" : "KXFED-C25", label: "x", ask: isHold ? "0.6000" : "0.4000")
                ])
        }

        let hold = "KXFED/KXFED-H0"
        let cut = "KXFED/KXFED-C25"
        let viewModel = ChartViewModel(
            ticker: hold, source: .kalshi, displayName: "Fed decision", api: makeService())
        viewModel.pmSeries = [
            PmSeriesConfig(tokenID: hold, label: "Hold", enabled: true),
            PmSeriesConfig(tokenID: cut, label: "Cut 25", enabled: true),
        ]

        await viewModel.fetchData(for: .oneDay, count: 2)

        XCTAssertTrue(viewModel.usesLineChart)
        XCTAssertEqual(viewModel.priceScale, .probability)
        XCTAssertEqual(viewModel.title, "Fed decision")
        XCTAssertEqual(viewModel.leadingMarketChoice?.label, "Hold")
        XCTAssertEqual(viewModel.leadingMarketChoice?.price, 0.60)
        XCTAssertNil(viewModel.errorMessage)
    }

    func testKalshiIsAPredictionMarketThatCannotBeConfusedWithPolymarket() {
        XCTAssertTrue(DataSourceType.kalshi.isPredictionMarket)
        XCTAssertEqual(DataSourceType.kalshi.rawValue, "Kalshi")
        XCTAssertEqual(DataSourceType.predictionMarkets, [.polymarket, .kalshi])
        XCTAssertFalse(DataSourceType.cryptoSources.contains(.kalshi))
        XCTAssertTrue(DataSourceFactory.shared.service(for: .kalshi) is KalshiService)
    }

    // MARK: - Fixtures

    private func decodeEvent(title: String, markets: [[String: Any]]) throws -> KalshiEvent {
        let data = try JSONSerialization.data(withJSONObject: [
            "event_ticker": "EVT", "series_ticker": "KXFED", "title": title, "markets": markets,
        ])
        return try KalshiJSON.decoder().decode(KalshiEvent.self, from: data)
    }
}
