import SwiftUI

extension PineChartLayer {
    /// Vertical domain for a script pane: every value-anchored output in view, padded
    /// like `ChartPlot.priceRange` so nothing touches the frame.
    func valueRange(padding: CGFloat) -> (min: Double, max: Double) {
        let values = (seriesValues() + drawingValues() + pine.hlines.map(\.value)).filter(\.isFinite)
        guard let low = values.min(), let high = values.max() else { return (0, 1) }
        if low == high {
            let inset = max(abs(low) * 0.01, 0.01)
            return (low - inset, high + inset)
        }
        let inset = (high - low) * Double(padding)
        return (low - inset, high + inset)
    }

    /// Plot and plotcandle values shown in the pane, plus the baseline of filled styles.
    private func seriesValues() -> [Double] {
        var values: [Double] = []
        for output in pine.plots where output.display.contains(.pane) {
            values += output.values.suffix(candles.count).compactMap { $0 }
            if output.style == .histogram || output.style == .columns || output.style == .area {
                values.append(output.histBase)
            }
        }
        for output in pine.candles where output.display.contains(.pane) {
            for bar in output.bars.suffix(candles.count).compactMap({ $0 }) { values += [bar.high, bar.low] }
        }
        return values
    }

    /// Boxes, lines and labels that overlap the visible bars.
    func drawingValues(extendingBy futureBars: Int = 0) -> [Double] {
        guard !candles.isEmpty else { return [] }
        let visibleBars = (pine.barCount - candles.count)...(pine.barCount - 1 + futureBars)
        func inView(_ a: Int, _ b: Int) -> Bool { visibleBars.overlaps(min(a, b)...max(a, b)) }
        var values: [Double] = []
        for box in pine.boxes where box.isComplete && inView(box.left, box.right) { values += [box.top, box.bottom] }
        for line in pine.lines where line.isComplete && inView(line.x1, line.x2) { values += [line.y1, line.y2] }
        for label in pine.labels where label.isComplete && visibleBars.contains(label.x) { values.append(label.y) }
        for polyline in pine.polylines {
            let indexes = polyline.points.map(\.index)
            guard let first = indexes.min(), let last = indexes.max(), inView(first, last) else { continue }
            values += polyline.points.map(\.price)
        }
        return values
    }
}

extension PineChartLayer {
    /// What overlay scripts add to the chart's own geometry: bar slots past the last candle for drawings
    /// anchored beyond the latest bar, and prices the vertical scale has to reach.
    struct OverlayExtent: Equatable {
        var futureBars = 0
        var values: [Double] = []
        static let none = OverlayExtent()
    }

    /// Farthest future slots the chart reserves, as a share of the candles shown, so a far-off drawing
    /// never squeezes the candles into a sliver.
    static let maximumFutureShare = 1.0

    /// The extent of every overlay script in `outputs` over `candles`. Scripts that draw in their own pane
    /// (`overlay=false`) add nothing: they have a value axis of their own.
    static func overlayExtent(
        of outputs: [PineVisualOutput], candles: [KlineData], style: ChartStyle
    ) -> OverlayExtent {
        guard !candles.isEmpty else { return .none }
        var extent = OverlayExtent()
        for output in outputs where output.overlay {
            let layer = PineChartLayer(pine: output, candles: candles, style: style)
            extent.futureBars = max(extent.futureBars, layer.futureBars())
        }
        for output in outputs where output.overlay {
            let layer = PineChartLayer(pine: output, candles: candles, style: style)
            extent.values += layer.overlayValues(futureBars: extent.futureBars)
        }
        return extent
    }

    /// Slots past the last candle that this script's drawings reach, capped (see `maximumFutureShare`).
    /// Drawings placed by time (`xloc.bar_time`) are mapped by the renderer and not counted.
    func futureBars() -> Int {
        guard !candles.isEmpty else { return 0 }
        let lastBar = pine.barCount - 1
        var bars: [Int] = []
        for box in pine.boxes where box.isComplete && !box.timeAnchored { bars += [box.left, box.right] }
        for line in pine.lines where line.isComplete && !line.timeAnchored { bars += [line.x1, line.x2] }
        for label in pine.labels where label.isComplete && !label.timeAnchored { bars.append(label.x) }
        let reach = bars.filter(PineDrawingCoordinate.isKnown).max() ?? lastBar
        let cap = Int((Double(candles.count) * Self.maximumFutureShare).rounded(.down))
        return max(0, min(reach - lastBar, cap))
    }

    /// Prices of what the script draws in the main pane over the candles shown, and over `futureBars` more.
    func overlayValues(futureBars: Int) -> [Double] {
        var values: [Double] = []
        for output in pine.plots where output.display.contains(.pane) {
            values += output.values.suffix(candles.count).compactMap { $0 }
        }
        for output in pine.candles where output.display.contains(.pane) {
            for bar in output.bars.suffix(candles.count).compactMap({ $0 }) { values += [bar.high, bar.low] }
        }
        values += pine.hlines.map(\.value)
        values += drawingValues(extendingBy: futureBars)
        return values.filter(\.isFinite)
    }
}
