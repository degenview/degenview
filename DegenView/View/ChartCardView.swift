import AppKit
import SwiftUI

struct ChartCardView: View {
    @ObservedObject var viewModel: ChartViewModel
    var chartHeight: CGFloat
    var timeRange: TimeRange = .oneDay
    var cardHeight: CGFloat? = nil
    let onRemove: () -> Void
    let onRetry: () -> Void
    let isFavorite: Bool
    let onToggleFavorite: () -> Void
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
    let onStyleChanged: () -> Void
    var onSettingsPresented: ((Bool) -> Void)? = nil
    var onLineEditorPresented: ((Bool) -> Void)? = nil
    var onPaperBuy: () -> Void = {}
    var onPaperSell: () -> Void = {}
    var paperConnected = false
    var paperPositions: [PaperPosition] = []
    var paperOrders: [PaperOrder] = []
    var paperAccountCurrency: PaperCurrency = .USD
    var paperUnrealizedPnL: (PaperPosition) -> Decimal = { _ in 0 }
    var onPaperModify: (PaperOrder, Decimal) -> Void = { _, _ in }
    var onPaperCancel: (PaperOrder) -> Void = { _ in }
    var onPaperClose: (PaperPosition) -> Void = { _ in }

    @State private var showSettings = false
    @State private var showAlertEditor = false
    @StateObject private var portfolioStore = PortfolioStore.shared
    @Environment(\.colorScheme) private var colorScheme

    @ViewBuilder
    /// Over a ruler's edge or corner the cursor says what a drag would do.
    private var plotCursor: PlotCursor {
        guard let hover = viewModel.hoveredRuler else { return .crosshair }
        switch hover.part {
        case .edge: return .move
        case .corner(let corner): return .resize(corner)
        }
    }

    var body: some View {
        if let config = viewModel.portfolioChart {
            PortfolioChartCard(
                config: Binding(
                    get: { viewModel.portfolioChart ?? config },
                    set: {
                        viewModel.portfolioChart = $0
                        onStyleChanged()
                    }
                ),
                store: portfolioStore,
                chartHeight: chartHeight,
                cardHeight: cardHeight,
                onRemove: onRemove
            )
        } else if viewModel.coinMarketCapChart != nil {
            CoinMarketCapChartView(
                viewModel: viewModel, chartHeight: chartHeight, cardHeight: cardHeight,
                onRemove: onRemove, onChanged: onStyleChanged
            )
        } else if viewModel.isBitcoinPowerLaw {
            BitcoinPowerLawChartView(
                viewModel: viewModel, chartHeight: chartHeight, cardHeight: cardHeight,
                onRemove: onRemove, onChanged: onStyleChanged,
                onZoomRegion: onZoomRegion, onAxisRegion: onAxisRegion
            )
        } else {
            marketCard
        }
    }

