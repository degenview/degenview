import XCTest

@testable import DegenView

/// The volume-confirmed support/resistance strategy this engine was extended to run,
/// verbatim, plus focused tests for each language feature and broker rule it needed.
private let volumeBreakoutStrategy = """
    // This Pine Script® code is subject to the terms of the Mozilla Public License 2.0 at https://mozilla.org/MPL/2.0/
    //@version=6
    strategy("Volume-Confirmed Support/Resistance Breakout Strategy", shorttitle = "Vol S/R Breakout",
         overlay = true, initial_capital = 10000,
         default_qty_type = strategy.percent_of_equity, default_qty_value = 10,
         commission_type = strategy.commission.percent, commission_value = 0.01,
         slippage = 2, pyramiding = 0)

    // Concept:
    // - Support/Resistance = confirmed swing pivots (classic structure, not repainting on live bar).
    // - Entries only trigger when price closes through a level AND volume is elevated vs its
    //   recent average, filtering out low-conviction/thin breakouts.
    // - Optional EMA trend filter to only take breakouts in the direction of the larger trend.
    // - Stops anchor to the broken structural level (old swing low/high) with a small ATR buffer;
    //   targets are ATR-multiple based.
    // - Symbol-agnostic: works on any index, stock, futures or crypto chart with real traded
    //   volume. Avoid spot FX / thin OTC symbols, where volume data isn't meaningful. Adjust
    //   initial_capital / position size to fit the instrument you run it on.

    //════════════════════════════════════
    //     ✧  INPUTS  ✧
    //════════════════════════════════════
    const string g_sr    = "✧ Support / Resistance ✧"
    const string g_vol   = "✧ Volume Confirmation ✧"
    const string g_trend = "✧ Trend Filter ✧"
    const string g_risk  = "✧ Risk Management ✧"
    const string g_date  = "✧ Backtest Range ✧"
    const string g_vis   = "✧ Visuals ✧"

    // --- Support / Resistance ---
    pivotLeft  = input.int(10, "Pivot Left Bars",  minval = 1, group = g_sr)
    pivotRight = input.int(10, "Pivot Right Bars", minval = 1, group = g_sr, tooltip = "Pivot confirms 'pivotRight' bars after it forms — inherent to swing-structure detection, not lookahead.")

    // --- Volume Confirmation ---
    volLength     = input.int(20, "Volume Average Length", minval = 1, group = g_vol)
    volMultiplier = input.float(1.5, "Volume Surge Multiplier", minval = 1.0, step = 0.1, group = g_vol, tooltip = "Needs real traded volume — works on stocks, futures, crypto, most index proxies. Spot FX volume is not meaningful.")

    // --- Trend Filter ---
    useTrendFilter = input.bool(true, "Require EMA trend filter", group = g_trend)
    emaLength      = input.int(200, "EMA Length", minval = 1, group = g_trend)

    // --- Risk Management ---
    atrLength     = input.int(14, "ATR Length", minval = 1, group = g_risk)
    atrStopMult   = input.float(1.5, "ATR Stop Multiplier", minval = 0.1, step = 0.1, group = g_risk)
    atrTargetMult = input.float(3.0, "ATR Target Multiplier", minval = 0.1, step = 0.1, group = g_risk)
    useStructStop = input.bool(true, "Use broken swing level as stop (else pure ATR)", group = g_risk)
    stopBufferAtr = input.float(0.25, "Structural Stop ATR Buffer", minval = 0.0, step = 0.05, group = g_risk)

    // --- Backtest Range ---
    startDate = input.time(timestamp("01 Jan 2018 00:00 +0000"), "Start Date", group = g_date)
    endDate   = input.time(timestamp("31 Dec 2025 00:00 +0000"), "End Date",   group = g_date)

    // --- Visuals ---
    colorCandles         = input.bool(true, "Color candles by position", group = g_vis)
    enableGlow           = input.bool(true, "Glow effect on S/R lines", group = g_vis)
    showChannelGradient  = input.bool(true, "Gradient fill inside S/R channel", group = g_vis, tooltip = "Fades from each line toward the channel midpoint, like a heat gradient rather than a flat fill.")
    bullColor            = input.color(#0097a7, "Bullish / Long Color", group = g_vis)
    bearColor            = input.color(#ff195f, "Bearish / Short Color", group = g_vis)

    //════════════════════════════════════
    //     CALCULATIONS
    //════════════════════════════════════

    // --- Support / Resistance (pivot based) ---
    float ph = ta.pivothigh(high, pivotLeft, pivotRight)
    float pl = ta.pivotlow(low, pivotLeft, pivotRight)

    var float resistance = na
    var float support    = na

    if not na(ph)
        resistance := ph
    if not na(pl)
        support := pl

    // --- Volume confirmation ---
    float avgVolume   = ta.sma(volume, volLength)
    bool  volumeSurge = volume > avgVolume * volMultiplier

    // --- Trend filter ---
    float emaVal      = ta.ema(close, emaLength)
    bool  trendUpOk   = not useTrendFilter or close > emaVal
    bool  trendDownOk = not useTrendFilter or close < emaVal

    // --- ATR for stops/targets ---
    float atrVal = ta.atr(atrLength)

    // --- Date range filter ---
    bool inRange = time >= startDate and time <= endDate

    //════════════════════════════════════
    //     ENTRY LOGIC
    //════════════════════════════════════
    bool longBreakout  = not na(resistance) and ta.crossover(close, resistance)  and volumeSurge and trendUpOk   and inRange
    bool shortBreakout = not na(support)    and ta.crossunder(close, support)    and volumeSurge and trendDownOk and inRange

    var float longStopPrice  = na
    var float longTgtPrice   = na
    var float shortStopPrice = na
    var float shortTgtPrice  = na

    if longBreakout
        strategy.entry("Long", strategy.long)
        longStopPrice := useStructStop and not na(support) ? support - atrVal * stopBufferAtr : close - atrVal * atrStopMult
        longTgtPrice  := close + atrVal * atrTargetMult
        alert("Long breakout — " + syminfo.ticker, alert.freq_once_per_bar_close)

    if shortBreakout
        strategy.entry("Short", strategy.short)
        shortStopPrice := useStructStop and not na(resistance) ? resistance + atrVal * stopBufferAtr : close + atrVal * atrStopMult
        shortTgtPrice  := close - atrVal * atrTargetMult
        alert("Short breakout — " + syminfo.ticker, alert.freq_once_per_bar_close)

    //════════════════════════════════════
    //     EXITS
    //════════════════════════════════════
    if strategy.position_size > 0
        strategy.exit("Long Exit", "Long", stop = longStopPrice, limit = longTgtPrice)

    if strategy.position_size < 0
        strategy.exit("Short Exit", "Short", stop = shortStopPrice, limit = shortTgtPrice)

    // --- Reset when flat ---
    bool flatNow  = strategy.position_size == 0
    bool flatPrev = strategy.position_size[1] != 0
    if flatNow and flatPrev
        longStopPrice  := na
        longTgtPrice   := na
        shortStopPrice := na
        shortTgtPrice  := na

    //════════════════════════════════════
    //     VISUALS
    //════════════════════════════════════

    // --- Position-based candle coloring ---
    color posColor = strategy.position_size > 0 ? bullColor : strategy.position_size < 0 ? bearColor : na

    plotcandle(open, high, low, close, "Position Coloring",
         colorCandles ? posColor : na, colorCandles ? posColor : na,
         bordercolor = colorCandles ? posColor : na,
         display = display.all - display.status_line - display.price_scale)

    // --- Support / Resistance lines, each with a double-layer glow (wide/soft outer,
    //     narrower/denser inner, solid core) rather than a single translucent copy ---
    plot(enableGlow ? resistance : na, "Resistance Glow Outer", color = color.new(bearColor, 88), linewidth = 7, style = plot.style_linebr)
    plot(enableGlow ? resistance : na, "Resistance Glow Inner", color = color.new(bearColor, 75), linewidth = 4, style = plot.style_linebr)
    p_res = plot(resistance, "Resistance", color = bearColor, style = plot.style_linebr, linewidth = 1)

    plot(enableGlow ? support : na, "Support Glow Outer", color = color.new(bullColor, 88), linewidth = 7, style = plot.style_linebr)
    plot(enableGlow ? support : na, "Support Glow Inner", color = color.new(bullColor, 75), linewidth = 4, style = plot.style_linebr)
    p_sup = plot(support, "Support", color = bullColor, style = plot.style_linebr, linewidth = 1)

    float midChannel = (not na(resistance) and not na(support)) ? (resistance + support) / 2 : na
    p_mid = plot(showChannelGradient ? midChannel : na, "Channel Midline", display = display.none, editable = false)

    fill(p_res, p_mid, resistance, midChannel, color.new(bearColor, 65), color.new(bearColor, 100), title = "Resistance Side Gradient")
    fill(p_mid, p_sup, midChannel, support,    color.new(bullColor, 100), color.new(bullColor, 65), title = "Support Side Gradient")

    plot(useTrendFilter ? emaVal : na, "EMA Trend", color = color.new(color.gray, 40))

    plotshape(longBreakout,  title = "Long Signal",  location = location.belowbar, style = shape.triangleup,   size = size.small, color = bullColor)
    plotshape(shortBreakout, title = "Short Signal", location = location.abovebar, style = shape.triangledown, size = size.small, color = bearColor)
    """

