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

    func testLargePublishedScriptsFitTheDefaultSourceLimit() throws {
        let filler = String(repeating: "// padding, as in a long published script, to make this long\n", count: 2_600)
        XCTAssertGreaterThan(filler.count, 150_000)
        let program = compile("\(filler)plot(close)")
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        let output = try PineRuntimeSession(program: program).evaluate(bars: bars([1, 2])).output
        XCTAssertEqual(output.plots[0].values, [1, 2])
    }

    func testSourceBeyondTheDefaultLimitIsReportedNotParsed() {
        let filler = String(repeating: "// padding, as in a long published script, to make this long\n", count: 9_000)
        XCTAssertGreaterThan(filler.count, PineLimits.default.sourceCharacters)
        let program = compile("\(filler)plot(close)")
        XCTAssertEqual(codes(program).filter { $0 == "PINE8001" }, ["PINE8001"])
        XCTAssertFalse(program.isValid)
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

    func testAnImportIsReportedOnceAndSkipped() {
        let program = compile(
            """
            import someone/Library/1 as lib
            plot(close)
            """)
        XCTAssertEqual(program.diagnostics.map(\.code), ["PINE9008"])
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
        let program = compile("x = request.dividends(syminfo.tickerid)\nplot(x)")
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

    func testXlocBarTimeMapsTimesToBarIndexesInConstructorsAndSetters() throws {
        let program = compile(
            """
            var box b = na
            var label l = na
            var line ln = na
            var box plain = na
            var box early = na
            if bar_index == 5
                b := box.new(time[2], 10.0, time + 3 * 60000, 5.0, xloc = xloc.bar_time)
                l := label.new(time[1], 9.0, "x", xloc = xloc.bar_time)
                ln := line.new(time[5], 1.0, time, 2.0, xloc = xloc.bar_time)
                plain := box.new(1, 3.0, 2, 2.0)
                early := box.new(time - 8 * 60000 - 5 * 60000, 3.0, time, 2.0, xloc = xloc.bar_time)
            if bar_index == 7
                box.set_right(b, time)
                label.set_x(l, time + 60000)
                line.set_x2(ln, time[3])
                box.set_right(plain, 4)
            """)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        let output = try PineRuntimeSession(program: program).evaluate(bars: bars(Array(repeating: 1, count: 8)))
            .output
        let boxes = output.boxes.sorted { $0.id < $1.id }
        XCTAssertEqual([boxes[0].left, boxes[0].right], [3, 7], "time[2] is bar 3; time at bar 7 is bar 7")
        XCTAssertEqual([boxes[1].left, boxes[1].right], [1, 4], "bar-index drawings are untouched")
        XCTAssertEqual(boxes[2].left, -8, "before the first bar: extrapolated by bar length")
        XCTAssertEqual(output.labels.first?.x, 8, "one bar after bar 7")
        XCTAssertEqual([output.lines[0].x1, output.lines[0].x2], [0, 4])
    }

    func testBoxTextBorderStyleAndLabelAlignmentAreStoredAndSettable() throws {
        let program = compile(
            """
            var box b = box.new(0, 10.0, 3, 5.0, border_style = line.style_dashed, text = "zone",
                 text_size = size.large, text_color = color.red, text_halign = text.align_left,
                 text_valign = text.align_top)
            var label l = label.new(0, 1.0, "a\\nbb", textalign = text.align_right)
            if bar_index == 1
                box.set_text(b, "moved")
                box.set_border_style(b, line.style_dotted)
                box.set_text_color(b, color.blue)
                box.set_text_size(b, size.small)
                box.set_text_halign(b, text.align_right)
                box.set_text_valign(b, text.align_bottom)
                box.set_text_wrap(b, text.wrap_auto)
                label.set_textalign(l, text.align_left)
                label.set_text_font_family(l, font.family_monospace)
            """)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        let first = try PineRuntimeSession(program: program).evaluate(bars: bars([1])).output
        let box = try XCTUnwrap(first.boxes.first)
        XCTAssertEqual(
            [
                box.text, "\(box.borderStyle)", "\(box.textSize)", "\(box.textHorizontalAlign)",
                "\(box.textVerticalAlign)",
            ],
            ["zone", "dashed", "large", "left", "top"])
        XCTAssertEqual(box.textColor, PineBuiltins.colors["color.red"])
        XCTAssertEqual(first.labels.first?.textAlign, .right)
        let later = try PineRuntimeSession(program: program).evaluate(bars: bars([1, 2])).output
        let moved = try XCTUnwrap(later.boxes.first)
        XCTAssertEqual(
            [
                moved.text, "\(moved.borderStyle)", "\(moved.textSize)", "\(moved.textHorizontalAlign)",
                "\(moved.textVerticalAlign)",
            ],
            ["moved", "dotted", "small", "right", "bottom"])
        XCTAssertEqual(moved.textColor, PineBuiltins.colors["color.blue"])
        XCTAssertEqual(later.labels.first?.textAlign, .left)
    }

    func testEveryLabelStyleConstantResolvesAndIsStored() throws {
        let styles = [
            "none", "label_down", "label_up", "label_left", "label_right", "label_center", "label_lower_left",
            "label_lower_right", "label_upper_left", "label_upper_right", "circle", "square", "diamond", "cross",
            "xcross", "flag", "triangleup", "triangledown", "arrowup", "arrowdown", "text_outline",
        ]
        let program = compile(
            styles.map { "label.new(bar_index, close, \"x\", style = label.style_\($0))" }.joined(separator: "\n"))
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        let output = try PineRuntimeSession(program: program).evaluate(bars: bars([1])).output
        XCTAssertEqual(output.labels.count, styles.count)
        XCTAssertEqual(
            output.labels.map { $0.style.rawValue }, styles, "each style survives creation with its own name")
    }

    func testLabelTooltipIsStoredFromNewAndSetTooltip() throws {
        let program = compile(
            """
            var label a = label.new(0, 1.0, "a", tooltip = "first")
            var label b = label.new(0, 2.0, "b")
            label.set_tooltip(b, "second")
            """)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        let output = try PineRuntimeSession(program: program).evaluate(bars: bars([1])).output
        XCTAssertEqual(output.labels.map(\.tooltip), ["first", "second"])
    }

    func testNaCountsAsFalseInConditionsButNumbersStillAreNot() throws {
        let program = compile(
            """
            bool up = close > open
            plot(not up[1] ? 1 : 0)
            plot(up[1] ? 1 : 0)
            plot(up[1] and true ? 1 : 0)
            plot(up[1] or true ? 1 : 0)
            float seen = 0.0
            if up[1]
                seen := 5.0
            plot(seen)
            """)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        // Bar 0: `up[1]` reads na, which is false in v6; bar 1 reads bar 0's value (close > open is false here).
        let output = try PineRuntimeSession(program: program).evaluate(bars: bars([1, 2])).output
        XCTAssertEqual(output.plots.map { $0.values[0] }, [1, 0, 0, 1, 0])
        for source in ["if 1\n    x = 1", "plot(1 ? 1 : 0)", "plot(not 1 ? 1 : 0)"] {
            let bad = compile(source + "\nplot(close)")
            XCTAssertThrowsError(try PineRuntimeSession(program: bad).evaluate(bars: bars([1])), source)
        }
    }

    func testColorConstantsFoldThroughEarlierConstants() throws {
        let program = compile(
            """
            const color BASE = #2962FF
            const int FADE = 88
            tint = input.color(color.new(BASE, FADE), "Tint")
            plot(close, color = tint)
            """)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        XCTAssertEqual(
            program.inputSchema.inputs.first?.defaultValue, .color(PineBuiltins.withTransparency(0x2962_FFFF, 88)))
    }

    func testChartTypeFlagsDescribePlainCandles() throws {
        let program = compile(
            "plot(chart.is_standard ? 1 : 0)\nplot(chart.is_heikinashi or chart.is_renko or chart.is_range ? 1 : 0)")
        let output = try PineRuntimeSession(program: program).evaluate(bars: bars([1])).output
        XCTAssertEqual(output.plots.map(\.values), [[1], [0]])
    }

    func testDeclarationArgumentsThatOnlyAffectDrawingOrderAreAccepted() {
        for argument in ["behind_chart = false", "behind_chart = true", "explicit_plot_zorder = true"] {
            let header = "indicator(\"T\", overlay = true, \(argument))"
            XCTAssertTrue(compile("plot(close)", header: header).isValid, argument)
        }
        let library = PineCompiler.compile(source: "//@version=6\nlibrary(\"L\", dynamic_requests = true)\n")
        XCTAssertTrue(library.isValid, "\(library.diagnostics)")
        XCTAssertEqual(
            codes(compile("plot(close)", header: "indicator(\"T\", scale = scale.none)")), ["PINE9001"],
            "arguments that change behaviour stay unsupported")
    }

    func testInputDefaultMayBeANamedConstantAndOptionsMayBePositional() throws {
        let program = compile(
            """
            tagSize = input.string(size.small, "Tag size", [size.tiny, size.small, size.normal], group = "G")
            label.new(0, 1.0, "x", size = tagSize)
            plot(close)
            """)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        let input = try XCTUnwrap(program.inputSchema.inputs.first)
        XCTAssertEqual(input.defaultValue, .string("size.small"))
        XCTAssertEqual(input.options, [.string("size.tiny"), .string("size.small"), .string("size.normal")])
        let output = try PineRuntimeSession(program: program).evaluate(bars: bars([1])).output
        XCTAssertEqual(output.labels.first?.size, .small)
    }

    func testSubscriptOnACallOrExpressionReadsThatExpressionsHistory() throws {
        let program = compile(
            """
            plot(ta.sma(close, 2)[1])
            plot(ta.highest(high, 3)[1])
            plot((close + 1)[2])
            f(float x) => (x * 2)[1]
            plot(f(close))
            """)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        let output = try PineRuntimeSession(program: program).evaluate(bars: bars([1, 2, 3, 4])).output
        XCTAssertEqual(output.plots[0].values, [nil, nil, 1.5, 2.5])
        XCTAssertEqual(output.plots[1].values, [nil, nil, nil, 4], "highest(high, 3) is 4 on bar 2 and 5 on bar 3")
        XCTAssertEqual(output.plots[2].values, [nil, nil, 2, 3])
        XCTAssertEqual(output.plots[3].values, [nil, 2, 4, 6])
    }

    func testSubscriptedExpressionNotReachedOnABarKeepsItsSlot() throws {
        let program = compile(
            """
            v = 0.0
            if bar_index > 1
                v := ta.sma(close, 1)[1]
            plot(v)
            """)
        let output = try PineRuntimeSession(program: program).evaluate(bars: bars([1, 2, 3, 4, 5])).output
        XCTAssertEqual(output.plots[0].values, [0, 0, nil, 3, 4])
    }

    func testDerivedPricesTimeAndBarIndexHaveHistory() throws {
        let program = compile(
            """
            plot(hl2[1])
            plot(hlc3[1])
            plot(ohlc4[1])
            plot(time[1])
            plot(time_close[1])
            plot(bar_index[1])
            """)
        let output = try PineRuntimeSession(program: program).evaluate(bars: bars([10, 20], spacing: 60)).output
        // Bar 0: open 10, high 11, low 9, close 10.
        let previous = output.plots.map { $0.values[1] }
        XCTAssertEqual(previous, [10, 10, 10, 0, 60_000, 0])
        XCTAssertTrue(output.plots.allSatisfy { $0.values[0] == nil })
    }

    func testColorAndStringCastsKeepTheirValueAndTurnOtherThingsIntoNa() throws {
        let program = compile(
            """
            plot(na(color(na)) ? 1 : 0)
            plot(color(color.red) == color.red ? 1 : 0)
            plot(na(string(na)) ? 1 : 0)
            plot(str.length(string("abc")))
            """)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        let output = try PineRuntimeSession(program: program).evaluate(bars: bars([1])).output
        XCTAssertEqual(output.plots.map(\.values), [[1], [1], [1], [3]])
    }

    func testStrMatchReturnsTheFirstMatchOrAnEmptyString() throws {
        let program = compile(
            """
            max_bars_back(close, 100)
            plot(str.length(str.match("tf 15m", "[0-9]+")))
            plot(str.length(str.match("tf", "^[0-9]+$")))
            plot(str.length(str.match("abc", "(")))
            plot(str.match("60", "^[0-9]+$") != "" ? 1 : 0)
            """)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        let output = try PineRuntimeSession(program: program).evaluate(bars: bars([1])).output
        XCTAssertEqual(output.plots.map(\.values), [[2], [0], [0], [1]])
    }

    func testTimeWithATimeframeIsTheOpenOfTheEnclosingBar() throws {
        let program = compile(
            """
            plot(time("5"))
            plot(ta.change(time("5")) != 0 ? 1 : 0)
            plot(time == time("") ? 1 : 0)
            """)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        // One-minute bars from 0: the 5-minute bar opens at 0 and at 300,000 ms.
        let output = try PineRuntimeSession(program: program).evaluate(bars: bars(Array(repeating: 1, count: 7)))
            .output
        XCTAssertEqual(output.plots[0].values, [0, 0, 0, 0, 0, 300_000, 300_000])
        XCTAssertEqual(output.plots[1].values, [0, 0, 0, 0, 0, 1, 0])
        XCTAssertEqual(output.plots[2].values, Array(repeating: 1, count: 7))
    }

    func testTimeCloseWithATimeframeIsTheCloseOfTheEnclosingBar() throws {
        let program = compile(
            """
            plot(time_close("5"))
            plot(time_close == time_close("") ? 1 : 0)
            plot(time_close == time_close("5") ? 1 : 0)
            """)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        let output = try PineRuntimeSession(program: program).evaluate(bars: bars(Array(repeating: 1, count: 6)))
            .output
        XCTAssertEqual(output.plots[0].values, [300_000, 300_000, 300_000, 300_000, 300_000, 600_000])
        XCTAssertEqual(output.plots[1].values, Array(repeating: 1, count: 6))
        XCTAssertEqual(
            output.plots[2].values, [0, 0, 0, 0, 1, 0], "only the last 1-minute bar closes the 5-minute bar")
    }

    func testSortIndicesReturnsThePermutationThatSorts() throws {
        let program = compile(
            """
            values = array.from(30.0, 10.0, 20.0, 10.0)
            ascending = array.sort_indices(values)
            descending = array.sort_indices(values, order.descending)
            plot(array.get(ascending, 0))
            plot(array.get(ascending, 1))
            plot(array.get(ascending, 3))
            plot(array.get(descending, 0))
            plot(array.get(values, 0))
            """)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        let output = try PineRuntimeSession(program: program).evaluate(bars: bars([1])).output
        // Ties (the two 10s at 1 and 3) keep their order; the source array is untouched.
        XCTAssertEqual(output.plots.map(\.values), [[1], [3], [0], [0], [30]])
    }

    func testADottedNameThatIsNotAPineConstantIsAnErrorNotAString() {
        for source in [
            "plot(chart.is_standrd ? 1 : 0)", "plot(size.smal == size.small ? 1 : 0)", "x = ta.smaa\nplot(close)",
        ] {
            let program = compile(source)
            XCTAssertTrue(program.isValid, "\(program.diagnostics)")
            XCTAssertThrowsError(try PineRuntimeSession(program: program).evaluate(bars: bars([1])), source) {
                XCTAssertEqual(($0 as? PineDiagnostic)?.code, "PINE4008")
            }
        }
        // Real constants still stand for their own names.
        let fine = compile(
            "plot(size.small == size.small ? 1 : 0)\nplot(plot.style_linebr == plot.style_linebr ? 1 : 0)")
        XCTAssertNoThrow(try PineRuntimeSession(program: fine).evaluate(bars: bars([1])))
    }

    func testPineVariablesThisReleaseDoesNotModelAreNa() throws {
        let program = compile(
            """
            plot(na(chart.left_visible_bar_time) ? 1 : 0)
            plot(na(chart.right_visible_bar_time) ? 1 : 0)
            plot(na(session.ismarket) ? 1 : 0)
            plot(na(syminfo.description) ? 1 : 0)
            plot(na(weekofyear) ? 1 : 0)
            """)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        let output = try PineRuntimeSession(program: program).evaluate(bars: bars([1])).output
        XCTAssertEqual(output.plots.map(\.values), [[1], [1], [1], [1], [1]])
    }

    func testSymbolFactsAndHlcc4() throws {
        let program = compile(
            """
            plot(str.length(syminfo.prefix))
            plot(str.length(syminfo.root))
            plot(str.length(syminfo.timezone))
            plot(syminfo.pointvalue)
            plot(hlcc4)
            """)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        let crypto = PineSymbolInfo(ticker: "BTC", tickerID: "binance:BTC", type: "crypto")
        let output = try PineRuntimeSession(program: program, symbol: crypto).evaluate(bars: bars([10])).output
        // Bar: high 11, low 9, close 10 → (11 + 9 + 10 + 10) / 4.
        XCTAssertEqual(output.plots.map(\.values), [[7], [3], [7], [1], [10]])
        let stock = PineSymbolInfo(ticker: "AAPL", tickerID: "AAPL", type: "stock")
        let other = try PineRuntimeSession(
            program: compile("plot(na(syminfo.timezone) ? 1 : 0)\nplot(na(syminfo.prefix) ? 1 : 0)"), symbol: stock
        )
        .evaluate(bars: bars([1])).output
        XCTAssertEqual(other.plots.map(\.values), [[1], [1]], "no known exchange zone or prefix")
    }

    func testARunawayLoopStopsAtThePerBarInstructionLimit() {
        var limits = PineLimits.default
        limits.instructionsPerBar = 5_000
        let program = compile("var int n = 0\nwhile true\n    n += 1\nplot(n)")
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        XCTAssertThrowsError(try PineRuntimeSession(program: program, limits: limits).evaluate(bars: bars([1]))) {
            XCTAssertEqual(($0 as? PineDiagnostic)?.code, "PINE8004")
        }
        XCTAssertGreaterThanOrEqual(PineLimits.default.instructionsPerBar, 5_000_000)
    }

    func testStrRepeatJoinsCopiesWithAnOptionalSeparator() throws {
        let program = compile(
            """
            plot(str.length(str.repeat("ab", 3)))
            plot(str.length(str.repeat("ab", 3, "-")))
            plot(str.length(str.repeat("ab", 0)))
            plot(na(str.repeat("ab", -1)) ? 1 : 0)
            """)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        let output = try PineRuntimeSession(program: program).evaluate(bars: bars([1])).output
        XCTAssertEqual(output.plots.map(\.values), [[6], [8], [0], [1]])
    }

    func testStrSplitReturnsAnArrayOfPieces() throws {
        let program = compile(
            """
            parts = str.split("a,bb,,c", ",")
            plot(array.size(parts))
            plot(str.length(array.get(parts, 1)))
            plot(str.length(array.get(parts, 2)))
            letters = str.split("xyz", "")
            plot(array.size(letters))
            plot(na(str.split(na, ",")) ? 1 : 0)
            """)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        let output = try PineRuntimeSession(program: program).evaluate(bars: bars([1])).output
        XCTAssertEqual(output.plots.map(\.values), [[4], [2], [0], [3], [1]])
    }

    func testIntegerDivisionMayInitialiseAnIntButAFloatLiteralMayNot() {
        XCTAssertTrue(compile("int half = 7 / 2\nplot(half)").isValid)
        XCTAssertTrue(codes(compile("int bad = 1.5\nplot(bad)")).contains("PINE3030"))
        XCTAssertTrue(codes(compile("int bad = close / 2\nplot(bad)")).contains("PINE3030"))
    }

    func testTypeKeywordCanNameAVariable() throws {
        let program = compile(
            """
            pick(float x) =>
                color = x > 1 ? color.green : color.red
                color
            color = pick(close)
            color := pick(close + 1)
            plot(close, color = color)
            int length = 3
            plot(length)
            """)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        let output = try PineRuntimeSession(program: program).evaluate(bars: bars([2])).output
        XCTAssertEqual(output.plots[0].colors, [PineBuiltins.colors["color.green"]])
        XCTAssertEqual(output.plots[1].values, [3])
    }

    func testLinefillJoinsTwoLinesAndFollowsThem() throws {
        let program = compile(
            """
            var line top = line.new(0, 10.0, 5, 12.0)
            var line bottom = line.new(0, 2.0, 5, 4.0)
            linefill gone = linefill.new(na, bottom, color.red)
            var linefill band = linefill.new(top, bottom, color.new(color.blue, 80))
            band.set_color(color.green)
            plot(na(gone) ? 1 : 0)
            """)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        let output = try PineRuntimeSession(program: program).evaluate(bars: bars([1, 2])).output
        XCTAssertEqual(output.plots[0].values, [1, 1])
        XCTAssertEqual(output.linefills.count, 1)
        XCTAssertEqual(output.linefills.first?.color, PineBuiltins.colors["color.green"])
        XCTAssertEqual(Set([output.linefills[0].line1, output.linefills[0].line2]), Set(output.lines.map(\.id)))
    }

    func testLinefillDisappearsWithEitherLine() throws {
        let program = compile(
            """
            var line top = line.new(0, 10.0, 5, 12.0)
            var line bottom = line.new(0, 2.0, 5, 4.0)
            var linefill band = linefill.new(top, bottom, color.blue)
            if bar_index == 1
                line.delete(top)
            """)
        let session = PineRuntimeSession(program: program)
        XCTAssertEqual(try session.evaluate(bars: bars([1])).output.linefills.count, 1)
        XCTAssertEqual(try session.evaluate(bars: bars([1, 2])).output.linefills.count, 0)
    }

    func testTableMergeCellsSpansTheStartCellAndDropsTheOnesItCovers() throws {
        let program = compile(
            """
            var table t = table.new(position.top_right, 3, 2)
            table.cell(t, 0, 0, "head")
            table.cell(t, 1, 0, "covered")
            table.cell(t, 0, 1, "below")
            table.cell(t, 2, 1, "far")
            table.merge_cells(t, 0, 0, 1, 1)
            table.merge_cells(t, 2, 0, 2, 0)
            """)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        let output = try PineRuntimeSession(program: program).evaluate(bars: bars([1])).output
        let cells = try XCTUnwrap(output.tables.first?.cells)
        let head = try XCTUnwrap(cells.first { $0.column == 0 && $0.row == 0 })
        XCTAssertEqual(head.text, "head")
        XCTAssertEqual([head.columnSpan, head.rowSpan], [2, 2])
        // (1,0) and (0,1) lie inside the merge and are gone; the empty (2,0) anchor was created.
        XCTAssertEqual(Set(cells.map { [$0.column, $0.row] }), [[0, 0], [2, 0], [2, 1]])
    }

    func testTableSettersChangeThePositionAndColors() throws {
        let program = compile(
            """
            var table t = table.new(position.top_right, 1, 1, border_width = 1)
            table.set_position(t, position.bottom_left)
            table.set_bgcolor(t, color.red)
            table.set_border_color(t, color.green)
            table.set_frame_color(t, color.blue)
            table.set_border_width(t, 3)
            table.set_frame_width(t, 2)
            """)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        let table = try XCTUnwrap(
            try PineRuntimeSession(program: program).evaluate(bars: bars([1])).output.tables.first)
        XCTAssertEqual(table.position, .parse("position.bottom_left", absent: .topRight))
        XCTAssertEqual(table.backgroundColor, PineBuiltins.colors["color.red"])
        XCTAssertEqual(table.borderColor, PineBuiltins.colors["color.green"])
        XCTAssertEqual(table.frameColor, PineBuiltins.colors["color.blue"])
        XCTAssertEqual([table.borderWidth, table.frameWidth], [3, 2])
    }

    func testTableClearDropsTheCellsInARangeOrAll() throws {
        let program = compile(
            """
            var table t = table.new(position.top_right, 3, 2)
            table.cell(t, 0, 0, "a")
            table.cell(t, 1, 0, "b")
            table.cell(t, 2, 0, "c")
            table.cell(t, 1, 1, "d")
            table.clear(t, 1, 0, 1, 1)
            var table u = table.new(position.top_left, 2, 1)
            table.cell(u, 0, 0, "x")
            table.cell(u, 1, 0, "y")
            table.clear(u)
            """)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        let tables = try PineRuntimeSession(program: program).evaluate(bars: bars([1])).output.tables
        XCTAssertEqual(tables[0].cells.map(\.text).sorted(), ["a", "c"])
        XCTAssertEqual(tables[1].cells.count, 0)
    }

    func testTableMergeCellsOutsideTheGridIsARuntimeError() {
        let program = compile(
            "var table t = table.new(position.top_right, 2, 2)\ntable.merge_cells(t, 0, 0, 2, 0)")
        XCTAssertThrowsError(try PineRuntimeSession(program: program).evaluate(bars: bars([1]))) {
            XCTAssertEqual(($0 as? PineDiagnostic)?.code, "PINE4013")
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
