import XCTest

@testable import DegenView

/// The `ta.*` functions beyond the original set, checked against an independent implementation of
/// Pine's definitions (computed in Python on this fixture) rather than against themselves.
final class PineTechnicalAnalysisTests: XCTestCase {
    private let closes: [Double] = [10, 12, 11, 15, 14, 13, 16, 18, 17, 19, 21, 20]

    /// Open = close, high = close + 1, low = close - 1, volume = bar number, one minute apart (one UTC day).
    private var bars: [KlineData] {
        closes.enumerated().map { i, c in
            .init(
                openTime: Date(timeIntervalSince1970: Double(i) * 60), openPrice: c, highPrice: c + 1,
                lowPrice: c - 1, closePrice: c, volume: Double(i + 1))
        }
    }

    private func plots(_ body: String) throws -> [[Double?]] {
        let program = PineCompiler.compile(source: "//@version=6\nindicator(\"T\")\n\(body)")
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        return try PineRuntimeSession(program: program).evaluate(bars: bars).output.plots.map(\.values)
    }

    private func assertSeries(
        _ actual: [Double?], _ expected: [Double?], _ name: String, file: StaticString = #filePath, line: UInt = #line
    ) {
        XCTAssertEqual(actual.count, expected.count, name, file: file, line: line)
        for (index, pair) in zip(actual, expected).enumerated() {
            switch pair {
            case (nil, nil): continue
            case (let a?, let e?): XCTAssertEqual(a, e, accuracy: 1e-6, "\(name)[\(index)]", file: file, line: line)
            default:
                XCTFail(
                    "\(name)[\(index)]: \(String(describing: pair.0)) vs \(String(describing: pair.1))", file: file,
                    line: line)
            }
        }
    }

    func testMedian() throws {
        assertSeries(
            try plots("plot(ta.median(close, 3))")[0], [nil, nil, 11, 12, 14, 14, 14, 16, 17, 18, 19, 20],
            "ta.median(close, 3)")
    }

    func testRange() throws {
        assertSeries(
            try plots("plot(ta.range(close, 3))")[0], [nil, nil, 2, 4, 4, 2, 3, 5, 2, 2, 4, 2], "ta.range(close, 3)")
    }

    func testVariance() throws {
        assertSeries(
            try plots("plot(ta.variance(close, 3))")[0],
            [
                nil, nil, 0.666666667, 2.888888889, 2.888888889, 0.666666667, 1.555555556, 4.222222222, 0.666666667,
                0.666666667, 2.666666667, 0.666666667,
            ], "ta.variance(close, 3)")
    }

    func testDev() throws {
        assertSeries(
            try plots("plot(ta.dev(close, 3))")[0],
            [
                nil, nil, 0.666666667, 1.555555556, 1.555555556, 0.666666667, 1.111111111, 1.777777778, 0.666666667,
                0.666666667, 1.333333333, 0.666666667,
            ], "ta.dev(close, 3)")
    }

    func testSwma() throws {
        assertSeries(
            try plots("plot(ta.swma(close))")[0],
            [nil, nil, nil, 11.833333333, 13.0, 13.666666667, 14.166666667, 15.0, 16.333333333, 17.5, 18.5, 19.5],
            "ta.swma(close)")
    }

    func testCmo() throws {
        assertSeries(
            try plots("plot(ta.cmo(close, 4))")[0],
            [
                nil, nil, nil, nil, 50.0, 14.285714286, 55.555555556, 42.857142857, 42.857142857, 75.0, 71.428571429,
                33.333333333,
            ], "ta.cmo(close, 4)")
    }

    func testCci() throws {
        assertSeries(
            try plots("plot(ta.cci(close, 4))")[0],
            [
                nil, nil, nil, 133.333333333, 44.444444444, -13.333333333, 100.0, 104.761904762, 44.444444444, 100.0,
                120.0, 40.0,
            ], "ta.cci(close, 4)")
    }

    func testHma() throws {
        assertSeries(
            try plots("plot(ta.hma(close, 4))")[0],
            [
                nil, nil, nil, nil, 14.988888889, 13.833333333, 14.655555556, 17.577777778, 18.2, 18.5, 20.5,
                21.033333333,
            ], "ta.hma(close, 4)")
    }

