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
