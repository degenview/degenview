import XCTest

@testable import DegenView

private final class KlinesURLProtocol: URLProtocol {
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

/// 3M and 1Y name the size of one candle: calendar quarters and calendar years.
final class QuarterlyYearlyCandleTests: XCTestCase {
    private func date(_ iso: String) -> Date {
        ISO8601DateFormatter().date(from: "\(iso)T00:00:00Z")!
    }

    private func candle(
        _ iso: String, open: Double = 1, high: Double = 2, low: Double = 0.5, close: Double = 1.5, volume: Double = 1
    )
        -> KlineData
    {
        KlineData(
            openTime: date(iso), openPrice: open, highPrice: high, lowPrice: low, closePrice: close, volume: volume,
            quoteVolume: volume * 10)
    }

    /// One candle per month over `months`, e.g. from 2025-01 for 21 months.
    private func monthly(from year: Int, month: Int, count: Int) -> [KlineData] {
        (0..<count).map { offset in
            let index = month - 1 + offset
            let iso = String(format: "%04d-%02d-01", year + index / 12, index % 12 + 1)
            return candle(iso, open: Double(offset + 1), close: Double(offset + 1))
        }
    }

    // MARK: - Timeframes

    func testTimeframesAreCandleSizes() {
        XCTAssertEqual(TimeRange.threeMonths.binanceInterval, "3M")
        XCTAssertEqual(TimeRange.oneYear.binanceInterval, "1Y")
        XCTAssertEqual(TimeRange.threeMonths.binanceIntervalSeconds, 7_776_000)
        XCTAssertEqual(TimeRange.oneYear.binanceIntervalSeconds, 31_536_000)
        XCTAssertEqual(
            Set(TimeRange.allCases.map(\.binanceInterval)).count, TimeRange.allCases.count, "tokens are unique")
    }

    func testDefaultCandleCountsFitTheHistoryThatExists() {
        XCTAssertEqual(TimeRange.threeMonths.dataPointLimit, 16)
        XCTAssertEqual(TimeRange.oneYear.dataPointLimit, 10)
    }

    func testPredictionMarketLineChartsKeepTheirOldWindows() {
        XCTAssertEqual(TimeRange.threeMonths.effectiveSpanDays, 90)
        XCTAssertEqual(TimeRange.oneYear.effectiveSpanDays, 364)
        XCTAssertEqual(TimeRange.threeMonths.lineChartPointCount, 90)
        XCTAssertEqual(TimeRange.oneYear.lineChartPointCount, 52)
        XCTAssertEqual(TimeRange.oneDay.effectiveSpanDays, 60)
        XCTAssertEqual(TimeRange.oneMonth.effectiveSpanDays, 360)
    }

    // MARK: - Calendar buckets

    func testQuartersOpenOnJanAprJulOct() {
        let quarter = TimeRange.threeMonths.binanceIntervalSeconds
        XCTAssertEqual(KlineData.bucketStart(of: date("2026-01-01"), interval: quarter), date("2026-01-01"))
        XCTAssertEqual(KlineData.bucketStart(of: date("2026-03-31"), interval: quarter), date("2026-01-01"))
        XCTAssertEqual(KlineData.bucketStart(of: date("2026-04-01"), interval: quarter), date("2026-04-01"))
        XCTAssertEqual(KlineData.bucketStart(of: date("2026-08-15"), interval: quarter), date("2026-07-01"))
        XCTAssertEqual(KlineData.bucketStart(of: date("2026-12-31"), interval: quarter), date("2026-10-01"))
    }

    func testYearsOpenOnJanuaryFirst() {
        let year = TimeRange.oneYear.binanceIntervalSeconds
        XCTAssertEqual(KlineData.bucketStart(of: date("2026-09-30"), interval: year), date("2026-01-01"))
        XCTAssertEqual(KlineData.bucketStart(of: date("2024-12-31"), interval: year), date("2024-01-01"), "leap year")
    }

    func testMonthsWeeksAndDaysAreNotDisturbed() {
        XCTAssertEqual(KlineData.bucketStart(of: date("2026-08-15"), interval: 2_592_000), date("2026-08-01"))
        XCTAssertEqual(KlineData.bucketStart(of: date("2026-09-30"), interval: 604_800), date("2026-09-28"))
        XCTAssertEqual(KlineData.bucketStart(of: date("2026-09-30"), interval: 86_400), date("2026-09-30"))
    }

