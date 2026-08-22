import SwiftUI
import AppKit

struct ChartCardView: View {
    @ObservedObject var viewModel: ChartViewModel
    var chartHeight: CGFloat
    let onRemove: () -> Void
    let onRetry: () -> Void
    /// Hands the card's backing `NSView` to the scroll-zoom monitor.
    let onZoomRegion: (NSView) -> Void
    /// Hands the Y-axis gutter's `NSView` to the price-zoom drag monitor.
    let onAxisRegion: (NSView) -> Void
    /// Hands the plot area's `NSView` to the trend-line drawing monitor.
    var onPlotRegion: (NSView) -> Void = { _ in }
    /// Whether any tool is armed — drives the crosshair cursor over the plot.
    var isToolArmed: Bool = false
    /// Narrower than `isToolArmed`: only the trend-line tool shows endpoint handles.
    var showTrendHandles: Bool = false
    /// The tab's crosshair, if this card is inside one. Optional so previews stand alone.
    var crosshair: CrosshairTracker? = nil
    /// Called when the pointer leaves this card — the mouse monitor can't see that.
    var onCrosshairExit: () -> Void = {}
    let onUpdateTicker: (String, DataSourceType, String?, [PmSeriesConfig]?) -> Void
    let onStyleChanged: () -> Void
    var onSettingsPresented: ((Bool) -> Void)? = nil

    @State private var showSettings = false
    @State private var iconURL: URL?

