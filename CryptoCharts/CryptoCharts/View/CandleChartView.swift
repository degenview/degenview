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
    var yZoom: Double = 1               // 1 = auto-fit; >1 = taller candles
    var showVolume: Bool = false

    var body: some View {
        GeometryReader { geometry in
            let plot = ChartPlot(
                plotRect: ChartPlot.rect(in: geometry.size, insets: style.chartInsets),
                priceRange: ChartPlot.zoomed(
                    ChartPlot.priceRange(for: candles, padding: style.pricePadding),
                    by: yZoom
                ),
                style: style,
                scale: .currency,
                yAxisDecimalPlaces: yAxisDecimalPlaces
            )

            Canvas { context, _ in
                plot.drawGrid(&context)

                // Only the series is clipped: a zoomed-in price scale pushes candles
                // past the plot, and the axis labels live outside it by design.
                context.drawLayer { layer in
                    layer.clip(to: Path(plot.plotRect))
                    if showVolume {
                        drawVolumeBars(context: &layer, plot: plot)
                    }
                    drawCandles(context: &layer, plot: plot)
                }

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

    // MARK: - Volume

    /// Turnover bars along the bottom of the plot, scaled so the busiest candle in
    /// view fills the strip. Uses quote volume (USDT), not base volume, so the bars
    /// compare as money rather than as coins.
    ///
    /// Sources other than Binance report no volume, which leaves every bar at zero —
    /// the guard below draws nothing rather than a flat smear along the axis.
    private func drawVolumeBars(context: inout GraphicsContext, plot: ChartPlot) {
        let peak = candles.map(\.quoteVolume).max() ?? 0
        guard peak > 0 else { return }

        let pane = plot.volumeRect(fraction: style.volumePaneFraction)
        let slotWidth = plot.slotWidth(forCount: candles.count)
        let barWidth = (slotWidth * style.candleBodyFraction).clamped(to: style.candleBodyMin...style.candleBodyMax)

        for (i, candle) in candles.enumerated() {
            guard candle.quoteVolume > 0 else { continue }
            let height = max(pane.height * CGFloat(candle.quoteVolume / peak), style.minBodyHeight)
            let x = plot.x(forIndex: i, slotWidth: slotWidth)
            let color = candle.closePrice >= candle.openPrice ? bullishColor : bearishColor

            let bar = CGRect(x: x - barWidth / 2, y: pane.maxY - height,
                             width: barWidth, height: height)
            context.fill(Path(bar), with: .color(color.opacity(style.volumeOpacity)))
        }
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
