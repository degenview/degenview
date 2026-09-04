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
    @Environment(\.colorScheme) private var colorScheme

    let points: [KlineData]
    var chartHeight: CGFloat
    var style: ChartStyle = .default

    // Per-chart overrides
    var bullishColor: Color = .green
    var bearishColor: Color = .red
    var yAxisDecimalPlaces: Int? = nil  // nil = auto-detect
    var scale: PriceScale = .currency
    var yZoom: Double = 1  // 1 = auto-fit; >1 = taller series
    /// Precomputed by the view model over the full buffer, already trimmed to these
    /// points — so a long-period overlay is warmed up at the left edge.
    var indicators: IndicatorSeries = .none

    /// Additional series for multi-choice Polymarket events. When non-empty, all series
    /// (including the primary represented by `points`) are drawn as plain lines without
    /// the gradient fill. Each entry carries its data, color, and a short label drawn
    /// inline at the right edge of the line.
    var extraSeries: [(data: [KlineData], color: Color, label: String)] = []

    // Hand-drawn trend lines, plus the one being drawn right now.
    var trendLines: [TrendLine] = []
    var trendDraft: (start: TrendAnchor, end: TrendAnchor)? = nil
    var selectedTrendLineID: UUID? = nil
    /// Endpoint circles show only while the line tool is armed.
    var showTrendHandles: Bool = false
    var fibonacciRetracements: [FibonacciRetracementDrawing] = []
    var fibonacciDraft: (start: TrendAnchor, end: TrendAnchor)? = nil
    var selectedFibonacciID: UUID? = nil

    @State private var hoveredSeriesIndex: Int?

    // Measuring rectangles, plus the one being drawn right now. Cleared with the tool,
    // so there is no armed flag to gate them on.
    var rulers: [RulerRect] = []
    var rulerDraft: (start: TrendAnchor, end: TrendAnchor)? = nil

    /// Green when the series ends above where it started, red otherwise.
    private var lineColor: Color {
        guard let first = points.first, let last = points.last else { return bullishColor }
        return last.closePrice >= first.closePrice ? bullishColor : bearishColor
    }

    /// Color for the current-price marker line — first series color in multi mode.
    private var currentPriceColor: Color {
        extraSeries.first?.color ?? lineColor
    }

    var body: some View {
        GeometryReader { geometry in
            // For Y-scale: include all series so every line fits within the axis.
            let yScalePoints = extraSeries.isEmpty ? points : (points + extraSeries.flatMap(\.data))
            let plot = ChartPlot.make(
                points: yScalePoints,
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
                    plot.drawTrendFlips(
                        &layer,
                        flips: indicators.trendFlips,
                        points: points,
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
                        points: points
                    )
                    plot.drawFibonacciRetracements(
                        &layer, drawings: fibonacciRetracements, draft: fibonacciDraft,
                        selectedID: selectedFibonacciID, showHandles: showTrendHandles,
                        points: points, decimalPlaces: yAxisDecimalPlaces)

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
                    plot.drawCurrentPriceLine(&context, price: last.closePrice, color: currentPriceColor)
                    plot.drawCurrentPriceBox(&context, price: last.closePrice, color: currentPriceColor)
                }

                plot.drawTimeGrid(&context, points: points)
            }
            .onContinuousHover { phase in
                switch phase {
                case .active(let location):
                    hoveredSeriesIndex = seriesIndex(near: location, plot: plot)
                case .ended:
                    hoveredSeriesIndex = nil
                }
            }
        }
        .frame(height: max(0, chartHeight))
        .clipped()
    }

    // MARK: - Series

    private func drawSeries(context: inout GraphicsContext, plot: ChartPlot) {
        if extraSeries.isEmpty {
            // Single series: gradient fill + line.
            drawLine(context: &context, plot: plot, pts: points, color: lineColor, withGradient: true)
        } else {
            // Multi-series: plain lines, no fill, then labels.
            for series in extraSeries {
                drawLine(context: &context, plot: plot, pts: series.data, color: series.color, withGradient: false)
            }
            drawSeriesLabels(context: &context, plot: plot)
        }
    }

    /// Draw short inline labels at the right edge of each series line.
    private func drawSeriesLabels(context: inout GraphicsContext, plot: ChartPlot) {
        struct LabelPos {
            let seriesIndex: Int
            var y: CGFloat
            let color: Color
            let label: String
        }

        var positions: [LabelPos] = extraSeries.enumerated().compactMap { index, series in
            guard let last = series.data.last else { return nil }
            let y = plot.y(for: last.closePrice)
            return LabelPos(seriesIndex: index, y: y, color: series.color, label: series.label)
        }

        // Sort top-to-bottom, then spread so labels don't stack.
        positions.sort { $0.y < $1.y }
        let minGap: CGFloat = 13
        for i in positions.indices.dropFirst() {
            if positions[i].y - positions[i - 1].y < minGap {
                positions[i].y = positions[i - 1].y + minGap
            }
        }
        // Clamp inside the plot area.
        for i in positions.indices {
            positions[i].y = max(plot.plotRect.minY + 6, min(plot.plotRect.maxY - 6, positions[i].y))
        }

        let rightX = plot.plotRect.maxX - 4
        for pos in positions {
            let isHovered = hoveredSeriesIndex == pos.seriesIndex
            let text = context.resolve(
                Text(pos.label)
                    .font(.system(size: 9))
                    .foregroundColor(isHovered || colorScheme == .dark ? .white : .black)
            )
            let size = text.measure(in: CGSize(width: 180, height: 24))
            let background = CGRect(
                x: rightX - size.width - 7,
                y: pos.y - size.height / 2 - 3,
                width: size.width + 7,
                height: size.height + 6
            )
            if isHovered {
                context.fill(
                    Path(roundedRect: background, cornerRadius: 4),
                    with: .color(.black.opacity(0.72))
                )
                context.fill(
                    Path(
                        roundedRect: CGRect(
                            x: background.minX, y: background.minY, width: 3, height: background.height),
                        cornerRadius: 1.5
                    ),
                    with: .color(pos.color)
                )
            }
            context.draw(text, at: CGPoint(x: rightX - 3, y: pos.y), anchor: .trailing)
        }
    }

    /// Find the closest plotted series within a comfortable pointer target.
    private func seriesIndex(near location: CGPoint, plot: ChartPlot) -> Int? {
        guard plot.plotRect.insetBy(dx: -6, dy: -6).contains(location) else { return nil }
        var nearest: (index: Int, distance: CGFloat)?

        for (index, series) in extraSeries.enumerated() where !series.data.isEmpty {
            let slotWidth = plot.slotWidth(forCount: series.data.count)
            let points = series.data.enumerated().map { pointIndex, point in
                CGPoint(
                    x: plot.x(forIndex: pointIndex, slotWidth: slotWidth),
                    y: plot.y(for: point.closePrice)
                )
            }
            for pointIndex in points.indices {
                let distance: CGFloat
                if pointIndex == points.startIndex {
                    distance = hypot(location.x - points[pointIndex].x, location.y - points[pointIndex].y)
                } else {
                    distance = Self.distance(
                        from: location, toSegmentFrom: points[pointIndex - 1], to: points[pointIndex])
                }
                if distance < (nearest?.distance ?? .infinity) {
                    nearest = (index, distance)
                }
            }
        }

        guard let nearest, nearest.distance <= 7 else { return nil }
        return nearest.index
    }

    private static func distance(from point: CGPoint, toSegmentFrom start: CGPoint, to end: CGPoint) -> CGFloat {
        let dx = end.x - start.x
        let dy = end.y - start.y
        let lengthSquared = dx * dx + dy * dy
        guard lengthSquared > 0 else { return hypot(point.x - start.x, point.y - start.y) }
        let projection = ((point.x - start.x) * dx + (point.y - start.y) * dy) / lengthSquared
        let t = max(0, min(1, projection))
        return hypot(point.x - (start.x + t * dx), point.y - (start.y + t * dy))
    }

    private func drawLine(
        context: inout GraphicsContext, plot: ChartPlot, pts: [KlineData], color: Color, withGradient: Bool
    ) {
        guard pts.count >= 2 else {
            if pts.count == 1 { drawSinglePoint(context: &context, plot: plot, pts: pts, color: color) }
            return
        }

        let slotWidth = plot.slotWidth(forCount: pts.count)
        let coordinates = pts.enumerated().map { index, point in
            CGPoint(x: plot.x(forIndex: index, slotWidth: slotWidth), y: plot.y(for: point.closePrice))
        }

        if withGradient {
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
                    Gradient(colors: [color.opacity(style.areaFillOpacity), color.opacity(0)]),
                    startPoint: CGPoint(x: plot.plotRect.midX, y: plot.plotRect.minY),
                    endPoint: CGPoint(x: plot.plotRect.midX, y: plot.plotRect.maxY)
                )
            )
        }

        var line = Path()
        line.addLines(coordinates)
        context.stroke(
            line,
            with: .color(color),
            style: StrokeStyle(lineWidth: style.lineWidth, lineCap: .round, lineJoin: .round)
        )
    }

    /// A brand-new market can report a single price; draw it as a dot so the card
    /// isn't blank.
    private func drawSinglePoint(context: inout GraphicsContext, plot: ChartPlot, pts: [KlineData], color: Color) {
        guard let only = pts.first else { return }
        let slotWidth = plot.slotWidth(forCount: pts.count)
        let center = CGPoint(x: plot.x(forIndex: 0, slotWidth: slotWidth), y: plot.y(for: only.closePrice))
        let radius = style.lineWidth
        context.fill(
            Path(
                ellipseIn: CGRect(
                    x: center.x - radius, y: center.y - radius,
                    width: radius * 2, height: radius * 2)),
            with: .color(color)
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