    var body: some View {
        VStack(spacing: 2) {
            headerView
            chartArea
        }
        .padding(6)
        .frame(height: chartHeight + ChartLayout.cardChrome)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 10))
        .background(ZoomHitRegion(onResolve: onZoomRegion))
        .task(id: viewModel.iconKey) {
            iconURL = nil
            iconURL = await IconResolver.shared.iconURL(
                ticker: viewModel.ticker,
                source: viewModel.source,
                baseSymbol: viewModel.baseSymbol
            )
        }
        .sheet(isPresented: $showSettings) {
            ChartSettingsSheet(
                viewModel: viewModel,
                onUpdateTicker: { sym, src, name, series in onUpdateTicker(sym, src, name, series) },
                onRemove: onRemove,
                onStyleChanged: onStyleChanged
            )
        }
        .onChange(of: showSettings) { _, new in
            onSettingsPresented?(new)
        }
        .onChange(of: viewModel.editingLineID) { old, new in
            if (old == nil) != (new == nil) { onSettingsPresented?(new != nil) }
        }
    }

    // MARK: - Header

    private var headerView: some View {
        HStack(alignment: .firstTextBaseline) {
            Button {
                showSettings = true
            } label: {
                VStack(alignment: .leading, spacing: 2) {
                    HStack(spacing: 6) {
                        TickerIconView(symbol: viewModel.baseSymbol, url: iconURL)
                        // Market questions are long — keep the header on one line.
                        Text(viewModel.title)
                            .font(.headline)
                            .fontWeight(.bold)
                            .lineLimit(1)
                            .truncationMode(.tail)
                        Image(systemName: viewModel.source.icon)
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                        Image(systemName: "gearshape.fill")
                            .font(.caption2)
                            .foregroundStyle(.secondary.opacity(0.6))
                    }

                    if let price = viewModel.currentPrice {
                        Text(PriceFormatter.headline(price, scale: viewModel.priceScale))
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                    }
                }
            }
            .buttonStyle(.plain)

            Spacer()

            if let change = viewModel.priceChangePercent {
                HStack(spacing: 4) {
                    Image(systemName: viewModel.priceChangeIsPositive ? "arrow.up.right" : "arrow.down.right")
                    Text(abs(change), format: .number.precision(.fractionLength(2)))
                        + Text("%")
                }
                .font(.caption.weight(.medium))
                .foregroundStyle(viewModel.priceChangeIsPositive ? .green : .red)
            }
        }
    }

    // MARK: - PM Series Legend

    /// Horizontally-scrolling row of toggleable colored chips, one per Polymarket choice.
    private var pmSeriesLegend: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 4) {
                ForEach(viewModel.pmSeries) { series in
                    let color = viewModel.pmColor(for: series.tokenID)
                    Button {
                        viewModel.togglePmSeries(series.tokenID)
                    } label: {
                        HStack(spacing: 3) {
                            Circle()
                                .fill(color)
                                .frame(width: 6, height: 6)
                            Text(series.label)
                                .font(.caption2)
                                .lineLimit(1)
                        }
                        .padding(.horizontal, 5)
                        .padding(.vertical, 2)
                        .background(
                            series.enabled
                                ? color.opacity(0.15)
                                : Color.secondary.opacity(0.08),
                            in: Capsule()
                        )
                        .foregroundStyle(series.enabled ? .primary : .secondary)
                    }
                    .buttonStyle(.plain)
                }
            }
        }
    }

    // MARK: - Chart Area

    @ViewBuilder
    private var chartArea: some View {
        // Computed once per layout pass and shared by both renderers — the warm-up
        // candles ahead of the visible window never reach the chart itself.
        let indicators = viewModel.indicators
        let multiSeries = viewModel.pmVisibleSeries.map { (data: $0.data, color: $0.color, label: $0.label) }

        return Group {
            if viewModel.usesLineChart {
                LineChartView(
                    points: viewModel.visibleKlines,
                    chartHeight: chartHeight,
                    bullishColor: viewModel.bullishColor,
                    bearishColor: viewModel.bearishColor,
                    yAxisDecimalPlaces: viewModel.yAxisDecimalPlaces,
                    scale: viewModel.priceScale,
                    yZoom: viewModel.yZoom,
                    indicators: indicators,
                    extraSeries: multiSeries,
                    trendLines: viewModel.trendLines,
                    trendDraft: viewModel.trendDraft,
                    selectedTrendLineID: viewModel.selectedLineID,
                    showTrendHandles: showTrendHandles,
                    rulers: viewModel.rulers,
                    rulerDraft: viewModel.rulerDraft
                )
            } else {
                CandleChartView(
                    candles: viewModel.visibleKlines,
                    chartHeight: chartHeight,
                    bullishColor: viewModel.bullishColor,
                    bearishColor: viewModel.bearishColor,
                    yAxisDecimalPlaces: viewModel.yAxisDecimalPlaces,
                    yZoom: viewModel.yZoom,
                    showVolume: viewModel.showVolume,
                    indicators: indicators,
                    trendLines: viewModel.trendLines,
                    trendDraft: viewModel.trendDraft,
                    selectedTrendLineID: viewModel.selectedLineID,
                    showTrendHandles: showTrendHandles,
                    rulers: viewModel.rulers,
                    rulerDraft: viewModel.rulerDraft
                )
            }
        }
        .overlay {
            PlotHitRegion(isArmed: isToolArmed, onResolve: onPlotRegion)
        }
        .overlay {
            if let crosshair {
                CrosshairOverlay(viewModel: viewModel, tracker: crosshair)
                    .allowsHitTesting(false)
            }
        }
        // The mouse monitor only sees moves inside the window, so a pointer that leaves
        // it altogether would strand the crosshair on the last chart it touched.
        .onHover { isInside in
            guard !isInside else { return }
            onCrosshairExit()
        }
        .overlay(alignment: .trailing) {
            PriceAxisRegion(onResolve: onAxisRegion)
                .frame(width: ChartStyle.default.chartInsets.trailing)
        }
        .overlay(alignment: .bottom) {
            if let error = viewModel.errorMessage {
                HStack(spacing: 6) {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .foregroundStyle(.orange)
                    Text(error)
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                    Button("Retry", action: onRetry)
                        .font(.caption2)
                        .buttonStyle(.plain)
                        .foregroundStyle(.blue)
                }
                .padding(.horizontal, 8)
                .padding(.vertical, 4)
                .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 4))
            }
        }
        .overlay(alignment: .top) {
            if let lineID = viewModel.editingLineID {
                TrendLineEditor(
                    viewModel: viewModel,
                    lineID: lineID,
                    onChange: onStyleChanged,
                    onDismiss: { viewModel.editingLineID = nil }
                )
                .padding(.top, 8)
            }
        }
    }

}

private struct TrendLineEditor: View {
    @ObservedObject var viewModel: ChartViewModel
    let lineID: UUID
    let onChange: () -> Void
    let onDismiss: () -> Void

    private var line: TrendLine? {
        return viewModel.trendLines.first { $0.id == lineID }
    }

