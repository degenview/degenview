import XCTest

@testable import DegenView

/// Pins behaviour that `docs/pine-compatibility.md` lists as a difference from TradingView or a limit, so
/// changing it means changing the documentation too.
final class PineDocumentedDifferencesTests: XCTestCase {
    private func bars(_ count: Int, spacing: TimeInterval = 60) -> [KlineData] {
        (0..<count).map { i in
            let close = Double(i + 1)
            return .init(
                openTime: Date(timeIntervalSince1970: Double(i) * spacing), openPrice: close, highPrice: close + 1,
                lowPrice: close - 1, closePrice: close, volume: 1)
        }
    }

    private func compile(_ body: String) -> PineCompiledProgram {
        PineCompiler.compile(source: "//@version=6\nindicator(\"T\")\n\(body)")
    }

    private func plots(_ body: String, count: Int = 1, spacing: TimeInterval = 60) throws -> [[Double?]] {
        let program = compile(body)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        return try PineRuntimeSession(program: program).evaluate(bars: bars(count, spacing: spacing)).output.plots
            .map(\.values)
    }

    private func runtimeError(_ body: String) -> String? {
        let program = compile(body)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        do {
            _ = try PineRuntimeSession(program: program).evaluate(bars: bars(1))
            return nil
        } catch {
            return (error as? PineDiagnostic)?.code
        }
    }

    func testDeclaredFieldTypesAreNotEnforcedAtRuntime() throws {
        let result = try plots(
            """
            type Zone
                float top

            z = Zone.new("not a number")
            z.top := "still not a number"
            plot(str.length(z.top))
            """)
        XCTAssertEqual(result, [[18]])
    }

    func testEnumValuesCompareEqualToTheirStringForm() throws {
        let result = try plots(
            """
            enum Mode
                fast

            plot(Mode.fast == "Mode.fast" ? 1 : 0)
            """)
        XCTAssertEqual(result, [[1]])
    }

    func testForOverASingleMapVariableBindsTheValue() throws {
        let result = try plots(
            """
            var map<int, float> m = map.new<int, float>()
            m.put(1, 5.0)
            m.put(2, 6.0)
            float sum = 0.0
            for v in m
                sum += v
            plot(sum)
            """)
        XCTAssertEqual(result, [[11]])
    }

    func testATwoWeekTimeframeBucketsAsOneWeek() throws {
        // Daily bars from 1970-01-01 (a Thursday): weeks start on Monday, so a new week opens on days 4, 11, 18.
        let result = try plots("plot(timeframe.change(\"2W\") ? 1 : 0)", count: 20, spacing: 86_400)
        XCTAssertEqual(result[0].enumerated().filter { $0.element == 1 }.map(\.offset), [0, 4, 11, 18])
    }

    func testTableCellSettersAreNotSupported() {
        XCTAssertEqual(
            runtimeError(
                """
                var table t = table.new(position.top_right, 1, 1)
                table.cell(t, 0, 0, "a")
                table.cell_set_text(t, 0, 0, "b")
                """), "PINE4007")
    }

    func testCurrencyAndCalcBarsCountAreAcceptedAndIgnoredByRequestSecurity() throws {
        let result = try plots(
            """
            plot(request.security(syminfo.tickerid, "60", close, currency = "EUR", calc_bars_count = 3))
            """, count: 3, spacing: 3_600)
        XCTAssertEqual(result, [[1, 2, 3]])
    }

    func testAHigherTimeframeSeriesIsNoDeeperThanTheChart() throws {
        // 30 hourly bars hold one complete day and part of another, so a 2-bar average over daily bars is
        // still na on the last bar.
        let result = try plots(
            "plot(request.security(syminfo.tickerid, \"D\", ta.sma(close, 2)))", count: 30, spacing: 3_600)
        XCTAssertTrue(result[0].allSatisfy { $0 == nil })
    }

    func testAMainVariableSeriesHasNoHistoryInsideTheExpression() throws {
        let result = try plots(
            """
            mine = close * 2
            plot(na(request.security(syminfo.tickerid, "D", mine[1])) ? 1 : 0)
            """, count: 48, spacing: 3_600)
        XCTAssertEqual(result[0].last, 1)
    }

    func testMethodsAreFirstMatchInSourceOrder() throws {
        let result = try plots(
            """
            method pick(float this) => 1
            method pick(int this) => 2
            plot(pick(3.0))
            plot(pick(3))
            """)
        // An int receiver also satisfies the earlier `float` definition, so it wins.
        XCTAssertEqual(result, [[1], [1]])
    }
}
