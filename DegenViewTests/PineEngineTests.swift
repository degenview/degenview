import XCTest

@testable import DegenView

final class PineEngineTests: XCTestCase {
    private func bars(_ closes: [Double]) -> [KlineData] {
        closes.enumerated().map { i, c in
            .init(
                openTime: Date(timeIntervalSince1970: Double(i) * 60), openPrice: c, highPrice: c + 1,
                lowPrice: c - 1, closePrice: c, volume: Double(i + 1))
        }
    }

    func testRequiredScriptsCompileAndExecute() throws {
        let scripts = [
            "//@version=6\nindicator(\"Close\")\nplot(close)",
            "//@version=6\nindicator(\"SMA\", overlay=true)\nlength = input.int(3)\navg = ta.sma(close, length)\nplot(avg)",
            "//@version=6\nindicator(\"Change\")\nchange = close - close[1]\nplot(change)",
            "//@version=6\nindicator(\"EMA Cross\", overlay=true)\nfast = ta.ema(close, 2)\nslow = ta.ema(close, 3)\nsignal = ta.crossover(fast, slow)\nplot(fast)\nplot(slow)\nplotshape(\n    signal,\n    style=shape.triangleup,\n    location=location.belowbar\n)",
            "//@version=6\nindicator(\"RSI\")\nr = ta.rsi(close, 3)\nplot(r)\nhline(70)\nhline(30)",
            "//@version=6\nindicator(\"Counter\")\nvar count = 0\ncount += 1\nplot(count)",
        ]
        for source in scripts {
            let program = PineCompiler.compile(source: source)
            XCTAssertTrue(program.isValid, "\(program.diagnostics)")
            let result = try PineRuntimeSession(program: program).evaluate(
                bars: bars([1, 2, 3, 4, 5, 6]))
            XCTAssertFalse(result.output.plots.isEmpty)
        }
    }

    func testHistoryAndPersistentState() throws {
        let program = PineCompiler.compile(
            source:
                "//@version=6\nindicator(\"State\")\nvar count = 0\ncount += 1\nchange = close - close[1]\nplot(count)\nplot(change)"
        )
        let output = try PineRuntimeSession(program: program).evaluate(bars: bars([10, 13, 12])).output
        XCTAssertEqual(output.plots[0].values.compactMap { $0 }, [1, 2, 3])
        XCTAssertEqual(output.plots[1].values[1], 3)
    }

    func testChartColorsFollowTheme() throws {
        let program = PineCompiler.compile(
            source: """
                //@version=6
                indicator("Theme")
                plot(close, color=chart.fg_color)
                plot(close, color=chart.bg_color)
                """)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        let dark = try PineRuntimeSession(program: program).evaluate(bars: bars([1])).output
        XCTAssertEqual(dark.plots.map(\.color), [PineChartTheme.dark.foreground, PineChartTheme.dark.background])
        let light = try PineRuntimeSession(program: program, theme: .light).evaluate(bars: bars([1])).output
        XCTAssertEqual(light.plots.map(\.color), [PineChartTheme.light.foreground, PineChartTheme.light.background])
    }

    func testOverlayDefaultsToSeparatePane() throws {
        let pane = PineCompiler.compile(source: "//@version=6\nindicator(\"Pane\")\nplot(close)")
        XCTAssertFalse(try PineRuntimeSession(program: pane).evaluate(bars: bars([1])).output.overlay)
        let overlay = PineCompiler.compile(source: "//@version=6\nindicator(\"Over\", overlay=true)\nplot(close)")
        XCTAssertTrue(try PineRuntimeSession(program: overlay).evaluate(bars: bars([1])).output.overlay)
    }

    func testPaneValueRangeCoversVisiblePlotsAndHlines() throws {
        let program = PineCompiler.compile(
            source: "//@version=6\nindicator(\"Osc\")\nplot(close - 5)\nhline(10)")
        let candles = bars([1, 2, 3, 4])
        let output = try PineRuntimeSession(program: program).evaluate(bars: candles).output
        // Only the last two bars are in view: plot values -2, -1 plus the hline at 10.
        let range = PineChartLayer(pine: output, candles: Array(candles.suffix(2)), inPane: true)
            .valueRange(padding: 0)
        XCTAssertEqual(range.min, -2)
        XCTAssertEqual(range.max, 10)
    }

