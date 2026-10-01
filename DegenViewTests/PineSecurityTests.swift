import XCTest

@testable import DegenView

/// `request.security` on the chart's own symbol: higher-timeframe series built from the chart's bars.
final class PineSecurityTests: XCTestCase {
    private typealias F = PineExecutionFixtures

    /// Hourly bars whose close is the bar number, starting 1970-01-01 00:00 UTC, so day *n* holds
    /// bars 24n…24n+23 and its close is 24(n+1).
    private func hourly(_ count: Int, spacing: TimeInterval = 3_600) -> [KlineData] {
        (0..<count).map { i in
            let close = Double(i + 1)
            return KlineData(
                openTime: Date(timeIntervalSince1970: Double(i) * spacing), openPrice: close,
                highPrice: close + 1, lowPrice: close - 1, closePrice: close, volume: 1)
        }
    }

    private func compile(_ body: String) -> PineCompiledProgram {
        PineCompiler.compile(source: "//@version=6\nindicator(\"T\")\n\(body)")
    }

    private func plots(_ body: String, bars: [KlineData]) throws -> [[Double?]] {
        let program = compile(body)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        return try PineRuntimeSession(program: program).evaluate(bars: bars).output.plots.map(\.values)
    }

    private func runtimeError(_ body: String, bars: [KlineData]) -> String? {
        let program = compile(body)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        do {
            _ = try PineRuntimeSession(program: program).evaluate(bars: bars)
            return nil
        } catch {
            return (error as? PineDiagnostic)?.code
        }
    }

    private let daily = "plot(request.security(syminfo.tickerid, \"D\", close))"

    // MARK: - Completed bars

    func testDailyValueArrivesOnTheChartBarThatClosesTheDay() throws {
        let values = try plots(daily, bars: hourly(48))[0]
        XCTAssertTrue(values[..<23].allSatisfy { $0 == nil }, "no day has completed yet")
        XCTAssertEqual(values[23], 24, "the last hour of day 0 completes it")
        XCTAssertTrue(values[24..<47].allSatisfy { $0 == 24 }, "carried until the next day completes")
        XCTAssertEqual(values[47], 48)
    }

    func testTheExpressionRunsOnTheHigherTimeframeSeries() throws {
        let values = try plots("plot(request.security(syminfo.tickerid, \"D\", ta.sma(close, 2)))", bars: hourly(72))[0]
        XCTAssertNil(values[23], "one day is not enough for a 2-bar average")
        XCTAssertEqual(try XCTUnwrap(values[47]), 36, accuracy: 1e-9)
        XCTAssertEqual(try XCTUnwrap(values[71]), 60, accuracy: 1e-9)
    }

    func testTupleWithHistoryOffsetsReadsThePreviousHigherTimeframeBar() throws {
        let result = try plots(
            """
            [h, l, t] = request.security(syminfo.tickerid, "D", [high[1], low[1], time[1]])
            plot(h)
            plot(l)
            plot(t)
            """, bars: hourly(48))
        XCTAssertNil(result[0][23], "day 0 has no previous day")
        XCTAssertEqual(result[0][47], 25, "day 0's high")
        XCTAssertEqual(result[1][47], 0, "day 0's low")
        XCTAssertEqual(result[2][47], 0, "day 0 opened at time 0")
    }

    func testSameTimeframeIsTheIdentity() throws {
        let series = hourly(5)
        let values = try plots("plot(request.security(syminfo.tickerid, \"60\", close))", bars: series)[0]
        XCTAssertEqual(values, series.map(\.closePrice))
        let chartDefault = try plots("plot(request.security(syminfo.tickerid, \"\", close))", bars: series)[0]
        XCTAssertEqual(chartDefault, series.map(\.closePrice))
    }

    func testMonthlyBucketsFollowTheCalendar() throws {
        let days = hourly(70, spacing: 86_400)
        let values = try plots("plot(request.security(syminfo.tickerid, \"M\", close))", bars: days)[0]
        XCTAssertTrue(values[..<30].allSatisfy { $0 == nil })
        XCTAssertEqual(values[30], 31, "January ends on day 30")
        XCTAssertEqual(values[31], 31)
        XCTAssertEqual(values[58], 59, "February ends on day 58")
    }

