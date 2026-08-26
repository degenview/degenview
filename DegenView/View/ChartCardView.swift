import SwiftUI
import AppKit

struct ChartCardView: View {
    @ObservedObject var viewModel: ChartViewModel
    var chartHeight: CGFloat
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
    let onUpdateTicker: (String, DataSourceType, String?, [PmSeriesConfig]?) -> Void
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
    @State private var iconURL: URL?
    @State private var showAlertEditor = false
    @StateObject private var portfolioStore = PortfolioStore.shared

    @ViewBuilder
    var body: some View {
        if let config = viewModel.portfolioChart {
            PortfolioChartCard(
                config: Binding(
                    get: { viewModel.portfolioChart ?? config },
                    set: { viewModel.portfolioChart = $0; onStyleChanged() }
                ),
                store: portfolioStore,
                chartHeight: chartHeight,
                onRemove: onRemove
            )
        } else {
            marketCard
        }
    }

    private var marketCard: some View {
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
        .sheet(isPresented: $showAlertEditor) { PriceAlertEditor(asset: alertAsset) }
        .onChange(of: showSettings) { _, new in
            onSettingsPresented?(new)
        }
        .onChange(of: viewModel.editingLineID) { old, new in
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
                        if viewModel.replayTimestamp != nil {
                            Label("Replay", systemImage: "clock.arrow.circlepath")
                                .font(.caption2.weight(.semibold))
                                .foregroundStyle(.orange)
                                .accessibilityLabel("Historical replay mode")
                        }
                    }

                    if let price = viewModel.displayedPrice {
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

            if viewModel.source != .polymarket {
                Button { showAlertEditor = true } label: { Image(systemName: "bell.badge") }
                    .buttonStyle(.plain).help("Create Price Alert")
            }

            if paperConnected, let price = viewModel.displayedPrice {
                HStack(spacing: 4) {
                    Button(action: onPaperSell) {
                        VStack(spacing: 0) { Text("SELL").font(.caption2.bold()); Text(PriceFormatter.format(price, scale: viewModel.priceScale)).font(.caption2.monospacedDigit()) }
                    }
                    .buttonStyle(.bordered).tint(.red)
                    .accessibilityLabel("Sell \(viewModel.title), paper order at last price \(price)")
                    Button(action: onPaperBuy) {
                        VStack(spacing: 0) { Text("BUY").font(.caption2.bold()); Text(PriceFormatter.format(price, scale: viewModel.priceScale)).font(.caption2.monospacedDigit()) }
                    }
                    .buttonStyle(.borderedProminent).tint(.blue)
                    .accessibilityLabel("Buy \(viewModel.title), paper order at last price \(price)")
                }
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
        PortfolioAsset(key: viewModel.iconKey, symbol: viewModel.baseSymbol, name: viewModel.title,
            source: viewModel.source, quoteCurrency: .USD, metadata: ["apiSymbol": viewModel.apiSymbol])
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
                    pine: viewModel.pineOutput,
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
        .overlay {
            if let date = viewModel.replaySelectionTimestamp {
                ReplaySelectionMarker(viewModel: viewModel, date: date)
                    .allowsHitTesting(false)
            }
        }
        .overlay {
            if paperConnected && viewModel.replayTimestamp == nil {
                PaperChartTradingOverlay(candles: viewModel.visibleKlines, positions: paperPositions,
                    orders: paperOrders, accountCurrency: paperAccountCurrency, unrealizedPnL: paperUnrealizedPnL,
                    onModify: onPaperModify, onCancel: onPaperCancel, onClose: onPaperClose)
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

private struct PortfolioChartCard: View {
    @Binding var config: PortfolioChartConfig
    @ObservedObject var store: PortfolioStore
    let chartHeight: CGFloat
    let onRemove: () -> Void
    @State private var valuesHidden = false

    private var portfolio: Portfolio? { store.portfolio(namedBy: config.portfolioID) }
    private var holdings: [PortfolioHolding] { store.holdings(for: config.portfolioID) }
    private var totalValue: Decimal { holdings.compactMap(\.currentValue).reduce(0, +) }
    private var currency: PortfolioCurrency { portfolio?.baseCurrency ?? .USD }
    private var history: [PortfolioSnapshot] {
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
                    Button { valuesHidden.toggle() } label: {
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
                Picker("History range", selection: $config.range) {
                    ForEach(PortfolioChartRange.allCases) { Text($0.rawValue).tag($0) }
                }
                .pickerStyle(.segmented).labelsHidden().frame(maxWidth: 380)
                timeframeChangeSummary
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
        .frame(
            height: chartHeight + ChartLayout.cardChrome(
                isPortfolioValue: config.kind == .valueChart
            )
        )
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 10))
        .task {
            await store.refresh()
            await store.refreshQuotes(forPortfolioID: config.portfolioID)
            await store.rebuildHistory(forPortfolioID: config.portfolioID)
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
                    + Text(change.percentage.map {
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
        .frame(maxWidth: 380)
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
        let percentage = timeframeChange.percentage.map {
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

    private struct Point { let date: Date; let value: Decimal }
    private func preparedPoints(now: Date = Date()) -> [Point] {
        snapshots.map { Point(date: $0.timestamp, value: $0.value) }
            + [Point(date: now, value: currentValue)]
    }

    var body: some View {
        let points = preparedPoints()
        GeometryReader { geometry in
            Canvas { context, size in
                guard !privacy, !points.isEmpty else { return }
                let values = points.map(\.value)
                var low = values.min() ?? 0, high = values.max() ?? 1
                if low == high { low -= 1; high += 1 }
                let padding = (high - low) * Decimal(string: "0.08")!
                low -= padding; high += padding
                let spread = high - low
                let plot = CGRect(x: 12, y: 8, width: max(1, size.width - 96), height: max(1, size.height - 34))

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

                var path = Path()
                for index in points.indices {
                    let x = points.count == 1 ? plot.midX : plot.minX + plot.width * CGFloat(index) / CGFloat(points.count - 1)
                    let fraction = CGFloat(((points[index].value - low) / spread).doubleValue)
                    let point = CGPoint(x: x, y: plot.maxY - plot.height * fraction)
                    index == 0 ? path.move(to: point) : path.addLine(to: point)
                }
                context.stroke(path, with: .color((values.last ?? 0) >= (values.first ?? 0) ? .green : .red), lineWidth: 2.5)

                let currentColor: Color = (values.last ?? 0) >= (values.first ?? 0) ? .green : .red
                drawCurrentValueOverlay(
                    context: &context, plot: plot, low: low, spread: spread,
                    color: currentColor
                )

                let labelIndices = Array(Set([0, max(0, points.count / 2), max(0, points.count - 1)])).sorted()
                for index in labelIndices {
                    let x = points.count == 1 ? plot.midX : plot.minX + plot.width * CGFloat(index) / CGFloat(points.count - 1)
                    context.draw(
                        Text(dateLabel(points[index].date)).font(.caption2).foregroundStyle(.secondary),
                        at: CGPoint(x: x, y: plot.maxY + 7), anchor: .top
                    )
                }
            }
            .overlay {
                if privacy { Text("••••••••").font(.title) }
                else if snapshots.isEmpty { Text("History will appear after portfolio snapshots are built.").foregroundStyle(.secondary) }
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

        let label = hidesYAxisValues ? "*****" : currentValue.formatted(
            .number.precision(.fractionLength(2))
        )
        let text = Text(label).font(.caption2).bold().foregroundStyle(.white)
        let resolved = context.resolve(text)
        let textSize = resolved.measure(in: CGSize(width: 100, height: 20))
        let boxSize = CGSize(width: textSize.width + 10, height: 18)
        guard let boxY = ChartPlot.overlayOriginY(
            centeredAt: y, in: plot, labelHeight: boxSize.height
        ) else { return }
        let box = CGRect(
            x: plot.maxX + 4, y: boxY,
            width: boxSize.width, height: boxSize.height
        )
        context.fill(Path(roundedRect: box, cornerRadius: 3), with: .color(color))
        context.draw(resolved, at: CGPoint(x: box.midX, y: box.midY))
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
    private var sorted: [PortfolioHolding] { holdings.filter { $0.allocation > 0 }.sorted { $0.allocation > $1.allocation } }

    var body: some View {
        if sorted.isEmpty {
            ContentUnavailableView("No Allocations", systemImage: "chart.pie", description: Text("Add priced holdings to this portfolio."))
        } else {
            HStack(spacing: 24) {
                Canvas { context, size in
                    var start = Angle.degrees(-90)
                    let radius = max(1, min(size.width, size.height) / 2 - 20)
                    for (index, holding) in sorted.enumerated() {
                        let end = start + .degrees(holding.allocation.doubleValue * 360)
                        var path = Path()
                        path.addArc(center: CGPoint(x: size.width / 2, y: size.height / 2), radius: radius,
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
                                Text(holding.asset.symbol).bold()
                                Spacer()
                                Text(privacy ? "••••" : holding.allocation.formatted(.percent.precision(.fractionLength(1))))
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
        isFavorite: false,
        onToggleFavorite: {},
        onZoomRegion: { _ in },
        onAxisRegion: { _ in },
        onUpdateTicker: { _, _, _, _ in },
        onStyleChanged: {}
    )
    .frame(width: 400)
    .padding()
}
