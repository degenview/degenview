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
    let onUpdateTicker: (String, DataSourceType, String?) -> Void
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
                onUpdateTicker: onUpdateTicker,
                onRemove: onRemove,
                onStyleChanged: onStyleChanged
            )
        }
        .onChange(of: showSettings) { _, new in
            onSettingsPresented?(new)
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

    // MARK: - Chart Area

    @ViewBuilder
    private var chartArea: some View {
        // Computed once per layout pass and shared by both renderers — the warm-up
        // candles ahead of the visible window never reach the chart itself.
        let indicators = viewModel.indicators

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
        onUpdateTicker: { _, _, _ in },
        onStyleChanged: {}
    )
    .frame(width: 400)
    .padding()
}