    func testRealtimeRollbackAndVarip() throws {
        let source =
            "//@version=6\nindicator(\"Realtime\")\nvar ordinary = 0\nvarip ticks = 0\nordinary += 1\nticks += 1\nplot(ordinary)\nplot(ticks)"
        let session = PineRuntimeSession(program: PineCompiler.compile(source: source))
        let first = bars([10])[0]
        try session.execute(.init(candle: first, phase: .historical))
        var open = bars([11])[0]
        open = .init(
            openTime: Date(timeIntervalSince1970: 60), openPrice: 11, highPrice: 11, lowPrice: 11,
            closePrice: 11, volume: 1)
        try session.execute(.init(candle: open, phase: .realtimeTick(isNew: true)))
        open.closePrice = 12
        try session.execute(.init(candle: open, phase: .realtimeTick(isNew: false)))
        let values = session.output().plots.sorted { $0.id < $1.id }.map { $0.values.last! }
        XCTAssertEqual(values[0], 2)
        XCTAssertEqual(values[1], 3)
    }

    func testV6Diagnostics() {
        XCTAssertEqual(
            PineCompiler.compile(source: "//@version=5\nindicator(\"x\")").diagnostics.first?.code,
            "PINE0002")
        let p = PineCompiler.compile(source: "//@version=6\nindicator(\"x\")\nbool value = na")
        XCTAssertTrue(p.diagnostics.contains { $0.code == "PINE3021" })
    }

    func testUnexpectedEndDiagnosticPointsAtEndOfSource() {
        let source = "//@version=6\nindicator(\"x\")\nplot("
        let diagnostic = PineCompiler.compile(source: source).diagnostics.first { $0.code == "PINE2008" }
        XCTAssertEqual(diagnostic?.range.start.line, 3)
        XCTAssertEqual(diagnostic?.range.start.offset, source.utf16.count)
    }

    func testEditorDiagnosticRangeMapsThroughCRLFLineEndings() throws {
        let source = [
            "//@version=6",
            "indicator(\"RSI\")",
            "rsiSource = input.source(close)",
            "rsiLength = input.int(9)",
            "rsiValue = ta.rsi(rsiSource, rsiLength)!!",
        ].joined(separator: "\r\n")
        let diagnostic = try XCTUnwrap(
            PineCompiler.compile(source: source).diagnostics.first { $0.code == "PINE1001" }
        )
        let range = try XCTUnwrap(PineDiagnosticRangeMapper.nsRange(for: diagnostic.range, in: source))
        XCTAssertEqual((source as NSString).substring(with: range), "!")
    }

    func testEditorDiagnosticRangeHandlesUnicodeBeforeToken() throws {
        let source = "//@version=6\n// 🪙 café\nindicator(\"x\")\nplot(close)!"
        let diagnostic = try XCTUnwrap(
            PineCompiler.compile(source: source).diagnostics.first { $0.code == "PINE1001" }
        )
        let range = try XCTUnwrap(PineDiagnosticRangeMapper.nsRange(for: diagnostic.range, in: source))
        XCTAssertEqual((source as NSString).substring(with: range), "!")
    }

    func testColorAndLinewidthNamedArgumentsRemainCallArguments() throws {
        let source = """
            //@version=6
            indicator("EMA Momentum", overlay=true)
            fastLength = input.int(12, "Fast EMA", minval=1)
            fast = ta.ema(close, fastLength)
            plot(fast, color=color.orange, linewidth=2)
            plot(close, color=color.blue, linewidth=2)
            """
        let program = PineCompiler.compile(source: source)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        XCTAssertEqual(
            try PineRuntimeSession(program: program).evaluate(bars: bars([1, 2, 3])).output.plots.count, 2
        )
    }

    func testCopiedScriptWithCopyrightHeaderAndNonstandardLineEndings() {
        let lines = [
            "// This Pine Script® code is subject to the terms of the Mozilla Public License 2.0",
            "// © Devjames",
            "",
            "//@version=6",
            "indicator(\"RSI-50 Step Line\", overlay=true)",
            "rsiLength = input.int(9, title=\"RSI Length\", minval=1)",
        ]

        for separator in ["\r", "\r\n", "\u{2028}", "\u{2029}"] {
            let program = PineCompiler.compile(source: lines.joined(separator: separator))
            XCTAssertTrue(program.isValid, "Separator \(separator.debugDescription): \(program.diagnostics)")
            XCTAssertEqual(program.declaration.title, "RSI-50 Step Line")
            XCTAssertEqual(program.inputSchema.inputs.first?.id, "rsiLength")
        }
    }

