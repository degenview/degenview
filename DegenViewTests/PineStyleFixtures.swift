import Foundation

@testable import DegenView

/// A script that draws one of every styleable output, and the output it produces.
enum PineStyleFixtures {
    static let source = """
        //@version=6
        indicator("Styles", overlay = true)
        trend = close > open ? color.green : color.red
        fast = plot(close, "Fast", color = color.blue, linewidth = 2)
        slow = plot(close - 1, "Slow", color = trend)
        plot(close + 5, color = color.orange, display = display.none)
        hline(50, "Level", color = color.gray)
        fill(fast, slow, color.new(color.blue, 80), title = "Band")
        plotshape(close > open, "Up", color = color.green)
        bgcolor(color.new(color.red, 90), title = "Zone")
        barcolor(trend, title = "Bars")
        plotcandle(open, high, low, close, "Candles", color = trend)
        """

    /// Alternating up and down bars, so a color chosen by `close > open` varies.
    static func bars(_ closes: [Double]) -> [KlineData] {
        closes.enumerated().map { index, close in
            KlineData(
                openTime: Date(timeIntervalSince1970: Double(index) * 60),
                openPrice: index % 2 == 0 ? close - 0.5 : close + 0.5,
                highPrice: close + 1, lowPrice: close - 1, closePrice: close, volume: 1)
        }
    }

    static func output(closes: [Double] = [10, 11, 10, 12]) throws -> PineVisualOutput {
        let program = PineCompiler.compile(source: source)
        precondition(program.isValid, "\(program.diagnostics)")
        return try PineRuntimeSession(program: program).evaluate(bars: bars(closes)).output
    }
}
