import SwiftUI

/// Draws a Pine script's visual output into a `ChartPlot` — over the candles for
/// `overlay=true` scripts, or into `PineScriptPaneView` for the rest.
///
/// Per-bar series are right-aligned with `candles`: the last value belongs to the last
/// candle. Drawing objects carry absolute `bar_index` coordinates.
struct PineChartLayer {
    let pine: PineVisualOutput
    let candles: [KlineData]
    /// In its own pane there are no candles to anchor `abovebar`/`belowbar` markers to.
    var inPane = false

    /// Behind the candles: `bgcolor()` bands.
    func drawBackground(_ context: inout GraphicsContext, plot: ChartPlot) {
        drawScriptBackgroundBands(context: &context, plot: plot)
    }

    /// Above the candles, in the chart's deterministic draw order.
    func drawForeground(_ context: inout GraphicsContext, plot: ChartPlot) {
        drawScriptFills(context: &context, plot: plot)
        drawScriptBoxes(context: &context, plot: plot)
        drawScriptPlots(context: &context, plot: plot)
        drawScriptLines(context: &context, plot: plot)
        drawScriptHorizontalLines(context: &context, plot: plot)
        drawScriptMarkers(context: &context, plot: plot)
        drawScriptLabels(context: &context, plot: plot)
    }

    /// Tables pin to the plot's corners and are never value-anchored, so callers draw
    /// them outside the series clip.
    func drawTables(_ context: inout GraphicsContext, plot: ChartPlot) {
        drawScriptTables(context: &context, plot: plot)
    }

    /// `barcolor()` for the candle at `index`, if the script set one.
    func barColor(at index: Int) -> Color? {
        guard let series = pine.barColors.first, let rgba = visible(series.colors, at: index) ?? nil
        else { return nil }
        return Color(pineRGBA: rgba)
    }

    /// Vertical domain for a script pane: every value-anchored output in view, padded
    /// like `ChartPlot.priceRange` so nothing touches the frame.
    func valueRange(padding: CGFloat) -> (min: Double, max: Double) {
        var values: [Double] = []
        for output in pine.plots {
            values += output.values.suffix(candles.count).compactMap { $0 }
        }
        values += pine.hlines.map(\.value)
        if !candles.isEmpty {
            let visibleBars = (pine.barCount - candles.count)...(pine.barCount - 1)
            func inView(_ a: Int, _ b: Int) -> Bool { visibleBars.overlaps(min(a, b)...max(a, b)) }
            for box in pine.boxes where inView(box.left, box.right) { values += [box.top, box.bottom] }
            for line in pine.lines where inView(line.x1, line.x2) { values += [line.y1, line.y2] }
            for label in pine.labels where visibleBars.contains(label.x) { values.append(label.y) }
        }
        values = values.filter(\.isFinite)
        guard let low = values.min(), let high = values.max() else { return (0, 1) }
        if low == high {
            let inset = max(abs(low) * 0.01, 0.01)
            return (low - inset, high + inset)
        }
        let inset = (high - low) * Double(padding)
        return (low - inset, high + inset)
    }

    /// Draws one candle-width band for each non-nil `bgcolor()` result.
    ///
    /// Backgrounds are separate from plots because they must sit behind both the candles
    /// and every foreground script visual in the chart's deterministic draw order.
    private func drawScriptBackgroundBands(context: inout GraphicsContext, plot: ChartPlot) {
        let slot = plot.slotWidth(forCount: candles.count)
        for output in pine.backgrounds {
            let visibleColors = Array(output.colors.suffix(candles.count))
            let firstCandleIndex = candles.count - visibleColors.count
            for (offset, color) in visibleColors.enumerated() {
                guard let color else { continue }
                let candleIndex = firstCandleIndex + offset
                let x = plot.x(forIndex: candleIndex, slotWidth: slot)
                context.fill(
                    Path(
                        CGRect(
                            x: x - slot / 2, y: plot.plotRect.minY, width: slot, height: plot.plotRect.height)),
                    with: .color(Color(pineRGBA: color)))
            }
        }
    }

    /// Value of a per-bar script series at a visible candle. Series are aligned to the
    /// right edge: the last element belongs to the last candle.
    private func visible<T>(_ series: [T], at candleIndex: Int) -> T? {
        let k = series.count - candles.count + candleIndex
        return series.indices.contains(k) ? series[k] : nil
    }

    /// Candle slot for an absolute Pine `bar_index` (drawing-object coordinates).
    private func candleIndex(forBar bar: Int) -> Int {
        bar - (pine.barCount - candles.count)
    }