final class PineStrategyTests: XCTestCase {
    // MARK: - Helpers

    private func bar(
        _ i: Int, open: Double, high: Double, low: Double, close: Double, volume: Double = 1,
        start: Date = Date(timeIntervalSince1970: 1_577_836_800)
    ) -> KlineData {
        .init(
            openTime: start.addingTimeInterval(Double(i) * 3600), openPrice: open, highPrice: high,
            lowPrice: low, closePrice: close, volume: volume)
    }

    private func closes(_ values: [Double]) -> [KlineData] {
        values.enumerated().map { bar($0, open: $1, high: $1 + 1, low: $1 - 1, close: $1) }
    }

    private func run(
        _ source: String, bars: [KlineData], symbol: PineSymbolInfo = PineSymbolInfo()
    ) throws -> PineRuntimeResult {
        let program = PineCompiler.compile(source: source)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        return try PineRuntimeSession(program: program, symbol: symbol).evaluate(bars: bars)
    }

    private func numbers(_ plot: PinePlotOutput) -> [Double?] { plot.values }

    /// A rising market with regular swings and a volume surge on every other bar, so the
    /// strategy finds pivots, clears its EMA filter, and breaks out repeatedly.
    private func swingingMarket(count: Int) -> [KlineData] {
        var previous = 100.0
        return (0..<count).map { i in
            let close = 100 + 0.08 * Double(i) + 10 * sin(Double(i) / 8)
            defer { previous = close }
            return bar(
                i, open: previous, high: max(previous, close) + 0.5, low: min(previous, close) - 0.5,
                close: close, volume: i % 2 == 0 ? 400 : 100)
        }
    }

