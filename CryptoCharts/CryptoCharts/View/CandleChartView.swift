import SwiftUI
import AppKit

// MARK: - CandleChartView

struct CandleChartView: View {
    let candles: [KlineData]
    var style: CandleChartStyle = .default
    var useLogScale = false
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
                drawCurrentPriceBox(context: &context, plotRect: plotRect, priceRange: priceRange)
            }
            .id(useLogScale) // force redraw on scale toggle
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
        .frame(height: 220)
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
        let rawMin = low - padding
        // Log scale requires positive values
        let safeMin = useLogScale ? max(rawMin, high * 0.0001) : rawMin
        return (safeMin, high + padding)
    }

    private func yForPrice(_ price: Double, plotRect: CGRect, priceRange: (min: Double, max: Double)) -> CGFloat {
        if useLogScale {
            let logMin = log(priceRange.min)
            let logMax = log(priceRange.max)
            let span = logMax - logMin
            guard span > 0 else { return plotRect.midY }
            let normalized = (log(max(price, priceRange.min)) - logMin) / span
            return plotRect.maxY - CGFloat(normalized) * plotRect.height
        } else {
            let span = priceRange.max - priceRange.min
            guard span > 0 else { return plotRect.midY }
            let normalized = (price - priceRange.min) / span
            return plotRect.maxY - CGFloat(normalized) * plotRect.height
        }
    }

    // MARK: - Grid (lines + Y-axis price labels on left side)

    private func drawGrid(context: inout GraphicsContext, plotRect: CGRect, priceRange: (min: Double, max: Double)) {
        if useLogScale {
            drawLogGrid(context: &context, plotRect: plotRect, priceRange: priceRange)
        } else {
            drawLinearGrid(context: &context, plotRect: plotRect, priceRange: priceRange)
        }
    }

    private func drawLinearGrid(context: inout GraphicsContext, plotRect: CGRect, priceRange: (min: Double, max: Double)) {
        let span = priceRange.max - priceRange.min
        guard span > 0 else { return }

        for i in 0...style.priceLabelCount {
            let fraction = CGFloat(i) / CGFloat(style.priceLabelCount)
            let y = plotRect.maxY - fraction * plotRect.height
            let price = priceRange.min + Double(fraction) * span
            drawGridLine(context: &context, plotRect: plotRect, y: y, price: price)
        }
    }

    private func drawLogGrid(context: inout GraphicsContext, plotRect: CGRect, priceRange: (min: Double, max: Double)) {
        let logMin = log10(priceRange.min)
        let logMax = log10(priceRange.max)

        // Generate nice grid levels at each power of 10 and at 2x/5x multiples
        var levels: [Double] = []
        let startPow = floor(logMin)
        let endPow = ceil(logMax)
        var pow10 = pow(10.0, startPow)
        while pow10 <= pow(10.0, endPow) {
            for mult in [1.0, 2.0, 5.0, 10.0] {
                let level = pow10 * mult
                if level >= priceRange.min && level <= priceRange.max && !levels.contains(where: { abs($0 - level) < level * 0.001 }) {
                    levels.append(level)
                }
            }
            pow10 *= 10
        }

        for price in levels {
            let y = yForPrice(price, plotRect: plotRect, priceRange: priceRange)
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
        let labelX = plotRect.minX - textSize.width - 4
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
            else if isBullish { candleColor = style.bullishColor }
            else { candleColor = style.bearishColor }

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

    // MARK: - Current Price Box (colored, to the right of candles)

    private func drawCurrentPriceBox(context: inout GraphicsContext, plotRect: CGRect, priceRange: (min: Double, max: Double)) {
        guard let lastCandle = candles.last else { return }
        let price = lastCandle.closePrice
        let isBullish = lastCandle.closePrice >= lastCandle.openPrice
        let boxColor = isBullish ? style.bullishColor : style.bearishColor

        let y = yForPrice(price, plotRect: plotRect, priceRange: priceRange)

        let label = formatPrice(price)
        let text = Text(label).font(.caption2).bold().foregroundStyle(.white)
        let resolved = context.resolve(text)
        let textSize = resolved.measure(in: .init(width: 100, height: 20))

        let boxWidth = textSize.width + 10
        let boxHeight: CGFloat = 18

        // Position to the right of the plot area, not overlapping candles
        let boxX = plotRect.maxX + 4
        let boxY = (y - boxHeight / 2).clamped(to: (plotRect.minY + 1)...(plotRect.maxY - boxHeight - 1))

        let boxRect = CGRect(x: boxX, y: boxY, width: boxWidth, height: boxHeight)
        context.fill(Path(roundedRect: boxRect, cornerRadius: 3), with: .color(boxColor))

        // Text centered in box
        context.draw(resolved, at: CGPoint(x: boxRect.midX, y: boxRect.midY))
    }

    // MARK: - Formatting

    private func formatPrice(_ price: Double) -> String {
        let digits: Int
        if price >= 1000 { digits = 0 }
        else if price >= 1 { digits = 2 }
        else if price >= 0.01 { digits = 4 }
        else { digits = 8 }

        return price.formatted(
            .number
            .precision(.fractionLength(0...digits))
            .grouping(.automatic)
        )
    }
}

#Preview {
    CandleChartView(candles: MockData.sampleKlines)
        .frame(width: 400, height: 260)
        .padding()
}