    func testHighestbars() throws {
        assertSeries(
            try plots("plot(ta.highestbars(close, 4))")[0], [nil, nil, nil, 0, -1, -2, 0, 0, -1, 0, 0, -1],
            "ta.highestbars(close, 4)")
    }

    func testLowestbars() throws {
        assertSeries(
            try plots("plot(ta.lowestbars(close, 4))")[0], [nil, nil, nil, -3, -2, -3, -1, -2, -3, -3, -2, -3],
            "ta.lowestbars(close, 4)")
    }

    func testPercentrank() throws {
        assertSeries(
            try plots("plot(ta.percentrank(close, 4))")[0],
            [nil, nil, nil, nil, 75.0, 50.0, 100.0, 100.0, 75.0, 100.0, 100.0, 75.0], "ta.percentrank(close, 4)")
    }

    func testCorrelation() throws {
        assertSeries(
            try plots("plot(ta.correlation(close, volume, 4))")[0],
            [
                nil, nil, nil, 0.836660027, 0.707106781, 0.377964473, 0.2, 0.873333765, 0.836660027, 0.8, 0.831521841,
                0.831521841,
            ], "ta.correlation(close, volume, 4)")
    }

    func testVwap() throws {
        assertSeries(
            try plots("plot(ta.vwap(close))")[0],
            [
                10.0, 11.333333333, 11.166666667, 12.7, 13.133333333, 13.095238095, 13.821428571, 14.75, 15.2,
                15.890909091, 16.742424242, 17.243589744,
            ], "ta.vwap(close)")
    }

    func testDirectionalMovementIndex() throws {
        let result = try plots(
            """
            [plus, minus, adx] = ta.dmi(3, 3)
            plot(plus)
            plot(minus)
            plot(adx)
            """)
        assertSeries(
            result[0],
            [
                nil, nil, nil, 62.068965517, 47.368421053, 34.951456311, 52.581521739, 57.246706043, 43.003412969,
                51.496088835, 56.805049338, 42.079338321,
            ], "dmi_plus")
        assertSeries(
            result[1],
            [
                nil, nil, nil, 10.344827586, 19.736842105, 27.669902913, 15.489130435, 10.35892776, 20.221843003,
                12.964279853, 8.42741577, 19.204377832,
            ], "dmi_minus")
        assertSeries(
            result[2],
            [
                nil, nil, nil, nil, nil, 41.410982998, 45.770994653, 53.632276005, 47.765646892, 51.769087066,
                59.233357035, 51.931013799,
            ], "dmi_adx")
    }

    func testVwapBandsAreSymmetricAndCollapseWithAZeroMultiplier() throws {
        let result = try plots(
            """
            [mid, upper, lower] = ta.vwap(close, false, 1.5)
            [m0, u0, l0] = ta.vwap(close, false, 0.0)
            plot(upper - mid)
            plot(mid - lower)
            plot(u0 - l0)
            plot(mid)
            """)
        XCTAssertEqual(result[0].count, 12)
        for index in 1..<12 {
            XCTAssertEqual(try XCTUnwrap(result[0][index]), try XCTUnwrap(result[1][index]), accuracy: 1e-9)
            XCTAssertGreaterThan(try XCTUnwrap(result[0][index]), 0)
        }
        XCTAssertTrue(result[2].allSatisfy { $0 == 0 })
        XCTAssertEqual(try XCTUnwrap(result[3][11]), 17.24359, accuracy: 1e-5)
    }