    // MARK: - The user's script

    func testStrategyScriptCompilesCleanly() throws {
        let program = PineCompiler.compile(source: volumeBreakoutStrategy)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        XCTAssertEqual(program.declaration.type, .strategy)
        XCTAssertEqual(program.declaration.shortTitle, "Vol S/R Breakout")
        XCTAssertTrue(program.declaration.overlay)

        let settings = try XCTUnwrap(program.declaration.strategy)
        XCTAssertEqual(settings.initialCapital, 10_000)
        XCTAssertEqual(settings.quantityType, .percentOfEquity)
        XCTAssertEqual(settings.quantityValue, 10)
        XCTAssertEqual(settings.commissionType, .percent)
        XCTAssertEqual(settings.commissionValue, 0.01)
        XCTAssertEqual(settings.slippage, 2)
        XCTAssertEqual(settings.pyramiding, 0)
    }

    func testStrategyScriptInputsCarryGroupsAndTimes() throws {
        let schema = PineCompiler.compile(source: volumeBreakoutStrategy).inputSchema
        XCTAssertEqual(schema.inputs.count, 18)
        XCTAssertEqual(schema.inputs.first?.group, "✧ Support / Resistance ✧")
        let start = try XCTUnwrap(schema.inputs.first { $0.id == "startDate" })
        XCTAssertEqual(start.type, .time)
        XCTAssertEqual(start.defaultValue, .int(1_514_764_800_000))
        XCTAssertEqual(start.group, "✧ Backtest Range ✧")
        let end = try XCTUnwrap(schema.inputs.first { $0.id == "endDate" })
        XCTAssertEqual(end.defaultValue, .int(1_767_139_200_000))
    }

    func testStrategyScriptRunsAndTrades() throws {
        let bars = swingingMarket(count: 700)
        let result = try run(
            volumeBreakoutStrategy, bars: bars,
            symbol: PineSymbolInfo(ticker: "BTCUSDT", tickerID: "Binance:BTCUSDT"))
        let output = result.output

        let report = try XCTUnwrap(output.strategy)
        XCTAssertFalse(report.trades.isEmpty)
        XCTAssertEqual(report.equity.count, bars.count)
        for trade in report.trades {
            XCTAssertGreaterThan(trade.quantity, 0)
            XCTAssertGreaterThanOrEqual(trade.exitBar, trade.entryBar)
        }

        XCTAssertEqual(output.candles.count, 1)
        XCTAssertEqual(output.candles[0].bars.count, bars.count)
        XCTAssertTrue(output.candles[0].bars.contains { $0 != nil })
        XCTAssertEqual(output.candles[0].display, 3)

        // The channel midline is display.none yet must stay so the gradient fills can use it.
        let midline = try XCTUnwrap(output.plots.first { $0.title == "Channel Midline" })
        XCTAssertEqual(midline.display, 0)
        XCTAssertEqual(output.fills.count, 2)
        XCTAssertTrue(output.fills.allSatisfy { !$0.gradients.isEmpty })
        XCTAssertTrue(output.fills[0].gradients.contains { $0 != nil })

        let alert = try XCTUnwrap(output.alerts.first)
        XCTAssertTrue(alert.message.hasSuffix("breakout — BTCUSDT"), alert.message)
    }

