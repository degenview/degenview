import SwiftUI

// MARK: - LineChartView

/// A continuous price line with a soft gradient beneath it, for sources that report
/// one price per timestamp rather than OHLC candles (Polymarket).
///
/// Shares all axis, grid and current-price rendering with `CandleChartView` through
/// `ChartPlot`; only the series drawing differs. The line takes `bullishColor` or
/// `bearishColor` from the series' overall direction, so the same two persisted
/// per-chart colors cover both renderers.
struct LineChartView: View {
    let points: [KlineData]
    var chartHeight: CGFloat
    var style: ChartStyle = .default

    // Per-chart overrides
    var bullishColor: Color = .green
    var bearishColor: Color = .red
    var yAxisDecimalPlaces: Int? = nil  // nil = auto-detect
    var scale: PriceScale = .currency
    var yZoom: Double = 1               // 1 = auto-fit; >1 = taller series
    /// Precomputed by the view model over the full buffer, already trimmed to these
    /// points — so a long-period overlay is warmed up at the left edge.
    var indicators: IndicatorSeries = .none

    // Hand-drawn trend lines, plus the one being drawn right now.
    var trendLines: [TrendLine] = []
    var trendDraft: (start: TrendAnchor, end: TrendAnchor)? = nil
    var selectedTrendLineID: UUID? = nil
    /// Endpoint circles show only while the line tool is armed.
    var showTrendHandles: Bool = false

    // Measuring rectangles, plus the one being drawn right now. Cleared with the tool,
    // so there is no armed flag to gate them on.
    var rulers: [RulerRect] = []
    var rulerDraft: (start: TrendAnchor, end: TrendAnchor)? = nil

    /// Green when the series ends above where it started, red otherwise.
    private var lineColor: Color {
        guard let first = points.first, let last = points.last else { return bullishColor }
        return last.closePrice >= first.closePrice ? bullishColor : bearishColor
    }

    var body: some View {
        GeometryReader { geometry in
            let plot = ChartPlot.make(
                points: points,
                size: geometry.size,
                yZoom: yZoom,
                scale: scale,
                yAxisDecimalPlaces: yAxisDecimalPlaces,
                style: style
            )

            Canvas { context, _ in
                plot.drawGrid(&context)

                // Only the series is clipped: a zoomed-in price scale pushes the line
                // past the plot, and the axis labels live outside it by design.
                context.drawLayer { layer in
                    layer.clip(to: Path(plot.plotRect))
                    drawSeries(context: &layer, plot: plot)

                    if let bands = indicators.bollinger {
                        plot.drawBollinger(&layer, bands: bands)
                    }
                    plot.drawEMA(&layer, values: indicators.ema)

                    // Above the series, still inside its clip — a line anchored off
                    // the visible window must not spill into the price gutter.
                    plot.drawTrendLines(
                        &layer,
                        lines: trendLines,
                        draft: trendDraft,
                        selectedID: selectedTrendLineID,
                        showHandles: showTrendHandles,
                        points: points
                    )

                    plot.drawRulers(
                        &layer,
                        rects: rulers,
                        draft: rulerDraft,
                        points: points,
                        bullish: bullishColor,
                        bearish: bearishColor
                    )
                }

                plot.drawRSI(&context, values: indicators.rsi)

                if let last = points.last {
                    plot.drawCurrentPriceLine(&context, price: last.closePrice, color: lineColor)
                    plot.drawCurrentPriceBox(&context, price: last.closePrice, color: lineColor)
                }

                plot.drawTimeGrid(&context, points: points)
            }
        }
        .frame(height: max(ChartLayout.chartMinHeight, chartHeight))
        .clipped()
    }

    // MARK: - Series

    private func drawSeries(context: inout GraphicsContext, plot: ChartPlot) {
        guard points.count >= 2 else {
            drawSinglePoint(context: &context, plot: plot)
            return
        }

        let slotWidth = plot.slotWidth(forCount: points.count)
        let coordinates = points.enumerated().map { index, point in
            CGPoint(x: plot.x(forIndex: index, slotWidth: slotWidth), y: plot.y(for: point.closePrice))
        }

        // Gradient under the line, closed along the bottom of the plot.
        // Built with explicit `addLine` calls — `addLines` opens a *new* subpath,
        // which would leave the baseline disconnected from the series.
        var area = Path()
        area.move(to: CGPoint(x: coordinates[0].x, y: plot.plotRect.maxY))
        for point in coordinates {
            area.addLine(to: point)
        }
        area.addLine(to: CGPoint(x: coordinates[coordinates.count - 1].x, y: plot.plotRect.maxY))
        area.closeSubpath()

        context.fill(
            area,
            with: .linearGradient(
                Gradient(colors: [lineColor.opacity(style.areaFillOpacity), lineColor.opacity(0)]),
                startPoint: CGPoint(x: plot.plotRect.midX, y: plot.plotRect.minY),
                endPoint: CGPoint(x: plot.plotRect.midX, y: plot.plotRect.maxY)
            )
        )

        var line = Path()
        line.addLines(coordinates)
        context.stroke(
            line,
            with: .color(lineColor),
            style: StrokeStyle(lineWidth: style.lineWidth, lineCap: .round, lineJoin: .round)
        )
    }

    /// A brand-new market can report a single price; draw it as a dot so the card
    /// isn't blank.
    private func drawSinglePoint(context: inout GraphicsContext, plot: ChartPlot) {
        guard let only = points.first else { return }
        let slotWidth = plot.slotWidth(forCount: points.count)
        let center = CGPoint(x: plot.x(forIndex: 0, slotWidth: slotWidth), y: plot.y(for: only.closePrice))
        let radius = style.lineWidth
        context.fill(
            Path(ellipseIn: CGRect(x: center.x - radius, y: center.y - radius,
                                   width: radius * 2, height: radius * 2)),
            with: .color(lineColor)
        )
    }
}

#Preview {
    LineChartView(
        points: MockData.sampleKlines,
        chartHeight: 220
    )
    .frame(width: 400, height: 260)
    .padding()
}
