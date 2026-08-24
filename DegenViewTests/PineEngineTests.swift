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
}