    func testStrategyScriptHonoursTheDateRange() throws {
        // Every bar sits before the default 2018 start date's end, so shifting the start
        // past the data must silence the strategy entirely.
        let program = PineCompiler.compile(source: volumeBreakoutStrategy)
        let bars = swingingMarket(count: 700)
        let late = PineTimestamp.make(year: 2030, month: 1, day: 1)
        let output = try PineRuntimeSession(program: program, inputs: ["startDate": .int(late)])
            .evaluate(bars: bars).output
        XCTAssertEqual(output.strategy?.trades.count, 0)
        XCTAssertTrue(output.alerts.isEmpty)
    }

    func testStrategyScriptFinishesQuicklyOnAThousandBars() throws {
        let program = PineCompiler.compile(source: volumeBreakoutStrategy)
        let bars = swingingMarket(count: 1500)
        let start = Date()
        XCTAssertNoThrow(try PineRuntimeSession(program: program).evaluate(bars: bars))
        XCTAssertLessThan(Date().timeIntervalSince(start), 8)
    }

    // MARK: - Language

    func testConstQualifierAndConstantGroups() throws {
        let program = PineCompiler.compile(
            source: """
                //@version=6
                indicator("Const")
                const string g = "Section"
                const int n = 3
                len = input.int(n, "Length", group = g)
                plot(len)
                """)
        XCTAssertTrue(program.isValid, "\(program.diagnostics)")
        XCTAssertEqual(program.inputSchema.inputs.first?.group, "Section")
        XCTAssertEqual(program.inputSchema.inputs.first?.defaultValue, .int(3))
    }

    func testSwitchWithAndWithoutSubject() throws {
        let output = try run(
            """
            //@version=6
            indicator("Switch")
            band = switch
                close > 5 => 1
                close > 2 => 2
                => 3
            named = switch band
                1 => 10
                2 => 20
                => 30
            plot(band)
            plot(named)
            """, bars: closes([1, 3, 9])
        ).output
        XCTAssertEqual(numbers(output.plots[0]), [3, 2, 1])
        XCTAssertEqual(numbers(output.plots[1]), [30, 20, 10])
    }

    func testWhileLoopWithBreak() throws {
        let output = try run(
            """
            //@version=6
            indicator("While")
            var total = 0
            i = 0
            while i < 10
                if i == 3
                    break
                total += 1
                i += 1
            plot(total)
            """, bars: closes([1, 1])
        ).output
        XCTAssertEqual(numbers(output.plots[0]), [3, 6])
    }

    func testTupleDestructuringAndIfExpression() throws {
        let output = try run(
            """
            //@version=6
            indicator("Tuple")
            [a, b] = [close, open]
            size = if a > 5
                2
            else
                1
            plot(a - b + size)
            """, bars: closes([1, 9])
        ).output
        XCTAssertEqual(numbers(output.plots[0]), [1, 2])
    }

    func testDisplayConstantsDoIntegerMath() throws {
        let output = try run(
            """
            //@version=6
            indicator("Display", overlay=true)
            plot(close, display = display.all - display.status_line - display.price_scale)
            plot(close, display = display.none)
            """, bars: closes([1])
        ).output
        XCTAssertEqual(output.plots.map(\.display), [3, 0])
    }

    func testTimestampForms() throws {
        let expected = 1_514_764_800_000
        XCTAssertEqual(PineTimestamp.parse("01 Jan 2018 00:00 +0000"), expected)
        XCTAssertEqual(PineTimestamp.parse("2018-01-01T00:00:00Z"), expected)
        XCTAssertEqual(PineTimestamp.parse("2018-01-01"), expected)
        XCTAssertEqual(PineTimestamp.parse("Jan 01 2018 02:00 GMT+2"), expected)
        XCTAssertEqual(PineTimestamp.parse("31 Dec 2025 00:00 +0000"), 1_767_139_200_000)
        XCTAssertEqual(PineTimestamp.evaluate(positional: [.int(2018), .int(1), .int(1)]), expected)
        XCTAssertEqual(
            PineTimestamp.evaluate(positional: [
                .string("GMT+2"), .int(2018), .int(1), .int(1), .int(2),
            ]), expected)
        XCTAssertNil(PineTimestamp.parse("not a date"))
        XCTAssertNil(PineTimestamp.evaluate(positional: [.int(2018), .na, .int(1)]))

        let parts = PineTimestamp.components(milliseconds: expected)
        XCTAssertEqual([parts.year, parts.month, parts.day, parts.weekday], [2018, 1, 1, 2])
        // Months roll over like Pine's: month 13 of 2017 is January 2018.
        XCTAssertEqual(PineTimestamp.make(year: 2017, month: 13, day: 1), expected)
    }

    func testTimestampAndTimePartsAtRuntime() throws {
        let output = try run(
            """
            //@version=6
            indicator("Time")
            plot(timestamp(2020, 1, 1))
            plot(year)
            plot(dayofweek)
            plot(hour(time))
            """, bars: closes([1])
        ).output
        XCTAssertEqual(numbers(output.plots[0]), [1_577_836_800_000])
        XCTAssertEqual(numbers(output.plots[1]), [2020])
        XCTAssertEqual(numbers(output.plots[2]), [4])  // Wednesday
        XCTAssertEqual(numbers(output.plots[3]), [0])
    }