    func testVwapRestartsEachDayAndOnAnAnchor() throws {
        let program = PineCompiler.compile(
            source: "//@version=6\nindicator(\"T\")\nplot(ta.vwap(close))\nplot(ta.vwap(close, bar_index % 3 == 0))")
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        // Hourly bars across a UTC midnight: bars 0-1 are on one day, bars 2-3 on the next.
        let day: [KlineData] = [(23, 10.0), (24, 20.0), (25, 30.0), (26, 40.0)].enumerated().map { i, entry in
            .init(
                openTime: Date(timeIntervalSince1970: Double(entry.0 - 23) * 3600 + 23 * 3600),
                openPrice: entry.1, highPrice: entry.1, lowPrice: entry.1, closePrice: entry.1, volume: 1)
        }
        let output = try PineRuntimeSession(program: program).evaluate(bars: day).output
        XCTAssertEqual(output.plots[0].values, [10, 20, 25, 30], "bar 0 is on day 0; bars 1-3 are on day 1")
        XCTAssertEqual(output.plots[1].values, [10, 15, 20, 40], "restarts on bars 0 and 3")
    }

    func testLengthOnlyFormsReadHighAndLow() throws {
        let result = try plots(
            """
            plot(ta.highest(3))
            plot(ta.lowest(3))
            plot(ta.highestbars(3))
            """)
        XCTAssertEqual(result[0][2], 13, "highest of the highs 11, 13, 12")
        XCTAssertEqual(result[1][2], 9, "lowest of the lows 9, 11, 10")
        XCTAssertEqual(result[2][3], 0, "bar 3 (high 16) is the highest of the last 3")
    }

    func testAMissingWindowIsNaAndBadLengthsDoNotCrash() throws {
        let result = try plots(
            """
            plot(ta.median(close, 0))
            plot(ta.cci(close, 1))
            plot(ta.hma(close, 1))
            plot(ta.percentrank(close, 100))
            """)
        XCTAssertTrue(result[0].allSatisfy { $0 == nil })
        XCTAssertTrue(result[1].allSatisfy { $0 == 0 }, "a one-bar window has no deviation")
        XCTAssertTrue(result[2].allSatisfy { $0 == nil })
        XCTAssertTrue(result[3].allSatisfy { $0 == nil })
    }

    func testParabolicSarInAnUptrend() throws {
        assertSeries(
            try plots("plot(ta.sar(0.02, 0.02, 0.2))")[0],
            [
                nil, 9, 9, 9.08, 9.3568, 9.622528, 9.87762688, 10.304969267, 11.000571726, 11.640525988, 12.476473389,
                13.619296582,
            ], "ta.sar")
    }

    func testParabolicSarFlipsWhenPriceCrossesIt() throws {
        let reversal: [Double] = [10, 12, 14, 16, 15, 13, 11, 9, 8, 10, 12, 14]
        let series: [KlineData] = reversal.enumerated().map { i, c in
            .init(
                openTime: Date(timeIntervalSince1970: Double(i) * 60), openPrice: c, highPrice: c + 1,
                lowPrice: c - 1, closePrice: c, volume: 1)
        }
        let program = PineCompiler.compile(source: "//@version=6\nindicator(\"T\")\nplot(ta.sar(0.02, 0.02, 0.2))")
        let values = try PineRuntimeSession(program: program).evaluate(bars: series).output.plots[0].values
        assertSeries(
            values,
            [nil, 9, 9, 9.24, 9.7056, 10.143264, 17, 16.86, 16.5056, 15.935264, 15.39914816, 7], "ta.sar reversal")
        // It sits below price while rising and above it once the trend turns down.
        XCTAssertLessThan(try XCTUnwrap(values[3]), reversal[3] - 1)
        XCTAssertGreaterThan(try XCTUnwrap(values[7]), reversal[7] + 1)
    }

    func testRunningMaximumAndMinimumKeepThePastExtreme() throws {
        let result = try plots("plot(ta.max(close))\nplot(ta.min(close))")
        XCTAssertEqual(result[0], [10, 12, 12, 15, 15, 15, 16, 18, 18, 19, 21, 21])
        XCTAssertEqual(result[1], [10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10])
    }

    func testAnUnimplementedTaFunctionIsAnErrorNotNa() {
        let program = PineCompiler.compile(source: "//@version=6\nindicator(\"T\")\nplot(ta.supertrend(3, 10))")
        XCTAssertTrue(program.isValid)
        XCTAssertThrowsError(try PineRuntimeSession(program: program).evaluate(bars: bars)) {
            XCTAssertEqual(($0 as? PineDiagnostic)?.code, "PINE4007")
        }
    }
}