    func testRSIStepLineScriptCompilesAndEvaluates() throws {
        let source = """
            //@version=6
            indicator("RSI-50 Step Line", overlay=true)
            rsiLength = input.int(9, title="RSI Length", minval=1)
            rsiSource = input.source(close, title="RSI Source")
            lineWidth = input.int(2, title="Line Width", minval=1, maxval=5)
            colorAboveRising = input.color(color.new(#00FF00, 0), title="Above + Rising")
            colorAboveFalling = input.color(color.new(#006400, 0), title="Above + Falling")
            colorBelowFalling = input.color(color.new(#FF0000, 0), title="Below + Falling")
            colorBelowRising = input.color(color.new(#8B0000, 0), title="Below + Rising")
            rsiValue = ta.rsi(rsiSource, rsiLength)
            var float level = na
            crossed = ta.cross(rsiValue, 50)
            if crossed
                level := close
            above = close > level
            rising = rsiValue > rsiValue[1]
            stateColor = above and rising ? colorAboveRising :
                         above and not rising ? colorAboveFalling :
                         not above and not rising ? colorBelowFalling :
                         colorBelowRising
            plot(level, title="RSI-50 Level", style=plot.style_stepline, color=stateColor, linewidth=lineWidth)
            plotshape(crossed ? close : na, title="RSI-50 Cross", style=shape.circle, location=location.absolute, size=size.tiny, color=stateColor)
            """

        let program = PineCompiler.compile(source: source)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        XCTAssertEqual(program.inputSchema.inputs.count, 7)

        let result = try PineRuntimeSession(program: program).evaluate(
            bars: bars([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 9, 8, 7]))
        XCTAssertEqual(result.output.plots.count, 1)
        XCTAssertEqual(result.output.markers.count, 1)
    }

    func testTernaryBindsMoreWeaklyThanLogicalOperators() throws {
        let source = """
            //@version=6
            indicator("Ternary precedence", overlay=true)
            selected = true and true ? color.red : color.green
            bgcolor(selected)
            """

        let program = PineCompiler.compile(source: source)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")

        let result = try PineRuntimeSession(program: program).evaluate(bars: bars([1]))
        XCTAssertEqual(result.output.backgrounds.first?.colors.first, 0xF236_45FF)
    }

    // MARK: - Control flow, functions, loops

    private func evaluate(_ body: String, closes: [Double] = [1, 2, 3]) throws -> PineVisualOutput {
        let program = PineCompiler.compile(source: "//@version=6\nindicator(\"Test\")\n" + body)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        return try PineRuntimeSession(program: program).evaluate(bars: bars(closes)).output
    }

    private func codes(_ body: String) -> [String] {
        PineCompiler.compile(source: "//@version=6\nindicator(\"Test\")\n" + body).diagnostics.map(\.code)
    }

    func testElseIfChainSelectsMatchingBranch() throws {
        let output = try evaluate(
            """
            pick(x) =>
                int r = 0
                if x == 1
                    r := 10
                else if x == 2
                    r := 20
                else if x == 3
                    r := 30
                else
                    r := 40
                r
            plot(pick(1))
            plot(pick(2))
            plot(pick(3))
            plot(pick(4))
            """, closes: [1])
        XCTAssertEqual(output.plots.map { $0.values.last! }, [10, 20, 30, 40])
    }

    func testUserFunctionsBindArgumentsAndKeepLocalsPrivate() throws {
        let output = try evaluate(
            """
            txt = "global"
            double(x) => x * 2
            describe(float value, string unit = "pts") =>
                string txt = str.tostring(value, "#.##")
                txt := txt + " " + unit
                txt
            label = describe(1.2345)
            named = describe(unit = "ticks", value = 3)
            plot(double(close))
            plot(txt == "global" ? 1 : 0)
            plot(label == "1.23 pts" ? 1 : 0)
            plot(named == "3 ticks" ? 1 : 0)
            """, closes: [5])
        XCTAssertEqual(output.plots.map { $0.values.last! }, [10, 1, 1, 1])
    }

    func testBuiltinsInsideFunctionsKeepPerCallSiteHistory() throws {
        let output = try evaluate(
            """
            avg(src) => ta.sma(src, 2)
            plot(avg(close))
            plot(avg(close * 10))
            """, closes: [1, 3, 5])
        XCTAssertEqual(output.plots[0].values, [nil, 2, 4])
        XCTAssertEqual(output.plots[1].values, [nil, 20, 40])
    }