    func testPivotsConfirmAfterTheirRightBars() throws {
        let highs: [Double] = [1, 2, 3, 10, 3, 2, 1, 1]
        let bars = highs.enumerated().map {
            bar($0, open: $1, high: $1, low: $1 - 1, close: $1)
        }
        let output = try run(
            """
            //@version=6
            indicator("Pivot")
            plot(ta.pivothigh(high, 2, 2))
            plot(ta.pivotlow(2, 2))
            """, bars: bars
        ).output
        // The 10 is bar 3; two more bars must pass before it is known.
        XCTAssertEqual(numbers(output.plots[0]).map { $0 ?? -1 }, [-1, -1, -1, -1, -1, 10, -1, -1])
        XCTAssertNil(numbers(output.plots[1])[4])
    }

    func testGradientFillKeepsPerBarValues() throws {
        let output = try run(
            """
            //@version=6
            indicator("Fill", overlay=true)
            top = plot(high)
            bottom = plot(low, display = display.none)
            fill(top, bottom, high, low, color.new(color.red, 50), color.new(color.red, 100))
            """, bars: closes([5, 6])
        ).output
        let fill = try XCTUnwrap(output.fills.first)
        XCTAssertEqual(fill.gradients.count, 2)
        XCTAssertEqual(fill.gradients[1]?.top, 7)
        XCTAssertEqual(fill.gradients[1]?.bottom, 5)
        XCTAssertEqual((fill.gradients[1]?.bottomColor ?? 1) & 0xFF, 0)
    }

    func testPlotStylesMap() throws {
        let output = try run(
            """
            //@version=6
            indicator("Styles")
            plot(close, style = plot.style_histogram, histbase = 5)
            plot(close, style = plot.style_linebr)
            plot(close, style = plot.style_circles)
            """, bars: closes([1])
        ).output
        XCTAssertEqual(output.plots.map(\.style), [.histogram, .line, .circles])
        XCTAssertEqual(output.plots[0].histBase, 5)
    }

    func testAlertsFollowTheirFrequency() throws {
        let output = try run(
            """
            //@version=6
            indicator("Alerts")
            alert("bar " + syminfo.ticker, alert.freq_once_per_bar_close)
            alertcondition(close > 2, "Big", "over two")
            """, bars: closes([1, 3]), symbol: PineSymbolInfo(ticker: "ETH")
        ).output
        XCTAssertEqual(output.alerts.map(\.message), ["bar ETH", "bar ETH", "over two"])
    }

    func testExtraIndicators() throws {
        let output = try run(
            """
            //@version=6
            indicator("TA")
            plot(ta.stdev(close, 3))
            plot(ta.wma(close, 3))
            plot(ta.rising(close, 2) ? 1 : 0)
            plot(ta.barssince(close == 3))
            plot(ta.cum(close))
            """, bars: closes([1, 2, 3, 4])
        ).output
        XCTAssertEqual(try XCTUnwrap(numbers(output.plots[0])[2]), (2.0 / 3).squareRoot(), accuracy: 1e-9)
        XCTAssertEqual(try XCTUnwrap(numbers(output.plots[1])[2]), (1 + 4 + 9) / 6, accuracy: 1e-9)
        XCTAssertEqual(numbers(output.plots[2]), [0, 0, 1, 1])
        XCTAssertEqual(numbers(output.plots[3])[2...], [0, 1])
        XCTAssertEqual(numbers(output.plots[4]), [1, 3, 6, 10])
    }

    func testMathStringAndArrayExtras() throws {
        let output = try run(
            """
            //@version=6
            indicator("Extras")
            plot(math.avg(2, 4, 9))
            plot(math.todegrees(math.pi))
            plot(str.length("héllo"))
            plot(str.contains("breakout", "out") ? 1 : 0)
            plot(str.tonumber("2.5"))
            plot(str.startswith(str.upper("abc"), "AB") ? 1 : 0)
            plot(str.length(str.format("{0} of {1,number,#.##}", 3, 2.5)))
            plot(str.length(str.substring("abcdef", 1, 4)))
            plot(str.length(str.replace_all("a-b-c", "-", "")))
            arr = array.from(3, 1, 2)
            array.sort(arr)
            plot(array.get(arr, 0))
            array.sort(arr, order.descending)
            plot(array.get(arr, 0))
            copy = array.copy(arr)
            array.reverse(copy)
            plot(array.get(copy, 0))
            plot(array.size(array.slice(arr, 1, 3)))
            plot(str.length(array.join(arr, "-")))
            """, bars: closes([1])
        ).output
        XCTAssertEqual(
            output.plots.map { $0.values[0] ?? -1 },
            [5, 180, 5, 1, 2.5, 1, 8, 3, 3, 1, 3, 1, 2, 5])
    }

