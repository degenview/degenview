import SwiftUI

/// `bgcolor`, `plot`, `fill`, `plotcandle` and `hline`.
extension PineChartLayer {
    private static let areaOpacity = 0.3
    private static let crossHalfSize: CGFloat = 3

    /// Behind the candles: one candle-width band for each non-nil `bgcolor()` result.
    ///
    /// Backgrounds are separate from plots because they must sit behind both the candles
    /// and every foreground script visual in the chart's deterministic draw order.
    func drawBackground(_ context: inout GraphicsContext, plot: ChartPlot) {
        let slot = slotWidth(plot)
        for output in pine.backgrounds {
            let visibleColors = Array(output.colors.suffix(candles.count))
            let firstCandleIndex = candles.count - visibleColors.count
            for (offset, color) in visibleColors.enumerated() {
                guard let color else { continue }
                let x = plot.x(forIndex: firstCandleIndex + offset, slotWidth: slot)
                let band = CGRect(
                    x: x - slot / 2, y: plot.plotRect.minY, width: slot, height: plot.plotRect.height)
                context.fill(Path(band), with: .color(Color(pineRGBA: color)))
            }
        }
    }

    // MARK: - plot

    /// Draws contiguous `plot()` segments. A nil value ends the current segment so Pine
    /// gaps do not get bridged by a line. The segment ending at a bar takes that bar's color.
    func drawPlots(context: inout GraphicsContext, plot: ChartPlot) {
        let slot = slotWidth(plot)
        // A `display.none` plot still exists for `fill()` to reference; it just isn't drawn.
        for output in pine.plots where output.display.contains(.pane) {
            func point(_ i: Int) -> CGPoint? {
                guard let value = visible(output.values, at: i) else { return nil }
                return CGPoint(x: plot.x(forIndex: i, slotWidth: slot), y: plot.y(for: value))
            }
            func color(_ i: Int) -> UInt32 { visible(output.colors, at: i) ?? output.color }
            if output.style == .line || output.style == .stepline {
                drawLinePlot(output, point: point, color: color, context: &context)
            } else {
                drawDiscretePlot(output, point: point, color: color, plot: plot, slot: slot, context: &context)
            }
        }
    }

    private func drawLinePlot(
        _ output: PinePlotOutput, point: (Int) -> CGPoint?, color: (Int) -> UInt32,
        context: inout GraphicsContext
    ) {
        var i = 0
        while i < candles.count {
            guard let start = point(i) else {
                i += 1
                continue
            }
            let runColor = i + 1 < candles.count ? color(i + 1) : color(i)
            var path = Path()
            path.move(to: start)
            var j = i
            var previous = start
            while j + 1 < candles.count, let next = point(j + 1), color(j + 1) == runColor {
                if output.style == .stepline { path.addLine(to: CGPoint(x: next.x, y: previous.y)) }
                path.addLine(to: next)
                previous = next
                j += 1
            }
            if runColor & 0xFF != 0 {
                context.stroke(
                    path, with: .color(Color(pineRGBA: runColor)), lineWidth: CGFloat(output.lineWidth))
            }
            i = j == i ? i + 1 : j
        }
    }

    /// Histogram, columns, area, circles, and cross plot styles.
    private func drawDiscretePlot(
        _ output: PinePlotOutput, point: (Int) -> CGPoint?, color: (Int) -> UInt32, plot: ChartPlot,
        slot: CGFloat, context: inout GraphicsContext
    ) {
        let base = plot.y(for: output.histBase)
        if output.style == .area {
            drawArea(point: point, color: color, base: base, context: &context)
            return
        }
        let width = output.style == .columns ? max(1, slot * 0.8) : max(1, slot * 0.35)
        let size = CGFloat(max(1, output.lineWidth))
        for i in candles.indices {
            guard let p = point(i), color(i) & 0xFF != 0 else { continue }
            let tint = Color(pineRGBA: color(i))
            switch output.style {
            case .histogram, .columns:
                let rect = CGRect(
                    x: p.x - width / 2, y: min(p.y, base), width: width, height: max(1, abs(base - p.y)))
                context.fill(Path(rect), with: .color(tint))
            case .circles:
                let r = size + 1
                context.fill(
                    Path(ellipseIn: CGRect(x: p.x - r, y: p.y - r, width: 2 * r, height: 2 * r)),
                    with: .color(tint))
            default:
                let h = Self.crossHalfSize
                var cross = Path()
                cross.move(to: CGPoint(x: p.x - h, y: p.y))
                cross.addLine(to: CGPoint(x: p.x + h, y: p.y))
                cross.move(to: CGPoint(x: p.x, y: p.y - h))
                cross.addLine(to: CGPoint(x: p.x, y: p.y + h))
                context.stroke(cross, with: .color(tint), lineWidth: size)
            }
        }
    }

    /// One filled polygon per contiguous run of values, tinted by the run's last bar.
    private func drawArea(
        point: (Int) -> CGPoint?, color: (Int) -> UInt32, base: CGFloat, context: inout GraphicsContext
    ) {
        var i = 0
        while i < candles.count {
            guard let first = point(i) else {
                i += 1
                continue
            }
            var path = Path()
            path.move(to: CGPoint(x: first.x, y: base))
            path.addLine(to: first)
            var last = first
            var j = i
            while j + 1 < candles.count, let next = point(j + 1) {
                path.addLine(to: next)
                last = next
                j += 1
            }
            path.addLine(to: CGPoint(x: last.x, y: base))
            path.closeSubpath()
            context.fill(path, with: .color(Color(pineRGBA: color(j)).opacity(Self.areaOpacity)))
            i = j + 1
        }
    }

