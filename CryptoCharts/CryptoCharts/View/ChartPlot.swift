import SwiftUI

/// Everything a chart draws that isn't the series itself: plot geometry, the price
/// scale, the horizontal grid with its Y-axis labels, the vertical time grid, and the
/// current-price marker.
///
/// `CandleChartView` and `LineChartView` each own only their series rendering and
/// share this. Values are computed once per layout pass and handed to the `Canvas`.
struct ChartPlot {
    let plotRect: CGRect
    let priceRange: (min: Double, max: Double)
    let style: ChartStyle
    let scale: PriceScale
    let yAxisDecimalPlaces: Int?

    // MARK: - Geometry

    /// The plot a renderer draws into, for a canvas of `size`.
    ///
    /// Both chart views build their geometry here, and so does the drawing-tool hit
    /// testing in `ChartViewModel` — which has to reproduce exactly what was drawn
    /// from nothing but the hit view's size. One factory keeps the two from drifting.
    static func make(
        points: [KlineData],
        size: CGSize,
        yZoom: Double,
        scale: PriceScale,
        yAxisDecimalPlaces: Int?,
        style: ChartStyle = .default
    ) -> ChartPlot {
        ChartPlot(
            plotRect: Self.rect(in: size, insets: style.chartInsets),
            priceRange: Self.zoomed(
                Self.priceRange(for: points, padding: style.pricePadding),
                by: yZoom
            ),
            style: style,
            scale: scale,
            yAxisDecimalPlaces: yAxisDecimalPlaces
        )
    }

    static func rect(in size: CGSize, insets: EdgeInsets) -> CGRect {
        CGRect(
            x: insets.leading,
            y: insets.top,
            width: size.width - insets.leading - insets.trailing,
            height: size.height - insets.top - insets.bottom
        )
    }

    /// Vertical domain covering every point, padded so the series never touches the
    /// frame. Uses high/low, which equal the close on flat (line) points.
    static func priceRange(for points: [KlineData], padding: CGFloat) -> (min: Double, max: Double) {
        guard !points.isEmpty else { return (0.01, 1) }
        let low  = points.map(\.lowPrice).min() ?? 0
        let high = points.map(\.highPrice).max() ?? 1
        if low == high { return (low * 0.99, high * 1.01) }
        let inset = (high - low) * Double(padding)
        return (low - inset, high + inset)
    }

    /// Narrow or widen a price domain around its center. `zoom > 1` shows a smaller
    /// slice of price, so the series draws taller. Data outside the slice falls
    /// outside `plotRect` and is clipped by the renderer.
    static func zoomed(_ range: (min: Double, max: Double), by zoom: Double) -> (min: Double, max: Double) {
        guard zoom > 0, zoom != 1 else { return range }
        let mid = (range.min + range.max) / 2
        let half = (range.max - range.min) / 2 / zoom
        return (mid - half, mid + half)
    }

    /// Slot width for a series of `count` points — the horizontal step between them.
    func slotWidth(forCount count: Int) -> CGFloat {
        plotRect.width / CGFloat(max(1, count))
    }

    /// Center X of the point at `index`.
    func x(forIndex index: Int, slotWidth: CGFloat) -> CGFloat {
        plotRect.minX + CGFloat(index) * slotWidth + slotWidth / 2
    }

    /// Same slot centers as `x(forIndex:)`, but between points — and outside the
    /// series, where a trend line anchored to a date off the visible window lands.
    func x(forFractionalIndex index: CGFloat, slotWidth: CGFloat) -> CGFloat {
        plotRect.minX + index * slotWidth + slotWidth / 2
    }

    /// Inverse of `x(forFractionalIndex:)` — where a click falls in point-index space.
    func fractionalIndex(forX x: CGFloat, slotWidth: CGFloat) -> CGFloat {
        guard slotWidth > 0 else { return 0 }
        return (x - plotRect.minX) / slotWidth - 0.5
    }

    /// Strip along the bottom of the plot that an indicator draws into — volume bars
    /// and the RSI line both use one.
    ///
    /// Overlaid on the price area rather than carved out of it: cards are short, and
    /// giving a fifth of the height to a separate subchart costs more than indicators
    /// sharing space the series rarely reaches.
    func bottomPane(fraction: CGFloat) -> CGRect {
        let height = plotRect.height * fraction
        return CGRect(x: plotRect.minX, y: plotRect.maxY - height,
                      width: plotRect.width, height: height)
    }

