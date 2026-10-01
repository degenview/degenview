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
    private func drawingValues() -> [Double] {
        guard !candles.isEmpty else { return [] }
        let visibleBars = (pine.barCount - candles.count)...(pine.barCount - 1)
        func inView(_ a: Int, _ b: Int) -> Bool { visibleBars.overlaps(min(a, b)...max(a, b)) }
        var values: [Double] = []
        for box in pine.boxes where inView(box.left, box.right) { values += [box.top, box.bottom] }
        for line in pine.lines where inView(line.x1, line.x2) { values += [line.y1, line.y2] }
        for label in pine.labels where visibleBars.contains(label.x) { values.append(label.y) }
        for polyline in pine.polylines {
            let indexes = polyline.points.map(\.index)
            guard let first = indexes.min(), let last = indexes.max(), inView(first, last) else { continue }
            values += polyline.points.map(\.price)
        }
        return values
    }
}