    func testFoldingMonthsIntoQuarters() {
        let quarters = monthly(from: 2025, month: 1, count: 7).folded(into: 7_776_000)

        XCTAssertEqual(quarters.map(\.openTime), [date("2025-01-01"), date("2025-04-01"), date("2025-07-01")])
        XCTAssertEqual(quarters[0].openPrice, 1)
        XCTAssertEqual(quarters[0].closePrice, 3, "close of the last month in the quarter")
        XCTAssertEqual(quarters[0].volume, 3)
        XCTAssertEqual(quarters[0].quoteVolume, 30)
        XCTAssertEqual(quarters[2].volume, 1, "a partial quarter keeps what there is")
    }

    func testFoldingKeepsExtremesAcrossTheBucket() {
        let candles = [
            candle("2026-01-01", high: 5, low: 4),
            candle("2026-02-01", high: 9, low: 3),
            candle("2026-03-01", high: 6, low: 0.1),
        ]

        let quarter = candles.folded(into: 7_776_000)

        XCTAssertEqual(quarter.count, 1)
        XCTAssertEqual(quarter[0].highPrice, 9)
        XCTAssertEqual(quarter[0].lowPrice, 0.1)
    }

    func testMonthlyFoldPlans() {
        XCTAssertEqual(KlineData.monthlyFold(for: "3M")?.months, 3)
        XCTAssertEqual(KlineData.monthlyFold(for: "1Y")?.months, 12)
        XCTAssertNil(KlineData.monthlyFold(for: "1M"), "a native interval needs no folding")
        XCTAssertNil(KlineData.monthlyFold(for: "1w"))
    }

    func testAxisLabelsNameTheCandle() {
        XCTAssertEqual(TimeAxisFormatter.format(for: 7_776_000), "MMM yyyy")
        XCTAssertEqual(TimeAxisFormatter.format(for: 31_536_000), "yyyy")
        XCTAssertEqual(TimeAxisFormatter.format(for: 2_592_000), "MMM yyyy")
    }

    // MARK: - Fibonacci timeframe groups follow the candle size

    func testFibonacciGroupsPlaceQuarterlyAndYearlyWithWeeklyAndMonthly() {
        XCTAssertTrue(DrawingTimeframeVisibility.weeklyAndMonthly.includes(.threeMonths))
        XCTAssertTrue(DrawingTimeframeVisibility.weeklyAndMonthly.includes(.oneYear))
        XCTAssertFalse(DrawingTimeframeVisibility.daily.includes(.threeMonths))
        XCTAssertTrue(DrawingTimeframeVisibility.daily.includes(.oneDay))
    }

    // MARK: - Binance

    private func binanceService() -> BinanceAPIService {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [KlinesURLProtocol.self]
        return BinanceAPIService(session: URLSession(configuration: configuration), baseURL: "https://binance.test")
    }

    private func klineRows(_ candles: [KlineData]) throws -> Data {
        let rows: [[Any]] = candles.map {
            [
                Int($0.openTime.timeIntervalSince1970 * 1_000), "\($0.openPrice)", "\($0.highPrice)", "\($0.lowPrice)",
                "\($0.closePrice)", "\($0.volume)", 0, "\($0.quoteVolume)",
            ]
        }
        return try JSONSerialization.data(withJSONObject: rows)
    }

    override func tearDown() {
        KlinesURLProtocol.handler = nil
        super.tearDown()
    }

    func testBinanceBuildsQuartersFromMonthlyKlines() async throws {
        var requested: [[String: String]] = []
        KlinesURLProtocol.handler = { request in
            let items = URLComponents(url: request.url!, resolvingAgainstBaseURL: false)?.queryItems ?? []
            requested.append(Dictionary(items.map { ($0.name, $0.value ?? "") }, uniquingKeysWith: { $1 }))
            return try self.klineRows(self.monthly(from: 2025, month: 1, count: 21))
        }

        let quarters = try await binanceService().fetchKlines(symbol: "BTCUSDT", interval: "3M", limit: 4)

        XCTAssertEqual(requested.first?["interval"], "1M", "Binance has no quarterly klines")
        XCTAssertEqual(requested.first?["limit"], "15", "(4 + 1) quarters of 3 months")
        XCTAssertEqual(
            quarters.map(\.openTime),
            [date("2025-10-01"), date("2026-01-01"), date("2026-04-01"), date("2026-07-01")])
    }