    func y(for price: Double) -> CGFloat {
        let span = priceRange.max - priceRange.min
        guard span > 0 else { return plotRect.midY }
        let normalized = (price - priceRange.min) / span
        return plotRect.maxY - CGFloat(normalized) * plotRect.height
    }

    /// Inverse of `y(for:)` — the price a click landed on.
    func price(forY y: CGFloat) -> Double {
        guard plotRect.height > 0 else { return priceRange.min }
        let normalized = Double((plotRect.maxY - y) / plotRect.height)
        return priceRange.min + normalized * (priceRange.max - priceRange.min)
    }

    // MARK: - Grid (horizontal lines + Y-axis price labels on the right)

    func drawGrid(_ context: inout GraphicsContext) {
        let span = priceRange.max - priceRange.min
        guard span > 0 else { return }

        for i in 0...style.priceLabelCount {
            let fraction = CGFloat(i) / CGFloat(style.priceLabelCount)
            let y = plotRect.maxY - fraction * plotRect.height
            let price = priceRange.min + Double(fraction) * span
            drawGridLine(&context, y: y, price: price)
        }
    }

    private func drawGridLine(_ context: inout GraphicsContext, y: CGFloat, price: Double) {
        var path = Path()
        path.move(to: CGPoint(x: plotRect.minX, y: y))
        path.addLine(to: CGPoint(x: plotRect.maxX, y: y))
        context.stroke(path, with: .color(style.gridColor), lineWidth: style.gridLineWidth)

        let label = PriceFormatter.format(price, decimalPlaces: yAxisDecimalPlaces, scale: scale)
        let text = Text(label).font(.caption2).foregroundStyle(.secondary)
        let resolved = context.resolve(text)
        let textSize = resolved.measure(in: .init(width: 80, height: 14))
        let labelX = plotRect.maxX + style.chartInsets.trailing - textSize.width - 4
        context.draw(resolved, at: CGPoint(x: labelX, y: y - textSize.height / 2))
    }

    // MARK: - Price-scale overlays

    /// Polyline through readings that share the price axis, skipping the nil warm-up
    /// at the front. Used by every overlay that plots in price units.
    private func overlayPath(_ values: [Double?]) -> Path {
        let slot = slotWidth(forCount: values.count)
        var path = Path()
        var started = false

        for (index, value) in values.enumerated() {
            guard let value else { continue }
            let point = CGPoint(x: x(forIndex: index, slotWidth: slot), y: y(for: value))
            if started {
                path.addLine(to: point)
            } else {
                path.move(to: point)
                started = true
            }
        }
        return path
    }

    func drawEMA(_ context: inout GraphicsContext, values: [Double?]) {
        guard values.contains(where: { $0 != nil }) else { return }
        context.stroke(
            overlayPath(values),
            with: .color(style.emaColor),
            style: StrokeStyle(lineWidth: style.emaLineWidth, lineCap: .round, lineJoin: .round)
        )
    }

    /// Bollinger bands: upper and lower rails with the moving average between them,
    /// and a light tint across the channel.
    func drawBollinger(_ context: inout GraphicsContext, bands: BollingerSeries) {
        guard bands.hasValues else { return }

        // Tint: down the upper rail, back along the lower one.
        let slot = slotWidth(forCount: bands.upper.count)
        var channel = Path()
        var started = false
        var lowerPoints: [CGPoint] = []

        for (index, upper) in bands.upper.enumerated() {
            guard let upper, let lower = bands.lower[index] else { continue }
            let xPosition = x(forIndex: index, slotWidth: slot)
            let top = CGPoint(x: xPosition, y: y(for: upper))
            if started {
                channel.addLine(to: top)
            } else {
                channel.move(to: top)
                started = true
            }
            lowerPoints.append(CGPoint(x: xPosition, y: y(for: lower)))
        }

        if started {
            for point in lowerPoints.reversed() { channel.addLine(to: point) }
            channel.closeSubpath()
            context.fill(channel, with: .color(style.bollingerColor.opacity(style.bollingerFillOpacity)))
        }

        let rail = StrokeStyle(lineWidth: style.bollingerLineWidth, lineCap: .round, lineJoin: .round)
        context.stroke(overlayPath(bands.upper), with: .color(style.bollingerColor.opacity(0.7)), style: rail)
        context.stroke(overlayPath(bands.lower), with: .color(style.bollingerColor.opacity(0.7)), style: rail)
        context.stroke(
            overlayPath(bands.middle),
            with: .color(style.bollingerColor.opacity(0.45)),
            style: StrokeStyle(lineWidth: style.bollingerLineWidth, dash: style.rsiGuideDashPattern)
        )
    }

