import XCTest

@testable import DegenView

private final class CoinbaseURLProtocol: URLProtocol {
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

final class CoinbaseAPIServiceTests: XCTestCase {
    private let day: TimeInterval = 86_400
    /// Monday 2026-09-21 00:00 UTC — weeks open on Mondays.
    private let monday = Date(timeIntervalSince1970: 1_789_948_800)

    override func tearDown() {
        CoinbaseURLProtocol.handler = nil
        super.tearDown()
    }

    private func makeService() -> CoinbaseAPIService {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [CoinbaseURLProtocol.self]
        return CoinbaseAPIService(session: URLSession(configuration: configuration), requestSpacing: 0)
    }

    /// `[time, low, high, open, close, volume]`, as Coinbase sends it.
    private func row(_ time: Date, open: Double, high: Double, low: Double, close: Double, volume: Double) -> [Any] {
        [Int(time.timeIntervalSince1970), low, high, open, close, volume]
    }

    private func json(_ value: Any) throws -> Data { try JSONSerialization.data(withJSONObject: value) }

    private func query(_ request: URLRequest) -> [String: String] {
        let items = URLComponents(url: request.url!, resolvingAgainstBaseURL: false)?.queryItems ?? []
        return Dictionary(items.map { ($0.name, $0.value ?? "") }, uniquingKeysWith: { $1 })
    }

    // MARK: - Parsing

    func testCandleReadsCoinbaseColumnOrder() throws {
        let candle = try XCTUnwrap(
            CoinbaseAPIService.candle(from: row(monday, open: 10, high: 14, low: 9, close: 12, volume: 2)))

        XCTAssertEqual(candle.openTime, monday)
        XCTAssertEqual(candle.openPrice, 10)
        XCTAssertEqual(candle.highPrice, 14)
        XCTAssertEqual(candle.lowPrice, 9)
        XCTAssertEqual(candle.closePrice, 12)
        XCTAssertEqual(candle.volume, 2)
        // No turnover from Coinbase: volume priced at the OHLC average, 2 × 11.25.
        XCTAssertEqual(candle.quoteVolume, 22.5, accuracy: 1e-9)
    }

    func testProductIDAcceptsSlashesAndBareTickers() {
        XCTAssertEqual(CoinbaseAPIService.productID("btc/usd"), "BTC-USD")
        XCTAssertEqual(CoinbaseAPIService.productID("ETH-EUR"), "ETH-EUR")
        XCTAssertEqual(CoinbaseAPIService.productID("sol"), "SOL-USD")
    }

    func testGranularityCoversNativeAndFoldedIntervals() throws {
        XCTAssertEqual(try XCTUnwrap(CoinbaseGranularity(interval: "1h")).source, 3_600)
        XCTAssertFalse(try XCTUnwrap(CoinbaseGranularity(interval: "1d")).needsAggregation)
        XCTAssertEqual(try XCTUnwrap(CoinbaseGranularity(interval: "1w")).source, 86_400)
        XCTAssertTrue(try XCTUnwrap(CoinbaseGranularity(interval: "1M")).needsAggregation)
        XCTAssertNil(CoinbaseGranularity(interval: "30m"), "Coinbase has no 30-minute candles")
        XCTAssertNil(CoinbaseGranularity(interval: "4h"))
    }

    // MARK: - Folding

    func testWeeklyFoldOpensOnMondayAndKeepsExtremes() throws {
        let plan = try XCTUnwrap(CoinbaseGranularity(interval: "1w"))
        // Sunday before, then Monday–Wednesday.
        let days = (-1...2).map { monday.addingTimeInterval(Double($0) * day) }
        let candles = [
            KlineData(openTime: days[0], openPrice: 1, highPrice: 2, lowPrice: 1, closePrice: 2, volume: 1, quoteVolume: 10),
            KlineData(openTime: days[1], openPrice: 2, highPrice: 9, lowPrice: 2, closePrice: 5, volume: 2, quoteVolume: 20),
            KlineData(openTime: days[2], openPrice: 5, highPrice: 6, lowPrice: 0.5, closePrice: 4, volume: 3, quoteVolume: 30),
            KlineData(openTime: days[3], openPrice: 4, highPrice: 5, lowPrice: 3, closePrice: 7, volume: 4, quoteVolume: 40),
        ]

        let weeks = CoinbaseAPIService.fold(candles, into: plan)

        XCTAssertEqual(weeks.map(\.openTime), [monday.addingTimeInterval(-7 * day), monday])
        let week = weeks[1]
        XCTAssertEqual(week.openPrice, 2)
        XCTAssertEqual(week.highPrice, 9)
        XCTAssertEqual(week.lowPrice, 0.5)
        XCTAssertEqual(week.closePrice, 7)
        XCTAssertEqual(week.volume, 9)
        XCTAssertEqual(week.quoteVolume, 90)
    }