    func testForLoops() throws {
        let output = try evaluate(
            """
            int up = 0
            for i = 1 to 4
                up += i
            int down = 0
            for i = 3 to 1
                down := down * 10 + i
            int stepped = 0
            for i = 0 to 10 by 5
                stepped += i
            int controlled = 0
            for i = 0 to 10
                if i == 2
                    continue
                if i == 5
                    break
                controlled += i
            values = array.from(3, 4, 5)
            int total = 0
            for v in values
                total += v
            int weighted = 0
            for [i, v] in values
                weighted += i * v
            plot(up)
            plot(down)
            plot(stepped)
            plot(controlled)
            plot(total)
            plot(weighted)
            """, closes: [1])
        XCTAssertEqual(output.plots.map { $0.values.last! }, [10, 321, 15, 8, 12, 14])
    }

    func testLoopControlOutsideLoopIsRejected() {
        XCTAssertTrue(codes("break").contains("PINE3023"))
    }

    func testTrailingTokensAfterStatementAreRejected() {
        let program = PineCompiler.compile(
            source: """
                //@version=6
                indicator("Step Range Breakout & Trailing Stop [BigBeluga]", overlay=true, precision=2, max_lines_count=500, max_boxes_count=500)
                aaa "hahah" 111
                """)
        XCTAssertFalse(program.isValid)
        XCTAssertTrue(program.diagnostics.contains { $0.code == "PINE2013" }, "\(program.diagnostics)")

        let junk = [
            "plot(close) 5",
            "x = 1 2",
            "var x = 0.0\nx := close high",
            "plot(close))",
            "for i = 0 to 3\n    break foo",
            "f(x) => x y",
            "if close > open\n    plot(close)\nelse foo",
        ]
        for body in junk {
            XCTAssertTrue(codes(body).contains("PINE2013"), "\(body.debugDescription): \(codes(body))")
        }
        XCTAssertFalse(codes("if close > open foo\n    plot(close)").isEmpty)
    }

    func testJunkLineReportsSingleDiagnostic() {
        let diagnostics = PineCompiler.compile(source: "//@version=6\nindicator(\"Test\")\naaa \"hahah\" 111")
            .diagnostics.filter { $0.code == "PINE2013" }
        XCTAssertEqual(diagnostics.count, 1)
        XCTAssertEqual(diagnostics.first?.range.start.line, 3)
        XCTAssertEqual(diagnostics.first?.range.start.column, 5)
    }

    func testBlockStatementsFollowedByNextLineStayValid() {
        let program = PineCompiler.compile(
            source: """
                //@version=6
                indicator("Test")
                calc(v) =>
                    r = v * 2
                    r
                float x = close +
                     open
                if close > open
                    if close > 1
                        x := 1
                    else
                        x := 2
                plot(calc(x))
                for i = 0 to 2
                    x += i
                """)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
    }

    func testFunctionsCannotReassignGlobals() {
        XCTAssertTrue(codes("var total = 0\nbump() =>\n    total := total + 1\nplot(close)").contains("PINE3022"))
    }

    func testObjectAndArrayTypeAnnotations() {
        let program = PineCompiler.compile(
            source: """
                //@version=6
                indicator("Types", overlay=true)
                var line l = na
                var label lb = na
                var table t = na
                var box[] boxes = array.new_box(0)
                var int[] ints = array.new_int(0)
                array<float> floats = array.new_float(2, 0.5)
                plot(close)
                """)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        let types = program.statements.compactMap { statement -> PineValueType? in
            if case .declaration(_, let type, _, _, _) = statement { return type }
            return nil
        }
        XCTAssertEqual(types, [.line, .label, .table, .array, .array, .array])
    }

    // MARK: - Builtins

    func testArraysStringsCastsAndMath() throws {
        let output = try evaluate(
            """
            var int[] results = array.new_int(0)
            array.push(results, 1)
            if array.size(results) > 2
                array.shift(results)
            wins = array.sum(results)
            ratio = float(wins) / float(array.size(results)) * 100.0
            plot(array.size(results))
            plot(wins)
            plot(int(7.9))
            plot(math.round(2.5))
            plot(math.round(1.23456, 2))
            plot(("a" + "b" == "ab") ? 1 : 0)
            plot(str.tostring(2.0, "#.##") == "2" ? 1 : 0)
            plot(str.tostring(2.5, "0.00") == "2.50" ? 1 : 0)
            plot(ratio)
            """, closes: [1, 2, 3, 4])
        XCTAssertEqual(output.plots.map { $0.values.last! }, [2, 2, 7, 3, 1.23, 1, 1, 1, 100])
    }

