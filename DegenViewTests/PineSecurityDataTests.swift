import XCTest

@testable import DegenView

/// `request.security` reading series the chart does not carry: another symbol, and the chart's own symbol
/// on a longer timeframe from before the chart's first bar.
final class PineSecurityDataTests: XCTestCase {
    private struct Stub: PineSecurityDataProvider {
        var series: [PineSecurityKey: [KlineData]]
        func candles(for key: PineSecurityKey) -> [KlineData]? { series[key] }
    }

    private let first: TimeInterval = 3_000
    private let chart = PineSymbolInfo(ticker: "BTCUSDT", tickerID: "binance:BTCUSDT")

    private func candle(_ open: TimeInterval, close: Double) -> KlineData {
        .init(
            openTime: Date(timeIntervalSince1970: open), openPrice: close, highPrice: close + 1,
            lowPrice: close - 1, closePrice: close, volume: 1)
    }

    /// Chart bars `spacing` apart starting at `first`, closing at 1, 2, 3…
    private func bars(_ count: Int, spacing: TimeInterval = 60) -> [KlineData] {
        (0..<count).map { candle(first + Double($0) * spacing, close: Double($0 + 1)) }
    }

    private func run(
        _ body: String, bars: [KlineData], data: [PineSecurityKey: [KlineData]]? = nil
    ) throws -> [[Double?]] {
        let program = PineCompiler.compile(source: "//@version=6\nindicator(\"T\")\n\(body)")
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        let session = PineRuntimeSession(
            program: program, symbol: chart, securityData: data.map { Stub(series: $0) })
        return try session.evaluate(bars: bars).output.plots.map(\.values)
    }

    private func code(_ body: String, bars: [KlineData], data: [PineSecurityKey: [KlineData]]? = nil) -> String? {
        do {
            _ = try run(body, bars: bars, data: data)
            return nil
        } catch {
            return (error as? PineDiagnostic)?.code
        }
    }

    // MARK: - Lower timeframes

    private var chartKey: String { chart.tickerID }

    func testLowerTimeframeServesTheIntrabarsOfEachChartBar() throws {
        let minutes = (0..<10).map { candle(first + Double($0) * 60, close: Double($0 + 1)) }
        let data = [PineSecurityKey(symbol: chartKey, interval: 60): minutes]
        let result = try run(
            """
            [o, c] = request.security_lower_tf(syminfo.tickerid, "1", [open, close])
            plot(array.size(c))
            plot(array.sum(c))
            plot(array.get(c, array.size(c) - 1))
            """, bars: bars(2, spacing: 300), data: data)
        XCTAssertEqual(result[0], [5, 5])
        XCTAssertEqual(result[1], [15, 40])
        XCTAssertEqual(result[2], [5, 10])
    }

    func testTheExpressionKeepsItsStateAcrossIntrabarsAndChartBars() throws {
        let minutes = (0..<10).map { candle(first + Double($0) * 60, close: Double($0 + 1)) }
        let data = [PineSecurityKey(symbol: chartKey, interval: 60): minutes]
        let result = try run(
            """
            running = request.security_lower_tf(syminfo.tickerid, "1", ta.cum(close))
            plot(array.get(running, array.size(running) - 1))
            """, bars: bars(2, spacing: 300), data: data)
        XCTAssertEqual(result[0], [15, 55], "a running total over all ten intrabars, not restarted per chart bar")
    }

    func testCalcBarsCountLimitsIntrabarsToTheNewestChartBars() throws {
        let minutes = (0..<15).map { candle(first + Double($0) * 60, close: Double($0 + 1)) }
        let data = [PineSecurityKey(symbol: chartKey, interval: 60, recentBars: 2): minutes]
        let result = try run(
            """
            c = request.security_lower_tf(syminfo.tickerid, "1", close, calc_bars_count = 2)
            plot(array.size(c))
            """, bars: bars(3, spacing: 300), data: data)
        XCTAssertEqual(result[0], [0, 5, 5], "the oldest chart bar is outside the window")
    }

