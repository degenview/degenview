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

    /// Strip along the bottom of the plot that volume bars rise into.
    ///
    /// Overlaid on the price area rather than carved out of it: cards are short, and
    /// giving a fifth of the height to a separate subchart costs more than bars
    /// sharing space the series rarely reaches.
    func volumeRect(fraction: CGFloat) -> CGRect {
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
    private static func fractionalIndex(of date: Date, in points: [KlineData]) -> CGFloat {
        var lo = 0, hi = points.count - 1
        while lo < hi {
            let mid = (lo + hi + 1) / 2
            if points[mid].openTime <= date {
                lo = mid
            } else {
                hi = mid - 1
            }
        }

        if lo < points.count - 1 {
            let span = points[lo + 1].openTime.timeIntervalSince(points[lo].openTime)
            if span > 0 {
                let fraction = date.timeIntervalSince(points[lo].openTime) / span
                return CGFloat(lo) + CGFloat(fraction)
            }
        }
        return CGFloat(lo)
    }
}