    func testArrayIndexOutOfBoundsIsARuntimeError() throws {
        let program = PineCompiler.compile(
            source: "//@version=6\nindicator(\"x\")\na = array.new_float(0)\nplot(array.get(a, 0))")
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        XCTAssertThrowsError(try PineRuntimeSession(program: program).evaluate(bars: bars([1]))) {
            XCTAssertEqual(($0 as? PineDiagnostic)?.code, "PINE4010")
        }
    }

    func testArraysRollBackOnRealtimeTicks() throws {
        let source =
            "//@version=6\nindicator(\"x\")\nvar a = array.new_float(0)\narray.push(a, close)\nplot(array.size(a))"
        let session = PineRuntimeSession(program: PineCompiler.compile(source: source))
        try session.execute(.init(candle: bars([1])[0], phase: .historical))
        let open = bars([1, 2])[1]
        try session.execute(.init(candle: open, phase: .realtimeTick(isNew: true)))
        try session.execute(.init(candle: open, phase: .realtimeTick(isNew: false)))
        XCTAssertEqual(session.output().plots[0].values.last!, 2)
    }

    func testChangeHl2AndMintick() throws {
        let output = try evaluate(
            """
            plot(ta.change(close))
            plot(ta.change(close, 2))
            plot(hl2)
            plot(syminfo.mintick)
            """, closes: [1, 4, 9])
        XCTAssertEqual(output.plots[0].values, [nil, 3, 5])
        XCTAssertEqual(output.plots[1].values, [nil, nil, 8])
        XCTAssertEqual(output.plots[2].values.last!, 9)
        XCTAssertEqual(output.plots[3].values.last!, 1)
    }

    func testInputColorConstantsOptionsAndDrawingCountArguments() {
        let program = PineCompiler.compile(
            source: """
                //@version=6
                indicator("Inputs", overlay=true, max_lines_count=500, max_labels_count=20, max_boxes_count=500)
                a = input.color(color.red, "A")
                b = input.color(color.rgb(10, 167, 151), "B")
                c = input.color(color.new(color.blue, 50), "C")
                mode = input.string("Price", "Mode", options=["Price", "Ticks", "Both"])
                plot(close)
                """)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        let inputs = program.inputSchema.inputs
        XCTAssertEqual(inputs[0].defaultValue, .color(0xF236_45FF))
        XCTAssertEqual(inputs[1].defaultValue, .color(0x0AA7_97FF))
        XCTAssertEqual(inputs[2].defaultValue, .color(0x2196_F37F))
        XCTAssertEqual(inputs[3].options, [.string("Price"), .string("Ticks"), .string("Both")])
        XCTAssertEqual(program.declaration.maxLinesCount, 500)
        XCTAssertEqual(program.declaration.maxLabelsCount, 20)
        XCTAssertEqual(program.declaration.maxBoxesCount, 500)
    }

    // MARK: - Drawings

    func testDrawingObjectsAndFill() throws {
        let output = try evaluate(
            """
            var line l = na
            var label lb = na
            var box bx = na
            if barstate.isfirst
                l := line.new(bar_index, close, bar_index, close, color = color.red, width = 2)
                lb := label.new(bar_index, high, text = "hi", style = label.style_label_up, size = size.small)
                bx := box.new(bar_index, high, bar_index, low, border_color = na, bgcolor = color.new(color.green, 50))
            line.set_x2(l, bar_index)
            label.set_text(lb, "bar " + str.tostring(bar_index))
            box.set_right(bx, bar_index)
            var table t = table.new(position.top_right, 2, 1)
            if barstate.islast
                table.cell(t, 1, 0, "done", text_color = color.white, text_size = size.small)
            p1 = plot(close)
            p2 = plot(close + 1)
            fill(p1, p2, color = close > 1 ? color.green : color.red)
            """)
        XCTAssertEqual(output.barCount, 3)
        XCTAssertEqual(output.lines.first?.x2, 2)
        XCTAssertEqual(output.lines.first?.color, 0xF236_45FF)
        XCTAssertEqual(output.lines.first?.width, 2)
        XCTAssertEqual(output.labels.first?.text, "bar 2")
        XCTAssertEqual(output.labels.first?.style, "label.style_label_up")
        XCTAssertEqual(output.boxes.first?.right, 2)
        XCTAssertNil(output.boxes.first?.borderColor)
        XCTAssertEqual(output.tables.first?.cells, [
            PineTableCell(
                column: 1, row: 0, text: "done", textColor: 0xFFFF_FFFF, backgroundColor: nil,
                textSize: "size.small")
        ])
        XCTAssertEqual(output.fills.count, 1)
        XCTAssertEqual(output.fills[0].colors, [0xF236_45FF, 0x4CAF_50FF, 0x4CAF_50FF])
        XCTAssertEqual(output.fills[0].plotA, output.plots[0].id)
    }