    func testStrategyCallsRequireAStrategy() throws {
        let program = PineCompiler.compile(
            source: "//@version=6\nindicator(\"X\")\nstrategy.entry(\"L\", strategy.long)")
        XCTAssertThrowsError(try PineRuntimeSession(program: program).evaluate(bars: closes([1])))
    }

    // MARK: - Strategy through the VM

    func testEntryAndCloseThroughScript() throws {
        let output = try run(
            """
            //@version=6
            strategy("T", overlay=true, initial_capital=10000, default_qty_type=strategy.fixed, default_qty_value=1)
            if bar_index == 1
                strategy.entry("L", strategy.long)
            if bar_index == 3
                strategy.close("L")
            plot(strategy.position_size)
            plot(strategy.position_size[1])
            """, bars: closes([10, 11, 12, 13, 14, 15])
        ).output
        // Entry queued on bar 1 fills at bar 2's open; the close queued on bar 3 at bar 4's.
        XCTAssertEqual(numbers(output.plots[0]), [0, 0, 1, 1, 0, 0])
        XCTAssertEqual(numbers(output.plots[1]).dropFirst().map { $0 ?? -1 }, [0, 0, 1, 1, 0])
        let trade = try XCTUnwrap(output.strategy?.trades.first)
        XCTAssertEqual(trade.entryPrice, 12)
        XCTAssertEqual(trade.exitPrice, 14)
        XCTAssertEqual(trade.profit, 2)
        XCTAssertEqual(output.strategy?.finalEquity, 10_002)
    }

    func testRealtimeTicksDoNotLeakOrders() throws {
        let program = PineCompiler.compile(
            source: """
                //@version=6
                strategy("T", overlay=true, default_qty_value=1)
                if close > 100
                    strategy.entry("L", strategy.long)
                plot(strategy.position_size)
                """)
        let session = PineRuntimeSession(program: program)
        for i in 0..<2 {
            try session.execute(
                .init(candle: bar(i, open: 90, high: 91, low: 89, close: 90), phase: .historical))
        }
        // A tick pushes the open bar above 100 (which queues an entry), then reverts.
        try session.execute(
            .init(
                candle: bar(2, open: 90, high: 111, low: 89, close: 110),
                phase: .realtimeTick(isNew: true)))
        try session.execute(
            .init(
                candle: bar(2, open: 90, high: 111, low: 89, close: 90),
                phase: .realtimeTick(isNew: false)))
        try session.execute(
            .init(
                candle: bar(2, open: 90, high: 111, low: 89, close: 90),
                phase: .realtimeClose(isNew: false)))
        try session.execute(
            .init(
                candle: bar(3, open: 90, high: 91, low: 89, close: 90),
                phase: .realtimeTick(isNew: true)))
        let report = try XCTUnwrap(session.output().strategy)
        XCTAssertTrue(report.openTrades.isEmpty)
        XCTAssertTrue(report.trades.isEmpty)
    }
}

// MARK: - Broker

final class PineBrokerEmulatorTests: XCTestCase {
    private func bar(_ i: Int, _ open: Double, _ high: Double, _ low: Double, _ close: Double) -> KlineData {
        .init(
            openTime: Date(timeIntervalSince1970: 1_577_836_800 + Double(i) * 3600), openPrice: open,
            highPrice: high, lowPrice: low, closePrice: close, volume: 1)
    }

    private func broker(
        quantity: Double = 1, type: PineStrategySettings.QuantityType = .fixed,
        commission: Double = 0, commissionType: PineStrategySettings.CommissionType = .percent,
        slippage: Int = 0, pyramiding: Int = 1, capital: Double = 10_000
    ) -> PineBrokerEmulator {
        PineBrokerEmulator(
            settings: .init(
                initialCapital: capital, quantityType: type, quantityValue: quantity,
                commissionType: commissionType, commissionValue: commission, slippage: slippage,
                pyramiding: pyramiding))
    }

    private func entry(
        _ id: String, long: Bool = true, quantity: Double? = nil, limit: Double? = nil, stop: Double? = nil
    ) -> PineBrokerEmulator.Order {
        .init(kind: .entry, id: id, isLong: long, quantity: quantity, limit: limit, stop: stop)
    }

    private func exit(
        _ id: String = "X", from: String? = nil, stop: Double? = nil, limit: Double? = nil
    ) -> PineBrokerEmulator.Order {
        .init(kind: .exit, id: id, limit: limit, stop: stop, fromEntry: from)
    }

    func testMarketEntryFillsAtNextOpen() {
        var b = broker()
        b.place(entry("L"))
        XCTAssertEqual(b.positionSize, 0)
        b.process(bar: bar(1, 100, 105, 95, 102), barIndex: 1, mintick: 0.5)
        XCTAssertEqual(b.positionSize, 1)
        XCTAssertEqual(b.averagePrice, 100)
    }

