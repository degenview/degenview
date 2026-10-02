import SwiftUI

/// The Overview's portfolio-value line, drawn by hand like the candle charts.
///
/// Segments and the fill beneath them are green while the portfolio is above what was put in
/// (`value ≥ netContributions`) and red below it. A live point at today's value ends the line.
struct PortfolioHistoryChart: View {
    let snapshots: [PortfolioSnapshot]
    let currentValue: Decimal
    let currency: PortfolioCurrency
    let range: PortfolioHistoryRange
    let privacy: Bool
    let isLoading: Bool
    var loadingMessage = "Loading market data…"
    @State private var hoveredIndex: Int?

    private let leftMargin: CGFloat = 12
    private let rightMargin: CGFloat = 76
    private let topMargin: CGFloat = 12
    private let bottomMargin: CGFloat = 28
    private static let labelSpacing: CGFloat = 110

    private func preparedPoints(now: Date = Date()) -> [PortfolioSnapshot] {
        let ranged = range.filter(snapshots, now: now)
        guard !isLoading, let latest = ranged.last ?? snapshots.last else { return ranged }
        let live = PortfolioSnapshot(
            portfolioID: latest.portfolioID, timestamp: now, value: currentValue,
            netContributions: latest.netContributions, realizedPnL: latest.realizedPnL,
            unrealizedPnL: currentValue - latest.netContributions, isComplete: latest.isComplete
        )
        return ranged + [live]
    }