    func testDrawingsArePrunedPastMaxCount() throws {
        let program = PineCompiler.compile(
            source: """
                //@version=6
                indicator("Prune", overlay=true, max_lines_count=2)
                line.new(bar_index, close, bar_index + 1, close)
                """)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        let output = try PineRuntimeSession(program: program).evaluate(bars: bars([1, 2, 3, 4])).output
        XCTAssertEqual(output.lines.map(\.x1), [2, 3])
    }

    func testStepRangeBreakoutScriptCompilesAndDraws() throws {
        let program = PineCompiler.compile(source: Self.stepRangeBreakoutSource)
        XCTAssertEqual(program.diagnostics, [])
        XCTAssertEqual(program.inputSchema.inputs.count, 9)

        // Flat range long enough to confirm a zone, a bullish breakout, then a collapse
        // through the trailing stop that closes the trade.
        let closes = Array(repeating: 100.0, count: 40) + (1...10).map { 100 + Double($0) * 3 } + [60, 55, 50]
        let output = try PineRuntimeSession(program: program).evaluate(bars: bars(closes)).output

        XCTAssertEqual(output.lines.count, 1)
        XCTAssertEqual(output.labels.count, 1)
        XCTAssertEqual(output.labels.first?.style, "label.style_label_up")
        XCTAssertTrue(output.labels.first?.text.hasPrefix("Range: ") == true, "\(output.labels)")
        XCTAssertEqual(output.boxes.count, 16)
        // Breakout recolors the zone with the bullish input color.
        XCTAssertTrue(output.boxes.allSatisfy { ($0.backgroundColor ?? 0) & 0xFFFF_FF00 == 0x0AA7_9700 })
        XCTAssertEqual(output.plots.count, 2)
        XCTAssertEqual(output.fills.count, 1)
        XCTAssertEqual(output.markers.count, 2)
        XCTAssertEqual(output.markers[0].values.filter { $0 }.count, 1)
        let cells = try XCTUnwrap(output.tables.first?.cells)
        XCTAssertEqual(cells.count, 6)
        XCTAssertEqual(cells.first { $0.column == 1 && $0.row == 1 }?.text, "1")
        XCTAssertEqual(cells.first { $0.column == 1 && $0.row == 0 }?.text.hasPrefix("BULLISH/"), true)
    }
}

