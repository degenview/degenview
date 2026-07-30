import SwiftUI

// MARK: - CandleChartView

struct CandleChartView: View {
    let candles: [KlineData]
    var chartHeight: CGFloat
    var style: CandleChartStyle = .default

    // Per-chart overrides
    var bullishColor: Color = .green
    var bearishColor: Color = .red
    var yAxisDecimalPlaces: Int? = nil  // nil = auto-detect

    var body: some View {
        GeometryReader { geometry in
            let plotRect = computePlotRect(in: geometry.size)
            let priceRange = computePriceRange()
            let candleSlotWidth = plotRect.width / CGFloat(max(1, candles.count))

            Canvas { context, size in
                drawGrid(context: &context, plotRect: plotRect, priceRange: priceRange)
                drawCandles(context: &context, plotRect: plotRect, priceRange: priceRange, slotWidth: candleSlotWidth)
                drawCurrentPriceLine(context: &context, plotRect: plotRect, priceRange: priceRange)
                drawCurrentPriceBox(context: &context, plotRect: plotRect, priceRange: priceRange)
                drawXAxis(context: &context, plotRect: plotRect)
            }
        }
        .frame(height: max(ChartLayout.chartMinHeight, chartHeight))
        .clipped()
    }

    // MARK: - Coordinates

    private func computePlotRect(in size: CGSize) -> CGRect {
        CGRect(
            x: style.chartInsets.leading,
            y: style.chartInsets.top,
            width: size.width - style.chartInsets.leading - style.chartInsets.trailing,
            height: size.height - style.chartInsets.top - style.chartInsets.bottom
        )
    }

    private func computePriceRange() -> (min: Double, max: Double) {
        guard !candles.isEmpty else { return (0.01, 1) }
        let low  = candles.map(\.lowPrice).min() ?? 0
        let high = candles.map(\.highPrice).max() ?? 1
        if low == high { return (low * 0.99, high * 1.01) }
        let padding = (high - low) * Double(style.pricePadding)
        return (low - padding, high + padding)
    }

    private func yForPrice(_ price: Double, plotRect: CGRect, priceRange: (min: Double, max: Double)) -> CGFloat {
        let span = priceRange.max - priceRange.min
        guard span > 0 else { return plotRect.midY }
        let normalized = (price - priceRange.min) / span
        return plotRect.maxY - CGFloat(normalized) * plotRect.height
    }

    // MARK: - Grid (lines + Y-axis price labels on right side)

    private func drawGrid(context: inout GraphicsContext, plotRect: CGRect, priceRange: (min: Double, max: Double)) {
        let span = priceRange.max - priceRange.min
        guard span > 0 else { return }

        for i in 0...style.priceLabelCount {
            let fraction = CGFloat(i) / CGFloat(style.priceLabelCount)
            let y = plotRect.maxY - fraction * plotRect.height
            let price = priceRange.min + Double(fraction) * span
            drawGridLine(context: &context, plotRect: plotRect, y: y, price: price)
        }
    }

    private func drawGridLine(context: inout GraphicsContext, plotRect: CGRect, y: CGFloat, price: Double) {
        var path = Path()
        path.move(to: CGPoint(x: plotRect.minX, y: y))
        path.addLine(to: CGPoint(x: plotRect.maxX, y: y))
        context.stroke(path, with: .color(style.gridColor), lineWidth: style.gridLineWidth)

        let label = PriceFormatter.format(price, decimalPlaces: yAxisDecimalPlaces)
        let text = Text(label).font(.caption2).foregroundStyle(.secondary)
        let resolved = context.resolve(text)
        let textSize = resolved.measure(in: .init(width: 80, height: 14))
        let labelX = plotRect.maxX + style.chartInsets.trailing - textSize.width - 4
        context.draw(resolved, at: CGPoint(x: labelX, y: y - textSize.height / 2))
    }

    // MARK: - Candles