    var body: some View {
        HStack(spacing: 8) {
            Menu {
                ForEach(TrendLineColor.allCases, id: \.self) { option in
                    Button {
                        viewModel.setColor(option, for: lineID)
                        onChange()
                    } label: {
                        HStack {
                            Circle()
                                .fill(option.color)
                                .frame(width: 10, height: 10)
                            Text(option.title)
                        }
                    }
                }
            } label: {
                HStack(spacing: 6) {
                    Circle()
                        .fill(line?.resolvedColor.color ?? .blue)
                        .frame(width: 10, height: 10)
                    Text(line?.resolvedColor.title ?? TrendLineColor.blue.title)
                    Image(systemName: "chevron.down")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
                .frame(minWidth: 78)
                .padding(.horizontal, 8)
                .padding(.vertical, 5)
                .background(.quaternary, in: RoundedRectangle(cornerRadius: 5))
            }
            .menuIndicator(.hidden)
            .buttonStyle(.plain)
            .accessibilityLabel("Line color")

            Menu {
                ForEach(TrendLineThickness.allCases, id: \.self) { option in
                    Button {
                        viewModel.setThickness(option, for: lineID)
                        onChange()
                    } label: {
                        Image(nsImage: option.menuImage)
                    }
                    .accessibilityLabel(option.title)
                }
            } label: {
                HStack(spacing: 4) {
                    ThicknessSample(thickness: line?.resolvedThickness ?? .medium)
                        .frame(width: 46, height: 18)
                    Image(systemName: "chevron.down")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
                .padding(.horizontal, 8)
                .padding(.vertical, 5)
                .background(.quaternary, in: RoundedRectangle(cornerRadius: 5))
            }
            .menuIndicator(.hidden)
            .buttonStyle(.plain)
            .accessibilityLabel("Line thickness")

            Button(action: onDismiss) {
                Image(systemName: "xmark")
                    .font(.caption.bold())
            }
            .buttonStyle(.plain)
            .foregroundStyle(.secondary)
            .accessibilityLabel("Close line editor")
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 8))
        .overlay {
            RoundedRectangle(cornerRadius: 8)
                .stroke(.secondary.opacity(0.25), lineWidth: 1)
        }
        .shadow(radius: 4, y: 2)
    }
}

private struct ThicknessSample: View {
    let thickness: TrendLineThickness

    var body: some View {
        ZStack {
            Color.clear
            Capsule()
                .fill(Color.primary)
                .frame(width: 48, height: CGFloat(thickness.rawValue))
        }
        .frame(width: 54, height: 18)
        .contentShape(Rectangle())
        .accessibilityLabel(thickness.title)
    }
}

private extension TrendLineThickness {
    /// Native menus discard arbitrary SwiftUI shapes from their rows, but retain an
    /// `NSImage`. A template image also lets AppKit choose the correct light/dark tint.
    var menuImage: NSImage {
        let image = NSImage(size: NSSize(width: 54, height: 16), flipped: false) { rect in
            NSColor.labelColor.setStroke()
            let line = NSBezierPath()
            line.lineWidth = CGFloat(rawValue)
            line.lineCapStyle = .round
            line.move(to: NSPoint(x: 3, y: rect.midY))
            line.line(to: NSPoint(x: rect.maxX - 3, y: rect.midY))
            line.stroke()
            return true
        }
        image.isTemplate = true
        return image
    }
}

/// The crosshair, drawn in its own thin `Canvas` above the series.
///
/// Separate from `CandleChartView`/`LineChartView` on purpose: the pointer moves 60×/sec,
/// and folding this into the series canvas would re-run every indicator and redraw every
/// candle that often. Only this view observes the tracker, so only this view redraws.
///
/// It rebuilds the geometry rather than being handed it — the overlay is exactly
/// co-extensive with the chart canvas, so `plot(in:)` on the same size reproduces what the
/// renderer drew, the same trick the hit regions rely on.
private struct CrosshairOverlay: View {
    @ObservedObject var viewModel: ChartViewModel
    @ObservedObject var tracker: CrosshairTracker

