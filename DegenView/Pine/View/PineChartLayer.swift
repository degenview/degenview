import SwiftUI

/// Draws a Pine script's visual output into a `ChartPlot` — over the candles for
/// `overlay=true` scripts, or into `PineScriptPaneView` for the rest. Drawing is split by
/// output kind into the `PineChartLayer+…` extensions.
///
/// Per-bar series are right-aligned with `candles`: the last value belongs to the last
/// candle. Drawing objects carry absolute `bar_index` coordinates.
struct PineChartLayer {
    let pine: PineVisualOutput
    let candles: [KlineData]
    /// In its own pane there are no candles to anchor `abovebar`/`belowbar` markers to.
    var inPane = false
    /// Sizes `plotcandle()` bodies and wicks like the real candles they overlay.
    var style: ChartStyle = .default

    /// Profit and loss colors, shared with the strategy report.
    static let winColor: UInt32 = 0x26a6_9aff
    static let lossColor: UInt32 = 0xef53_50ff

    /// Above the candles, in the chart's deterministic draw order.
    func drawForeground(_ context: inout GraphicsContext, plot: ChartPlot) {
        drawFills(context: &context, plot: plot)
        drawBoxes(context: &context, plot: plot)
        drawCandles(context: &context, plot: plot)
        drawPlots(context: &context, plot: plot)
        drawLines(context: &context, plot: plot)
        drawHorizontalLines(context: &context, plot: plot)
        drawMarkers(context: &context, plot: plot)
        drawLabels(context: &context, plot: plot)
        drawStrategyTrades(context: &context, plot: plot)
    }

    /// `barcolor()` for the candle at `index`, if the script set one.
    func barColor(at index: Int) -> Color? {
        guard let series = pine.barColors.first, let rgba = visible(series.colors, at: index) else {
            return nil
        }
        return Color(pineRGBA: rgba)
    }

    // MARK: - Coordinates

    /// Value of a per-bar script series at a visible candle. Series are aligned to the
    /// right edge: the last element belongs to the last candle.
    func visible<T>(_ series: [T], at candleIndex: Int) -> T? {
        let k = series.count - candles.count + candleIndex
        return series.indices.contains(k) ? series[k] : nil
    }

    /// The same for series whose samples can be `na`: `nil` covers both "no sample" and `na`.
    func visible<T>(_ series: [T?], at candleIndex: Int) -> T? {
        let k = series.count - candles.count + candleIndex
        return series.indices.contains(k) ? series[k] : nil
    }

    /// Candle slot for an absolute Pine `bar_index` (drawing-object coordinates).
    func candleIndex(forBar bar: Int) -> Int {
        bar - (pine.barCount - candles.count)
    }

    func x(forBar bar: Int, plot: ChartPlot, slot: CGFloat) -> CGFloat {
        plot.x(forIndex: candleIndex(forBar: bar), slotWidth: slot)
    }

    func slotWidth(_ plot: ChartPlot) -> CGFloat { plot.slotWidth(forCount: candles.count) }
}