    private func drawCandles(context: inout GraphicsContext, plotRect: CGRect, priceRange: (min: Double, max: Double), slotWidth: CGFloat) {
        let priceRangeSpan = priceRange.max - priceRange.min
        let dojiAbsThreshold = priceRangeSpan * style.dojiThreshold
        let bodyWidth = (slotWidth * style.candleBodyFraction).clamped(to: style.candleBodyMin...style.candleBodyMax)
        let wickWidth = (slotWidth * style.wickFraction).clamped(to: style.wickMin...style.wickMax)

        for (i, candle) in candles.enumerated() {
            let x = plotRect.minX + CGFloat(i) * slotWidth + slotWidth / 2
            let wickTop    = yForPrice(candle.highPrice, plotRect: plotRect, priceRange: priceRange)
            let wickBottom = yForPrice(candle.lowPrice,  plotRect: plotRect, priceRange: priceRange)
            let bodyTop    = yForPrice(max(candle.openPrice, candle.closePrice), plotRect: plotRect, priceRange: priceRange)
            let bodyBottom = yForPrice(min(candle.openPrice, candle.closePrice), plotRect: plotRect, priceRange: priceRange)

            let isDoji = abs(candle.closePrice - candle.openPrice) <= dojiAbsThreshold
            let isBullish = candle.closePrice > candle.openPrice

            let candleColor: Color
            if isDoji { candleColor = style.dojiColor }
            else if isBullish { candleColor = bullishColor }
            else { candleColor = bearishColor }

            // Wick
            let wickRect = CGRect(x: x - wickWidth / 2, y: wickTop,
                                  width: wickWidth, height: max(wickBottom - wickTop, 0.5))
            context.fill(Path(wickRect), with: .color(candleColor))

            // Body
            var bodyHeight = bodyBottom - bodyTop
            if bodyHeight < style.minBodyHeight {
                bodyHeight = style.minBodyHeight
                let midY = (bodyTop + bodyBottom) / 2
                let bodyRect = CGRect(x: x - bodyWidth / 2, y: midY - style.minBodyHeight / 2,
                                      width: bodyWidth, height: style.minBodyHeight)
                context.fill(Path(bodyRect), with: .color(candleColor))
            } else {
                let bodyRect = CGRect(x: x - bodyWidth / 2, y: bodyTop,
                                      width: bodyWidth, height: bodyHeight)
                context.fill(Path(bodyRect), with: .color(candleColor))
            }
        }
    }

    // MARK: - Current Price Line (dashed horizontal line across chart)

    private func drawCurrentPriceLine(context: inout GraphicsContext, plotRect: CGRect, priceRange: (min: Double, max: Double)) {
        guard let lastCandle = candles.last else { return }
        let price = lastCandle.closePrice
        let isBullish = lastCandle.closePrice >= lastCandle.openPrice
        let lineColor = isBullish ? bullishColor : bearishColor

        let y = yForPrice(price, plotRect: plotRect, priceRange: priceRange)

        var path = Path()
        path.move(to: CGPoint(x: plotRect.minX, y: y))
        path.addLine(to: CGPoint(x: plotRect.maxX, y: y))

        let dashStyle = StrokeStyle(
            lineWidth: style.currentPriceLineWidth,
            dash: style.currentPriceDashPattern
        )
        context.stroke(path, with: .color(lineColor.opacity(0.5)), style: dashStyle)
    }

    // MARK: - Current Price Box (right of candles; overlaps Y-axis when too wide)

    private func drawCurrentPriceBox(context: inout GraphicsContext, plotRect: CGRect, priceRange: (min: Double, max: Double)) {
        guard let lastCandle = candles.last else { return }
        let price = lastCandle.closePrice
        let isBullish = lastCandle.closePrice >= lastCandle.openPrice
        let boxColor = isBullish ? bullishColor : bearishColor

        let y = yForPrice(price, plotRect: plotRect, priceRange: priceRange)

        let label = PriceFormatter.format(price, decimalPlaces: yAxisDecimalPlaces)
        let text = Text(label).font(.caption2).bold().foregroundStyle(.white)
        let resolved = context.resolve(text)
        let textSize = resolved.measure(in: .init(width: 100, height: 20))

        let boxWidth = textSize.width + 10
        let boxHeight: CGFloat = 18

        // Position right of plot area; naturally overlaps Y-axis labels when too wide
        let boxX = plotRect.maxX + 4
        let boxY = (y - boxHeight / 2).clamped(to: (plotRect.minY + 1)...(plotRect.maxY - boxHeight - 1))

        let boxRect = CGRect(x: boxX, y: boxY, width: boxWidth, height: boxHeight)
        context.fill(Path(roundedRect: boxRect, cornerRadius: 3), with: .color(boxColor))

        // Text centered in box
        context.draw(resolved, at: CGPoint(x: boxRect.midX, y: boxRect.midY))
    }

