import AppKit
import SwiftUI

struct BitcoinPowerLawChartView: View {
    @ObservedObject var viewModel: ChartViewModel
    let chartHeight: CGFloat
    let cardHeight: CGFloat?
    let onRemove: () -> Void
    let onChanged: () -> Void
    let onZoomRegion: (NSView) -> Void
    let onAxisRegion: (NSView) -> Void

    @State private var showSettings = false

    var body: some View {
        VStack(spacing: 4) {
            header
            Group {
                if viewModel.isFetching && viewModel.powerLawHistory.isEmpty {
                    ProgressView("Loading Bitstamp history…").frame(maxWidth: .infinity, maxHeight: .infinity)
                } else if viewModel.powerLawHistory.isEmpty {
                    ContentUnavailableView(
                        "Couldn’t Load History", systemImage: "wifi.exclamationmark",
                        description: Text(viewModel.errorMessage ?? "No cached history is available.")
                    )
                    .overlay(alignment: .bottom) {
                        Button("Retry") { Task { await viewModel.fetchPowerLaw(force: true) } }
                    }
                } else {
                    BitcoinPowerLawPlotView(
                        history: viewModel.powerLawHistory,
                        config: viewModel.bitcoinPowerLaw ?? .default,
                        xZoom: viewModel.powerLawXZoom,
                        yZoom: viewModel.yZoom,
                        onZoomRegion: onZoomRegion,
                        onAxisRegion: onAxisRegion)
                }
            }
            if let warning = viewModel.powerLawWarning {
                Label(warning, systemImage: "exclamationmark.triangle")
                    .font(.caption2).foregroundStyle(.orange).lineLimit(1)
            }
        }
        .padding(6)
        .frame(height: cardHeight ?? chartHeight + ChartLayout.cardChrome)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 10))
        .task { await viewModel.fetchPowerLaw() }
        .sheet(isPresented: $showSettings) {
            BitcoinPowerLawSettingsSheet(initial: viewModel.bitcoinPowerLaw ?? .default) { config in
                viewModel.bitcoinPowerLaw = config
                onChanged()
            }
        }
    }

    private var header: some View {
        HStack {
            Button {
                showSettings = true
            } label: {
                Label("Bitcoin Power Law", systemImage: "chart.xyaxis.line")
                    .font(.headline).fontWeight(.bold)
            }.buttonStyle(.plain)
            Spacer()
            if let latest = viewModel.powerLawHistory.last,
                let model = (viewModel.bitcoinPowerLaw ?? .default).price(
                    days: BitcoinPowerLawModel.daysSinceGenesis(latest.date))
            {
                Text(
                    "BTC $\(latest.close.formatted(.number.precision(.fractionLength(0)))) · Model $\(model.formatted(.number.precision(.fractionLength(0))))"
                )
                .font(.caption.monospacedDigit()).foregroundStyle(.secondary)
            }
            Button(role: .destructive, action: onRemove) { Image(systemName: "trash") }.buttonStyle(.plain)
        }
    }
}

private struct BitcoinPowerLawPlotView: View {
    let history: [BitcoinDailyClose]
    let config: BitcoinPowerLawConfig
    let xZoom: Double
    let yZoom: Double
    let onZoomRegion: (NSView) -> Void
    let onAxisRegion: (NSView) -> Void

    @State private var hoverLocation: CGPoint?

    private let insets = EdgeInsets(top: 22, leading: 18, bottom: 25, trailing: 58)

    var body: some View {
        GeometryReader { geometry in
            if let plot = BitcoinPowerLawPlot(history: history, config: config) {
                Canvas { context, size in
                    draw(context: &context, size: size, plot: plot.scaled(xZoom: xZoom, yZoom: yZoom))
                }
                .accessibilityLabel("Bitcoin daily closes and power-law corridor on logarithmic axes")
                .background(ZoomHitRegion(onResolve: onZoomRegion))
                .overlay {
                    hoverOverlay(
                        size: geometry.size,
                        plot: plot.scaled(xZoom: xZoom, yZoom: yZoom)
                    )
                }
                .overlay {
                    PowerLawHoverRegion(location: $hoverLocation)
                }
                .overlay(alignment: .trailing) {
                    PriceAxisRegion(onResolve: onAxisRegion).frame(width: insets.trailing)
                }
            }
        }
    }