    private var marketCard: some View {
        VStack(spacing: 2) {
            headerView
            VStack(spacing: 0) {
                chartArea
                // Below, not inside, the chart area: its overlays and hit regions must
                // keep the price canvas's size, or `viewModel.plot(in:)` would drift.
                if showsPinePane {
                    PineScriptPaneView(
                        pine: viewModel.pineOutput, candles: viewModel.visibleKlines, height: pinePaneHeight)
                }
            }
        }
        .padding(6)
        .frame(height: cardHeight ?? chartHeight + ChartLayout.cardChrome)
        .clipped()
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 10))
        .background(ZoomHitRegion(onResolve: onZoomRegion))
        .onChange(of: colorScheme, initial: true) { _, scheme in
            viewModel.setPineTheme(scheme == .dark ? .dark : .light)
        }
        .task(id: viewModel.iconKey) {
            await viewModel.resolveCoinSymbol()
        }
        .sheet(isPresented: $showSettings) {
            ChartSettingsSheet(
                viewModel: viewModel,
                onRemove: onRemove,
                onStyleChanged: onStyleChanged
            )
        }
        .sheet(isPresented: $showAlertEditor) { PriceAlertEditor(asset: alertAsset) }
        .onChange(of: showSettings) { _, new in
            onSettingsPresented?(new)
        }
        .onChange(of: viewModel.editingLineID) { old, new in
            if (old == nil) != (new == nil) { onLineEditorPresented?(new != nil) }
        }
        .onChange(of: viewModel.editingFibonacciID) { old, new in
            if (old == nil) != (new == nil) { onLineEditorPresented?(new != nil) }
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
                        ChartIconView(viewModel: viewModel)
                        // Market questions are long — keep the header on one line.
                        Text(viewModel.title)
                            .font(.headline)
                            .fontWeight(.bold)
                            .lineLimit(1)
                            .truncationMode(.tail)
                        Image(systemName: "gearshape.fill")
                            .font(.caption2)
                            .foregroundStyle(.secondary.opacity(0.6))
                        if viewModel.replayTimestamp != nil {
                            Label("Replay", systemImage: "clock.arrow.circlepath")
                                .font(.caption2.weight(.semibold))
                                .foregroundStyle(.orange)
                                .accessibilityLabel("Historical replay mode")
                        }
                    }

                    if let choice = viewModel.leadingMarketChoice {
                        Text(
                            "\(choice.label) · \(PriceFormatter.headline(choice.price, scale: viewModel.priceScale))"
                        )
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                    } else if let price = viewModel.displayedPrice {
                        Text(PriceFormatter.headline(price, scale: viewModel.priceScale))
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                    }
                }
            }
            .buttonStyle(.plain)

            Spacer()

            Button(action: onToggleFavorite) {
                Image(systemName: isFavorite ? "star.fill" : "star")
            }
            .buttonStyle(.plain)
            .accessibilityLabel(isFavorite ? "Remove from Favorites" : "Add to Favorites")
            .help(isFavorite ? "Remove from Favorites" : "Add to Favorites")

            if !viewModel.source.isPredictionMarket {
                Button {
                    showAlertEditor = true
                } label: {
                    Image(systemName: "bell.badge")
                }
                .buttonStyle(.plain).help("Create Price Alert")
            }

            if paperConnected, let price = viewModel.displayedPrice {
                PaperQuickTradeButtons(
                    priceText: PriceFormatter.format(price, scale: viewModel.priceScale),
                    title: viewModel.title, onSell: onPaperSell, onBuy: onPaperBuy)
            }

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

    private var alertAsset: PortfolioAsset {
        PortfolioAsset(
            key: viewModel.iconKey, symbol: viewModel.baseSymbol, name: viewModel.title,
            source: viewModel.source, quoteCurrency: PortfolioCurrency(quoteSymbol: viewModel.marketPair?.quote),
            metadata: ["apiSymbol": viewModel.apiSymbol])
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

    private var showsPinePane: Bool { viewModel.showsPinePane }

    private var pinePaneHeight: CGFloat { viewModel.pinePaneHeight(forChartHeight: chartHeight) }

    @ViewBuilder
    private var chartArea: some View {
        // Computed once per layout pass and shared by both renderers — the warm-up
        // candles ahead of the visible window never reach the chart itself.
        let indicators = viewModel.indicators
        let multiSeries = viewModel.pmVisibleSeries.map { (data: $0.data, color: $0.color, label: $0.label) }

        Group {
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
                    fibonacciRetracements: visibleFibonacciRetracements,
                    fibonacciDraft: viewModel.fibonacciDraft,
                    selectedFibonacciID: viewModel.selectedFibonacciID,
                    rulerOverlay: viewModel.rulerOverlay
                )
            } else {
                CandleChartView(
                    candles: viewModel.visibleKlines,
                    chartHeight: chartHeight - pinePaneHeight,
                    bullishColor: viewModel.bullishColor,
                    bearishColor: viewModel.bearishColor,
                    yAxisDecimalPlaces: viewModel.yAxisDecimalPlaces,
                    yZoom: viewModel.yZoom,
                    showVolume: viewModel.showVolume,
                    indicators: indicators,
                    pine: viewModel.pineOutput,
                    trendLines: viewModel.trendLines,
                    trendDraft: viewModel.trendDraft,
                    selectedTrendLineID: viewModel.selectedLineID,
                    showTrendHandles: showTrendHandles,
                    fibonacciRetracements: visibleFibonacciRetracements,
                    fibonacciDraft: viewModel.fibonacciDraft,
                    selectedFibonacciID: viewModel.selectedFibonacciID,
                    rulerOverlay: viewModel.rulerOverlay
                )
            }
        }
        .overlay {
            PlotHitRegion(isArmed: isToolArmed, cursor: plotCursor, onResolve: onPlotRegion)
        }
        .overlay {
            if let crosshair {
                CrosshairOverlay(viewModel: viewModel, tracker: crosshair)
                    .allowsHitTesting(false)
            }
        }
        .overlay {
            if let date = viewModel.replaySelectionTimestamp {
                ReplaySelectionMarker(viewModel: viewModel, date: date)
                    .allowsHitTesting(false)
            }
        }
        .overlay {
            if paperConnected && viewModel.replayTimestamp == nil {
                PaperChartTradingOverlay(
                    candles: viewModel.visibleKlines, positions: paperPositions,
                    orders: paperOrders, accountCurrency: paperAccountCurrency, unrealizedPnL: paperUnrealizedPnL,
                    onModify: onPaperModify, onCancel: onPaperCancel, onClose: onPaperClose)
            }
        }
        // The mouse monitor only sees moves inside the window, so a pointer that leaves
        // it altogether would strand the crosshair on the last chart it touched.
        .onHover { isInside in
            guard !isInside else { return }
            onCrosshairExit()
            viewModel.setRulerHover(nil)
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
            } else if let fibID = viewModel.editingFibonacciID {
                FibonacciEditor(viewModel: viewModel, drawingID: fibID, onChange: onStyleChanged)
                    .padding(.top, 8)
            }
        }
    }

    private var visibleFibonacciRetracements: [FibonacciRetracementDrawing] {
        viewModel.fibonacciRetracements.filter { $0.timeframeVisibility.includes(timeRange) }
    }

}

private struct PortfolioChartCard: View {
    @Binding var config: PortfolioChartConfig
    @ObservedObject var store: PortfolioStore
    let chartHeight: CGFloat
    let cardHeight: CGFloat?
    let onRemove: () -> Void
    @State private var valuesHidden = false

    private var portfolio: Portfolio? { store.portfolio(namedBy: config.portfolioID) }
    private var holdings: [PortfolioHolding] { store.holdings(for: config.portfolioID) }
    private var totalValue: Decimal { holdings.compactMap(\.currentValue).reduce(0, +) }
    private var currency: PortfolioCurrency { portfolio?.baseCurrency ?? .USD }
    private var history: [PortfolioSnapshot] {
        if config.range == .oneDay {
            let intraday = store.intradayHistory(for: config.portfolioID)
            if !intraday.isEmpty { return intraday }
        }
        let all = store.history(for: config.portfolioID)
        guard let duration = config.range.duration else { return all }
        return all.filter { $0.timestamp >= Date().addingTimeInterval(-duration) }
    }
    private var timeframeChange: PortfolioValueChange? {
        history.first.map { PortfolioValueChange(from: $0.value, to: totalValue) }
    }