    func testAChartBarWithoutIntrabarsGetsEmptyArrays() throws {
        let minutes = (0..<5).map { candle(first + Double($0) * 60, close: Double($0 + 1)) }
        let data = [PineSecurityKey(symbol: chartKey, interval: 60): minutes]
        let result = try run(
            """
            c = request.security_lower_tf(syminfo.tickerid, "1", close)
            plot(array.size(c))
            """, bars: bars(3, spacing: 300), data: data)
        XCTAssertEqual(result[0], [5, 0, 0])
    }

    func testWithoutAProviderLowerTimeframeArraysAreEmpty() throws {
        let result = try run(
            """
            c = request.security_lower_tf(syminfo.tickerid, "1", close)
            plot(array.size(c))
            """, bars: bars(2, spacing: 300))
        XCTAssertEqual(result[0], [0, 0])
    }

    func testTheIntrabarBaseIsTheCoarsestSizeThatDividesTheTimeframe() {
        let expected: [(TimeInterval, ReplayInterval?)] = [
            (60, .oneMinute), (180, .oneMinute), (720, .oneMinute), (900, .fifteenMinutes),
            (2_880, .oneMinute), (3_600, .oneHour), (14_400, .oneHour), (86_400, .oneDay), (30, nil),
        ]
        for (seconds, base) in expected {
            XCTAssertEqual(PineIntrabarSeries.base(forSeconds: seconds), base, "\(seconds) s")
        }
    }

    // MARK: - Another symbol

    func testAnotherSymbolOnTheChartsTimeframeReadsItsCloses() throws {
        let other = (0..<5).map { candle(first + Double($0) * 60, close: Double(($0 + 1) * 10)) }
        let data = [PineSecurityKey(symbol: "OTHER", interval: 60): other]
        let result = try run("plot(request.security(\"OTHER\", \"1\", close))", bars: bars(5), data: data)
        XCTAssertEqual(result, [[10, 20, 30, 40, 50]])
    }

    func testALongerTimeframeAnswersWithTheLastCompletedCandle() throws {
        let other = [candle(first, close: 7), candle(first + 300, close: 8)]
        let data = [PineSecurityKey(symbol: "OTHER", interval: 300): other]
        let result = try run("plot(request.security(\"OTHER\", \"5\", close))", bars: bars(10), data: data)
        XCTAssertEqual(result, [[nil, nil, nil, nil, 7, 7, 7, 7, 7, 8]])
    }

    func testAFinerSeriesAnswersWithTheLastCandleOfEachChartBar() throws {
        let other = (0..<10).map { candle(first + Double($0) * 60, close: Double(100 + $0)) }
        let data = [PineSecurityKey(symbol: "OTHER", interval: 60): other]
        let result = try run(
            "plot(request.security(\"OTHER\", \"1\", close))", bars: bars(2, spacing: 300), data: data)
        XCTAssertEqual(result, [[104, 109]])
    }

    func testTheExpressionKeepsItsOwnHistoryOnTheOtherSeries() throws {
        let other = (0..<4).map { candle(first + Double($0) * 60, close: Double(($0 + 1) * 10)) }
        let data = [PineSecurityKey(symbol: "OTHER", interval: 60): other]
        let result = try run("plot(request.security(\"OTHER\", \"1\", ta.change(close)))", bars: bars(4), data: data)
        XCTAssertEqual(result, [[nil, 10, 10, 10]])
    }

    func testGapsOnAnswersNaWhereNothingCompleted() throws {
        let other = [candle(first, close: 7)]
        let data = [PineSecurityKey(symbol: "OTHER", interval: 300): other]
        let source = "plot(request.security(\"OTHER\", \"5\", close, gaps = barmerge.gaps_on))"
        XCTAssertEqual(try run(source, bars: bars(6), data: data), [[nil, nil, nil, nil, 7, nil]])
    }

    func testASymbolWithoutDataIsAnErrorUnlessIgnored() throws {
        let data: [PineSecurityKey: [KlineData]] = [:]
        XCTAssertEqual(code("plot(request.security(\"NOPE\", \"1\", close))", bars: bars(2), data: data), "PINE4022")
        let ignored = "plot(request.security(\"NOPE\", \"1\", close, ignore_invalid_symbol = true))"
        XCTAssertEqual(try run(ignored, bars: bars(2), data: data), [[nil, nil]])
    }