    private func hoverOverlay(size: CGSize, plot: BitcoinPowerLawPlot) -> some View {
        Canvas { context, _ in
            let rect = plotRect(size: size)
            guard let hoverLocation, rect.contains(hoverLocation) else { return }

            var horizontal = Path()
            horizontal.move(to: CGPoint(x: rect.minX, y: hoverLocation.y))
            horizontal.addLine(to: CGPoint(x: rect.maxX, y: hoverLocation.y))
            var vertical = Path()
            vertical.move(to: CGPoint(x: hoverLocation.x, y: rect.minY))
            vertical.addLine(to: CGPoint(x: hoverLocation.x, y: rect.maxY))
            let chartStyle = ChartStyle.default
            let stroke = StrokeStyle(
                lineWidth: chartStyle.crosshairLineWidth,
                dash: chartStyle.crosshairDashPattern
            )
            context.stroke(horizontal, with: .color(chartStyle.crosshairColor), style: stroke)
            context.stroke(vertical, with: .color(chartStyle.crosshairColor), style: stroke)

            let normalized = CGPoint(
                x: (hoverLocation.x - rect.minX) / rect.width,
                y: (rect.maxY - hoverLocation.y) / rect.height
            )
            let value = plot.value(at: normalized)
            let date = BitcoinPowerLawModel.genesisDate.addingTimeInterval(value.days * 86_400)
            let priceLabel = resolveBadge(preciseCurrency(value.price), in: context)
            let priceY = (hoverLocation.y - priceLabel.size.height / 2).clamped(
                to: rect.minY...max(rect.minY, rect.maxY - priceLabel.size.height)
            )
            drawBadge(
                priceLabel,
                in: CGRect(
                    origin: CGPoint(x: rect.maxX + 4, y: priceY),
                    size: priceLabel.size
                ),
                context: &context
            )
            let dateLabel = resolveBadge(
                date.formatted(.dateTime.year().month(.abbreviated).day()), in: context)
            let dateX = (hoverLocation.x - dateLabel.size.width / 2).clamped(
                to: rect.minX...max(rect.minX, rect.maxX - dateLabel.size.width)
            )
            drawBadge(
                dateLabel,
                in: CGRect(
                    origin: CGPoint(x: dateX, y: rect.maxY - dateLabel.size.height - 2),
                    size: dateLabel.size
                ),
                context: &context
            )
        }
        .allowsHitTesting(false)
    }

    private func resolveBadge(_ text: String, in context: GraphicsContext) -> (
        text: GraphicsContext.ResolvedText, size: CGSize
    ) {
        let resolved = context.resolve(
            Text(text).font(.caption2).bold().foregroundStyle(.white)
        )
        let measured = resolved.measure(in: CGSize(width: 140, height: 20))
        return (resolved, CGSize(width: measured.width + 10, height: 18))
    }

    private func drawBadge(
        _ badge: (text: GraphicsContext.ResolvedText, size: CGSize),
        in rect: CGRect,
        context: inout GraphicsContext
    ) {
        context.fill(
            Path(roundedRect: rect, cornerRadius: 3),
            with: .color(ChartStyle.default.crosshairLabelColor)
        )
        context.draw(badge.text, at: CGPoint(x: rect.midX, y: rect.midY))
    }

    private func plotRect(size: CGSize) -> CGRect {
        CGRect(
            x: insets.leading,
            y: insets.top,
            width: max(1, size.width - insets.leading - insets.trailing),
            height: max(1, size.height - insets.top - insets.bottom)
        )
    }

    private func draw(context: inout GraphicsContext, size: CGSize, plot: BitcoinPowerLawPlot) {
        let rect = CGRect(
            x: insets.leading, y: insets.top,
            width: max(1, size.width - insets.leading - insets.trailing),
            height: max(1, size.height - insets.top - insets.bottom))
        func screen(_ point: CGPoint) -> CGPoint {
            CGPoint(x: rect.minX + point.x * rect.width, y: rect.maxY - point.y * rect.height)
        }
        drawAxes(context: &context, rect: rect, plot: plot, screen: screen)

        let end = BitcoinPowerLawModel.projectionEnd(from: Date())
        let firstDays = BitcoinPowerLawModel.daysSinceGenesis(history[0].date)
        let endDays = BitcoinPowerLawModel.daysSinceGenesis(end)
        let samples = (0...240).map { index in
            pow(10, log10(firstDays) + Double(index) / 240 * (log10(endDays) - log10(firstDays)))
        }
        let upper = samples.compactMap { days -> CGPoint? in
            guard let price = config.bandPrices(days: days)?.upper, let point = plot.point(days: days, price: price)
            else { return nil }
            return screen(point)
        }
        let lower = samples.reversed().compactMap { days -> CGPoint? in
            guard let price = config.bandPrices(days: days)?.lower, let point = plot.point(days: days, price: price)
            else { return nil }
            return screen(point)
        }
        var corridor = Path()
        if let first = upper.first {
            corridor.move(to: first)
            upper.dropFirst().forEach { corridor.addLine(to: $0) }
            lower.forEach { corridor.addLine(to: $0) }
            corridor.closeSubpath()
            context.fill(corridor, with: .color(.orange.opacity(0.14)))
        }
        stroke(
            samples: samples, price: { config.price(days: $0) }, color: .orange, width: 2, context: &context,
            plot: plot, screen: screen)
        stroke(
            samples: samples, price: { config.bandPrices(days: $0)?.lower }, color: .orange.opacity(0.65), width: 1,
            context: &context, plot: plot, screen: screen)
        stroke(
            samples: samples, price: { config.bandPrices(days: $0)?.upper }, color: .orange.opacity(0.65), width: 1,
            context: &context, plot: plot, screen: screen)

        var dataPath = Path()
        for (index, value) in history.enumerated() {
            guard let point = plot.point(days: BitcoinPowerLawModel.daysSinceGenesis(value.date), price: value.close)
            else { continue }
            index == 0 ? dataPath.move(to: screen(point)) : dataPath.addLine(to: screen(point))
        }
        context.stroke(dataPath, with: .color(.blue), lineWidth: 1.25)
        context.draw(Text("BTC/USD").font(.caption2).foregroundColor(.blue), at: CGPoint(x: rect.minX + 28, y: 9))
        context.draw(
            Text("Model / corridor").font(.caption2).foregroundColor(.orange), at: CGPoint(x: rect.minX + 125, y: 9))
    }