    var body: some View {
        VStack(spacing: 8) {
            HStack {
                Image(systemName: "briefcase.fill").foregroundStyle(.secondary)
                VStack(alignment: .leading, spacing: 1) {
                    Text(config.kind.title).font(.headline).bold()
                    Text(portfolio?.name ?? "All Portfolios").font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                if config.kind == .valueChart {
                    Button {
                        valuesHidden.toggle()
                    } label: {
                        Image(systemName: valuesHidden ? "eye.slash" : "eye")
                    }
                    .buttonStyle(.plain).foregroundStyle(.secondary)
                    .help(valuesHidden ? "Show Y-axis values" : "Hide Y-axis values")
                    .accessibilityLabel(valuesHidden ? "Show Y-axis values" : "Hide Y-axis values")
                }
                Button(role: .destructive, action: onRemove) {
                    Image(systemName: "xmark.circle.fill")
                }
                .buttonStyle(.plain).foregroundStyle(.secondary)
                .accessibilityLabel("Remove portfolio chart")
            }

            switch config.kind {
            case .valueChart:
                HStack {
                    timeframeChangeSummary
                    Spacer(minLength: 8)
                    Picker("History range", selection: $config.range) {
                        ForEach(PortfolioChartRange.allCases) { Text($0.rawValue).tag($0) }
                    }
                    .pickerStyle(.segmented).labelsHidden().controlSize(.mini).font(.caption2).fixedSize()
                }
                PortfolioValueMiniChart(
                    snapshots: history, currentValue: totalValue,
                    currency: currency, range: config.range,
                    privacy: store.privacyMode, hidesYAxisValues: valuesHidden
                )
            case .value:
                VStack(spacing: 8) {
                    Text(store.privacyMode ? "••••••••" : money(totalValue))
                        .font(.system(size: 42, weight: .semibold, design: .rounded))
                    Text("Current portfolio value").foregroundStyle(.secondary)
                    if holdings.contains(where: { $0.currentPrice == nil }) {
                        Text("Some assets could not be priced").font(.caption).foregroundStyle(.orange)
                    }
                }.frame(maxWidth: .infinity, maxHeight: .infinity)
            case .allocation:
                PortfolioAllocationMiniChart(holdings: holdings, privacy: store.privacyMode)
            }
        }
        .padding(10)
        .frame(height: cardHeight ?? chartHeight + ChartLayout.cardChrome)
        .clipped()
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 10))
        .task {
            await store.refresh()
            await store.refreshQuotes(forPortfolioID: config.portfolioID)
            await store.rebuildHistory(forPortfolioID: config.portfolioID)
        }
        .task(id: config.range) {
            guard config.kind == .valueChart, config.range == .oneDay else { return }
            while !Task.isCancelled {
                await store.refreshIntraday(forPortfolioID: config.portfolioID)
                try? await Task.sleep(for: .seconds(300))
            }
        }
    }

    private func money(_ value: Decimal) -> String {
        value.formatted(.currency(code: currency.rawValue).precision(.fractionLength(2)))
    }

    @ViewBuilder private var timeframeChangeSummary: some View {
        HStack(spacing: 6) {
            if store.privacyMode {
                Text("********")
            } else if let change = timeframeChange {
                Image(systemName: changeIcon(change.direction))
                Text(valuesHidden ? "*****" : money(abs(change.amount)))
                    + Text(" · ")
                    + Text(
                        change.percentage.map {
                            abs($0).formatted(.percent.precision(.fractionLength(2)))
                        } ?? "—")
            } else {
                Text("Unavailable")
                    .foregroundStyle(.secondary)
            }
        }
        .font(.caption.weight(.medium))
        .monospacedDigit()
        .foregroundStyle(changeColor)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(changeAccessibilityLabel)
    }

    private var changeColor: Color {
        guard !store.privacyMode, let timeframeChange else { return .secondary }
        switch timeframeChange.direction {
        case .up: return .green
        case .down: return .red
        case .unchanged: return .secondary
        }
    }

    private func changeIcon(_ direction: PortfolioValueChange.Direction) -> String {
        switch direction {
        case .up: return "arrow.up.right"
        case .down: return "arrow.down.right"
        case .unchanged: return "minus"
        }
    }

    private var changeAccessibilityLabel: String {
        guard !store.privacyMode else { return "Portfolio change hidden" }
        guard let timeframeChange else { return "Portfolio change unavailable" }
        let direction: String
        switch timeframeChange.direction {
        case .up: direction = "up"
        case .down: direction = "down"
        case .unchanged: direction = "unchanged"
        }
        let percentage =
            timeframeChange.percentage.map {
                abs($0).formatted(.percent.precision(.fractionLength(2)))
            } ?? "percentage unavailable"
        let amount = valuesHidden ? "amount hidden" : money(abs(timeframeChange.amount))
        return "\(config.range.rawValue) portfolio change, \(direction) \(amount), \(percentage)"
    }
}