extension PineEngineTests {
    // Verbatim third-party script, kept with its license header.
    static let stepRangeBreakoutSource = #"""
// This work is licensed under Creative Commons Attribution-NonCommercial-ShareAlike 4.0 International
// https://creativecommons.org/licenses/by-nc-sa/4.0/
// © BigBeluga

//@version=6
indicator("Step Range Breakout & Trailing Stop [BigBeluga]", overlay=true, precision=2, max_lines_count=500, max_boxes_count=500)

// ＩＮＰＵＴＳ ――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――{
len    = input.int(20, "Structure Length", minval=1, tooltip="Lookback period to calculate the highest high and lowest low for structural boundaries.")
len1   = input.int(5, "Consolidation Bars / Length", minval=1, tooltip="Number of consecutive bars the range average must remain unchanged to confirm a valid consolidation zone.")

// Range Display Mode Input
dspMd  = input.string("Price", "Range Display Format", options=["Price", "Ticks", "Both"], group="Display Settings", tooltip="Choose whether to show the range value in Price, Ticks/Pips, or Both.")

aLen   = input.int(14, "Trailing Stop ATR Length", minval=1, tooltip="Average True Range (ATR) length used for calculating the trailing stop distance.")
aMul   = input.float(3.0, "Trailing Stop Multiplier", minval=0.1, step=0.1, tooltip="Multiplier applied to the ATR to set how far the trailing stop trails behind the price.")

wLen   = input.int(100, "Win Rate Lookback Trades", minval=1, tooltip="Number of recent closed trades to calculate the win rate percentage.")

// Custom Color Inputs
cBul   = input.color(color.rgb(10, 167, 151), "Bullish Color", group="Color Settings", tooltip="Color used for bullish breakouts, fills, and trailing stops.")
cBar   = input.color(color.red, "Bearish Color", group="Color Settings", tooltip="Color used for bearish breakouts, fills, and trailing stops.")
cMid   = input.color(color.rgb(190, 193, 204), "Mid Line Color", group="Color Settings", tooltip="Color used for the center line of the range.")
// }

// ＣＡＬＣＵＬＡＴＩＯＮＳ――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――{
hi     = ta.highest(high, len)
lo     = ta.lowest(low, len)
rAvg   = (hi + lo) / 2


// Check if the range average remains completely unchanged over the specified length bars
stbl   = ta.change(rAvg, len1) == 0

// Helper function to format the range text based on settings
f_txt(float pUp, float pLo) =>
    float pDif = pUp - pLo
    float tDif = pDif / syminfo.mintick
    string txt = ""
    if dspMd == "Price"
        txt := "Range: " + str.tostring(pDif, "#.##")
    else if dspMd == "Ticks"
        txt := "Range: " + str.tostring(tDif, "#.##") + " Ticks"
    else
        txt := "Range: " + str.tostring(pDif, "#.##") + " (" + str.tostring(tDif, "#.##") + " Ticks)"
    txt

// State variables
var float mLVal = na
var float uLVal = na
var float lLVal = na
var float tStop = na
var string stat = "SEARCHING" // "SEARCHING", "ZONE_ACTIVE", "TRAILING"
var bool isBul  = true

// Line and Label references for dynamic drawing
var line mLin   = na
var label rLab  = na
var int zBar    = na

// Box array for current active range gradient fill (historical boxes are preserved)
var box[] aBox  = array.new_box(0)
int nSli        = 16

cAtr   = ta.atr(aLen)
bool jstTr      = false

// Dashboard tracking variables
var string lDir = "None"
var float lPrc  = na
var float eCls  = na
var int[] trRes = array.new_int(0)

// State Machine Logic
if stat == "SEARCHING"
    if stbl
        mLVal := rAvg
        uLVal := hi
        lLVal := lo
        zBar  := bar_index - len1

        // Create permanent historical lines for the new zone using user input color
        mLin  := line.new(zBar, mLVal, bar_index, mLVal, width=1, color = cMid)

        // Create range label positioned at the horizontal middle of the range length
        int midBar = math.round(zBar + (bar_index - zBar) / 2.0)
        rLab  := label.new(midBar, uLVal, text=f_txt(uLVal, lLVal), color=color.new(cMid, 100), textcolor=chart.fg_color, style=label.style_label_down, size=size.small)

        // Initialize new active range boxes without deleting historical past boxes
        aBox  := array.new_box(0)
        sSiz  = (uLVal - lLVal) / nSli

        for i = 0 to nSli - 1
            bBot = lLVal + (i * sSiz)
            bTop = lLVal + ((i + 1) * sSiz)

            // Calculate gradient transparency for active consolidation state using neutral mid color
            dCtr = math.abs(i - (nSli - 1) / 2.0)
            mDst = (nSli - 1) / 2.0
            trns = math.round(110 - (90 - 50) * (dCtr / mDst))


            b = box.new(zBar, bTop, bar_index, bBot, border_color=na, bgcolor=color.new(cMid, int(trns)))
            array.push(aBox, b)

        stat := "ZONE_ACTIVE"

else if stat == "ZONE_ACTIVE"
    // Extend active lines, current gradient boxes, and keep label centered horizontally as time moves on
    line.set_x2(mLin, bar_index)

    int midBar = math.round(zBar + (bar_index - zBar) / 2.0)
    label.set_x(rLab, midBar)
    label.set_y(rLab, uLVal)
    label.set_text(rLab, f_txt(uLVal, lLVal))

    if array.size(aBox) > 0
        for b in aBox
            box.set_right(b, bar_index)

    if close > uLVal and barstate.isconfirmed
        isBul  := true
        tStop  := low - (cAtr * aMul)

        // Record dashboard metrics for breakout
        lDir   := "BULLISH"
        lPrc   := close
        eCls   := close

        // Keep label centered horizontally and pinned to lower line pointing up on bullish breakout
        label.set_x(rLab, math.round(zBar + (bar_index - zBar) / 2.0))
        label.set_y(rLab, lLVal)
        label.set_style(rLab, label.style_label_up)
        label.set_textcolor(rLab, chart.fg_color)

        // Color gradient boxes pale green on upward breakout
        if array.size(aBox) > 0
            for i = 0 to array.size(aBox) - 1
                b = array.get(aBox, i)
                dCtr = math.abs(i - (nSli - 1) / 2.0)
                mDst = (nSli - 1) / 2.0
                trns = math.round(110 - (90 - 50) * (dCtr / mDst))
                box.set_bgcolor(b, color.new(cBul, int(trns)))