    private func stroke(
        samples: [Double], price: (Double) -> Double?, color: Color, width: CGFloat,
        context: inout GraphicsContext, plot: BitcoinPowerLawPlot, screen: (CGPoint) -> CGPoint
    ) {
        var path = Path()
        var started = false
        for days in samples {
            guard let price = price(days), let point = plot.point(days: days, price: price) else { continue }
            if started {
                path.addLine(to: screen(point))
            } else {
                path.move(to: screen(point))
                started = true
            }
        }
        context.stroke(path, with: .color(color), lineWidth: width)
    }

    private func drawAxes(
        context: inout GraphicsContext, rect: CGRect, plot: BitcoinPowerLawPlot,
        screen: (CGPoint) -> CGPoint
    ) {
        let minPower = Int(floor(plot.yRange.lowerBound))
        let maxPower = Int(ceil(plot.yRange.upperBound))
        for power in minPower...maxPower {
            let price = pow(10, Double(power))
            guard let p = plot.point(days: pow(10, plot.xRange.lowerBound), price: price) else { continue }
            let y = screen(p).y
            var line = Path()
            line.move(to: CGPoint(x: rect.minX, y: y))
            line.addLine(to: CGPoint(x: rect.maxX, y: y))
            context.stroke(line, with: .color(.secondary.opacity(0.18)), lineWidth: 1)
            context.draw(
                Text(currency(price)).font(.caption2).foregroundColor(.secondary), at: CGPoint(x: rect.maxX + 27, y: y))
        }
        let calendar = BitcoinPowerLawModel.utcCalendar
        let firstYear = calendar.component(.year, from: history[0].date)
        let finalYear = calendar.component(.year, from: BitcoinPowerLawModel.projectionEnd(from: Date()))
        let stride = max(1, Int(ceil(Double(finalYear - firstYear) / max(3, Double(rect.width / 80)))))
        for year in Swift.stride(from: firstYear, through: finalYear, by: stride) {
            guard let date = calendar.date(from: DateComponents(year: year, month: 1, day: 1)),
                let p = plot.point(
                    days: BitcoinPowerLawModel.daysSinceGenesis(date), price: pow(10, plot.yRange.lowerBound))
            else { continue }
            let x = screen(p).x
            context.draw(
                Text(String(year)).font(.caption2).foregroundColor(.secondary), at: CGPoint(x: x, y: rect.maxY + 11))
        }
    }

    private func currency(_ value: Double) -> String {
        if value >= 1_000_000 { return "$\(Int(value / 1_000_000))M" }
        if value >= 1_000 { return "$\(Int(value / 1_000))K" }
        if value >= 1 { return "$\(Int(value))" }
        return "$\(value.formatted(.number.precision(.significantDigits(1))))"
    }

    private func preciseCurrency(_ value: Double) -> String {
        let digits = value >= 1_000 ? 0 : value >= 1 ? 2 : 6
        return "$"
            + value.formatted(
                .number.grouping(.automatic).precision(.fractionLength(digits)))
    }
}

private struct PowerLawHoverRegion: NSViewRepresentable {
    @Binding var location: CGPoint?

    func makeCoordinator() -> Coordinator {
        Coordinator(location: $location)
    }

    func makeNSView(context: Context) -> NSView {
        let view = TrackingView()
        view.onMove = { context.coordinator.location.wrappedValue = $0 }
        view.onExit = { context.coordinator.location.wrappedValue = nil }
        return view
    }

    func updateNSView(_ nsView: NSView, context: Context) {}

    final class Coordinator {
        var location: Binding<CGPoint?>

        init(location: Binding<CGPoint?>) {
            self.location = location
        }
    }

    private final class TrackingView: NSView {
        var onMove: ((CGPoint) -> Void)?
        var onExit: (() -> Void)?
        private var trackingArea: NSTrackingArea?

        override var isFlipped: Bool { true }
        override func hitTest(_ point: NSPoint) -> NSView? { nil }

        override func updateTrackingAreas() {
            if let trackingArea { removeTrackingArea(trackingArea) }
            let area = NSTrackingArea(
                rect: bounds,
                options: [.mouseMoved, .mouseEnteredAndExited, .activeInKeyWindow],
                owner: self
            )
            addTrackingArea(area)
            trackingArea = area
            super.updateTrackingAreas()
        }

        override func mouseMoved(with event: NSEvent) {
            onMove?(convert(event.locationInWindow, from: nil))
        }

        override func mouseEntered(with event: NSEvent) {
            onMove?(convert(event.locationInWindow, from: nil))
        }

        override func mouseExited(with event: NSEvent) {
            onExit?()
        }
    }
}
