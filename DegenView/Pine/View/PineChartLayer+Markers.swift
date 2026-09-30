import SwiftUI

/// `plotshape`/`plotchar` markers and strategy round trips.
extension PineChartLayer {
    /// Gap between a candle's extreme and its marker.
    private static let markerOffset: CGFloat = 8

    func drawMarkers(context: inout GraphicsContext, plot: ChartPlot) {
        let slot = slotWidth(plot)
        for marker in pine.markers where marker.display.contains(.pane) {
            for candleIndex in candles.indices where visible(marker.values, at: candleIndex) == true {
                let x = plot.x(forIndex: candleIndex, slotWidth: slot)
                let y = markerY(marker, candleIndex: candleIndex, plot: plot)
                let color = Color(pineRGBA: visible(marker.colors, at: candleIndex) ?? marker.color)
                if marker.style == .circle {
                    let r = marker.size.markerRadius
                    context.fill(
                        Path(ellipseIn: CGRect(x: x - r, y: y - r, width: 2 * r, height: 2 * r)),
                        with: .color(color))
                    continue
                }
                let glyph = marker.character ?? (marker.style.pointsDown ? "▼" : "▲")
                context.draw(Text(glyph).font(.caption).foregroundColor(color), at: CGPoint(x: x, y: y))
            }
        }
    }

    private func markerY(_ marker: PineMarkerOutput, candleIndex: Int, plot: ChartPlot) -> CGFloat {
        let offset = Self.markerOffset
        if marker.location == .absolute, let price = visible(marker.prices, at: candleIndex) {
            return plot.y(for: price)
        }
        if inPane {
            // No candles in a script pane: above/below bar pin to its edges.
            return marker.location.isBelow ? plot.plotRect.maxY - offset : plot.plotRect.minY + offset
        }
        let candle = candles[candleIndex]
        return marker.location.isBelow
            ? plot.y(for: candle.lowPrice) + offset : plot.y(for: candle.highPrice) - offset
    }

    // MARK: - Strategy trades

    /// Strategy round trips: an arrow at each entry, a dot at each exit, and a dashed
    /// connector colored by the trade's result. Prices are absolute, so this is chart-only.
    func drawStrategyTrades(context: inout GraphicsContext, plot: ChartPlot) {
        guard !inPane, let report = pine.strategy else { return }
        let slot = slotWidth(plot)
        let visibleBars = (pine.barCount - candles.count)..<pine.barCount
        let win = Color(pineRGBA: Self.winColor)
        let loss = Color(pineRGBA: Self.lossColor)
        func point(bar: Int, price: Double) -> CGPoint {
            CGPoint(x: x(forBar: bar, plot: plot, slot: slot), y: plot.y(for: price))
        }
        for trade in report.trades
        where visibleBars.contains(trade.entryBar) || visibleBars.contains(trade.exitBar) {
            let entry = point(bar: trade.entryBar, price: trade.entryPrice)
            let exit = point(bar: trade.exitBar, price: trade.exitPrice)
            let tint = trade.profit >= 0 ? win : loss
            var connector = Path()
            connector.move(to: entry)
            connector.addLine(to: exit)
            context.stroke(
                connector, with: .color(tint.opacity(0.7)), style: StrokeStyle(lineWidth: 1, dash: [3, 3]))
            drawEntryArrow(long: trade.isLong, at: entry, color: trade.isLong ? win : loss, &context)
            context.fill(
                Path(ellipseIn: CGRect(x: exit.x - 3, y: exit.y - 3, width: 6, height: 6)), with: .color(tint))
        }
        for trade in report.openTrades where visibleBars.contains(trade.entryBar) {
            let entry = point(bar: trade.entryBar, price: trade.entryPrice)
            drawEntryArrow(long: trade.isLong, at: entry, color: trade.isLong ? win : loss, &context)
        }
    }

    /// A triangle whose tip touches `point`: pointing up for longs, down for shorts.
    private func drawEntryArrow(
        long: Bool, at point: CGPoint, color: Color, _ context: inout GraphicsContext
    ) {
        let half: CGFloat = 5
        let depth = half * 1.6 * (long ? 1 : -1)
        var path = Path()
        path.move(to: point)
        path.addLine(to: CGPoint(x: point.x - half, y: point.y + depth))
        path.addLine(to: CGPoint(x: point.x + half, y: point.y + depth))
        path.closeSubpath()
        context.fill(path, with: .color(color))
    }
}