    func testBinanceBuildsYearsFromMonthlyKlinesAndCapsTheRequest() async throws {
        var limits: [String] = []
        KlinesURLProtocol.handler = { request in
            let items = URLComponents(url: request.url!, resolvingAgainstBaseURL: false)?.queryItems ?? []
            limits.append(items.first { $0.name == "limit" }?.value ?? "")
            return try self.klineRows(self.monthly(from: 2024, month: 6, count: 28))
        }

        let years = try await binanceService().fetchKlines(symbol: "BTCUSDT", interval: "1Y", limit: 210)

        XCTAssertEqual(limits.first, "1000", "Binance allows at most 1000 klines")
        XCTAssertEqual(years.map(\.openTime), [date("2024-01-01"), date("2025-01-01"), date("2026-01-01")])
        XCTAssertEqual(years[0].volume, 7, "the partial first year keeps the months that exist (Jun–Dec)")
    }

    func testBinanceNativeIntervalsAreAskedForDirectly() async throws {
        var intervals: [String] = []
        KlinesURLProtocol.handler = { request in
            let items = URLComponents(url: request.url!, resolvingAgainstBaseURL: false)?.queryItems ?? []
            intervals.append(items.first { $0.name == "interval" }?.value ?? "")
            return try self.klineRows(self.monthly(from: 2026, month: 1, count: 3))
        }

        let candles = try await binanceService().fetchKlines(symbol: "BTCUSDT", interval: "1M", limit: 3)

        XCTAssertEqual(intervals, ["1M"])
        XCTAssertEqual(candles.count, 3, "monthly stays monthly")
    }

    func testBinanceReplayKnowsTheNewChartSizes() {
        let service = binanceService()
        XCTAssertTrue(service.supportedReplayIntervals(chartInterval: "3M").contains(.oneDay))
        XCTAssertTrue(service.supportedReplayIntervals(chartInterval: "1Y").contains(.oneHour))
    }

    // MARK: - Coinbase

    func testCoinbaseFoldsDailyCandlesIntoQuartersAndYears() throws {
        let quarter = try XCTUnwrap(CoinbaseGranularity(interval: "3M"))
        let year = try XCTUnwrap(CoinbaseGranularity(interval: "1Y"))
        XCTAssertEqual(quarter.source, 86_400)
        XCTAssertTrue(quarter.needsAggregation)
        XCTAssertEqual(year.source, 86_400)

        let days = [
            candle("2025-12-31"), candle("2026-01-01"), candle("2026-03-31"), candle("2026-04-01"),
        ]
        XCTAssertEqual(
            CoinbaseAPIService.fold(days, into: quarter).map(\.openTime),
            [date("2025-10-01"), date("2026-01-01"), date("2026-04-01")])
        XCTAssertEqual(
            CoinbaseAPIService.fold(days, into: year).map(\.openTime), [date("2025-01-01"), date("2026-01-01")])
    }

    func testCoinbaseBucketEndsLandOnTheNextQuarterAndYear() throws {
        let quarter = try XCTUnwrap(CoinbaseGranularity(interval: "3M"))
        let year = try XCTUnwrap(CoinbaseGranularity(interval: "1Y"))

        XCTAssertEqual(quarter.bucketEnd(after: date("2026-07-01")), date("2026-10-01"))
        XCTAssertEqual(quarter.bucketEnd(after: date("2026-10-01")), date("2027-01-01"))
        XCTAssertEqual(quarter.bucketEnd(after: date("2026-01-01")), date("2026-04-01"))
        XCTAssertEqual(year.bucketEnd(after: date("2024-01-01")), date("2025-01-01"), "leap year")
    }

    @MainActor
    func testLiveTradesLandInTheCurrentQuarter() throws {
        let quarter = try XCTUnwrap(CoinbaseGranularity(interval: "3M"))
        let chart = ChartViewModel(ticker: "BTC-USD", source: .coinbase, api: StubSource())
        chart.klineData = [candle("2026-07-01", open: 100, high: 110, low: 95, close: 105)]

        chart.applyTick(
            CoinbaseTick(productID: "BTC-USD", price: 120, size: 1, time: date("2026-09-30"), tradeID: 1), plan: quarter
        )
        XCTAssertEqual(chart.klineData.count, 1)
        XCTAssertEqual(chart.klineData[0].highPrice, 120)

        chart.applyTick(
            CoinbaseTick(productID: "BTC-USD", price: 90, size: 1, time: date("2026-10-01"), tradeID: 2), plan: quarter)
        XCTAssertEqual(chart.klineData.count, 2)
        XCTAssertEqual(chart.klineData[1].openTime, date("2026-10-01"))
    }

    private final class StubSource: TickerDataSource {
        let type = DataSourceType.coinbase
        func fetchKlines(symbol: String, interval: String, limit: Int) async throws -> [KlineData] { [] }
        func searchTickers(query: String) async throws -> [TickerSearchResult] { [] }
    }
}