    func testSlippageIsAdverseOnMarketAndStopFillsOnly() {
        var b = broker(slippage: 2)
        b.place(entry("L"))
        b.process(bar: bar(1, 100, 101, 99, 100), barIndex: 1, mintick: 0.5)
        XCTAssertEqual(b.averagePrice, 101)  // bought 2 ticks high

        b.place(exit(limit: 110))
        b.process(bar: bar(2, 105, 112, 104, 111), barIndex: 2, mintick: 0.5)
        XCTAssertEqual(b.closedTrades.first?.exitPrice, 110)  // limit: no slippage

        var s = broker(slippage: 2)
        s.place(entry("L"))
        s.process(bar: bar(1, 100, 101, 99, 100), barIndex: 1, mintick: 0.5)
        s.place(exit(stop: 95))
        s.process(bar: bar(2, 99, 99.5, 90, 92), barIndex: 2, mintick: 0.5)
        XCTAssertEqual(s.closedTrades.first?.exitPrice, 94)  // stop: 2 ticks worse
    }

    func testCommissionModes() {
        var percent = broker(quantity: 10, commission: 0.1)
        percent.place(entry("L"))
        percent.process(bar: bar(1, 100, 100, 100, 100), barIndex: 1, mintick: 0.01)
        percent.place(.init(kind: .closeAll, id: "all"))
        percent.process(bar: bar(2, 110, 110, 110, 110), barIndex: 2, mintick: 0.01)
        // 10 × (110 − 100) − 1.00 entry − 1.10 exit
        XCTAssertEqual(percent.closedTrades.first?.profit ?? 0, 97.9, accuracy: 1e-9)

        var perOrder = broker(quantity: 10, commission: 2, commissionType: .cashPerOrder)
        perOrder.place(entry("L"))
        perOrder.process(bar: bar(1, 100, 100, 100, 100), barIndex: 1, mintick: 0.01)
        perOrder.place(.init(kind: .closeAll, id: "all"))
        perOrder.process(bar: bar(2, 110, 110, 110, 110), barIndex: 2, mintick: 0.01)
        XCTAssertEqual(perOrder.closedTrades.first?.profit ?? 0, 96, accuracy: 1e-9)

        var perContract = broker(quantity: 10, commission: 0.5, commissionType: .cashPerContract)
        perContract.place(entry("L"))
        perContract.process(bar: bar(1, 100, 100, 100, 100), barIndex: 1, mintick: 0.01)
        perContract.place(.init(kind: .closeAll, id: "all"))
        perContract.process(bar: bar(2, 110, 110, 110, 110), barIndex: 2, mintick: 0.01)
        XCTAssertEqual(perContract.closedTrades.first?.profit ?? 0, 90, accuracy: 1e-9)
    }

    func testIntrabarPathDecidesWhichExitFillsFirst() {
        // Open nearer the low → path O→L→H: the stop is reached before the limit.
        var stopFirst = broker()
        stopFirst.place(entry("L"))
        stopFirst.process(bar: bar(1, 100, 100, 100, 100), barIndex: 1, mintick: 0.01)
        stopFirst.place(exit(stop: 95, limit: 105))
        stopFirst.process(bar: bar(2, 100, 106, 94, 100), barIndex: 2, mintick: 0.01)
        XCTAssertEqual(stopFirst.closedTrades.first?.exitPrice, 95)

        // Open nearer the high → path O→H→L: the limit is reached first.
        var limitFirst = broker()
        limitFirst.place(entry("L"))
        limitFirst.process(bar: bar(1, 100, 100, 100, 100), barIndex: 1, mintick: 0.01)
        limitFirst.place(exit(stop: 95, limit: 105))
        limitFirst.process(bar: bar(2, 100, 108, 90, 100), barIndex: 2, mintick: 0.01)
        XCTAssertEqual(limitFirst.closedTrades.first?.exitPrice, 105)
    }

    func testGapThroughStopFillsAtTheOpen() {
        var b = broker()
        b.place(entry("L"))
        b.process(bar: bar(1, 100, 100, 100, 100), barIndex: 1, mintick: 0.01)
        b.place(exit(stop: 95))
        b.process(bar: bar(2, 90, 92, 88, 91), barIndex: 2, mintick: 0.01)
        XCTAssertEqual(b.closedTrades.first?.exitPrice, 90)
    }