private struct PortfolioValueMiniChart: View {
    let snapshots: [PortfolioSnapshot]
    let currentValue: Decimal
    let currency: PortfolioCurrency
    let range: PortfolioChartRange
    let privacy: Bool
    let hidesYAxisValues: Bool
    @State private var hoverLocation: CGPoint?

    private struct Point {
        let date: Date
        let value: Decimal
    }
    private func preparedPoints(now: Date = Date()) -> [Point] {
        snapshots.map { Point(date: $0.timestamp, value: $0.value) }
            + [Point(date: now, value: currentValue)]
    }

    /// The value axis: the data's range plus 8% either side, so the line clears the edges.
    private func valueRange(_ points: [Point]) -> (low: Decimal, spread: Decimal) {
        let values = points.map(\.value)
        var low = values.min() ?? 0
        var high = values.max() ?? 1
        if low == high {
            low -= 1
            high += 1
        }
        let padding = (high - low) * Decimal(string: "0.08")!
        low -= padding
        high += padding
        return (low, high - low)
    }

    private func plot(in size: CGSize) -> CGRect {
        CGRect(x: 12, y: 8, width: max(1, size.width - 96), height: max(1, size.height - 34))
    }

    private func x(forIndex index: Int, count: Int, in plot: CGRect) -> CGFloat {
        count == 1 ? plot.midX : plot.minX + plot.width * CGFloat(index) / CGFloat(count - 1)
    }

    var body: some View {
        let points = preparedPoints()
        GeometryReader { geometry in
            Canvas { context, size in
                guard !privacy, !points.isEmpty else { return }
                let values = points.map(\.value)
                let (low, spread) = valueRange(points)
                let plot = plot(in: size)

                for tick in 0...4 {
                    let fraction = CGFloat(tick) / 4
                    let y = plot.maxY - plot.height * fraction
                    var grid = Path()
                    grid.move(to: CGPoint(x: plot.minX, y: y))
                    grid.addLine(to: CGPoint(x: plot.maxX, y: y))
                    context.stroke(grid, with: .color(.secondary.opacity(0.18)), lineWidth: 1)
                    let value = low + spread * Decimal(Double(fraction))
                    let label = hidesYAxisValues ? "*****" : shortMoney(value)
                    context.draw(
                        Text(label).font(.caption2).foregroundStyle(.secondary),
                        at: CGPoint(x: plot.maxX + 7, y: y), anchor: .leading
                    )
                }

                let coordinates = points.indices.map { index in
                    let fraction = CGFloat(((points[index].value - low) / spread).doubleValue)
                    return CGPoint(
                        x: x(forIndex: index, count: points.count, in: plot),
                        y: plot.maxY - plot.height * fraction)
                }
                let currentColor: Color = (values.last ?? 0) >= (values.first ?? 0) ? .green : .red
                if coordinates.count > 1 {
                    context.fillAreaUnderLine(
                        coordinates, plot: plot, color: currentColor,
                        opacity: ChartStyle.default.areaFillOpacity)
                }
                var path = Path()
                for (index, point) in coordinates.enumerated() {
                    index == 0 ? path.move(to: point) : path.addLine(to: point)
                }
                context.stroke(
                    path, with: .color(currentColor),
                    style: StrokeStyle(lineWidth: ChartStyle.default.lineWidth, lineCap: .round, lineJoin: .round))

                drawCurrentValueOverlay(
                    context: &context, plot: plot, low: low, spread: spread,
                    color: currentColor
                )

                let labelIndices = Array(Set([0, max(0, points.count / 2), max(0, points.count - 1)])).sorted()
                for index in labelIndices {
                    context.draw(
                        Text(dateLabel(points[index].date)).font(.caption2).foregroundStyle(.secondary),
                        at: CGPoint(x: x(forIndex: index, count: points.count, in: plot), y: plot.maxY + 7),
                        anchor: .top
                    )
                }
            }
            .overlay {
                Canvas { context, size in
                    guard let hover = hoverLocation, !privacy, points.count > 1 else { return }
                    drawCrosshair(context: &context, plot: plot(in: size), points: points, at: hover)
                }
                .allowsHitTesting(false)
            }
            .contentShape(Rectangle())
            .onContinuousHover { phase in
                switch phase {
                case .active(let location):
                    hoverLocation = plot(in: geometry.size).contains(location) ? location : nil
                case .ended:
                    hoverLocation = nil
                }
            }
            .overlay {
                if privacy {
                    Text("••••••••").font(.title)
                } else if snapshots.isEmpty {
                    Text("History will appear after portfolio snapshots are built.").foregroundStyle(.secondary)
                }
            }
        }
    }

    private func shortMoney(_ value: Decimal) -> String {
        let absolute = abs(value)
        let formatted: String
        if absolute >= 1_000_000 {
            formatted = (value / 1_000_000).formatted(.number.precision(.fractionLength(0...1))) + "M"
        } else if absolute >= 1_000 {
            formatted = (value / 1_000).formatted(.number.precision(.fractionLength(0...1))) + "K"
        } else {
            formatted = value.formatted(.number.precision(.fractionLength(0...2)))
        }
        return "\(currency.rawValue) \(formatted)"
    }

