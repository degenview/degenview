import XCTest

@testable import DegenView

/// One test per defect fixed during the Pine engine refactor.
final class PineRegressionTests: XCTestCase {
    private func bars(_ closes: [Double], spacing: TimeInterval = 60) -> [KlineData] {
        closes.enumerated().map { i, c in
            .init(
                openTime: Date(timeIntervalSince1970: Double(i) * spacing), openPrice: c,
                highPrice: c + 1, lowPrice: c - 1, closePrice: c, volume: Double(i + 1))
        }
    }

    private func compile(_ body: String, header: String = "indicator(\"T\")") -> PineCompiledProgram {
        PineCompiler.compile(source: "//@version=6\n\(header)\n\(body)")
    }

    private func codes(_ program: PineCompiledProgram) -> [String] { program.diagnostics.map(\.code) }

    // MARK: - Lexer

    func testDiagnosticOffsetsAreUTF16EvenAfterEmoji() {
        let source = "//@version=6\nindicator(\"T\")\nx = \"😀😀\" $\n"
        let diagnostic = PineCompiler.compile(source: source).diagnostics.first { $0.code == "PINE1001" }
        let expected = (source as NSString).range(of: "$").location
        XCTAssertEqual(diagnostic?.range.start.offset, expected)
    }

    func testSingleQuotedStringsLexLikeDoubleQuotedOnes() throws {
        // `it's "x"` is 8 characters whichever quote delimits it.
        let program = compile(
            """
            plot(str.length('it\\'s "x"'))
            plot(str.length("it's \\"x\\""))
            plot(str.length('{"a":1}'))
            """)
        XCTAssertTrue(program.isValid, "\(codes(program))")
        let output = try PineRuntimeSession(program: program).evaluate(bars: bars([1])).output
        XCTAssertEqual(output.plots.map(\.values), [[8], [8], [7]])
    }

    func testLineIndentedByANonMultipleOfFourContinuesThePreviousLine() throws {
        let program = compile(
            """
            pick(float x) =>
                x > 2 ? 10
                     : x > 1 ? 20
                     : 30
            total = close
              + 1
            plot(pick(total))
            plot(total)
            """)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        let output = try PineRuntimeSession(program: program).evaluate(bars: bars([0.5, 1.5, 5])).output
        XCTAssertEqual(output.plots.map(\.values), [[20, 10, 10], [1.5, 2.5, 6]])
    }

    func testMisindentedFirstLineStillReportsIndentation() {
        XCTAssertTrue(codes(PineCompiler.compile(source: " //@version=6\n  x = 1\n")).contains("PINE1002"))
    }

    func testUnterminatedSingleQuotedStringIsADiagnostic() {
        XCTAssertTrue(codes(compile("x = 'abc\nplot(close)")).contains("PINE1003"))
    }

    func testFullWidthHexDigitsInColorAreRejectedNotCrashed() {
        let program = compile("plot(close, color=#ＡＢＣＤＥＦ)")
        XCTAssertTrue(codes(program).contains("PINE1004"))
    }

    func testOversizedSourceIsADiagnosticNotACrash() {
        var limits = PineLimits.default
        limits.sourceCharacters = 20
        let program = PineCompiler.compile(
            source: "//@version=6\nindicator(\"T\")\nplot(close)\n", limits: limits)
        XCTAssertTrue(codes(program).contains("PINE8001"), "\(codes(program))")
        XCTAssertFalse(program.isValid)
    }

    // MARK: - Parser and compiler

    func testGenericArrayConstructorMatchesTheTypedOne() throws {
        let program = compile(
            """
            var array<float> a = array.new<float>(3, 2.5)
            var array<int> b = array.new<int>()
            array.push(b, 1)
            plot(array.sum(a))
            plot(array.size(b))
            """)
        XCTAssertTrue(program.isValid, "\(codes(program))")
        let output = try PineRuntimeSession(program: program).evaluate(bars: bars([1])).output
        XCTAssertEqual(output.plots.map(\.values), [[7.5], [1]])
    }