    var body: some View {
        let points = preparedPoints()
        GeometryReader { geometry in
            Canvas { context, size in draw(context: &context, size: size, points: points) }
                .contentShape(Rectangle())
                .onContinuousHover { phase in
                    guard !isLoading, !privacy, !points.isEmpty else {
                        hoveredIndex = nil
                        return
                    }
                    switch phase {
                    case .active(let location):
                        let width = max(1, geometry.size.width - leftMargin - rightMargin)
                        let fraction = ((location.x - leftMargin) / width).clamped(to: 0...1)
                        let index = points.count == 1 ? 0 : Int((fraction * CGFloat(points.count - 1)).rounded())
                        if hoveredIndex != index { hoveredIndex = index }
                    case .ended: hoveredIndex = nil
                    }
                }
        }
        .overlay {
            if isLoading {
                HStack(spacing: 8) {
                    ProgressView().controlSize(.small)
                    Text(loadingMessage).font(.callout)
                }
                .padding(.horizontal, 14)
                .padding(.vertical, 10)
                .background(.thinMaterial, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                .accessibilityElement(children: .combine)
                .accessibilityLabel(loadingMessage)
            } else if privacy {
                Label("Values hidden", systemImage: "eye.slash")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            } else if points.isEmpty {
                ContentUnavailableView(
                    "No History Yet", systemImage: "chart.line.uptrend.xyaxis",
                    description: Text("History builds from transaction dates and available market candles."))
            }
        }
        .animation(.easeOut(duration: 0.25), value: isLoading)
        .accessibilityLabel(
            isLoading
                ? loadingMessage
                : privacy ? "Portfolio history values hidden" : "Portfolio value history, \(points.count) observations")
    }

    private func draw(context: inout GraphicsContext, size: CGSize, points: [PortfolioSnapshot]) {
        guard !isLoading, !privacy, !points.isEmpty else { return }
        let plot = CGRect(
            x: leftMargin, y: topMargin,
            width: max(1, size.width - leftMargin - rightMargin),
            height: max(1, size.height - topMargin - bottomMargin))
        let values = points.map(\.value)
        guard var minimum = values.min(), var maximum = values.max() else { return }
        if minimum == maximum {
            minimum -= 1
            maximum += 1
        }
        let padding = (maximum - minimum) * Decimal(string: "0.08")!
        minimum -= padding
        maximum += padding
        let spread = maximum - minimum

        func position(_ index: Int) -> CGPoint {
            let x = points.count == 1 ? plot.midX : plot.minX + plot.width * CGFloat(index) / CGFloat(points.count - 1)
            let normalized = CGFloat(((points[index].value - minimum) / spread).doubleValue)
            return CGPoint(x: x, y: plot.maxY - plot.height * normalized)
        }

        for tick in 0...4 {
            let fraction = CGFloat(tick) / 4
            let y = plot.maxY - plot.height * fraction
            var grid = Path()
            grid.move(to: CGPoint(x: plot.minX, y: y))
            grid.addLine(to: CGPoint(x: plot.maxX, y: y))
            context.stroke(grid, with: .color(.secondary.opacity(0.18)), lineWidth: 1)
            let value = minimum + spread * Decimal(Double(fraction))
            context.draw(
                Text(shortMoney(value)).font(.caption2).foregroundStyle(.secondary),
                at: CGPoint(x: plot.maxX + 6, y: y), anchor: .leading)
        }

        if points.count > 1 {
            // One polygon per stretch of same-colored segments, not per segment — adjacent
            // fills would show anti-aliasing seams. Each segment takes the color of the
            // point it ends on, like the stroke below.
            var runStart = 0
            for index in 1..<points.count {
                let profit = points[index].value >= points[index].netContributions
                let isLast = index == points.count - 1
                let nextProfit = isLast ? profit : points[index + 1].value >= points[index + 1].netContributions
                guard isLast || nextProfit != profit else { continue }
                context.fillAreaUnderLine(
                    (runStart...index).map(position), plot: plot, color: profit ? .green : .red,
                    opacity: ChartStyle.default.areaFillOpacity)
                runStart = index
            }

            for index in 1..<points.count {
                var segment = Path()
                segment.move(to: position(index - 1))
                segment.addLine(to: position(index))
                let profit = points[index].value >= points[index].netContributions
                context.stroke(
                    segment, with: .color(profit ? .green : .red),
                    style: StrokeStyle(lineWidth: 2.25, lineCap: .round, lineJoin: .round))
            }
        }

        // Marks the live point, so "now" is findable even on a flat line.
        let last = points.count - 1
        let lastPoint = position(last)
        let lastProfit = points[last].value >= points[last].netContributions
        context.fill(
            Path(ellipseIn: CGRect(x: lastPoint.x - 3.5, y: lastPoint.y - 3.5, width: 7, height: 7)),
            with: .color(lastProfit ? .green : .red))

        var previousLabel: String?
        for index in axisLabelIndices(count: points.count, width: plot.width) {
            let label = dateLabel(points[index].timestamp)
            // A coarse format over a short stretch repeats ("Sep 2026" twice); say it once.
            guard label != previousLabel else { continue }
            previousLabel = label
            let anchor: UnitPoint = index == 0 && points.count > 1 ? .topLeading : .top
            context.draw(
                Text(label).font(.caption2).foregroundStyle(.secondary),
                at: CGPoint(x: position(index).x, y: plot.maxY + 8), anchor: anchor)
        }

        guard let index = hoveredIndex, points.indices.contains(index) else { return }
        drawHover(context: &context, plot: plot, point: points[index], at: position(index))
    }

    private func drawHover(context: inout GraphicsContext, plot: CGRect, point: PortfolioSnapshot, at spot: CGPoint) {
        var vertical = Path()
        vertical.move(to: CGPoint(x: spot.x, y: plot.minY))
        vertical.addLine(to: CGPoint(x: spot.x, y: plot.maxY))
        var horizontal = Path()
        horizontal.move(to: CGPoint(x: plot.minX, y: spot.y))
        horizontal.addLine(to: CGPoint(x: plot.maxX, y: spot.y))
        context.stroke(
            vertical, with: .color(.secondary.opacity(0.65)), style: StrokeStyle(lineWidth: 1, dash: [4, 3]))
        context.stroke(
            horizontal, with: .color(.secondary.opacity(0.45)), style: StrokeStyle(lineWidth: 1, dash: [4, 3]))
        let tint: Color = point.value >= point.netContributions ? .green : .red
        context.fill(Path(ellipseIn: CGRect(x: spot.x - 4.5, y: spot.y - 4.5, width: 9, height: 9)), with: .color(tint))
        context.stroke(
            Path(ellipseIn: CGRect(x: spot.x - 4.5, y: spot.y - 4.5, width: 9, height: 9)),
            with: .color(Color(nsColor: .windowBackgroundColor)), lineWidth: 1.5)

        let value = context.resolve(Text(money(point.value)).font(.callout.weight(.semibold)).foregroundStyle(.primary))
        let date = context.resolve(
            Text(point.timestamp.formatted(date: .abbreviated, time: .shortened)).font(.caption)
                .foregroundStyle(.secondary))
        let valueSize = value.measure(in: CGSize(width: 260, height: 40))
        let dateSize = date.measure(in: CGSize(width: 260, height: 40))
        let box = CGSize(
            width: max(valueSize.width, dateSize.width) + 20, height: valueSize.height + dateSize.height + 14)
        let x = min(plot.maxX - box.width / 2, max(plot.minX + box.width / 2, spot.x))
        // Above the point unless that would leave the plot, then below.
        let above = spot.y - box.height - 12
        let y = above >= plot.minY - 4 ? above : spot.y + 12
        let rect = CGRect(x: x - box.width / 2, y: y, width: box.width, height: box.height)
        let shape = Path(roundedRect: rect, cornerRadius: 8, style: .continuous)
        context.fill(shape, with: .color(Color(nsColor: .windowBackgroundColor).opacity(0.94)))
        context.stroke(shape, with: .color(.secondary.opacity(0.35)), lineWidth: 1)
        context.draw(value, at: CGPoint(x: rect.midX, y: rect.minY + 6 + valueSize.height / 2), anchor: .center)
        context.draw(
            date, at: CGPoint(x: rect.midX, y: rect.minY + 8 + valueSize.height + dateSize.height / 2), anchor: .center)
    }

    /// Evenly spaced points along the x axis, about one label per `labelSpacing` points of width.
    private func axisLabelIndices(count: Int, width: CGFloat) -> [Int] {
        guard count > 1 else { return [0] }
        let labels = max(2, min(count, Int(width / Self.labelSpacing)))
        return (0..<labels).map { Int((Double($0) * Double(count - 1) / Double(labels - 1)).rounded()) }
    }

    private func money(_ value: Decimal) -> String { currency.format(value) }
    private func shortMoney(_ value: Decimal) -> String { currency.formatCompact(value) }
    private func dateLabel(_ date: Date) -> String {
        switch range {
        case .oneDay: date.formatted(date: .omitted, time: .shortened)
        case .oneWeek, .oneMonth: date.formatted(.dateTime.month(.abbreviated).day())
        case .oneYear, .all: date.formatted(.dateTime.month(.abbreviated).year())
        }
    }
}