    private func drawCurrentValueOverlay(
        context: inout GraphicsContext,
        plot: CGRect,
        low: Decimal,
        spread: Decimal,
        color: Color
    ) {
        let fraction = CGFloat(((currentValue - low) / spread).doubleValue)
        let y = plot.maxY - plot.height * fraction

        var line = Path()
        line.move(to: CGPoint(x: plot.minX, y: y))
        line.addLine(to: CGPoint(x: plot.maxX, y: y))
        context.stroke(
            line,
            with: .color(color.opacity(0.5)),
            style: StrokeStyle(lineWidth: 1, dash: [5, 3])
        )

        let label =
            hidesYAxisValues
            ? "*****"
            : currentValue.formatted(
                .number.precision(.fractionLength(2))
            )
        let text = Text(label).font(.caption2).bold().foregroundStyle(.white)
        let resolved = context.resolve(text)
        let textSize = resolved.measure(in: CGSize(width: 100, height: 20))
        let boxSize = CGSize(width: textSize.width + 10, height: 18)
        guard
            let boxY = ChartPlot.overlayOriginY(
                centeredAt: y, in: plot, labelHeight: boxSize.height
            )
        else { return }
        let box = CGRect(
            x: plot.maxX + 4, y: boxY,
            width: boxSize.width, height: boxSize.height
        )
        context.fill(Path(roundedRect: box, cornerRadius: 3), with: .color(color))
        context.draw(resolved, at: CGPoint(x: box.midX, y: box.midY))
    }

    /// Same crosshair the market charts draw: dashed lines through the pointer, the value in
    /// the right gutter and the date of the nearest point along the bottom.
    private func drawCrosshair(
        context: inout GraphicsContext, plot: CGRect, points: [Point], at location: CGPoint
    ) {
        let style = ChartStyle.default
        let stroke = StrokeStyle(lineWidth: style.crosshairLineWidth, dash: style.crosshairDashPattern)
        var vertical = Path()
        vertical.move(to: CGPoint(x: location.x, y: plot.minY))
        vertical.addLine(to: CGPoint(x: location.x, y: plot.maxY))
        var horizontal = Path()
        horizontal.move(to: CGPoint(x: plot.minX, y: location.y))
        horizontal.addLine(to: CGPoint(x: plot.maxX, y: location.y))
        context.stroke(vertical, with: .color(style.crosshairColor), style: stroke)
        context.stroke(horizontal, with: .color(style.crosshairColor), style: stroke)

        // Snapped to the point under the pointer: the label names a snapshot, and a
        // snapshot has one timestamp.
        let fraction = ((location.x - plot.minX) / plot.width).clamped(to: 0...1)
        let index = Int((fraction * CGFloat(points.count - 1)).rounded())
        let time = pill(context, dateLabel(points[index].date))
        let timeX = (location.x - time.size.width / 2)
            .clamped(to: plot.minX...max(plot.minX, plot.maxX - time.size.width))
        drawPill(
            time, in: CGRect(origin: CGPoint(x: timeX, y: plot.maxY - time.size.height - 2), size: time.size),
            into: &context)

        let (low, spread) = valueRange(points)
        let value = low + spread * Decimal(Double((plot.maxY - location.y) / plot.height))
        let price = pill(
            context, hidesYAxisValues ? "*****" : value.formatted(.number.precision(.fractionLength(2))))
        guard
            let originY = ChartPlot.overlayOriginY(
                centeredAt: location.y, in: plot, labelHeight: price.size.height)
        else { return }
        drawPill(
            price, in: CGRect(origin: CGPoint(x: plot.maxX + 4, y: originY), size: price.size),
            into: &context)
    }

    private func pill(
        _ context: GraphicsContext, _ string: String
    ) -> (text: GraphicsContext.ResolvedText, size: CGSize) {
        let text = context.resolve(Text(string).font(.caption2).bold().foregroundStyle(.white))
        let measured = text.measure(in: CGSize(width: 140, height: 20))
        return (text, CGSize(width: measured.width + 10, height: 18))
    }

    private func drawPill(
        _ pill: (text: GraphicsContext.ResolvedText, size: CGSize), in rect: CGRect,
        into context: inout GraphicsContext
    ) {
        context.fill(Path(roundedRect: rect, cornerRadius: 3), with: .color(ChartStyle.default.crosshairLabelColor))
        context.draw(pill.text, at: CGPoint(x: rect.midX, y: rect.midY))
    }

    private func dateLabel(_ date: Date) -> String {
        switch range {
        case .oneDay: return date.formatted(date: .omitted, time: .shortened)
        case .oneWeek, .oneMonth: return date.formatted(.dateTime.month(.abbreviated).day())
        case .oneYear, .all: return date.formatted(.dateTime.month(.abbreviated).year())
        }
    }
}

private struct PortfolioAllocationMiniChart: View {
    let holdings: [PortfolioHolding]
    let privacy: Bool
    private let colors: [Color] = [.blue, .orange, .green, .purple, .pink, .cyan]
    private var sorted: [PortfolioHolding] {
        holdings.filter { $0.allocation > 0 }.sorted { $0.allocation > $1.allocation }
    }

    var body: some View {
        if sorted.isEmpty {
            ContentUnavailableView(
                "No Allocations", systemImage: "chart.pie", description: Text("Add priced holdings to this portfolio."))
        } else {
            HStack(spacing: 24) {
                Canvas { context, size in
                    var start = Angle.degrees(-90)
                    let radius = max(1, min(size.width, size.height) / 2 - 20)
                    for (index, holding) in sorted.enumerated() {
                        let end = start + .degrees(holding.allocation.doubleValue * 360)
                        var path = Path()
                        path.addArc(
                            center: CGPoint(x: size.width / 2, y: size.height / 2), radius: radius,
                            startAngle: start, endAngle: end, clockwise: false)
                        context.stroke(path, with: .color(colors[index % colors.count]), lineWidth: 26)
                        start = end
                    }
                }.frame(maxWidth: 240)
                ScrollView {
                    LazyVStack(spacing: 8) {
                        ForEach(Array(sorted.enumerated()), id: \.element.id) { index, holding in
                            HStack {
                                Circle().fill(colors[index % colors.count]).frame(width: 8, height: 8)
                                Text(holding.asset.displayTicker).bold()
                                Spacer()
                                Text(
                                    privacy
                                        ? "••••" : holding.allocation.formatted(.percent.precision(.fractionLength(1))))
                            }
                        }
                    }
                }
            }
        }
    }
}