    private func x(forBar bar: Int, plot: ChartPlot, slot: CGFloat) -> CGFloat {
        plot.x(forIndex: candleIndex(forBar: bar), slotWidth: slot)
    }

    /// Draws contiguous `plot()` segments. A nil value ends the current segment so Pine
    /// gaps do not get bridged by a line. The segment ending at a bar takes that bar's color.
    private func drawScriptPlots(context: inout GraphicsContext, plot: ChartPlot) {
        let slot = plot.slotWidth(forCount: candles.count)
        for output in pine.plots {
            func point(_ i: Int) -> CGPoint? {
                guard let value = visible(output.values, at: i) ?? nil else { return nil }
                return CGPoint(x: plot.x(forIndex: i, slotWidth: slot), y: plot.y(for: value))
            }
            func color(_ i: Int) -> UInt32 { (visible(output.colors, at: i) ?? nil) ?? output.color }
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
                while j + 1 < candles.count, let next = point(j + 1), color(j + 1) == runColor {
                    path.addLine(to: next)
                    j += 1
                }
                if runColor & 0xFF != 0 {
                    context.stroke(
                        path, with: .color(Color(pineRGBA: runColor)), lineWidth: CGFloat(output.lineWidth))
                }
                i = j == i ? i + 1 : j
            }
        }
    }

    /// `fill()` between two plots: one polygon per run of bars where both plots have
    /// values and the fill color stays the same.
    private func drawScriptFills(context: inout GraphicsContext, plot: ChartPlot) {
        let slot = plot.slotWidth(forCount: candles.count)
        let plots = Dictionary(pine.plots.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
        for fill in pine.fills {
            guard let a = plots[fill.plotA], let b = plots[fill.plotB] else { continue }
            func pair(_ i: Int) -> (CGFloat, CGFloat, CGFloat)? {
                guard let top = visible(a.values, at: i) ?? nil, let bottom = visible(b.values, at: i) ?? nil
                else { return nil }
                return (plot.x(forIndex: i, slotWidth: slot), plot.y(for: top), plot.y(for: bottom))
            }
            func color(_ i: Int) -> UInt32? { visible(fill.colors, at: i) ?? nil }
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
                path.move(to: CGPoint(x: points[0].0, y: points[0].1))
                for p in points.dropFirst() { path.addLine(to: CGPoint(x: p.0, y: p.1)) }
                for p in points.reversed() { path.addLine(to: CGPoint(x: p.0, y: p.2)) }
                path.closeSubpath()
                context.fill(path, with: .color(Color(pineRGBA: runColor)))
                i = j
            }
        }
    }

    private func drawScriptHorizontalLines(context: inout GraphicsContext, plot: ChartPlot) {
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

    private func drawScriptMarkers(context: inout GraphicsContext, plot: ChartPlot) {
        let slot = plot.slotWidth(forCount: candles.count)
        for marker in pine.markers {
            for candleIndex in candles.indices where visible(marker.values, at: candleIndex) == true {
                let x = plot.x(forIndex: candleIndex, slotWidth: slot)
                let candle = candles[candleIndex]
                let price = visible(marker.prices, at: candleIndex) ?? nil
                let y: CGFloat
                if marker.location == "location.absolute", let price {
                    y = plot.y(for: price)
                } else if inPane {
                    // No candles in a script pane: above/below bar pin to its edges.
                    y = marker.location.contains("below") ? plot.plotRect.maxY - 8 : plot.plotRect.minY + 8
                } else if marker.location.contains("below") {
                    y = plot.y(for: candle.lowPrice) + 8
                } else {
                    y = plot.y(for: candle.highPrice) - 8
                }
                let color = Color(pineRGBA: (visible(marker.colors, at: candleIndex) ?? nil) ?? marker.color)
                if marker.style == "shape.circle" {
                    let r = Self.markerRadius(marker.size)
                    context.fill(
                        Path(ellipseIn: CGRect(x: x - r, y: y - r, width: 2 * r, height: 2 * r)), with: .color(color))
                    continue
                }
                let text = Text(marker.character ?? (marker.style.contains("down") ? "▼" : "▲")).font(
                    .caption
                ).foregroundColor(color)
                context.draw(text, at: CGPoint(x: x, y: y))
            }
        }
    }

    private static func markerRadius(_ size: String) -> CGFloat {
        switch size {
        case "size.tiny": 3
        case "size.small": 5
        case "size.normal": 7
        case "size.large": 10
        case "size.huge": 14
        default: 5
        }
    }

    private static func fontSize(_ size: String) -> CGFloat {
        switch size {
        case "size.tiny": 8
        case "size.small": 10
        case "size.large": 15
        case "size.huge": 20
        default: 12
        }
    }

    // MARK: - Script drawing objects

    private func drawScriptBoxes(context: inout GraphicsContext, plot: ChartPlot) {
        let slot = plot.slotWidth(forCount: candles.count)
        for box in pine.boxes {
            let left = x(forBar: box.left, plot: plot, slot: slot)
            let right = x(forBar: box.right, plot: plot, slot: slot)
            let top = plot.y(for: box.top)
            let bottom = plot.y(for: box.bottom)
            let rect = CGRect(
                x: min(left, right), y: min(top, bottom), width: abs(right - left), height: abs(bottom - top))
            if let fill = box.backgroundColor {
                context.fill(Path(rect), with: .color(Color(pineRGBA: fill)))
            }
            if let border = box.borderColor, box.borderWidth > 0 {
                context.stroke(
                    Path(rect), with: .color(Color(pineRGBA: border)), lineWidth: CGFloat(box.borderWidth))
            }
        }
    }

    private func drawScriptLines(context: inout GraphicsContext, plot: ChartPlot) {
        let slot = plot.slotWidth(forCount: candles.count)
        for line in pine.lines {
            var start = CGPoint(x: x(forBar: line.x1, plot: plot, slot: slot), y: plot.y(for: line.y1))
            var end = CGPoint(x: x(forBar: line.x2, plot: plot, slot: slot), y: plot.y(for: line.y2))
            if start.x != end.x {
                let slope = (end.y - start.y) / (end.x - start.x)
                let (leftPoint, rightPoint) = start.x < end.x ? (start, end) : (end, start)
                var l = leftPoint
                var r = rightPoint
                if line.extend == "extend.left" || line.extend == "extend.both" {
                    l = CGPoint(x: plot.plotRect.minX, y: leftPoint.y - slope * (leftPoint.x - plot.plotRect.minX))
                }
                if line.extend == "extend.right" || line.extend == "extend.both" {
                    r = CGPoint(x: plot.plotRect.maxX, y: rightPoint.y + slope * (plot.plotRect.maxX - rightPoint.x))
                }
                (start, end) = (l, r)
            }
            var path = Path()
            path.move(to: start)
            path.addLine(to: end)
            let dash: [CGFloat] =
                switch line.style {
                case "line.style_dashed": [6, 4]
                case "line.style_dotted": [1, 3]
                default: []
                }
            context.stroke(
                path, with: .color(Color(pineRGBA: line.color)),
                style: StrokeStyle(lineWidth: CGFloat(max(1, line.width)), lineCap: .round, dash: dash))
        }
    }

    /// Labels draw as a bubble with a pointer toward their anchor. `label_down` sits
    /// above the anchor, `label_up` below it; a transparent label color leaves just text.
    private func drawScriptLabels(context: inout GraphicsContext, plot: ChartPlot) {
        let slot = plot.slotWidth(forCount: candles.count)
        let pointer: CGFloat = 5
        for label in pine.labels {
            let anchor = CGPoint(x: x(forBar: label.x, plot: plot, slot: slot), y: plot.y(for: label.y))
            let text = context.resolve(
                Text(label.text)
                    .font(.system(size: Self.fontSize(label.size)))
                    .foregroundColor(Color(pineRGBA: label.textColor)))
            let measured = text.measure(in: CGSize(width: 600, height: 400))
            let size = CGSize(width: measured.width + 10, height: measured.height + 6)
            var bubble = CGRect(origin: .zero, size: size)
            var tip: [CGPoint] = []
            switch label.style {
            case "label.style_label_down":
                bubble.origin = CGPoint(x: anchor.x - size.width / 2, y: anchor.y - pointer - size.height)
                tip = [
                    CGPoint(x: anchor.x - pointer, y: bubble.maxY),
                    anchor,
                    CGPoint(x: anchor.x + pointer, y: bubble.maxY),
                ]
            case "label.style_label_up":
                bubble.origin = CGPoint(x: anchor.x - size.width / 2, y: anchor.y + pointer)
                tip = [
                    CGPoint(x: anchor.x - pointer, y: bubble.minY),
                    anchor,
                    CGPoint(x: anchor.x + pointer, y: bubble.minY),
                ]
            case "label.style_label_left":
                bubble.origin = CGPoint(x: anchor.x + pointer, y: anchor.y - size.height / 2)
                tip = [
                    CGPoint(x: bubble.minX, y: anchor.y - pointer),
                    anchor,
                    CGPoint(x: bubble.minX, y: anchor.y + pointer),
                ]
            case "label.style_label_right":
                bubble.origin = CGPoint(x: anchor.x - pointer - size.width, y: anchor.y - size.height / 2)
                tip = [
                    CGPoint(x: bubble.maxX, y: anchor.y - pointer),
                    anchor,
                    CGPoint(x: bubble.maxX, y: anchor.y + pointer),
                ]
            default:
                bubble.origin = CGPoint(x: anchor.x - size.width / 2, y: anchor.y - size.height / 2)
            }
            if let fill = label.color, fill & 0xFF != 0, label.style != "label.style_none" {
                var shape = Path(roundedRect: bubble, cornerRadius: 3)
                if !tip.isEmpty {
                    shape.move(to: tip[0])
                    shape.addLine(to: tip[1])
                    shape.addLine(to: tip[2])
                    shape.closeSubpath()
                }
                context.fill(shape, with: .color(Color(pineRGBA: fill)))
            }
            context.draw(text, at: CGPoint(x: bubble.midX, y: bubble.midY))
        }
    }

    /// `table.new` grids, sized to their cell text and pinned to a plot corner/edge.
    private func drawScriptTables(context: inout GraphicsContext, plot: ChartPlot) {
        let margin: CGFloat = 8
        let padding = CGSize(width: 12, height: 6)
        for table in pine.tables where table.columns > 0 && table.rows > 0 && !table.cells.isEmpty {
            var widths = Array(repeating: CGFloat(0), count: table.columns)
            var heights = Array(repeating: CGFloat(0), count: table.rows)
            var resolved: [(PineTableCell, GraphicsContext.ResolvedText, CGSize)] = []
            for cell in table.cells {
                let text = context.resolve(
                    Text(cell.text)
                        .font(.system(size: Self.fontSize(cell.textSize)))
                        .foregroundColor(Color(pineRGBA: cell.textColor)))
                let size = text.measure(in: CGSize(width: 400, height: 200))
                widths[cell.column] = max(widths[cell.column], size.width + padding.width)
                heights[cell.row] = max(heights[cell.row], size.height + padding.height)
                resolved.append((cell, text, size))
            }
            let total = CGSize(width: widths.reduce(0, +), height: heights.reduce(0, +))
            let area = plot.plotRect.insetBy(dx: margin, dy: margin)
            let position = table.position
            let originX: CGFloat =
                position.hasSuffix("_left")
                ? area.minX
                : position.hasSuffix("_center") ? area.midX - total.width / 2 : area.maxX - total.width
            let originY: CGFloat =
                position.contains(".top_")
                ? area.minY
                : position.contains(".middle_") ? area.midY - total.height / 2 : area.maxY - total.height
            let frame = CGRect(x: originX, y: originY, width: total.width, height: total.height)

            if let background = table.backgroundColor {
                context.fill(Path(frame), with: .color(Color(pineRGBA: background)))
            }
            func cellRect(_ column: Int, _ row: Int) -> CGRect {
                CGRect(
                    x: originX + widths[..<column].reduce(0, +), y: originY + heights[..<row].reduce(0, +),
                    width: widths[column], height: heights[row])
            }
            for (cell, text, _) in resolved {
                let rect = cellRect(cell.column, cell.row)
                if let background = cell.backgroundColor {
                    context.fill(Path(rect), with: .color(Color(pineRGBA: background)))
                }
                context.draw(text, at: CGPoint(x: rect.midX, y: rect.midY))
            }
            if let border = table.borderColor, table.borderWidth > 0 {
                var grid = Path()
                var x = originX
                for width in widths.dropLast() {
                    x += width
                    grid.move(to: CGPoint(x: x, y: frame.minY))
                    grid.addLine(to: CGPoint(x: x, y: frame.maxY))
                }
                var y = originY
                for height in heights.dropLast() {
                    y += height
                    grid.move(to: CGPoint(x: frame.minX, y: y))
                    grid.addLine(to: CGPoint(x: frame.maxX, y: y))
                }
                context.stroke(grid, with: .color(Color(pineRGBA: border)), lineWidth: CGFloat(table.borderWidth))
            }
            if let frameColor = table.frameColor, table.frameWidth > 0 {
                context.stroke(
                    Path(frame), with: .color(Color(pineRGBA: frameColor)), lineWidth: CGFloat(table.frameWidth))
            }
        }
    }
}

private extension Color {
    init(pineRGBA value: UInt32) {
        self.init(
            .sRGB,
            red: Double((value >> 24) & 255) / 255,
            green: Double((value >> 16) & 255) / 255,
            blue: Double((value >> 8) & 255) / 255,
            opacity: Double(value & 255) / 255)
    }
}