    func testUnsupportedDeclarationsReportOnceAndSkipTheirBodies() {
        let program = compile(
            """
            type Zone
                float top
                float bottom
                int    bars

            enum Mode
                fast
                slow

            method area(Zone z) =>
                z.top - z.bottom

            import someone/Library/1 as lib
            plot(close)
            """)
        XCTAssertEqual(
            program.diagnostics.map(\.code), ["PINE9006", "PINE9007", "PINE9009", "PINE9008"])
    }

    func testTypeAndEnumRemainUsableAsVariableNames() {
        XCTAssertTrue(compile("type = 1\nenum = type + 1\nplot(enum)").isValid)
    }

    func testCommaSeparatedStatementsShareALine() throws {
        let program = compile(
            """
            pair(float x) =>
                var float lo = na, var float hi = na
                switch int(x)
                    1 => lo := 1.0, hi := 10.0
                    => lo := 2.0, hi := 20.0
                [lo, hi]
            [a, b] = pair(close)
            plot(a)
            plot(b)
            """)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        let output = try PineRuntimeSession(program: program).evaluate(bars: bars([1, 2])).output
        XCTAssertEqual(output.plots.map(\.values), [[1, 2], [10, 20]])
    }

    func testExportIsAcceptedInALibraryAndRejectedElsewhere() {
        let body = "export const int answer = 42\nexport double(float x) =>\n    x * 2\n"
        let library = PineCompiler.compile(source: "//@version=6\nlibrary(\"Lib\")\n" + body)
        XCTAssertTrue(library.isValid, "\(library.diagnostics)")
        XCTAssertEqual(codes(compile(body)).filter { $0 == "PINE3037" }.count, 2)
    }

    func testOversizedIntegerLiteralIsADiagnosticNotACrash() {
        let program = compile("x = 99999999999999999999\nplot(x)")
        XCTAssertTrue(codes(program).contains("PINE2014"), "\(codes(program))")
    }

    func testConstantFoldingWrapsIntegerOverflowInsteadOfTrapping() {
        let program = compile(
            "big = 9000000000000000 * 9000000000000000\nplot(close)")
        XCTAssertFalse(codes(program).contains("PINE2014"))
    }

    func testStatementLimitCountsNestedStatements() {
        var limits = PineLimits.default
        limits.astNodes = 4
        let program = PineCompiler.compile(
            source: """
                //@version=6
                indicator("T")
                if close > 0
                    a = 1
                    b = 2
                    c = 3
                plot(close)
                """, limits: limits)
        XCTAssertTrue(codes(program).contains("PINE8003"), "\(codes(program))")
    }

    func testRequestCallsAreFlaggedInsideExpressions() {
        let program = compile("x = request.security(syminfo.tickerid, \"D\", close)\nplot(x)")
        XCTAssertTrue(codes(program).contains("PINE9003"), "\(codes(program))")
    }

    // MARK: - Operators

    func testIntegerModuloOfMinByMinusOneIsNaNotATrap() {
        XCTAssertEqual(PineOperators.apply(.modulo, .int(.min), .int(-1)), .na)
        XCTAssertEqual(PineOperators.negate(.int(.min)), .int(.min))
    }

    // MARK: - Runtime

    func testHugeHistoryOffsetIsNaNotATrap() throws {
        let program = compile("plot(close[1e300])")
        let output = try PineRuntimeSession(program: program).evaluate(bars: bars([1, 2])).output
        XCTAssertEqual(output.plots[0].values, [nil, nil])
    }

    func testMethodSyntaxOnArraysMatchesTheNamespacedCalls() throws {
        let program = compile(
            """
            var array<float> a = array.new<float>()
            if barstate.isfirst
                a.push(4.0)
                a.push(5.0)
                a.push(6.0)
            a.set(0, 10.0)
            a.remove(1)
            total(array<float> values) => values.get(0) + values.get(values.size() - 1)
            plot(total(a))
            plot(a.size())
            """)
        XCTAssertTrue(program.isValid, "\(codes(program))")
        let output = try PineRuntimeSession(program: program).evaluate(bars: bars([1])).output
        XCTAssertEqual(output.plots.map(\.values), [[16], [2]])
    }