    func testStopEntryAndLimitEntry() {
        var stop = broker()
        stop.place(entry("Break", stop: 105))
        stop.place(entry("Dip", limit: 95))
        // The low stays above the limit, so only the stop entry triggers.
        stop.process(bar: bar(1, 100, 106, 97, 100), barIndex: 1, mintick: 0.01)
        XCTAssertEqual(stop.openTrades.map(\.entryID), ["Break"])
        XCTAssertEqual(stop.openTrades.first?.entryPrice, 105)

        var limit = broker()
        limit.place(entry("Dip", limit: 95))
        limit.process(bar: bar(1, 100, 101, 94, 100), barIndex: 1, mintick: 0.01)
        XCTAssertEqual(limit.openTrades.first?.entryPrice, 95)

        // A limit below the open never fills above the market.
        var better = broker()
        better.place(entry("Dip", limit: 105))
        better.process(bar: bar(1, 100, 101, 99, 100), barIndex: 1, mintick: 0.01)
        XCTAssertEqual(better.openTrades.first?.entryPrice, 100)
    }

    func testReversalClosesThenOpensOppositeSide() {
        var b = broker()
        b.place(entry("L"))
        b.process(bar: bar(1, 100, 100, 100, 100), barIndex: 1, mintick: 0.01)
        b.place(entry("S", long: false))
        b.process(bar: bar(2, 90, 90, 90, 90), barIndex: 2, mintick: 0.01)
        XCTAssertEqual(b.positionSize, -1)
        XCTAssertEqual(b.closedTrades.count, 1)
        XCTAssertEqual(b.closedTrades[0].profit, -10)
        XCTAssertEqual(b.closedTrades[0].exitID, "S")
    }

    func testPyramidingLimits() {
        for (limit, expected) in [(0, 1), (1, 1), (2, 2), (3, 3)] {
            var b = broker(pyramiding: limit)
            for (i, id) in ["A", "B", "C", "D"].enumerated() {
                b.place(entry(id))
                b.process(bar: bar(i + 1, 100, 100, 100, 100), barIndex: i + 1, mintick: 0.01)
            }
            XCTAssertEqual(b.openTrades.count, expected, "pyramiding \(limit)")
        }
    }

    func testPercentOfEquitySizing() {
        var b = broker(quantity: 10, type: .percentOfEquity, capital: 10_000)
        b.place(entry("L"))
        b.process(bar: bar(1, 100, 100, 100, 100), barIndex: 1, mintick: 0.01)
        XCTAssertEqual(b.positionSize, 10, accuracy: 1e-9)  // 10% of 10 000 at 100

        var cash = broker(quantity: 500, type: .cash)
        cash.place(entry("L"))
        cash.process(bar: bar(1, 50, 50, 50, 50), barIndex: 1, mintick: 0.01)
        XCTAssertEqual(cash.positionSize, 10, accuracy: 1e-9)
    }

    func testExitFromEntryOnlyClosesThatEntry() {
        var b = broker(pyramiding: 2)
        b.place(entry("A"))
        b.place(entry("B"))
        b.process(bar: bar(1, 100, 100, 100, 100), barIndex: 1, mintick: 0.01)
        XCTAssertEqual(b.openTrades.count, 2)
        b.place(exit(from: "A", limit: 105))
        b.process(bar: bar(2, 100, 106, 99, 105), barIndex: 2, mintick: 0.01)
        XCTAssertEqual(b.closedTrades.map(\.entryID), ["A"])
        XCTAssertEqual(b.openTrades.map(\.entryID), ["B"])
    }

    func testCloseAllAndCancel() {
        var b = broker(pyramiding: 2)
        b.place(entry("A"))
        b.place(entry("B"))
        b.cancel(id: "B")
        b.process(bar: bar(1, 100, 100, 100, 100), barIndex: 1, mintick: 0.01)
        XCTAssertEqual(b.openTrades.map(\.entryID), ["A"])
        b.place(.init(kind: .closeAll, id: "all"))
        b.process(bar: bar(2, 101, 101, 101, 101), barIndex: 2, mintick: 0.01)
        XCTAssertEqual(b.positionSize, 0)
        XCTAssertEqual(b.netProfit, 1)
    }

    func testStaleExitDoesNotAttachToTheNextEntry() {
        var b = broker()
        b.place(entry("L"))
        b.process(bar: bar(1, 100, 100, 100, 100), barIndex: 1, mintick: 0.01)
        b.place(exit(from: "L", stop: 95))
        b.process(bar: bar(2, 96, 96, 90, 92), barIndex: 2, mintick: 0.01)  // stopped out
        XCTAssertEqual(b.positionSize, 0)
        // A later entry with the same id must not inherit the old stop.
        b.place(entry("L"))
        b.process(bar: bar(3, 92, 92, 80, 85), barIndex: 3, mintick: 0.01)
        XCTAssertEqual(b.positionSize, 1)
    }

    func testEquityMarksOpenPositionsToMarket() {
        var b = broker(quantity: 2, capital: 1000)
        b.place(entry("L"))
        b.process(bar: bar(1, 100, 100, 100, 100), barIndex: 1, mintick: 0.01)
        b.recordEquity(close: 110)
        XCTAssertEqual(b.equity(at: 110), 1020)
        let report = b.report()
        XCTAssertEqual(report.equity, [1020])
        XCTAssertEqual(report.openProfit, 20)
    }
}