    // MARK: - State and scope

    func testVarStateInsideTheExpressionAdvancesOncePerHigherTimeframeBar() throws {
        let values = try plots(
            """
            counter() =>
                var int n = 0
                n += 1
                n
            plot(request.security(syminfo.tickerid, "D", counter()))
            plot(counter())
            """, bars: hourly(48))
        XCTAssertEqual(values[0][23], 1)
        XCTAssertEqual(values[0][46], 1)
        XCTAssertEqual(values[0][47], 2)
        XCTAssertEqual(values[1][47], 48, "the chart's own counter is a separate state")
    }

    func testInputsAndConstantsAreVisibleInsideTheExpression() throws {
        let values = try plots(
            """
            factor = input.int(2, "Factor")
            offset = 0.5
            plot(request.security(syminfo.tickerid, "D", close * factor + offset))
            """, bars: hourly(24))[0]
        XCTAssertEqual(values[23], 48.5)
    }

    func testTwoCallSitesKeepIndependentState() throws {
        let values = try plots(
            """
            plot(request.security(syminfo.tickerid, "D", ta.sma(close, 2)))
            plot(request.security(syminfo.tickerid, "60", ta.sma(close, 2)))
            """, bars: hourly(48))
        XCTAssertEqual(values[0][47], 36)
        XCTAssertEqual(try XCTUnwrap(values[1][47]), 47.5, accuracy: 1e-9)
    }

    func testACallInsideAUserFunctionHasItsOwnSite() throws {
        let values = try plots(
            """
            dailyClose() => request.security(syminfo.tickerid, "D", close)
            plot(dailyClose())
            """, bars: hourly(48))[0]
        XCTAssertEqual(values[23], 24)
        XCTAssertEqual(values[47], 48)
    }

    // MARK: - Options

    func testGapsOnReturnsNaExceptWhereANewValueArrives() throws {
        let values = try plots(
            "plot(request.security(syminfo.tickerid, \"D\", close, gaps = barmerge.gaps_on))", bars: hourly(48))[0]
        XCTAssertEqual(values.enumerated().filter { $0.element != nil }.map(\.offset), [23, 47])
    }

    func testLookaheadOnReturnsTheDevelopingBar() throws {
        let developing = try plots(
            "plot(request.security(syminfo.tickerid, \"D\", close, lookahead = barmerge.lookahead_on))",
            bars: hourly(30))[0]
        XCTAssertEqual(developing[5], 6)
        XCTAssertEqual(developing[25], 26)
        let previousDay = try plots(
            "plot(request.security(syminfo.tickerid, \"D\", close[1], lookahead = barmerge.lookahead_on))",
            bars: hourly(48))[0]
        XCTAssertTrue(previousDay[..<24].allSatisfy { $0 == nil })
        XCTAssertTrue(previousDay[24...].allSatisfy { $0 == 24 }, "the non-repainting idiom")
    }

    // MARK: - Errors

    func testUnsupportedRequestsAreRuntimeErrors() {
        let series = hourly(30)
        XCTAssertEqual(
            runtimeError("plot(request.security(syminfo.tickerid, \"1\", close))", bars: series), "PINE4021")
        XCTAssertEqual(
            runtimeError("plot(request.security(syminfo.tickerid, \"5X\", close))", bars: series), "PINE4021")
        XCTAssertEqual(
            runtimeError("plot(request.security(\"BINANCE:ETHUSDT\", \"D\", close))", bars: series), "PINE4022")
        XCTAssertEqual(
            runtimeError(
                "plot(request.security(syminfo.tickerid, \"D\", request.security(syminfo.tickerid, \"D\", close)))",
                bars: series), "PINE4023")
        XCTAssertEqual(runtimeError("plot(request.security(syminfo.tickerid, \"D\"))", bars: series), "PINE4024")
    }

    func testOtherRequestFunctionsAreStillReportedAtCompileTime() {
        let program = compile("x = request.dividends(syminfo.tickerid)\nplot(close)")
        XCTAssertEqual(program.diagnostics.map(\.code), ["PINE9003"])
    }