    func testMethodSyntaxOnDrawingHandlesMatchesTheNamespacedCalls() throws {
        let program = compile(
            """
            var line ln = line.new(bar_index, 1.0, bar_index + 1, 2.0)
            ln.set_y2(7.0)
            """)
        XCTAssertTrue(program.isValid, "\(codes(program))")
        let output = try PineRuntimeSession(program: program).evaluate(bars: bars([1, 2])).output
        XCTAssertEqual(output.lines.first?.y2, 7)
    }

    func testMethodCallOnANonHandleIsStillAnUnknownFunction() {
        let program = compile("x = 1.0\nplot(x.get(0))")
        XCTAssertThrowsError(try PineRuntimeSession(program: program).evaluate(bars: bars([1]))) {
            XCTAssertEqual(($0 as? PineDiagnostic)?.code, "PINE4007")
        }
    }

    func testNamedDefvalIsTheInputDefaultEvenAfterANamedTitle() throws {
        let program = compile(
            """
            show = input.bool(title="Show", defval=true, group="G")
            length = input.int(title="Length", defval=5, minval=1)
            label = input.string(defval="x", title="Label")
            plot(show ? length : -1)
            plot(str.length(label))
            """)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        XCTAssertEqual(
            program.inputSchema.inputs.map(\.defaultValue), [.bool(true), .int(5), .string("x")])
        let output = try PineRuntimeSession(program: program).evaluate(bars: bars([1])).output
        XCTAssertEqual(output.plots.map(\.values), [[5], [1]])
    }

    func testLastBarIndexAndTimeAreKnownOnEveryHistoricalBar() throws {
        let program = compile("plot(last_bar_index)\nplot(last_bar_time)\nplot(timenow > 0 ? 1 : 0)")
        let series = bars([1, 2, 3])
        let output = try PineRuntimeSession(program: program).evaluate(bars: series).output
        let lastTime = series[2].openTime.timeIntervalSince1970 * 1000
        XCTAssertEqual(output.plots.map(\.values), [[2, 2, 2], [lastTime, lastTime, lastTime], [1, 1, 1]])
    }

    func testLastBarIndexAdvancesWhenARealtimeBarOpensPastHistory() throws {
        let program = compile("plot(last_bar_index)")
        let session = PineRuntimeSession(program: program)
        let series = bars([1, 2, 3, 4])
        try session.load(history: Array(series.prefix(3)), precedesLiveBar: true)
        try session.execute(.init(candle: series[3], phase: .realtimeTick(isNew: true)), isLast: true)
        XCTAssertEqual(session.output().plots[0].values, [3, 3, 3, 3])
    }

    func testTimeframeInSecondsParsesTimeframeStrings() throws {
        let program = compile(
            """
            plot(timeframe.in_seconds())
            plot(timeframe.in_seconds("15"))
            plot(timeframe.in_seconds("30S"))
            plot(timeframe.in_seconds("D"))
            plot(timeframe.in_seconds("2W"))
            plot(timeframe.isticks ? 1 : 0)
            """)
        let output = try PineRuntimeSession(program: program).evaluate(bars: bars([1, 2])).output
        XCTAssertEqual(output.plots.map { $0.values.last ?? nil }, [60, 900, 30, 86_400, 1_209_600, 0])
    }

    func testColorFromGradientInterpolatesEachChannelAndClamps() throws {
        let program = compile(
            """
            low = color.rgb(0, 0, 0, 0)
            high = color.rgb(200, 100, 50, 100)
            bgcolor(color.from_gradient(close, 0, 10, low, high))
            """)
        let series = bars([-5, 0, 5, 10, 15])
        let output = try PineRuntimeSession(program: program).evaluate(bars: series).output
        let colors = output.backgrounds.first?.colors ?? []
        let low: UInt32 = 0x0000_00FF
        let high: UInt32 = 0xC864_32FF
        // The alpha channel runs from 255 (transparency 0) to 0 (transparency 100): 128 halfway.
        XCTAssertEqual(colors.count, 5)
        XCTAssertEqual(colors[0], low)
        XCTAssertEqual(colors[1], low)
        XCTAssertEqual(colors[2], 0x6432_1980)
        XCTAssertEqual(colors[3], high & 0xFFFF_FF00)
        XCTAssertEqual(colors[4], high & 0xFFFF_FF00)
    }

