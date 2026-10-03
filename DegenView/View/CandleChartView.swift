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
    var yZoom: Double = 1  // 1 = auto-fit; >1 = taller candles
    var showVolume: Bool = false
    /// Precomputed by the view model over the full buffer, already trimmed to these
    /// candles — so a long-period overlay is warmed up at the left edge.
    var indicators: IndicatorSeries = .none
    var pine: PineVisualOutput = .empty

    // Hand-drawn trend lines, plus the one being drawn right now.
    var trendLines: [TrendLine] = []
    var trendDraft: (start: TrendAnchor, end: TrendAnchor)? = nil
    var selectedTrendLineID: UUID? = nil
    /// Endpoint circles show only while the line tool is armed.
    var showTrendHandles: Bool = false
    var fibonacciRetracements: [FibonacciRetracementDrawing] = []
    var fibonacciDraft: (start: TrendAnchor, end: TrendAnchor)? = nil
    var selectedFibonacciID: UUID? = nil

    // Measuring rectangles, the one being drawn right now, and which is selected or
    // hovered. Cleared with the tool, so there is no armed flag to gate them on.
    var rulerOverlay: RulerOverlayState = .empty

    var body: some View {
        GeometryReader { geometry in
            let plot = ChartPlot.make(
                points: candles,
                size: geometry.size,
                yZoom: yZoom,
                scale: .currency,
                yAxisDecimalPlaces: yAxisDecimalPlaces,
                style: style
            )

            let script = PineChartLayer(pine: pine, candles: candles, style: style)

            Canvas { context, _ in
                plot.drawGrid(&context)

                // Only the series is clipped: a zoomed-in price scale pushes candles
                // past the plot, and the axis labels live outside it by design.
                context.drawLayer { layer in
                    layer.clip(to: Path(plot.plotRect))
                    // `overlay=false` scripts draw in their own pane below the chart.
                    if pine.overlay {
                        script.drawBackground(&layer, plot: plot)
                    }
                    if showVolume {
                        drawVolumeBars(context: &layer, plot: plot)
                    }
                    drawCandles(context: &layer, plot: plot, script: script)
                    if pine.overlay {
                        script.drawForeground(&layer, plot: plot)
                    }

                    // Price-scale overlays share the candles' clip: a zoomed-in
                    // domain pushes them past the plot just the same.
                    if let bands = indicators.bollinger {
                        plot.drawBollinger(&layer, bands: bands)
                    }
                    plot.drawEMA(&layer, values: indicators.ema)
                    plot.drawTrendFlips(
                        &layer,
                        flips: indicators.trendFlips,
                        points: candles,
                        bullish: bullishColor,
                        bearish: bearishColor
                    )

                    // Above the series, still inside its clip — a line anchored off
                    // the visible window must not spill into the price gutter.
                    plot.drawTrendLines(
                        &layer,
                        lines: trendLines,
                        draft: trendDraft,
                        selectedID: selectedTrendLineID,
                        showHandles: showTrendHandles,
                        points: candles
                    )
                    plot.drawFibonacciRetracements(
                        &layer, drawings: fibonacciRetracements, draft: fibonacciDraft,
                        selectedID: selectedFibonacciID, showHandles: showTrendHandles,
                        points: candles, decimalPlaces: yAxisDecimalPlaces)

                    plot.drawRulers(
                        &layer,
                        overlay: rulerOverlay,
                        points: candles,
                        bullish: bullishColor,
                        bearish: bearishColor
                    )
                }

                // Tables pin to the plot's corners and are never price-anchored, so they
                // sit outside the series clip.
                if pine.overlay {
                    script.drawTables(&context, plot: plot)
                }

                plot.drawRSI(&context, values: indicators.rsi)

                if let last = candles.last {
                    let color = last.closePrice >= last.openPrice ? bullishColor : bearishColor
                    plot.drawCurrentPriceLine(&context, price: last.closePrice, color: color)
                    plot.drawCurrentPriceBox(&context, price: last.closePrice, color: color)
                }

                // After the current-price pill: a measurement's end prices are what the
                // user is reading, and they take its slot in the gutter.
                plot.drawRulerPriceTags(
                    &context, overlay: rulerOverlay, bullish: bullishColor, bearish: bearishColor)

                plot.drawTimeGrid(&context, points: candles)
            }
        }
        .frame(height: max(0, chartHeight))
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

        let pane = plot.bottomPane(fraction: style.volumePaneFraction)
        let slotWidth = plot.slotWidth(forCount: candles.count)
        let barWidth = (slotWidth * style.candleBodyFraction).clamped(
            to: style.candleBodyMin...style.candleBodyMax)

        for (i, candle) in candles.enumerated() {
            guard candle.quoteVolume > 0 else { continue }
            let height = max(pane.height * CGFloat(candle.quoteVolume / peak), style.minBodyHeight)
            let x = plot.x(forIndex: i, slotWidth: slotWidth)
            let color = candle.closePrice >= candle.openPrice ? bullishColor : bearishColor

            let bar = CGRect(
                x: x - barWidth / 2, y: pane.maxY - height,
                width: barWidth, height: height)
            context.fill(Path(bar), with: .color(color.opacity(style.volumeOpacity)))
        }
    }

    // MARK: - Candles

    /// `barcolor()` recolors candles whichever pane the script draws in, as on TradingView.
    private func drawCandles(context: inout GraphicsContext, plot: ChartPlot, script: PineChartLayer) {
        let slotWidth = plot.slotWidth(forCount: candles.count)
        let priceRangeSpan = plot.priceRange.max - plot.priceRange.min
        let dojiAbsThreshold = priceRangeSpan * style.dojiThreshold
        let bodyWidth = (slotWidth * style.candleBodyFraction).clamped(
            to: style.candleBodyMin...style.candleBodyMax)
        let wickWidth = (slotWidth * style.wickFraction).clamped(to: style.wickMin...style.wickMax)

        for (i, candle) in candles.enumerated() {
            let x = plot.x(forIndex: i, slotWidth: slotWidth)
            let wickTop = plot.y(for: candle.highPrice)
            let wickBottom = plot.y(for: candle.lowPrice)
            let bodyTop = plot.y(for: max(candle.openPrice, candle.closePrice))
            let bodyBottom = plot.y(for: min(candle.openPrice, candle.closePrice))

            let isDoji = abs(candle.closePrice - candle.openPrice) <= dojiAbsThreshold
            let isBullish = candle.closePrice > candle.openPrice

            let candleColor: Color
            if let scripted = script.barColor(at: i) {
                candleColor = scripted
            } else if isDoji {
                candleColor = style.dojiColor
            } else if isBullish {
                candleColor = bullishColor
            } else {
                candleColor = bearishColor
            }

            // Wick
            let wickRect = CGRect(
                x: x - wickWidth / 2, y: wickTop,
                width: wickWidth, height: max(wickBottom - wickTop, 0.5))
            context.fill(Path(wickRect), with: .color(candleColor))

            // Body
            let bodyHeight = bodyBottom - bodyTop
            if bodyHeight < style.minBodyHeight {
                let midY = (bodyTop + bodyBottom) / 2
                let bodyRect = CGRect(
                    x: x - bodyWidth / 2, y: midY - style.minBodyHeight / 2,
                    width: bodyWidth, height: style.minBodyHeight)
                context.fill(Path(bodyRect), with: .color(candleColor))
            } else {
                let bodyRect = CGRect(
                    x: x - bodyWidth / 2, y: bodyTop,
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