    func testInvalidSymbolOrTimeframeIsNaWhenTheScriptAsksToIgnoreIt() throws {
        let result = try plots(
            """
            other = request.security("BINANCE:ETHUSDT", "D", close, ignore_invalid_symbol = true)
            finer = request.security(syminfo.tickerid, "1", close, ignore_invalid_timeframe = true)
            [a, b] = request.security("BINANCE:ETHUSDT", "D", [open, close], ignore_invalid_symbol = true)
            plot(na(other) ? 1 : 0)
            plot(na(finer) ? 1 : 0)
            plot(na(a) and na(b) ? 1 : 0)
            """, bars: hourly(3))
        XCTAssertEqual(result.map { $0[2] }, [1, 1, 1])
    }

    func testIgnoreFlagsDoNotHideTheOtherKindOfInvalidRequest() {
        let series = hourly(3)
        // Ignoring an invalid timeframe does not excuse another symbol, and the reverse.
        XCTAssertEqual(
            runtimeError(
                "plot(request.security(\"BINANCE:ETHUSDT\", \"D\", close, ignore_invalid_timeframe = true))",
                bars: series), "PINE4022")
        XCTAssertEqual(
            runtimeError(
                "plot(request.security(syminfo.tickerid, \"1\", close, ignore_invalid_symbol = true))",
                bars: series), "PINE4021")
    }

    // MARK: - Lower timeframes

    func testLowerTimeframeRequestsReturnEmptyArraysBecauseThereIsNoIntrabarData() throws {
        let values = try plots(
            """
            [o, c, v] = request.security_lower_tf(syminfo.tickerid, "1", [open, close, volume])
            single = request.security_lower_tf(syminfo.tickerid, "1", close)
            plot(array.size(o) + array.size(c) + array.size(v))
            plot(array.size(single))
            plot(na(array.avg(single)) ? 1 : 0)
            """, bars: hourly(3))
        XCTAssertEqual(values.map { $0[2] }, [0, 0, 1])
    }

    func testLowerTimeframeRequestRejectsTheChartsOwnOrHigherTimeframeUnlessIgnored() throws {
        let series = hourly(3)
        XCTAssertEqual(
            runtimeError("x = request.security_lower_tf(syminfo.tickerid, \"D\", close)\nplot(close)", bars: series),
            "PINE4021")
        let ignored = try plots(
            """
            x = request.security_lower_tf(syminfo.tickerid, "D", close, ignore_invalid_timeframe = true)
            plot(array.size(x))
            """, bars: series)
        XCTAssertEqual(ignored, [[0, 0, 0]])
    }

    func testLowerTimeframeRequestForAnotherSymbolIsAnErrorUnlessIgnored() throws {
        let series = hourly(2)
        XCTAssertEqual(
            runtimeError("x = request.security_lower_tf(\"BINANCE:ETHUSDT\", \"1\", close)\nplot(close)", bars: series),
            "PINE4022")
        let ignored = try plots(
            """
            x = request.security_lower_tf("BINANCE:ETHUSDT", "1", close, ignore_invalid_symbol = true)
            plot(array.size(x))
            """, bars: series)
        XCTAssertEqual(ignored, [[0, 0]])
    }

    // MARK: - Realtime

    func testRealtimeTicksShowTheDevelopingBarAndCommitOncePerHigherTimeframeBar() throws {
        let controller = F.controller(
            """
            counter() =>
                var int n = 0
                n += 1
                n
            plot(request.security(syminfo.tickerid, "5", counter()))
            """)
        // Four 1-minute bars, then bar 4 ticks twice and closes (it completes the 5-minute bucket).
        _ = F.update(controller.rebuild(bars: F.history([1, 2, 3, 4]), live: false))
        let updates = [(5.0, false), (6.0, false), (7.0, true)].compactMap { close, closed in
            F.update(controller.ingest(F.stream(F.bar(4, open: 5, close: close, closed: closed))))
        }
        XCTAssertEqual(updates.map { F.last($0.output) }, [1, 1, 1], "developing, developing, then committed")
        // Bar 5 opens the next bucket: its developing value sits on top of the one committed bar.
        let next = try XCTUnwrap(F.update(controller.ingest(F.stream(F.bar(5, open: 7, close: 8)))))
        XCTAssertEqual(F.last(next.output), 2)
    }
}