    // MARK: - X Axis (time labels below the plot, aligned to natural boundaries)

    private func drawXAxis(context: inout GraphicsContext, plotRect: CGRect) {
        guard candles.count >= 2 else { return }

        let firstDate = candles.first!.openTime
        let lastDate = candles.last!.openTime
        guard lastDate > firstDate else { return }

        let candleInterval = candles[1].openTime.timeIntervalSince(candles[0].openTime)
        let slotWidth = plotRect.width / CGFloat(max(1, candles.count))

        let calendar: Calendar = {
            var c = Calendar(identifier: .gregorian)
            c.timeZone = TimeZone(identifier: "UTC")!
            return c
        }()

        let boundaries = generateTimeBoundaries(
            first: firstDate,
            last: lastDate,
            candleInterval: candleInterval,
            calendar: calendar
        )

        for boundary in boundaries {
            let index = candleIndex(for: boundary)
            let x = plotRect.minX + index * slotWidth

            // Vertical grid line at boundary position
            var vLine = Path()
            vLine.move(to: CGPoint(x: x, y: plotRect.minY))
            vLine.addLine(to: CGPoint(x: x, y: plotRect.maxY))
            context.stroke(vLine, with: .color(style.gridColor), lineWidth: style.gridLineWidth)
        }
    }

    // MARK: - Time boundary generation

    /// Generate natural time boundaries (month starts, hour marks, etc.) between first and last candle.
    private func generateTimeBoundaries(first: Date, last: Date, candleInterval: TimeInterval, calendar: Calendar) -> [Date] {
        var boundaries: [Date] = []

        switch candleInterval {
        case ..<7200:   // sub-2h candles (1h, 30m, 15m, etc.)
            // Boundaries at 00:00 each day, or every 6h if span is short
            let span = last.timeIntervalSince(first)
            let hourStep = span < 172800 ? 6 : 24
            boundaries = hourAlignedBoundaries(first: first, last: last, calendar: calendar, hourStep: hourStep)

        case 7200..<172800:  // 2h to <2d candles
            // Daily boundaries at 00:00
            boundaries = dayAlignedBoundaries(first: first, last: last, calendar: calendar)

        default:  // 2d+ candles (daily, weekly, monthly)
            // Monthly boundaries at 1st of month
            boundaries = monthAlignedBoundaries(first: first, last: last, calendar: calendar)
        }

        return boundaries
    }

    /// Boundaries at round hour marks (00:00, 06:00, 12:00, 18:00, or every N hours).
    private func hourAlignedBoundaries(first: Date, last: Date, calendar: Calendar, hourStep: Int) -> [Date] {
        var boundaries: [Date] = []

        // Find first aligned hour boundary
        var comps = calendar.dateComponents([.year, .month, .day, .hour], from: first)
        comps.minute = 0
        comps.second = 0
        let alignedHour = (comps.hour! / hourStep) * hourStep
        comps.hour = alignedHour

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

    /// Boundaries at midnight (00:00) each day.
    private func dayAlignedBoundaries(first: Date, last: Date, calendar: Calendar) -> [Date] {
        // Reuse hour-aligned with step=24
        return hourAlignedBoundaries(first: first, last: last, calendar: calendar, hourStep: 24)
    }

    /// Boundaries at 1st of each month.
    private func monthAlignedBoundaries(first: Date, last: Date, calendar: Calendar) -> [Date] {
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

    /// Map a boundary date to a fractional candle index for X positioning.
    /// Uses binary search on candle open times; interpolates when boundary falls between candles.
    private func candleIndex(for date: Date) -> CGFloat {
        var lo = 0, hi = candles.count - 1
        while lo < hi {
            let mid = (lo + hi + 1) / 2
            if candles[mid].openTime <= date {
                lo = mid
            } else {
                hi = mid - 1
            }
        }

        // Interpolate position between candle[lo] and candle[lo+1]
        if lo < candles.count - 1 {
            let span = candles[lo + 1].openTime.timeIntervalSince(candles[lo].openTime)
            if span > 0 {
                let fraction = date.timeIntervalSince(candles[lo].openTime) / span
                return CGFloat(lo) + CGFloat(fraction)
            }
        }
        return CGFloat(lo)
    }

}

#Preview {
    CandleChartView(candles: MockData.sampleKlines, chartHeight: 220)
        .frame(width: 400, height: 260)
        .padding()
}