    var body: some View {
        GeometryReader { geometry in
            Canvas { context, _ in
                guard let crosshair = tracker.current else { return }
                viewModel.plot(in: geometry.size).drawCrosshair(
                    &context,
                    crosshair: crosshair,
                    isOwner: crosshair.ownerID == viewModel.uniqueID,
                    points: viewModel.visibleKlines
                )
            }
        }
    }
}

/// Non-interactive AppKit view stretched over one chart card.
///
/// Scroll-zoom runs off a window-wide `NSEvent` monitor, which knows nothing
/// about what the pointer is over. Handing it a real `NSView` lets it hit-test
/// the scroll location against AppKit geometry instead of reconstructing
/// SwiftUI's flipped coordinate space by hand.
/// `hitTest` returns nil so the card's own controls keep every mouse event.
private struct ZoomHitRegion: NSViewRepresentable {
    let onResolve: (NSView) -> Void

    func makeNSView(context: Context) -> NSView {
        let view = PassthroughView()
        onResolve(view)
        return view
    }

    func updateNSView(_ nsView: NSView, context: Context) {}

    private final class PassthroughView: NSView {
        override func hitTest(_ point: NSPoint) -> NSView? { nil }
    }
}

/// Marks the Y-axis gutter as the drag target for vertical price zoom, and shows the
/// resize cursor over it. The drag itself is handled by `ContentViewModel`'s mouse
/// monitor, which hit-tests against this view.
///
/// The monitor rather than `mouseDown`/`mouseDragged` overrides here: SwiftUI's
/// hosting view claims mouse events for the card's `.onDrag` reordering before AppKit
/// ever offers them to a child view, so an event-handling `NSView` in this position
/// never fires. A local monitor sees events ahead of the window, which is also how
/// scroll-zoom already works.
/// Marks the chart canvas as the target for the trend-line tool, and shows the
/// crosshair over it while a tool is armed.
///
/// Same arrangement as `PriceAxisRegion` and for the same reason — the drawing itself
/// is handled by `ContentViewModel`'s mouse monitor, which hit-tests against this view.
/// Flipped, so its coordinates match the `Canvas` space `ChartPlot` maps into and the
/// monitor can convert a click without reconstructing the flip by hand.
private struct PlotHitRegion: NSViewRepresentable {
    let isArmed: Bool
    let onResolve: (NSView) -> Void

    func makeNSView(context: Context) -> NSView {
        let view = PlotRegionView()
        view.isArmed = isArmed
        onResolve(view)
        return view
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        guard let view = nsView as? PlotRegionView, view.isArmed != isArmed else { return }
        view.isArmed = isArmed
        view.window?.invalidateCursorRects(for: view)
    }

    private final class PlotRegionView: NSView {
        var isArmed = false

        override var isFlipped: Bool { true }

        /// Cursor rects only — the monitor does the rest, and letting this view take
        /// hits would swallow clicks meant for the card underneath.
        override func hitTest(_ point: NSPoint) -> NSView? { nil }

        override func resetCursorRects() {
            guard isArmed else { return }
            // Stop short of the price gutter, which keeps its own resize cursor.
            var rect = bounds
            rect.size.width = max(0, rect.width - ChartStyle.default.chartInsets.trailing)
            addCursorRect(rect, cursor: .crosshair)
        }
    }
}

private struct PriceAxisRegion: NSViewRepresentable {
    let onResolve: (NSView) -> Void

    func makeNSView(context: Context) -> NSView {
        let view = AxisRegionView()
        onResolve(view)
        return view
    }

    func updateNSView(_ nsView: NSView, context: Context) {}

    private final class AxisRegionView: NSView {
        /// Cursor rects only — the monitor does the rest, and letting this view take
        /// hits would swallow clicks meant for the card underneath.
        override func hitTest(_ point: NSPoint) -> NSView? { nil }

        override func resetCursorRects() {
            addCursorRect(bounds, cursor: .resizeUpDown)
        }
    }
}

#Preview {
    ChartCardView(
        viewModel: {
            let vm = ChartViewModel(ticker: "BTC")
            vm.klineData = MockData.sampleKlines
            vm.currentPrice = 68432.15
            return vm
        }(),
        chartHeight: 220,
        onRemove: {},
        onRetry: {},
        onZoomRegion: { _ in },
        onAxisRegion: { _ in },
        onUpdateTicker: { _, _, _, _ in },
        onStyleChanged: {}
    )
    .frame(width: 400)
    .padding()
}