    func testColorChannelReadersReturnWhatRGBPacked() throws {
        let program = compile(
            """
            c = color.rgb(200, 100, 50, 40)
            plot(color.r(c))
            plot(color.g(c))
            plot(color.b(c))
            plot(color.t(c))
            """)
        let output = try PineRuntimeSession(program: program).evaluate(bars: bars([1])).output
        for (plot, expected) in zip(output.plots, [200.0, 100, 50, 40]) {
            XCTAssertEqual(try XCTUnwrap(plot.values[0]), expected, accuracy: 0.5)
        }
    }

    func testTimeframeChangeFlagsTheFirstBarOfEachPeriod() throws {
        let program = compile("plot(timeframe.change(\"1D\") ? 1 : 0)\nplot(timeframe.change(\"60\") ? 1 : 0)")
        // 12 half-day bars: a new day opens on the 1st, 3rd, 5th... and a new hour on every bar.
        let series = bars(Array(repeating: 1, count: 6), spacing: 43_200)
        let output = try PineRuntimeSession(program: program).evaluate(bars: series).output
        XCTAssertEqual(output.plots[0].values, [1, 0, 1, 0, 1, 0])
        XCTAssertEqual(output.plots[1].values, [1, 1, 1, 1, 1, 1])
    }

    func testTimeTradingDayIsMidnightUTCOfTheBarsDay() throws {
        let program = compile("plot(time_tradingday)")
        // Bars open at 0, 50,000 and 100,000 s; the third is past the first midnight at 86,400 s.
        let output = try PineRuntimeSession(program: program).evaluate(bars: bars([1, 2, 3], spacing: 50_000)).output
        XCTAssertEqual(output.plots[0].values, [0, 0, 86_400_000])
    }

    func testTimeframeInSecondsRejectsAMalformedTimeframe() {
        let program = compile("plot(timeframe.in_seconds(\"5X\"))")
        XCTAssertThrowsError(try PineRuntimeSession(program: program).evaluate(bars: bars([1]))) {
            XCTAssertEqual(($0 as? PineDiagnostic)?.code, "PINE4017")
        }
        XCTAssertNil(PineTime.seconds(ofTimeframe: ""))
        XCTAssertNil(PineTime.seconds(ofTimeframe: "0"))
    }

    func testUndefinedVariableIsARuntimeError() {
        let program = compile("plot(typo)")
        XCTAssertThrowsError(try PineRuntimeSession(program: program).evaluate(bars: bars([1]))) {
            XCTAssertEqual(($0 as? PineDiagnostic)?.code, "PINE4008")
        }
    }

    func testDottedEnumerationConstantsStillResolveToThemselves() throws {
        let program = compile("plotshape(close > 0, style=shape.circle, location=location.top)")
        XCTAssertTrue(program.isValid, "\(codes(program))")
        let output = try PineRuntimeSession(program: program).evaluate(bars: bars([1])).output
        XCTAssertEqual(output.markers.first?.style, .circle)
        XCTAssertEqual(output.markers.first?.location, .top)
    }

    func testTimeCloseIsTheBarsCloseTime() throws {
        let program = compile("plot(time_close - time)")
        let output = try PineRuntimeSession(program: program).evaluate(bars: bars([1, 2, 3])).output
        XCTAssertEqual(output.plots[0].values.last ?? nil, 60_000)
    }

