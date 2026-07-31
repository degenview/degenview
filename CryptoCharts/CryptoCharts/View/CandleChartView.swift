import SwiftUI

// MARK: - CandleChartView

/// OHLC candlesticks. Grid, price axis, time grid and the current-price marker all
/// come from `ChartPlot`; this view owns only the candle bodies and wicks.
struct CandleChartView: View {
    let candles: [KlineData]
    var chartHeight: CGFloat
    var style: ChartStyle = .default

    // Per-chart overrides
    var bullishColor: Color = .green
    var bearishColor: Color = .red
    var yAxisDecimalPlaces: Int? = nil  // nil = auto-detect

    var body: some View {
        GeometryReader { geometry in
            let plot = ChartPlot(
                plotRect: ChartPlot.rect(in: geometry.size, insets: style.chartInsets),
                priceRange: ChartPlot.priceRange(for: candles, padding: style.pricePadding),
                style: style,
                scale: .currency,
                yAxisDecimalPlaces: yAxisDecimalPlaces
            )

            Canvas { context, _ in
                plot.drawGrid(&context)
                drawCandles(context: &context, plot: plot)

                if let last = candles.last {
                    let color = last.closePrice >= last.openPrice ? bullishColor : bearishColor
                    plot.drawCurrentPriceLine(&context, price: last.closePrice, color: color)
                    plot.drawCurrentPriceBox(&context, price: last.closePrice, color: color)
                }

                plot.drawTimeGrid(&context, points: candles)
            }
        }
        .frame(height: max(ChartLayout.chartMinHeight, chartHeight))
        .clipped()
    }

    // MARK: - Candles

    private func drawCandles(context: inout GraphicsContext, plot: ChartPlot) {
        let slotWidth = plot.slotWidth(forCount: candles.count)
        let priceRangeSpan = plot.priceRange.max - plot.priceRange.min
        let dojiAbsThreshold = priceRangeSpan * style.dojiThreshold
        let bodyWidth = (slotWidth * style.candleBodyFraction).clamped(to: style.candleBodyMin...style.candleBodyMax)
        let wickWidth = (slotWidth * style.wickFraction).clamped(to: style.wickMin...style.wickMax)

        for (i, candle) in candles.enumerated() {
            let x = plot.x(forIndex: i, slotWidth: slotWidth)
            let wickTop    = plot.y(for: candle.highPrice)
            let wickBottom = plot.y(for: candle.lowPrice)
            let bodyTop    = plot.y(for: max(candle.openPrice, candle.closePrice))
            let bodyBottom = plot.y(for: min(candle.openPrice, candle.closePrice))

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
            let bodyHeight = bodyBottom - bodyTop
            if bodyHeight < style.minBodyHeight {
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
}

#Preview {
    CandleChartView(candles: MockData.sampleKlines, chartHeight: 220)
        .frame(width: 400, height: 260)
        .padding()
}