    // MARK: - RSI

    /// RSI line across the bottom strip, with guides at the overbought and oversold
    /// levels. `values` is aligned 1:1 with the series and nil until the indicator has
    /// enough history, so the line starts partway across rather than at the left edge.
    ///
    /// Lives here rather than in a renderer because it isn't the series — both the
    /// candle and line charts draw it the same way.
    func drawRSI(_ context: inout GraphicsContext, values: [Double?]) {
        guard values.contains(where: { $0 != nil }) else { return }

        let pane = bottomPane(fraction: style.rsiPaneFraction)
        let slot = slotWidth(forCount: values.count)

        func y(for reading: Double) -> CGFloat {
            pane.maxY - CGFloat(reading / 100) * pane.height
        }

        for level in [RSI.overbought, RSI.oversold] {
            var guideLine = Path()
            guideLine.move(to: CGPoint(x: pane.minX, y: y(for: level)))
            guideLine.addLine(to: CGPoint(x: pane.maxX, y: y(for: level)))
            context.stroke(
                guideLine,
                with: .color(style.rsiGuideColor),
                style: StrokeStyle(lineWidth: style.gridLineWidth, dash: style.rsiGuideDashPattern)
            )
        }

        var line = Path()
        var started = false
        for (index, reading) in values.enumerated() {
            guard let reading else { continue }
            let point = CGPoint(x: x(forIndex: index, slotWidth: slot), y: y(for: reading))
            if started {
                line.addLine(to: point)
            } else {
                line.move(to: point)
                started = true
            }
        }

        context.stroke(
            line,
            with: .color(style.rsiColor),
            style: StrokeStyle(lineWidth: style.rsiLineWidth, lineCap: .round, lineJoin: .round)
        )
    }

    // MARK: - Trend lines

    /// Where an anchor sits under the current geometry. Anchors are time + price, so
    /// this is the whole reason a line survives a timeframe switch: the projection is
    /// redone against whatever points are on screen now.
    func position(of anchor: TrendAnchor, points: [KlineData], slotWidth slot: CGFloat) -> CGPoint {
        let index = Self.fractionalIndex(of: anchor.date, in: points)
        return CGPoint(x: x(forFractionalIndex: index, slotWidth: slot), y: y(for: anchor.price))
    }

    /// Hand-drawn trend lines, plus the rubber band of one being drawn right now.
    /// Handles show only while the tool is armed — a disarmed chart is just the lines.
    func drawTrendLines(
        _ context: inout GraphicsContext,
        lines: [TrendLine],
        draft: (start: TrendAnchor, end: TrendAnchor)?,
        selectedID: UUID?,
        showHandles: Bool,
        points: [KlineData]
    ) {
        guard !points.isEmpty, !lines.isEmpty || draft != nil else { return }
        let slot = slotWidth(forCount: points.count)

        for line in lines {
            let isSelected = line.id == selectedID
            drawTrendSegment(
                &context,
                from: position(of: line.start, points: points, slotWidth: slot),
                to: position(of: line.end, points: points, slotWidth: slot),
                color: isSelected ? style.trendLineSelectedColor : style.trendLineColor,
                width: isSelected ? style.trendLineSelectedWidth : style.trendLineWidth,
                dashed: false,
                handles: showHandles
            )
        }

        if let draft {
            drawTrendSegment(
                &context,
                from: position(of: draft.start, points: points, slotWidth: slot),
                to: position(of: draft.end, points: points, slotWidth: slot),
                color: style.trendLineSelectedColor,
                width: style.trendLineWidth,
                dashed: true,
                handles: true
            )
        }
    }