        stat  := "TRAILING"
        jstTr := true

    else if close < lLVal and barstate.isconfirmed
        isBul  := false
        tStop  := high + (cAtr * aMul)

        // Record dashboard metrics for breakout
        lDir   := "BEARISH"
        lPrc   := close
        eCls   := close

        // Keep label centered horizontally and pinned to upper line pointing down on bearish breakout
        label.set_x(rLab, math.round(zBar + (bar_index - zBar) / 2.0))
        label.set_y(rLab, uLVal)
        label.set_style(rLab, label.style_label_down)
        label.set_textcolor(rLab, chart.fg_color)

        // Color gradient boxes pale red on downward breakout
        if array.size(aBox) > 0
            for i = 0 to array.size(aBox) - 1
                b = array.get(aBox, i)
                dCtr = math.abs(i - (nSli - 1) / 2.0)
                mDst = (nSli - 1) / 2.0
                trns = math.round(110 - (90 - 50) * (dCtr / mDst))
                box.set_bgcolor(b, color.new(cBar, int(trns)))

        stat  := "TRAILING"
        jstTr := true

else if stat == "TRAILING"
    if isBul
        tStop := math.max(tStop, low - (cAtr * aMul))
        if close < tStop and barstate.isconfirmed
            bool isWin = isBul ? (close > eCls) : (close < eCls)
            array.push(trRes, isWin ? 1 : 0)
            if array.size(trRes) > wLen
                array.shift(trRes)
            stat := "SEARCHING"
    else
        tStop := math.min(tStop, high + (cAtr * aMul))
        if close > tStop and barstate.isconfirmed
            bool isWin = isBul ? (close > eCls) : (close < eCls)
            array.push(trRes, isWin ? 1 : 0)
            if array.size(trRes) > wLen
                array.shift(trRes)
            stat := "SEARCHING"
// }

// ＰＬＯＴ ――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――――{
tSPlt  = stat == "TRAILING" ? tStop : na
hPlt   = stat == "TRAILING" ? hl2 : na

p1 = plot(tSPlt, title="Trailing Stop", color=isBul ? cBul : cBar, linewidth=1, style=plot.style_linebr)
p2 = plot(tSPlt + (isBul ? +cAtr : -cAtr), title="Trailing Stop Reference", color=color.new(color.white, 100), style=plot.style_linebr)
fill(p1, p2, color=color.new(isBul ? cBul : cBar, 90), title="Trailing Stop Fill")

// Circle marker at the exact starting point of the trailing stop
plotshape(jstTr ? tSPlt : na, title="Trailing Start Circle", style=shape.circle, location=location.absolute, color= color.new(isBul ? cBul : cBar, 50), size=size.small)
plotshape(jstTr ? tSPlt : na, title="Trailing Start Circle", style=shape.circle, location=location.absolute, color= color.new(isBul ? cBul : cBar, 0), size=size.tiny)

// Dashboard Table Generation
var table dash = table.new(position = position.top_right, columns = 2, rows = 3, bgcolor = color.new(color.black, 50), border_color = chart.bg_color, border_width = 2)
if barstate.islast
    // Calculate Win Rate statistics
    int tTrd  = array.size(trRes)
    int wins  = tTrd > 0 ? array.sum(trRes) : 0
    float wRat = tTrd > 0 ? (float(wins) / float(tTrd)) * 100.0 : 0.0

    // Populate Table Cells
    table.cell(dash, 0, 0, "Last Breakout", text_color=chart.fg_color, text_size=size.normal)
    table.cell(dash, 1, 0, lDir + "/" + str.tostring(lPrc, "#.##"), text_color=lDir == "BULLISH" ? cBul : cBar, text_size=size.normal)

    table.cell(dash, 0, 1, "Closed Trades", text_color=chart.fg_color, text_size=size.normal)
    table.cell(dash, 1, 1, str.tostring(tTrd), text_color=chart.fg_color, text_size=size.normal)

    table.cell(dash, 0, 2, "Win Rate", text_color=chart.fg_color, text_size=size.normal)
    table.cell(dash, 1, 2, str.tostring(wins) + "/" + str.tostring(tTrd) + " (" + str.tostring(wRat, "#.##") + "%)", text_color=wRat >= 50 ? cBul : cBar, text_size=size.normal)
// }
"""#
}