private struct ReplaySelectionMarker: View {
    @ObservedObject var viewModel: ChartViewModel
    let date: Date

    var body: some View {
        GeometryReader { geometry in
            Canvas { context, _ in
                let points = viewModel.visibleKlines
                guard let index = points.firstIndex(where: { $0.openTime == date }) else { return }
                let plot = viewModel.plot(in: geometry.size)
                let x = plot.x(forIndex: index, slotWidth: plot.slotWidth(forCount: points.count))
                var path = Path()
                path.move(to: CGPoint(x: x, y: plot.plotRect.minY))
                path.addLine(to: CGPoint(x: x, y: plot.plotRect.maxY))
                context.stroke(path, with: .color(.orange), style: StrokeStyle(lineWidth: 2, dash: [5, 4]))
            }
        }
        .accessibilityHidden(true)
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

            Button {
                _ = viewModel.removeLine(id: lineID)
            } label: {
                Image(systemName: "trash")
                    .font(.caption.bold())
            }
            .buttonStyle(.plain)
            .foregroundStyle(.red)
            .accessibilityLabel("Delete trend line")
            .help("Delete trend line")

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

private struct FibonacciEditor: View {
    @ObservedObject var viewModel: ChartViewModel
    let drawingID: UUID
    let onChange: () -> Void
    @State private var showSettings = false

    private var drawing: FibonacciRetracementDrawing? {
        viewModel.fibonacciRetracements.first { $0.id == drawingID }
    }

    var body: some View {
        HStack(spacing: 10) {
            Button {
                mutate { $0.style.reverse.toggle() }
            } label: {
                Label("Reverse", systemImage: "arrow.up.arrow.down")
            }
            Button {
                mutate { $0.style.extendRight.toggle() }
            } label: {
                Label("Extend Right", systemImage: "arrow.right.to.line")
            }
            Button {
                viewModel.beginFibonacciSettingsEdit(id: drawingID)
                showSettings = true
            } label: {
                Image(systemName: "gearshape")
            }
            .accessibilityLabel("Fibonacci Retracement settings")
            Button(role: .destructive) {
                _ = viewModel.removeFibonacci(id: drawingID)
            } label: {
                Image(systemName: "trash")
            }
            .accessibilityLabel("Delete Fibonacci Retracement")
        }
        .buttonStyle(.plain)
        .padding(.horizontal, 12)
        .padding(.vertical, 7)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 8))
        .sheet(isPresented: $showSettings) {
            if let drawing {
                FibonacciSettingsView(
                    drawing: binding(for: drawing),
                    onCommit: {
                        onChange()
                    })
            }
        }
        .onChange(of: showSettings) { wasShowing, isShowing in
            if wasShowing, !isShowing {
                viewModel.endFibonacciSettingsEdit(id: drawingID)
            }
        }
    }

    private func binding(for fallback: FibonacciRetracementDrawing) -> Binding<FibonacciRetracementDrawing> {
        Binding(
            get: { drawing ?? fallback },
            set: { viewModel.updateFibonacci($0, recordUndo: false) }
        )
    }

    private func mutate(_ change: (inout FibonacciRetracementDrawing) -> Void) {
        guard var drawing else { return }
        change(&drawing)
        drawing.updatedAt = Date()
        viewModel.updateFibonacci(drawing)
        onChange()
    }
}

private struct FibonacciSettingsView: View {
    @Binding var drawing: FibonacciRetracementDrawing
    let onCommit: () -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var selectedPage: Page = .style

    private enum Page: String, CaseIterable, Identifiable {
        case style = "Style"
        case coordinates = "Coordinates"
        case visibility = "Visibility"

        var id: Self { self }

        var icon: String {
            switch self {
            case .style: return "paintbrush"
            case .coordinates: return "scope"
            case .visibility: return "eye"
            }
        }