    func testResetRestoresInferredTimeframeState() throws {
        let program = compile("plot(timeframe.multiplier)")
        let session = PineRuntimeSession(program: program)
        _ = try session.evaluate(bars: bars([1, 2, 3], spacing: 300))
        session.reset()
        XCTAssertEqual(session.barSeconds, 0)
        let output = try session.evaluate(bars: bars([1, 2, 3], spacing: 900)).output
        XCTAssertEqual(output.plots[0].values.last ?? nil, 15)
    }

    func testRealtimeBarsTeachTheSessionTheBarLength() throws {
        let program = compile("plot(time_close - time)")
        let session = PineRuntimeSession(program: program)
        let candles = bars([1, 2], spacing: 120)
        try session.execute(.init(candle: candles[0], phase: .historical))
        try session.execute(.init(candle: candles[1], phase: .realtimeClose(isNew: true)))
        XCTAssertEqual(session.output().plots[0].values.last ?? nil, 120_000)
    }

    func testFunctionLocalsDoNotLeakIntoSeriesHistories() throws {
        let program = compile(
            """
            f(x) =>
                var total = 0.0
                total += x
                total
            plot(f(close))
            """)
        XCTAssertTrue(program.isValid, "\(codes(program))")
        let session = PineRuntimeSession(program: program)
        _ = try session.evaluate(bars: bars([1, 2, 3]))
        XCTAssertFalse(session.committed.histories.keys.contains { $0.hasPrefix(PineRuntimeSession.internalPrefix) })
    }

    func testArrayOperationsAcceptNamedArguments() throws {
        let program = compile(
            """
            a = array.new_float(0)
            array.push(id = a, value = 4.0)
            array.push(a, 6.0)
            plot(array.get(id = a, index = 1))
            plot(array.sum(a))
            """)
        XCTAssertTrue(program.isValid, "\(codes(program))")
        let output = try PineRuntimeSession(program: program).evaluate(bars: bars([1])).output
        XCTAssertEqual(output.plots.map { $0.values.last ?? nil }, [6, 10])
    }

    func testIndicatorsSkipNaInputsLikeBefore() throws {
        let program = compile("plot(ta.sma(close > 2 ? close : na, 2))")
        let output = try PineRuntimeSession(program: program).evaluate(bars: bars([1, 3, 4, 5])).output
        XCTAssertEqual(output.plots[0].values, [nil, nil, 3.5, 4.5])
    }

    func testRsiWithNonPositiveLengthIsNaNotNaN() throws {
        let program = compile("plot(ta.rsi(close, 0))")
        let output = try PineRuntimeSession(program: program).evaluate(bars: bars([1, 2, 3])).output
        XCTAssertEqual(output.plots[0].values, [nil, nil, nil])
    }

    func testNonFiniteColorTransparencyIsOpaqueNotATrap() throws {
        let program = compile("plot(close, color=color.new(color.red, math.sqrt(-1)))")
        let output = try PineRuntimeSession(program: program).evaluate(bars: bars([1])).output
        XCTAssertEqual(output.plots[0].color & 0xFF, 0xFF)
    }

    // MARK: - Timestamps

    func testMonthNamesMatchWholeWordsOrAbbreviations() {
        XCTAssertNotNil(PineTimestamp.parse("01 Sept 2018"))
        XCTAssertNotNil(PineTimestamp.parse("01 September 2018"))
        XCTAssertNotNil(PineTimestamp.parse("01 Jan. 2018"))
        XCTAssertNil(PineTimestamp.parse("01 mayhem 2018"))
    }

    func testAbsurdTimestampFieldsAreNaNotATrap() {
        XCTAssertNil(PineTimestamp.evaluate(positional: [.int(999_999_999_999), .int(1), .int(1)]))
        XCTAssertNil(PineTimestamp.evaluate(positional: [.int(2018), .int(999_999_999_999), .int(1)]))
    }

    // MARK: - Display

    func testDisplayArithmeticStillWorksOnBuiltinConstants() throws {
        let program = compile("plot(close, display=display.pane + display.data_window)")
        let output = try PineRuntimeSession(program: program).evaluate(bars: bars([1])).output
        XCTAssertEqual(output.plots[0].display, [.pane, .dataWindow])
    }
}