    func testMonthlyFoldSplitsOnTheFirst() throws {
        let plan = try XCTUnwrap(CoinbaseGranularity(interval: "1M"))
        let sep1 = Date(timeIntervalSince1970: 1_788_220_800)  // 2026-09-01 00:00 UTC
        let candles = [-2, -1, 0, 1].map { offset in
            KlineData(
                openTime: sep1.addingTimeInterval(Double(offset) * day), openPrice: 1, highPrice: 1, lowPrice: 1,
                closePrice: 1, volume: 1)
        }

        let months = CoinbaseAPIService.fold(candles, into: plan)

        XCTAssertEqual(months.count, 2)
        XCTAssertEqual(months[1].openTime, sep1)
        XCTAssertEqual(months[0].volume, 2)
        XCTAssertEqual(months[1].volume, 2)
    }

    func testBucketEndHandlesMonthsOfDifferentLengths() throws {
        let plan = try XCTUnwrap(CoinbaseGranularity(interval: "1M"))
        let feb1 = Date(timeIntervalSince1970: 1_769_904_000)  // 2026-02-01 00:00 UTC
        XCTAssertEqual(plan.bucketEnd(after: feb1).timeIntervalSince(feb1), 28 * day)
    }

    // MARK: - Klines

    func testFetchKlinesReturnsOldestFirstAndTrimsToLimit() async throws {
        var requested: [[String: String]] = []
        CoinbaseURLProtocol.handler = { request in
            requested.append(self.query(request))
            // Newest first, as Coinbase sends them.
            let rows = (0..<5).map { i in
                self.row(self.monday.addingTimeInterval(Double(4 - i) * 3_600), open: 1, high: 2, low: 1, close: 2, volume: 1)
            }
            return (200, try self.json(rows))
        }

        let candles = try await makeService().fetchKlines(symbol: "BTC-USD", interval: "1h", limit: 3)

        XCTAssertEqual(candles.count, 3)
        XCTAssertEqual(candles.map(\.openTime), candles.map(\.openTime).sorted())
        XCTAssertEqual(candles.last?.openTime, monday.addingTimeInterval(4 * 3_600))
        XCTAssertEqual(requested.first?["granularity"], "3600")
    }

    func testUnsupportedIntervalThrowsWithoutARequest() async {
        CoinbaseURLProtocol.handler = { _ in
            XCTFail("No request expected")
            return (200, Data("[]".utf8))
        }
        do {
            _ = try await makeService().fetchKlines(symbol: "BTC-USD", interval: "30m", limit: 10)
            XCTFail("Expected an error")
        } catch let error as CoinbaseAPIError {
            guard case .unsupportedInterval = error else { return XCTFail("\(error)") }
        } catch {
            XCTFail("\(error)")
        }
    }

    func testRateLimitMapsTo429Error() async {
        CoinbaseURLProtocol.handler = { _ in (429, Data("{}".utf8)) }
        do {
            _ = try await makeService().fetchKlines(symbol: "BTC-USD", interval: "1h", limit: 10)
            XCTFail("Expected an error")
        } catch let error as CoinbaseAPIError {
            guard case .rateLimited = error else { return XCTFail("\(error)") }
        } catch {
            XCTFail("\(error)")
        }
    }

    func testDeepHistoryPagesBackwardAndStopsWhenListingStarts() async throws {
        let newest = monday
        var windowed = 0
        var latest = 0
        CoinbaseURLProtocol.handler = { request in
            let q = self.query(request)
            guard q["start"] != nil else {
                latest += 1
                let rows = (0..<350).map { i in
                    self.row(newest.addingTimeInterval(-Double(i) * self.day), open: 1, high: 2, low: 1, close: 2, volume: 1)
                }
                return (200, try self.json(rows))
            }
            windowed += 1
            // One older window holds 100 more days; the one before it is empty: the listing began.
            guard windowed == 1 else { return (200, try self.json([Any]())) }
            let rows = (0..<100).map { i in
                self.row(
                    newest.addingTimeInterval(-Double(350 + i) * self.day), open: 1, high: 2, low: 1, close: 2, volume: 1)
            }
            return (200, try self.json(rows))
        }
        let service = makeService()

        let first = try await service.fetchKlines(symbol: "NEW-USD", interval: "1d", limit: 1_000)
        XCTAssertEqual(first.count, 450)
        XCTAssertEqual(windowed, 2)

        // A refresh wants the same depth again. The listing's start is known, so only the newest page is fetched.
        _ = try await service.fetchKlines(symbol: "NEW-USD", interval: "1d", limit: 1_000)
        XCTAssertEqual(windowed, 2, "A young listing must not be walked back to its first candle on every refresh")
        XCTAssertEqual(latest, 2)
    }

