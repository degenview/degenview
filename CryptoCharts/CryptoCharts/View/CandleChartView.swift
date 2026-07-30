import SwiftUI
import AppKit

// MARK: - CandleChartView

struct CandleChartView: View {
    let candles: [KlineData]
    var chartHeight: CGFloat = 220
    var style: CandleChartStyle = .default

    // Per-chart overrides
    var bullishColor: Color = .green
    var bearishColor: Color = .red
    var yAxisDecimalPlaces: Int? = nil  // nil = auto-detect

    var onZoom: ((CGFloat) -> Void)?

    @State private var monitor: Any?
    @State private var chartFrame: CGRect = .zero

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
            .onAppear {
                chartFrame = geometry.frame(in: .global)
                monitor = NSEvent.addLocalMonitorForEvents(matching: .scrollWheel) { event in
                    let mouseScreen = NSEvent.mouseLocation
                    guard let window = NSApp.keyWindow ?? NSApp.mainWindow else { return event }
                    let mouseWindow = window.convertPoint(fromScreen: mouseScreen)
                    let viewFrame = window.convertToScreen(chartFrame)

                    if let contentView = window.contentView {
                        let chartInWindow = contentView.convert(viewFrame, from: nil)
                        if chartInWindow.contains(mouseWindow) {
                            DispatchQueue.main.async {
                                onZoom?(event.scrollingDeltaY)
                            }
                        }
                    }
                    return event
                }
            }
            .onDisappear {
                if let m = monitor { NSEvent.removeMonitor(m) }
            }
            .onChange(of: geometry.frame(in: .global)) {
                chartFrame = geometry.frame(in: .global)
            }
        }
        .frame(height: max(50, chartHeight))
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

        let label = formatPrice(price)
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
        let bodyWidth = slotWidth * style.bodyWidthRatio

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
            let wickRect = CGRect(x: x - style.wickWidth / 2, y: wickTop,
                                  width: style.wickWidth, height: max(wickBottom - wickTop, 0.5))
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

        let label = formatPrice(price)
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

    // MARK: - X Axis (time labels below the plot)

    private func drawXAxis(context: inout GraphicsContext, plotRect: CGRect) {
        guard candles.count >= 2 else { return }

        let totalSpan = candles.last!.openTime.timeIntervalSince(candles.first!.openTime)
        guard totalSpan > 0 else { return }

        let formatter = DateFormatter()
        formatter.dateFormat = timeAxisFormat(span: totalSpan)
        formatter.timeZone = TimeZone(identifier: "UTC")!

        let labelCount = style.timeLabelCount

        for i in 0..<labelCount {
            let fraction = CGFloat(i) / CGFloat(max(1, labelCount - 1))
            let timestamp = candles.first!.openTime.addingTimeInterval(totalSpan * Double(fraction))

            let label = formatter.string(from: timestamp)
            let text = Text(label).font(.caption2).foregroundStyle(.secondary)
            let resolved = context.resolve(text)
            let textSize = resolved.measure(in: .init(width: 120, height: 14))

            let x = plotRect.minX + fraction * plotRect.width
            let drawX: CGFloat
            if i == 0 {
                drawX = x
            } else if i == labelCount - 1 {
                drawX = x - textSize.width
            } else {
                drawX = x - textSize.width / 2
            }

            let drawY = plotRect.maxY + 8
            context.draw(resolved, at: CGPoint(x: drawX, y: drawY))
        }
    }

    /// Pick a sensible date format for the visible time span.
    private func timeAxisFormat(span: TimeInterval) -> String {
        if span < 7200 { return "HH:mm" }            // < 2 hours
        if span < 172800 { return "EEE HH:mm" }       // < 2 days
        if span < 2592000 { return "MMM d" }          // < ~1 month
        return "MMM yy"                               // months–years
    }

    // MARK: - Formatting

    /// Format price with subscript zero-count for very small numbers (CoinMarketCap style).
    /// Examples: 0.00000278 → "0.0₅278", 45.23 → "45.23", 1,234,567 → "1,234,567"
    private func formatPrice(_ price: Double) -> String {
        guard price > 0 else { return "0" }

        // Very small: CoinMarketCap subscript zero-count notation
        if price < 0.001 {
            var zeroCount = 0
            var scaled = price
            while scaled < 0.1 {
                scaled *= 10
                zeroCount += 1
            }
            // scaled is in [0.1, 1.0) — extract 3 fixed significant digits
            let sigValue = Int(round(scaled * 1_000))
            let sigStr: String
            if sigValue >= 1_000 {
                // Rounding overflow (e.g., 0.099999 → 1000)
                zeroCount = max(0, zeroCount - 1)
                sigStr = "100"
            } else {
                sigStr = String(format: "%03d", sigValue)
            }
            return "0.0\(zeroCount.subscriptUnicode)\(sigStr)"
        }

        // Large numbers: just grouped decimal
        if price >= 1_000_000 {
            if let places = yAxisDecimalPlaces {
                return price.formatted(.number.grouping(.automatic).precision(.fractionLength(places)))
            }
            return price.formatted(.number.grouping(.automatic).precision(.fractionLength(0)))
        }

        let digits: Int
        if let places = yAxisDecimalPlaces {
            digits = places
        } else if price >= 1000 { digits = 0 }
        else if price >= 1 { digits = 2 }
        else if price >= 0.01 { digits = 4 }
        else { digits = 6 }

        return price.formatted(
            .number
            .precision(.fractionLength(digits))
            .grouping(.automatic)
        )
    }
}

private extension Int {
    /// Unicode subscript digits: 0→₀, 1→₁, …, 9→₉
    var subscriptUnicode: String {
        String(self).map { char -> String in
            guard let digit = char.wholeNumberValue,
                  let scalar = UnicodeScalar(0x2080 + digit) else {
                return String(char)
            }
            return String(scalar)
        }.joined()
    }
}

#Preview {
    CandleChartView(candles: MockData.sampleKlines)
        .frame(width: 400, height: 260)
        .padding()
}