        var subtitle: String {
            switch self {
            case .style: return "Customize levels, lines, labels, and fills."
            case .coordinates: return "Place both anchors precisely in chart space."
            case .visibility: return "Control when the drawing appears and whether it can move."
            }
        }
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 0) {
                List(Page.allCases, selection: $selectedPage) { page in
                    Label(page.rawValue, systemImage: page.icon).tag(page)
                }
                .listStyle(.sidebar)
                .frame(width: 176)

                Divider()

                VStack(alignment: .leading, spacing: 0) {
                    settingsHeader
                    Divider()
                    selectedContent
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            }

            Divider()

            HStack {
                Text("Changes update the drawing immediately.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Spacer()
                Button("Done") {
                    onCommit()
                    dismiss()
                }
                .buttonStyle(.borderedProminent)
                .keyboardShortcut(.defaultAction)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
        }
        .frame(width: 760, height: 620)
    }

    private var settingsHeader: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(selectedPage.rawValue)
                .font(.title3.weight(.semibold))
            Text(selectedPage.subtitle)
                .font(.subheadline)
                .foregroundStyle(.secondary)
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 16)
    }

    @ViewBuilder private var selectedContent: some View {
        switch selectedPage {
        case .style: stylePage
        case .coordinates: coordinatesPage
        case .visibility: visibilityPage
        }
    }

    private var stylePage: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                settingsSection("Lines", icon: "line.diagonal") {
                    settingsToggle(
                        "Trend Line", subtitle: "Show the line connecting both anchors.",
                        isOn: $drawing.style.showTrendLine)
                    Divider()
                    HStack {
                        Picker("Trend style", selection: $drawing.style.trendLineStyle) {
                            ForEach(DrawingLineStyle.allCases, id: \.self) { Text($0.title).tag($0) }
                        }
                        Picker("Level style", selection: $drawing.style.levelLineStyle) {
                            ForEach(DrawingLineStyle.allCases, id: \.self) { Text($0.title).tag($0) }
                        }
                    }
                    HStack(spacing: 18) {
                        Toggle("Extend left", isOn: $drawing.style.extendLeft)
                        Toggle("Extend right", isOn: $drawing.style.extendRight)
                        Spacer()
                    }
                    .toggleStyle(.switch)
                }

                settingsSection("Labels & Fill", icon: "textformat") {
                    settingsToggle(
                        "Background", subtitle: "Tint the regions between visible levels.",
                        isOn: $drawing.style.backgroundVisible)
                    if drawing.style.backgroundVisible {
                        HStack {
                            Text("Opacity").foregroundStyle(.secondary)
                            Slider(value: $drawing.style.backgroundOpacity, in: 0...1)
                            Text(drawing.style.backgroundOpacity, format: .percent.precision(.fractionLength(0)))
                                .monospacedDigit().frame(width: 38, alignment: .trailing)
                        }
                    }
                    Divider()
                    HStack(spacing: 18) {
                        Toggle("Prices", isOn: $drawing.style.showPrices)
                        Toggle("Levels", isOn: $drawing.style.showLevels)
                        Toggle("One color", isOn: $drawing.style.useOneColor)
                        Spacer()
                    }
                    .toggleStyle(.switch)
                    HStack {
                        Picker("Format", selection: $drawing.style.labelFormat) {
                            ForEach(FibonacciLabelFormat.allCases, id: \.self) { Text($0.title).tag($0) }
                        }
                        Picker("Position", selection: $drawing.style.labelPosition) {
                            ForEach(FibonacciLabelPosition.allCases, id: \.self) { Text($0.title).tag($0) }
                        }
                        TextField("Font size", value: $drawing.style.fontSize, format: .number)
                            .frame(width: 110)
                    }
                    Divider()
                    settingsToggle(
                        "Reverse", subtitle: "Reflect ratio mapping while keeping both anchors fixed.",
                        isOn: $drawing.style.reverse)
                    settingsToggle(
                        "Logarithmic levels", subtitle: "Available when the chart uses a logarithmic price scale.",
                        isOn: $drawing.style.useLogCalculation
                    )
                    .disabled(true)
                }

                levelsSection
            }
            .padding(20)
        }
    }

    private var levelsSection: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Label("Levels", systemImage: "line.3.horizontal")
                    .font(.subheadline.weight(.semibold))
                Text("\(drawing.levels.count) of \(FibonacciDefaults.maximumLevelCount)")
                    .font(.caption).foregroundStyle(.secondary)
                Spacer()
                Button("Add Level", systemImage: "plus") {
                    _ = drawing.addLevel(FibonacciLevel(ratio: 1))
                }
                .buttonStyle(.borderless)
                .disabled(drawing.levels.count >= FibonacciDefaults.maximumLevelCount)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 12)

            Divider()

            HStack(spacing: 10) {
                Text("On").frame(width: 28)
                Text("Ratio").frame(width: 76, alignment: .leading)
                Text("Color").frame(width: 90, alignment: .leading)
                Text("Custom text")
                Spacer()
            }
            .font(.caption.weight(.medium))
            .foregroundStyle(.secondary)
            .padding(.horizontal, 14)
            .padding(.vertical, 7)

            ForEach($drawing.levels) { $level in
                Divider().padding(.leading, 14)
                HStack(spacing: 10) {
                    Toggle("", isOn: $level.isVisible).labelsHidden().frame(width: 28)
                    TextField("Ratio", value: $level.ratio, format: .number)
                        .textFieldStyle(.roundedBorder).frame(width: 76)
                    Picker("Color", selection: $level.color) {
                        ForEach(TrendLineColor.allCases, id: \.self) { option in
                            Text(option.title).tag(option)
                        }
                    }
                    .labelsHidden().frame(width: 90)
                    TextField("Optional label", text: $level.customText)
                        .textFieldStyle(.roundedBorder)
                    Button(role: .destructive) {
                        removeLevel(level.id)
                    } label: {
                        Image(systemName: "minus.circle.fill")
                    }
                    .buttonStyle(.borderless)
                    .foregroundStyle(.secondary)
                    .help("Remove level")
                }
                .padding(.horizontal, 14)
                .padding(.vertical, 7)
            }
        }
        .fibSettingsCard()
    }

    private var coordinatesPage: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                coordinateCard(
                    title: "Point 1", icon: "1.circle.fill",
                    date: $drawing.point1.date, price: $drawing.point1.price)
                coordinateCard(
                    title: "Point 2", icon: "2.circle.fill",
                    date: $drawing.point2.date, price: $drawing.point2.price)
            }
            .padding(20)
        }
    }

    private var visibilityPage: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                settingsSection("Timeframes", icon: "clock") {
                    HStack {
                        VStack(alignment: .leading, spacing: 3) {
                            Text("Show drawing on")
                                .font(.subheadline.weight(.medium))
                            Text("The anchors remain unchanged when the chart timeframe changes.")
                                .font(.caption).foregroundStyle(.secondary)
                        }
                        Spacer()
                        Picker("Show drawing on", selection: $drawing.timeframeVisibility) {
                            ForEach(DrawingTimeframeVisibility.allCases, id: \.self) {
                                Text($0.title).tag($0)
                            }
                        }
                        .labelsHidden().frame(width: 170)
                    }
                }

                settingsSection("Drawing", icon: "lock.shield") {
                    settingsToggle(
                        "Lock Drawing", subtitle: "Prevent anchor and whole-drawing movement.", isOn: $drawing.isLocked)
                    Divider()
                    settingsToggle(
                        "Hide Drawing", subtitle: "Keep the drawing saved without showing it on the chart.",
                        isOn: $drawing.isHidden)
                }
            }
            .padding(20)
        }
    }

    private func settingsSection<Content: View>(
        _ title: String, icon: String, @ViewBuilder content: () -> Content
    ) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            Label(title, systemImage: icon)
                .font(.subheadline.weight(.semibold))
            content()
        }
        .fibSettingsCard()
    }

    private func settingsToggle(
        _ title: String, subtitle: String, isOn: Binding<Bool>
    ) -> some View {
        HStack(spacing: 12) {
            VStack(alignment: .leading, spacing: 3) {
                Text(title).font(.subheadline.weight(.medium))
                Text(subtitle).font(.caption).foregroundStyle(.secondary)
            }
            Spacer(minLength: 16)
            Toggle(title, isOn: isOn).labelsHidden().toggleStyle(.switch)
        }
    }

    private func coordinateCard(
        title: String, icon: String, date: Binding<Date>, price: Binding<Double>
    ) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            Label(title, systemImage: icon)
                .font(.subheadline.weight(.semibold))
            Divider()
            LabeledContent("Date / Time") {
                DatePicker("Date / Time", selection: date)
                    .labelsHidden()
            }
            LabeledContent("Price") {
                TextField("Price", value: price, format: .number)
                    .multilineTextAlignment(.trailing)
                    .frame(width: 180)
            }
        }
        .fibSettingsCard()
    }

    private func removeLevel(_ id: UUID) {
        drawing.levels.removeAll { $0.id == id }
    }
}