    func testWeeklyChartIsBuiltFromDailyCandles() async throws {
        var granularities: [String] = []
        CoinbaseURLProtocol.handler = { request in
            let q = self.query(request)
            granularities.append(q["granularity"] ?? "")
            // Mon 21st … Wed 30th is 10 days; newest first.
            let rows = (0..<10).map { i in
                self.row(
                    self.monday.addingTimeInterval(Double(9 - i) * self.day), open: Double(10 - i), high: 20, low: 1,
                    close: 5, volume: 1)
            }
            return (200, try self.json(rows))
        }

        let weeks = try await makeService().fetchKlines(symbol: "BTC-USD", interval: "1w", limit: 2)

        XCTAssertEqual(Set(granularities), ["86400"])
        XCTAssertEqual(weeks.count, 2)
        XCTAssertEqual(weeks[1].openTime, monday.addingTimeInterval(7 * day))
        XCTAssertEqual(weeks[0].volume, 7)
        XCTAssertEqual(weeks[1].volume, 3)
    }

    // MARK: - Search

    private var catalogue: [[String: Any]] {
        func product(_ base: String, _ quote: String, status: String = "online", disabled: Bool = false) -> [String: Any] {
            [
                "id": "\(base)-\(quote)", "base_currency": base, "quote_currency": quote,
                "status": status, "trading_disabled": disabled,
            ]
        }
        return [
            product("BTC", "EUR"), product("BTC", "USD"), product("BTC", "USDT"),
            product("BTC", "GBP", status: "delisted"), product("BTC", "JPY", disabled: true),
            product("ETH", "BTC"), product("ETH", "USD"),
        ]
    }

    func testSearchSkipsDelistedAndDisabledAndListsDollarPairsFirst() async throws {
        CoinbaseURLProtocol.handler = { _ in (200, try self.json(self.catalogue)) }

        let results = try await makeService().searchTickers(query: "btc")

        XCTAssertEqual(results.map(\.fullSymbol).prefix(2), ["BTC-USD", "BTC-USDT"], "USD leads its USDT twin")
        XCTAssertFalse(results.contains { $0.fullSymbol == "BTC-GBP" }, "delisted")
        XCTAssertFalse(results.contains { $0.fullSymbol == "BTC-JPY" }, "trading disabled")
        XCTAssertTrue(results.allSatisfy { $0.source == .coinbase })
        XCTAssertEqual(results.first?.symbol, "BTC/USD")
    }

    func testSearchMatchesPairsWrittenWithoutSeparator() async throws {
        CoinbaseURLProtocol.handler = { _ in (200, try self.json(self.catalogue)) }
        let service = makeService()

        let hyphenless = try await service.searchTickers(query: "ETHUSD")
        XCTAssertEqual(hyphenless.map(\.fullSymbol), ["ETH-USD"])

        let exact = try await service.searchTickers(query: "BTC-EUR")
        XCTAssertEqual(exact.first?.fullSymbol, "BTC-EUR")
    }

    func testSearchFetchesTheCatalogueOnce() async throws {
        var calls = 0
        CoinbaseURLProtocol.handler = { _ in
            calls += 1
            return (200, try self.json(self.catalogue))
        }
        let service = makeService()

        _ = try await service.searchTickers(query: "btc")
        _ = try await service.searchTickers(query: "eth")

        XCTAssertEqual(calls, 1)
    }

    // MARK: - Replay

    func testReplayIntervalsExcludeThirtyMinutesAndAnythingCoarserThanTheChart() {
        let options = makeService().supportedReplayIntervals(chartInterval: "1h")

        XCTAssertTrue(options.contains(.oneMinute))
        XCTAssertTrue(options.contains(.fifteenMinutes))
        XCTAssertTrue(options.contains(.oneHour))
        XCTAssertFalse(options.contains(.thirtyMinutes))
        XCTAssertFalse(options.contains(.oneDay))
        XCTAssertTrue(options.contains(.automatic) && options.contains(.chartBar))
    }

    func testReplayWalksThroughWindowsWithinTheRequestCap() async throws {
        var spans: [TimeInterval] = []
        CoinbaseURLProtocol.handler = { request in
            let q = self.query(request)
            let formatter = ISO8601DateFormatter()
            if let s = q["start"].flatMap(formatter.date), let e = q["end"].flatMap(formatter.date) {
                spans.append(e.timeIntervalSince(s))
            }
            return (200, try self.json([Any]()))
        }

        // 12 hours of minutes is 720 buckets — more than one request may span.
        _ = try await makeService().fetchReplayKlines(
            symbol: "BTC-USD", interval: .oneMinute, start: monday, end: monday.addingTimeInterval(12 * 3_600))

        XCTAssertGreaterThan(spans.count, 1)
        XCTAssertTrue(spans.allSatisfy { $0 <= Double(300 * 60) }, "Coinbase answers 400 above 300 buckets")
    }
}