    // MARK: - plotcandle

    /// `plotcandle()`: drawn over the real candles at the same width, so a solid color
    /// recolors them and an `na` color leaves them showing.
    func drawCandles(context: inout GraphicsContext, plot: ChartPlot) {
        let slot = slotWidth(plot)
        let bodyWidth = (slot * style.candleBodyFraction).clamped(
            to: style.candleBodyMin...style.candleBodyMax)
        let wickWidth = (slot * style.wickFraction).clamped(to: style.wickMin...style.wickMax)
        for output in pine.candles where output.display.contains(.pane) {
            for i in candles.indices {
                guard let bar = visible(output.bars, at: i) else { continue }
                let x = plot.x(forIndex: i, slotWidth: slot)
                if let wick = bar.wickColor, wick & 0xFF != 0 {
                    let top = plot.y(for: bar.high)
                    let bottom = plot.y(for: bar.low)
                    let rect = CGRect(
                        x: x - wickWidth / 2, y: top, width: wickWidth, height: max(0.5, bottom - top))
                    context.fill(Path(rect), with: .color(Color(pineRGBA: wick)))
                }
                let top = plot.y(for: max(bar.open, bar.close))
                let bottom = plot.y(for: min(bar.open, bar.close))
                let body = CGRect(
                    x: x - bodyWidth / 2, y: top, width: bodyWidth,
                    height: max(style.minBodyHeight, bottom - top))
                if let fill = bar.color, fill & 0xFF != 0 {
                    context.fill(Path(body), with: .color(Color(pineRGBA: fill)))
                }
                if let border = bar.borderColor, border & 0xFF != 0 {
                    context.stroke(Path(body), with: .color(Color(pineRGBA: border)), lineWidth: 1)
                }
            }
        }
    }

    // MARK: - fill and hline

    /// `fill()` between two plots: one polygon per run of bars where both plots have
    /// values and the fill color stays the same.
    func drawFills(context: inout GraphicsContext, plot: ChartPlot) {
        let slot = slotWidth(plot)
        let plots = Dictionary(pine.plots.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
        for fill in pine.fills {
            guard let a = plots[fill.plotA], let b = plots[fill.plotB] else { continue }
            func pair(_ i: Int) -> (x: CGFloat, top: CGFloat, bottom: CGFloat)? {
                guard let top = visible(a.values, at: i), let bottom = visible(b.values, at: i) else {
                    return nil
                }
                return (plot.x(forIndex: i, slotWidth: slot), plot.y(for: top), plot.y(for: bottom))
            }
            if fill.gradients.isEmpty {
                drawFlatFill(fill, pair: pair, context: &context)
            } else {
                drawGradientFill(fill, pair: pair, plot: plot, context: &context)
            }
        }
    }

    /// One quad per bar pair, blended top-to-bottom by the bar's own price range.
    private func drawGradientFill(
        _ fill: PineFillOutput, pair: (Int) -> (x: CGFloat, top: CGFloat, bottom: CGFloat)?,
        plot: ChartPlot, context: inout GraphicsContext
    ) {
        for i in 0..<max(0, candles.count - 1) {
            guard let p = pair(i), let q = pair(i + 1), let g = visible(fill.gradients, at: i + 1) else {
                continue
            }
            var quad = Path()
            quad.move(to: CGPoint(x: p.x, y: p.top))
            quad.addLine(to: CGPoint(x: q.x, y: q.top))
            quad.addLine(to: CGPoint(x: q.x, y: q.bottom))
            quad.addLine(to: CGPoint(x: p.x, y: p.bottom))
            quad.closeSubpath()
            context.fill(
                quad,
                with: .linearGradient(
                    Gradient(colors: [Color(pineRGBA: g.topColor), Color(pineRGBA: g.bottomColor)]),
                    startPoint: CGPoint(x: q.x, y: plot.y(for: g.top)),
                    endPoint: CGPoint(x: q.x, y: plot.y(for: g.bottom))))
        }
    }

    private func drawFlatFill(
        _ fill: PineFillOutput, pair: (Int) -> (x: CGFloat, top: CGFloat, bottom: CGFloat)?,
        context: inout GraphicsContext
    ) {
        func color(_ i: Int) -> UInt32? { visible(fill.colors, at: i) }
        var i = 0
        while i + 1 < candles.count {
            guard pair(i) != nil, pair(i + 1) != nil, let runColor = color(i + 1) else {
                i += 1
                continue
            }
            var j = i + 1
            while j + 1 < candles.count, pair(j + 1) != nil, color(j + 1) == runColor { j += 1 }
            let points = (i...j).compactMap(pair)
            var path = Path()
            path.move(to: CGPoint(x: points[0].x, y: points[0].top))
            for p in points.dropFirst() { path.addLine(to: CGPoint(x: p.x, y: p.top)) }
            for p in points.reversed() { path.addLine(to: CGPoint(x: p.x, y: p.bottom)) }
            path.closeSubpath()
            context.fill(path, with: .color(Color(pineRGBA: runColor)))
            i = j
        }
    }

    func drawHorizontalLines(context: inout GraphicsContext, plot: ChartPlot) {
        for line in pine.hlines {
            let y = plot.y(for: line.value)
            var p = Path()
            p.move(to: CGPoint(x: plot.plotRect.minX, y: y))
            p.addLine(to: CGPoint(x: plot.plotRect.maxX, y: y))
            context.stroke(
                p, with: .color(Color(pineRGBA: line.color).opacity(0.8)),
                style: StrokeStyle(lineWidth: 1, dash: [4, 3]))
        }
    }
}