    func testWithoutAProviderAnotherSymbolStaysUnsupported() {
        XCTAssertEqual(code("plot(request.security(\"OTHER\", \"1\", close))", bars: bars(2)), "PINE4022")
    }

    // MARK: - The chart's own symbol

    func testHistoryFromBeforeTheChartStartsTheSeriesWarm() throws {
        let earlier = [candle(2_100, close: 1), candle(2_400, close: 2), candle(2_700, close: 3)]
        let data = [PineSecurityKey(symbol: "binance:BTCUSDT", interval: 300): earlier]
        let source = "plot(request.security(syminfo.tickerid, \"5\", close))"
        XCTAssertEqual(try run(source, bars: bars(10), data: data), [[3, 3, 3, 3, 5, 5, 5, 5, 5, 10]])
        XCTAssertEqual(
            try run(source, bars: bars(10)), [[nil, nil, nil, nil, 5, 5, 5, 5, 5, 10]],
            "without a provider the series is built from the chart's bars alone")
    }

    func testWarmUpHistoryFeedsTheExpressionsIndicators() throws {
        let earlier = [candle(2_100, close: 1), candle(2_400, close: 2), candle(2_700, close: 3)]
        let data = [PineSecurityKey(symbol: "binance:BTCUSDT", interval: 300): earlier]
        let source = "plot(request.security(syminfo.tickerid, \"5\", ta.sma(close, 3)))"
        // (2 + 3 + 5) / 3 once the first chart bucket closes; (3 + 5 + 10) / 3 after the second.
        let result = try run(source, bars: bars(10), data: data)[0]
        XCTAssertEqual(result[4] ?? .nan, 10.0 / 3, accuracy: 1e-9)
        XCTAssertEqual(result[9] ?? .nan, 6, accuracy: 1e-9)
    }

    func testACandleThatOverlapsTheChartsFirstBarIsNotUsedForWarmUp() throws {
        let overlapping = [candle(2_700, close: 3), candle(3_000, close: 99)]
        let data = [PineSecurityKey(symbol: "binance:BTCUSDT", interval: 300): overlapping]
        let source = "plot(request.security(syminfo.tickerid, \"5\", close))"
        XCTAssertEqual(try run(source, bars: bars(5), data: data), [[3, 3, 3, 3, 5]])
    }

    // MARK: - Collections and objects

    func testAnArrayReturnedByTheExpressionIsCopiedIntoTheChart() throws {
        let source = """
            mine = array.from(111.0)
            theirs = request.security(syminfo.tickerid, "5", array.from(close, close * 2))
            plot(na(theirs) ? na : array.get(theirs, 1))
            plot(array.get(mine, 0))
            """
        let result = try run(source, bars: bars(10))
        XCTAssertEqual(result[0], [nil, nil, nil, nil, 10, 10, 10, 10, 10, 20])
        XCTAssertEqual(result[1], Array(repeating: 111, count: 10), "the chart's own array is left alone")
    }

    func testAnInstanceAndItsArrayFieldAreCopiedIntoTheChart() throws {
        let source = """
            type Box
                float top
                array<float> levels

            make() =>
                Box.new(close, array.from(close, close + 1))
            box = request.security(syminfo.tickerid, "5", make())
            plot(na(box) ? na : box.top + array.get(box.levels, 1))
            """
        XCTAssertEqual(try run(source, bars: bars(10))[0], [nil, nil, nil, nil, 11, 11, 11, 11, 11, 21])
    }

    func testAnotherSymbolsCollectionsAreCopiedToo() throws {
        let other = (0..<5).map { candle(first + Double($0) * 60, close: Double(($0 + 1) * 10)) }
        let data = [PineSecurityKey(symbol: "OTHER", interval: 60): other]
        let source = """
            theirs = request.security("OTHER", "1", array.from(close, 1.0))
            plot(array.get(theirs, 0))
            """
        XCTAssertEqual(try run(source, bars: bars(5), data: data)[0], [10, 20, 30, 40, 50])
    }
}