private extension View {
    func fibSettingsCard() -> some View {
        padding(.horizontal, 14)
            .padding(.vertical, 12)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(.quaternary.opacity(0.45), in: RoundedRectangle(cornerRadius: 12))
            .overlay {
                RoundedRectangle(cornerRadius: 12)
                    .stroke(.separator.opacity(0.45), lineWidth: 1)
            }
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
struct ZoomHitRegion: NSViewRepresentable {
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
/// What the pointer looks like over the plot while a tool is armed. The ruler is the only
/// tool that changes it: grabbing an edge moves a measurement, a corner resizes it.
private enum PlotCursor: Equatable {
    case crosshair
    case move
    case resize(RulerCorner)

    var nsCursor: NSCursor {
        switch self {
        case .crosshair:
            return .crosshair
        case .move:
            return .openHand
        case .resize(let corner):
            // Diagonal resize cursors are public API from macOS 15 only.
            if #available(macOS 15.0, *) {
                let position: NSCursor.FrameResizePosition =
                    switch (corner.isTop, corner.isLeft) {
                    case (true, true): .topLeft
                    case (true, false): .topRight
                    case (false, true): .bottomLeft
                    case (false, false): .bottomRight
                    }
                return .frameResize(position: position, directions: .all)
            }
            return .crosshair
        }
    }
}

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
    let cursor: PlotCursor
    let onResolve: (NSView) -> Void

    func makeNSView(context: Context) -> NSView {
        let view = PlotRegionView()
        view.isArmed = isArmed
        view.cursor = cursor
        onResolve(view)
        return view
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        guard let view = nsView as? PlotRegionView,
            view.isArmed != isArmed || view.cursor != cursor
        else { return }
        view.isArmed = isArmed
        view.cursor = cursor
        view.window?.invalidateCursorRects(for: view)
    }

    private final class PlotRegionView: NSView {
        var isArmed = false
        var cursor = PlotCursor.crosshair

        override var isFlipped: Bool { true }

        /// Cursor rects only — the monitor does the rest, and letting this view take
        /// hits would swallow clicks meant for the card underneath.
        override func hitTest(_ point: NSPoint) -> NSView? { nil }

        override func resetCursorRects() {
            guard isArmed else { return }
            // Stop short of the price gutter, which keeps its own resize cursor.
            var rect = bounds
            rect.size.width = max(0, rect.width - ChartStyle.default.chartInsets.trailing)
            addCursorRect(rect, cursor: cursor.nsCursor)
        }
    }
}

struct PriceAxisRegion: NSViewRepresentable {
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
        isFavorite: false,
        onToggleFavorite: {},
        onZoomRegion: { _ in },
        onAxisRegion: { _ in },
        onStyleChanged: {}
    )
    .frame(width: 400)
    .padding()
}