    private func drawTrendSegment(
        _ context: inout GraphicsContext,
        from start: CGPoint,
        to end: CGPoint,
        color: Color,
        width: CGFloat,
        dashed: Bool,
        handles: Bool
    ) {
        var path = Path()
        path.move(to: start)
        path.addLine(to: end)
        context.stroke(
            path,
            with: .color(color),
            style: StrokeStyle(
                lineWidth: width,
                lineCap: .round,
                dash: dashed ? style.trendDashPattern : []
            )
        )

        guard handles else { return }
        let radius = style.trendHandleRadius
        for point in [start, end] {
            let box = CGRect(x: point.x - radius, y: point.y - radius,
                             width: radius * 2, height: radius * 2)
            context.fill(Path(ellipseIn: box), with: .color(color))
            context.stroke(Path(ellipseIn: box), with: .color(.white.opacity(0.9)), lineWidth: 1)
        }
    }

    // MARK: - Current price marker

    /// Dashed horizontal line at the latest price. Skipped when a zoomed-in price
    /// scale has pushed the latest price off the plot — the pill still marks it.
    func drawCurrentPriceLine(_ context: inout GraphicsContext, price: Double, color: Color) {
        let y = self.y(for: price)
        guard y >= plotRect.minY, y <= plotRect.maxY else { return }

        var path = Path()
        path.move(to: CGPoint(x: plotRect.minX, y: y))
        path.addLine(to: CGPoint(x: plotRect.maxX, y: y))

        let dashStyle = StrokeStyle(
            lineWidth: style.currentPriceLineWidth,
            dash: style.currentPriceDashPattern
        )
        context.stroke(path, with: .color(color.opacity(0.5)), style: dashStyle)
    }

    /// Filled price pill right of the plot; overlaps the Y-axis labels when too wide.
    func drawCurrentPriceBox(_ context: inout GraphicsContext, price: Double, color: Color) {
        let y = self.y(for: price)

        let label = PriceFormatter.format(price, decimalPlaces: yAxisDecimalPlaces, scale: scale)
        let text = Text(label).font(.caption2).bold().foregroundStyle(.white)
        let resolved = context.resolve(text)
        let textSize = resolved.measure(in: .init(width: 100, height: 20))

        let boxWidth = textSize.width + 10
        let boxHeight: CGFloat = 18

        let boxX = plotRect.maxX + 4
        let boxY = (y - boxHeight / 2).clamped(to: (plotRect.minY + 1)...(plotRect.maxY - boxHeight - 1))

        let boxRect = CGRect(x: boxX, y: boxY, width: boxWidth, height: boxHeight)
        context.fill(Path(roundedRect: boxRect, cornerRadius: 3), with: .color(color))
        context.draw(resolved, at: CGPoint(x: boxRect.midX, y: boxRect.midY))
    }

    // MARK: - Time axis (vertical grid lines at natural time boundaries)

    func drawTimeGrid(_ context: inout GraphicsContext, points: [KlineData]) {
        guard points.count >= 2 else { return }

        let firstDate = points[0].openTime
        let lastDate = points[points.count - 1].openTime
        guard lastDate > firstDate else { return }

        let interval = points[1].openTime.timeIntervalSince(points[0].openTime)
        let slot = slotWidth(forCount: points.count)

        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "UTC")!

        let boundaries = Self.timeBoundaries(
            first: firstDate,
            last: lastDate,
            pointInterval: interval,
            calendar: calendar
        )

        for boundary in boundaries {
            let index = Self.fractionalIndex(of: boundary, in: points)
            let x = plotRect.minX + index * slot

            var line = Path()
            line.move(to: CGPoint(x: x, y: plotRect.minY))
            line.addLine(to: CGPoint(x: x, y: plotRect.maxY))
            context.stroke(line, with: .color(style.gridColor), lineWidth: style.gridLineWidth)
        }
    }

    // MARK: - Time boundary generation

    /// Natural time boundaries (month starts, hour marks) between the first and last point.
    private static func timeBoundaries(first: Date, last: Date, pointInterval: TimeInterval, calendar: Calendar) -> [Date] {
        switch pointInterval {
        case ..<7200:   // sub-2h points (1h, 30m, 15m, …)
            // Boundaries at 00:00 each day, or every 6h if the span is short
            let span = last.timeIntervalSince(first)
            let hourStep = span < 172800 ? 6 : 24
            return hourAligned(first: first, last: last, calendar: calendar, hourStep: hourStep)

        case 7200..<172800:  // 2h to <2d points — daily boundaries at 00:00
            return hourAligned(first: first, last: last, calendar: calendar, hourStep: 24)

        default:  // 2d+ points (daily, weekly, monthly) — 1st of each month
            return monthAligned(first: first, last: last, calendar: calendar)
        }
    }

    /// Boundaries at round hour marks (00:00, 06:00, 12:00, 18:00, or every N hours).
    private static func hourAligned(first: Date, last: Date, calendar: Calendar, hourStep: Int) -> [Date] {
        var boundaries: [Date] = []

        var comps = calendar.dateComponents([.year, .month, .day, .hour], from: first)
        comps.minute = 0
        comps.second = 0
        comps.hour = (comps.hour! / hourStep) * hourStep

        guard var boundary = calendar.date(from: comps) else { return [] }
        if boundary < first {
            boundary = calendar.date(byAdding: .hour, value: hourStep, to: boundary)!
        }

        while boundary <= last {
            boundaries.append(boundary)
            guard let next = calendar.date(byAdding: .hour, value: hourStep, to: boundary) else { break }
            boundary = next
        }

        return boundaries
    }

    /// Boundaries at the 1st of each month.
    private static func monthAligned(first: Date, last: Date, calendar: Calendar) -> [Date] {
        var boundaries: [Date] = []

        var comps = calendar.dateComponents([.year, .month], from: first)
        comps.day = 1
        comps.hour = 0
        comps.minute = 0
        comps.second = 0

        guard var boundary = calendar.date(from: comps) else { return [] }
        if boundary < first {
            boundary = calendar.date(byAdding: .month, value: 1, to: boundary)!
        }

        while boundary <= last {
            boundaries.append(boundary)
            guard let next = calendar.date(byAdding: .month, value: 1, to: boundary) else { break }
            boundary = next
        }

        return boundaries
    }

    /// Map a date to a fractional point index for X positioning.
    /// Binary search on open times; interpolates when the date falls between points.
    ///
    /// Dates outside the visible window extrapolate rather than clamp — a trend line
    /// drawn on 1H and viewed on 1D is anchored before the first candle on screen, and
    /// pinning it to index 0 would collapse it into a stub along the left edge instead
    /// of letting the visible part keep its true slope. The renderer clips the rest.
    static func fractionalIndex(of date: Date, in points: [KlineData]) -> CGFloat {
        guard points.count > 1 else { return 0 }
        let last = points.count - 1

        var lo = 0, hi = last
        while lo < hi {
            let mid = (lo + hi + 1) / 2
            if points[mid].openTime <= date {
                lo = mid
            } else {
                hi = mid - 1
            }
        }

        // At or past the final point: extrapolate on the closing interval.
        if lo >= last {
            let span = points[last].openTime.timeIntervalSince(points[last - 1].openTime)
            guard span > 0 else { return CGFloat(last) }
            return CGFloat(last) + CGFloat(date.timeIntervalSince(points[last].openTime) / span)
        }

        // Between two points, or before the first — where the fraction goes negative
        // and extrapolates off the left edge for the same reason.
        let span = points[lo + 1].openTime.timeIntervalSince(points[lo].openTime)
        guard span > 0 else { return CGFloat(lo) }
        return CGFloat(lo) + CGFloat(date.timeIntervalSince(points[lo].openTime) / span)
    }

    /// Inverse of `fractionalIndex(of:in:)` — the date a click's X position lands on,
    /// extrapolating past either end on the neighbouring interval.
    static func date(atFractionalIndex index: CGFloat, in points: [KlineData]) -> Date {
        guard let first = points.first, let final = points.last else { return Date() }
        guard points.count > 1 else { return first.openTime }
        let last = points.count - 1

        let floorIndex = Int(index.rounded(.down))

        if floorIndex < 0 {
            let span = points[1].openTime.timeIntervalSince(first.openTime)
            return first.openTime.addingTimeInterval(span * Double(index))
        }
        if floorIndex >= last {
            let span = final.openTime.timeIntervalSince(points[last - 1].openTime)
            return final.openTime.addingTimeInterval(span * Double(index - CGFloat(last)))
        }

        let span = points[floorIndex + 1].openTime.timeIntervalSince(points[floorIndex].openTime)
        return points[floorIndex].openTime
            .addingTimeInterval(span * Double(index - CGFloat(floorIndex)))
    }
}
