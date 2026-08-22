import Foundation
import SwiftUI
import Combine

@MainActor
final class ChartViewModel: ObservableObject {
    @Published var ticker: String
    @Published var source: DataSourceType

    /// Human-readable label, for sources whose `ticker` is an opaque identifier.
    /// A Polymarket CLOB token id is 77 digits, so the market question rides along.
    @Published var displayName: String?

    /// Unique identifier — derived from initial ticker+source, stable across updates.
    let uniqueID: String

    private var api: TickerDataSource

    /// What the card header and settings sheet call this chart.
    var title: String {
        if let displayName, !displayName.isEmpty { return displayName }
        return ticker.uppercased()
    }

    /// The symbol used for API calls — source-dependent.
    var apiSymbol: String {
        switch source {
        case .binance:
            let upper = ticker.uppercased()
            if upper.hasSuffix("USDT") || upper.hasSuffix("USDC") || upper.hasSuffix("BUSD") {
                return upper
            }
            return "\(upper)USDT"
        case .coingecko, .dexscreener, .alpaca, .polymarket:
            // ticker IS the fullSymbol (coin ID, pair address, or CLOB token id)
            return ticker
        }
    }

    /// Base asset symbol for icon lookup (strips quote currency suffixes).
    var baseSymbol: String {
        switch source {
        case .binance:
            let upper = ticker.uppercased()
            for quote in ["USDT", "USDC", "BUSD", "USD", "BTC", "ETH", "BNB"] {
                if upper.hasSuffix(quote), upper.count > quote.count {
                    return String(upper.dropLast(quote.count))
                }
            }
            return upper
        case .coingecko, .dexscreener:
            let parts = ticker.components(separatedBy: "/")
            return parts.first?.uppercased() ?? ticker.uppercased()
        case .polymarket:
            // The ticker is a token id; the monogram fallback needs the question.
            return title
        case .alpaca:
            return ticker.uppercased()
        }
    }

    /// Whether prices read as USD or as probabilities.
    var priceScale: PriceScale { source.priceScale }

    /// Prediction markets report one price per timestamp, so they draw as a line.
    var usesLineChart: Bool { source == .polymarket }

    /// Identity of the icon currently wanted. `uniqueID` deliberately survives
    /// `updateTicker`, so it can't drive the icon lookup — the card would keep
    /// showing the previous coin's artwork.
    var iconKey: String { "\(source.rawValue):\(ticker)" }

    /// All tradable choices for multi-outcome Polymarket events. Empty for single-choice
    /// markets and all non-Polymarket sources.
    @Published var pmSeries: [PmSeriesConfig] = []

    /// Fetched price history keyed by CLOB token id, populated during multi-series fetches.
    @Published private(set) var pmSeriesData: [String: [KlineData]] = [:]

    /// Fixed palette for multi-series lines. Index maps to `pmSeries` order.
    static let seriesPalette: [Color] = [.blue, .orange, .purple, .green, .red, .cyan, .pink]

    /// Color for a given token id within `pmSeries`.
    func pmColor(for tokenID: String) -> Color {
        guard let index = pmSeries.firstIndex(where: { $0.tokenID == tokenID }) else {
            return Self.seriesPalette[0]
        }
        return Self.seriesPalette[index % Self.seriesPalette.count]
    }

    /// Enabled series with their fetched data and display color, in `pmSeries` order.
    var pmVisibleSeries: [(tokenID: String, label: String, data: [KlineData], color: Color)] {
        guard pmSeries.count > 1 else { return [] }
        return pmSeries.filter(\.enabled).compactMap { s in
            guard let data = pmSeriesData[s.tokenID], !data.isEmpty else { return nil }
            return (s.tokenID, s.label, data, pmColor(for: s.tokenID))
        }
    }

    /// Toggle a Polymarket series on/off by its token id and refresh the primary data.
    func togglePmSeries(_ tokenID: String) {
        guard let i = pmSeries.firstIndex(where: { $0.tokenID == tokenID }) else { return }
        pmSeries[i].enabled.toggle()
        syncPrimaryKlineData()
    }

    /// Keep `klineData` / `currentPrice` in sync with the first enabled PM series.
    private func syncPrimaryKlineData() {
        guard !pmSeries.isEmpty else { return }
        if let first = pmSeries.first(where: \.enabled),
           let data = pmSeriesData[first.tokenID], !data.isEmpty {
            klineData = data
            currentPrice = data.last?.closePrice
        }
    }

    /// Everything fetched, including the warm-up candles that sit off the left edge.
    /// Read `visibleKlines` for what the chart actually draws.
    @Published var klineData: [KlineData] = []

    /// How many of the newest candles are on screen. The rest are warm-up for
    /// period-based indicators, and slack that lets a zoom redraw without refetching.
    @Published private(set) var visibleCount: Int = TimeRange.oneDay.dataPointLimit

    /// The candles the chart draws — the newest `visibleCount` of the buffer.
    var visibleKlines: [KlineData] {
        guard visibleCount > 0, klineData.count > visibleCount else { return klineData }
        return Array(klineData.suffix(visibleCount))
    }

    @Published var errorMessage: String?
    @Published var lastUpdated: Date?
    @Published var currentPrice: Double?

    // MARK: - Chart appearance settings

    @Published var bullishColor: Color = .green
    @Published var bearishColor: Color = .red
    @Published var yAxisDecimalPlaces: Int? = nil  // nil = auto-detect

    /// Turnover bars under the candles. Off by default — only Binance reports the
    /// quote volume they're drawn from.
    @Published var showVolume: Bool = false

    /// RSI line across the bottom of the plot. Off by default. Computed from closes,
    /// so unlike volume it works on every source.
    @Published var showRSI: Bool = false

    /// Price-scale overlays: an EMA at `emaPeriod`, and Bollinger bands.
    @Published var showEMA: Bool = false
    @Published var emaPeriod: Int = Indicator.emaDefaultPeriod
    @Published var showBollinger: Bool = false
    @Published var showTrendFlips: Bool = false

    /// Every enabled indicator, computed over the full buffer and trimmed to the
    /// visible tail so warm-up happens off screen.
    var indicators: IndicatorSeries {
        IndicatorSeries.make(
            candles: klineData,
            visibleCount: visibleCount,
            showRSI: showRSI,
            showEMA: showEMA,
            emaPeriod: emaPeriod,
            showBollinger: showBollinger,
            showTrendFlips: showTrendFlips
        )
    }

    /// Vertical price-scale zoom. 1 = auto-fit; >1 shows a narrower slice of price,
    /// drawing the series taller. Driven by dragging the Y-axis gutter.
    @Published var yZoom: Double = 1

    /// Zoom when the axis drag began. The drag maps absolutely from this, rather
    /// than accumulating per mouse-move.
    private var yZoomDragStart: Double = 1

    // MARK: - Trend lines

    /// Lines drawn by hand on this chart, anchored to time and price.
    @Published var trendLines: [TrendLine] = []
    private var drawingStoreSubscription: AnyCancellable?

    /// First click of a line in progress, and the rubber-band end that follows the
    /// pointer until the second click lands.
    @Published private(set) var draftStart: TrendAnchor?
    @Published private(set) var draftEnd: TrendAnchor?

    @Published var selectedLineID: UUID?
    /// The selected line whose appearance popover is open. Kept separate from
    /// selection so grabbing an endpoint does not open a popover under the drag.
    @Published var editingLineID: UUID?

    var trendDraft: (start: TrendAnchor, end: TrendAnchor)? {
        guard let draftStart, let draftEnd else { return nil }
        return (draftStart, draftEnd)
    }

    var hasDraft: Bool { draftStart != nil }

    // MARK: - Ruler

    /// Measuring rectangles on this chart. Never persisted and never restored — a ruler
    /// answers a question and then goes away on the next click.
    @Published private(set) var rulers: [RulerRect] = []

    /// First corner of a rectangle in progress, and the opposite corner that follows the
    /// pointer until the second click lands.
    @Published private(set) var rulerDraftStart: TrendAnchor?
    @Published private(set) var rulerDraftEnd: TrendAnchor?

    var rulerDraft: (start: TrendAnchor, end: TrendAnchor)? {
        guard let rulerDraftStart, let rulerDraftEnd else { return nil }
        return (rulerDraftStart, rulerDraftEnd)
    }

    var hasRulerDraft: Bool { rulerDraftStart != nil }

    private var fetchTask: Task<Void, Never>?
    private var fetchGeneration = 0
    private var isFetching = false

    /// Change across the *visible* window — the warm-up candles are off screen, so
    /// counting them would report a move the user can't see.
    var priceChangePercent: Double? {
        visibleKlines.priceChangePercent
    }

    var priceChangeIsPositive: Bool {
        guard let change = priceChangePercent else { return true }
        return change >= 0
    }

    init(ticker: String, source: DataSourceType = .binance, displayName: String? = nil, api: TickerDataSource? = nil) {
        self.ticker = ticker
        self.source = source
        self.displayName = displayName
        self.uniqueID = "\(ticker)_\(source.rawValue)"
        self.api = api ?? DataSourceFactory.shared.service(for: source)
        observeDrawings()
    }

    /// Apply persisted chart settings from a TickerConfig.
    func applyConfig(_ config: TickerConfig) {
        if let hex = config.bullishColorHex { bullishColor = Color(hex: hex) }
        if let hex = config.bearishColorHex { bearishColor = Color(hex: hex) }
        yAxisDecimalPlaces = config.yAxisDecimalPlaces
        yZoom = config.yZoom ?? 1
        showVolume = config.showVolume ?? false
        showRSI = config.showRSI ?? false
        showEMA = config.showEMA ?? false
        emaPeriod = config.emaPeriod ?? Indicator.emaDefaultPeriod
        showBollinger = config.showBollinger ?? false
        showTrendFlips = config.showTrendFlips ?? false
        if let legacyLines = config.trendLines {
            DrawingStore.shared.importLegacy(legacyLines, ticker: ticker, source: source)
        }
        trendLines = DrawingStore.shared.lines(ticker: ticker, source: source)
        if let name = config.displayName { displayName = name }
        if let series = config.pmSeries, !series.isEmpty { pmSeries = series }
    }

    // MARK: - Vertical zoom

    func beginYZoomDrag() {
        yZoomDragStart = yZoom
    }

    /// `dy` is the distance dragged since the gesture began, positive upward.
    /// Up narrows the price slice, so the candles grow taller.
    func updateYZoom(dragOffset dy: CGFloat) {
        let factor = pow(2, Double(dy) / PriceZoom.pointsPerDoubling)
        yZoom = (yZoomDragStart * factor).clamped(to: PriceZoom.minFactor...PriceZoom.maxFactor)
    }

    func resetYZoom() {
        yZoom = 1
    }

    // MARK: - Drawing geometry

    /// The geometry this chart is currently drawn with, for a canvas of `size`.
    ///
    /// The drawing tool runs off a window-wide `NSEvent` monitor, which sees only a
    /// point and a view — it has to rebuild the mapping the renderer used. Sharing
    /// `ChartPlot.make` with both chart views is what keeps hits landing on pixels.
    func plot(in size: CGSize) -> ChartPlot {
        ChartPlot.make(
            points: visibleKlines,
            size: size,
            yZoom: yZoom,
            scale: priceScale,
            yAxisDecimalPlaces: yAxisDecimalPlaces
        )
    }

    /// Convert a click into a time+price anchor that survives zoom and timeframe changes.
    func anchor(at point: CGPoint, in plot: ChartPlot) -> TrendAnchor {
        let points = visibleKlines
        let index = plot.fractionalIndex(
            forX: point.x,
            slotWidth: plot.slotWidth(forCount: points.count)
        )
        return TrendAnchor(
            date: ChartPlot.date(atFractionalIndex: index, in: points),
            price: plot.price(forY: point.y)
        )
    }

    /// The endpoint circle under `point`. Ends are reported separately so a drag
    /// knows which one it moves; lines are searched newest first, matching what the
    /// renderer draws on top.
    func handleHit(at point: CGPoint, in plot: ChartPlot) -> (id: UUID, isStart: Bool)? {
        let points = visibleKlines
        guard !points.isEmpty else { return nil }
        let slot = plot.slotWidth(forCount: points.count)

        for line in trendLines.reversed() {
            for (anchor, isStart) in [(line.start, true), (line.end, false)] {
                let position = plot.position(of: anchor, points: points, slotWidth: slot)
                if hypot(position.x - point.x, position.y - point.y) <= Drawing.hitTolerance {
                    return (line.id, isStart)
                }
            }
        }
        return nil
    }

    /// The line whose body passes within `Drawing.hitTolerance` of `point`.
    func lineHit(at point: CGPoint, in plot: ChartPlot) -> UUID? {
        let points = visibleKlines
        guard !points.isEmpty else { return nil }
        let slot = plot.slotWidth(forCount: points.count)

        for line in trendLines.reversed() {
            let start = plot.position(of: line.start, points: points, slotWidth: slot)
            let end = plot.position(of: line.end, points: points, slotWidth: slot)
            if Self.distance(from: point, toSegmentFrom: start, to: end) <= Drawing.hitTolerance {
                return line.id
            }
        }
        return nil
    }

    /// Shortest distance from a point to a line segment.
    private static func distance(from point: CGPoint, toSegmentFrom start: CGPoint, to end: CGPoint) -> CGFloat {
        let dx = end.x - start.x
        let dy = end.y - start.y
        let lengthSquared = dx * dx + dy * dy
        guard lengthSquared > 0 else { return hypot(point.x - start.x, point.y - start.y) }

        let projection = ((point.x - start.x) * dx + (point.y - start.y) * dy) / lengthSquared
        let t = projection.clamped(to: 0...1)
        return hypot(point.x - (start.x + t * dx), point.y - (start.y + t * dy))
    }

    // MARK: - Drawing a line

    /// First click: place the starting circle. The rubber band tracks the pointer
    /// from here until the second click.
    func beginDraft(at anchor: TrendAnchor) {
        draftStart = anchor
        draftEnd = anchor
        selectedLineID = nil
    }

    func updateDraft(to anchor: TrendAnchor) {
        guard draftStart != nil else { return }
        draftEnd = anchor
    }

    /// Second click. Rejects a click that landed back on the first one — a
    /// zero-length line would be invisible and impossible to select or delete — and
    /// leaves the draft open so the next click can still finish it.
    @discardableResult
    func commitDraft(at anchor: TrendAnchor, in plot: ChartPlot) -> Bool {
        guard let start = draftStart else { return false }
        let points = visibleKlines
        let slot = plot.slotWidth(forCount: points.count)
        let from = plot.position(of: start, points: points, slotWidth: slot)
        let to = plot.position(of: anchor, points: points, slotWidth: slot)
        guard hypot(to.x - from.x, to.y - from.y) >= Drawing.hitTolerance else { return false }

        trendLines.append(TrendLine(start: start, end: anchor))
        persistTrendLines()
        draftStart = nil
        draftEnd = nil
        return true
    }

    func cancelDraft() {
        draftStart = nil
        draftEnd = nil
    }

    /// Move one end of a line — called per mouse-move while a handle is dragged.
    func moveAnchor(lineID: UUID, isStart: Bool, to anchor: TrendAnchor) {
        guard let index = trendLines.firstIndex(where: { $0.id == lineID }) else { return }
        if isStart {
            trendLines[index].start = anchor
        } else {
            trendLines[index].end = anchor
        }
    }

    /// Flush endpoint movement once the drag finishes. Keeping this separate avoids
    /// an atomic disk write for every mouse-move event.
    func persistTrendLines() {
        DrawingStore.shared.save(trendLines, ticker: ticker, source: source)
    }

    func setColor(_ color: TrendLineColor, for lineID: UUID) {
        guard let index = trendLines.firstIndex(where: { $0.id == lineID }) else { return }
        trendLines[index].color = color
        persistTrendLines()
    }

    func setThickness(_ thickness: TrendLineThickness, for lineID: UUID) {
        guard let index = trendLines.firstIndex(where: { $0.id == lineID }) else { return }
        trendLines[index].thickness = thickness
        persistTrendLines()
    }

    @discardableResult
    func removeSelectedLine() -> Bool {
        guard let id = selectedLineID,
              let index = trendLines.firstIndex(where: { $0.id == id }) else { return false }
        trendLines.remove(at: index)
        selectedLineID = nil
        editingLineID = nil
        persistTrendLines()
        return true
    }

    // MARK: - Drawing a ruler

    /// First click: pin one corner. The rectangle tracks the pointer from here until the
    /// second click.
    func beginRulerDraft(at anchor: TrendAnchor) {
        rulerDraftStart = anchor
        rulerDraftEnd = anchor
    }

    func updateRulerDraft(to anchor: TrendAnchor) {
        guard rulerDraftStart != nil else { return }
        rulerDraftEnd = anchor
    }

    /// Second click. Rejects a click that landed back on the first one — a rectangle with
    /// no area would report 0% over one candle — and leaves the draft open so the next
    /// click can still finish it.
    @discardableResult
    func commitRulerDraft(at anchor: TrendAnchor, in plot: ChartPlot) -> Bool {
        guard let start = rulerDraftStart else { return false }
        let points = visibleKlines
        let slot = plot.slotWidth(forCount: points.count)
        let from = plot.position(of: start, points: points, slotWidth: slot)
        let to = plot.position(of: anchor, points: points, slotWidth: slot)
        guard hypot(to.x - from.x, to.y - from.y) >= Drawing.hitTolerance else { return false }

        rulers.append(RulerRect(start: start, end: anchor))
        rulerDraftStart = nil
        rulerDraftEnd = nil
        return true
    }

    func cancelRulerDraft() {
        rulerDraftStart = nil
        rulerDraftEnd = nil
    }

    /// Put every measurement on this chart away. Reports whether there was anything to
    /// clear, so the caller can tell a dismissing click from one that starts a rectangle.
    @discardableResult
    func clearRulers() -> Bool {
        guard !rulers.isEmpty else { return false }
        rulers.removeAll()
        return true
    }

    /// Update ticker symbol and/or source, re-fetch data.
    func updateTicker(symbol: String, source: DataSourceType, displayName: String? = nil, pmSeries: [PmSeriesConfig]? = nil) {
        ticker = symbol
        self.source = source
        self.displayName = displayName
        if let series = pmSeries { self.pmSeries = series } else if source != .polymarket { self.pmSeries = [] }
        self.pmSeriesData = [:]
        api = DataSourceFactory.shared.service(for: source)
        trendLines = DrawingStore.shared.lines(ticker: ticker, source: source)
    }

    private func observeDrawings() {
        drawingStoreSubscription = DrawingStore.shared.$linesByInstrument
            .sink { [weak self] allLines in
                guard let self else { return }
                let key = DrawingStore.shared.key(ticker: self.ticker, source: self.source)
                let sharedLines = allLines[key] ?? []
                if self.trendLines != sharedLines { self.trendLines = sharedLines }
            }
    }

    /// Merge a WebSocket kline tick into the current dataset.
    /// - Updates the in-progress (rightmost) candle in-place when the openTime matches.
    /// - Does NOT append new candles; REST refresh picks those up.
    func applyKlineUpdate(_ kline: KlineData) {
        guard let last = klineData.last else { return }

        if last.openTime == kline.openTime {
            klineData[klineData.count - 1].closePrice = kline.closePrice
            klineData[klineData.count - 1].highPrice = kline.highPrice
            klineData[klineData.count - 1].lowPrice = kline.lowPrice
            klineData[klineData.count - 1].volume = kline.volume
            klineData[klineData.count - 1].quoteVolume = kline.quoteVolume
        }
        // New candle (openTime > last.openTime): skip — REST fetch adds it within 5 s

        if currentPrice != kline.closePrice {
            currentPrice = kline.closePrice
        }
    }

    /// Fold a completed lower-resolution live bar into the current displayed candle.
    /// Alpaca's free stream emits minute bars even when the chart is hourly or daily.
    func applyLiveBar(_ bar: KlineData, candleDuration: TimeInterval) {
        guard !klineData.isEmpty else { return }
        let index = klineData.count - 1
        let last = klineData[index]
        guard bar.openTime >= last.openTime,
              bar.openTime < last.openTime.addingTimeInterval(candleDuration) else { return }

        klineData[index].highPrice = max(last.highPrice, bar.highPrice)
        klineData[index].lowPrice = min(last.lowPrice, bar.lowPrice)
        klineData[index].closePrice = bar.closePrice
        klineData[index].volume += bar.volume
        klineData[index].quoteVolume += bar.quoteVolume
        currentPrice = bar.closePrice
    }

    /// Show a different slice of the buffer without going back to the network.
    ///
    /// Zooming changes how many candles are on screen far more often than it exhausts
    /// the warm-up headroom, so the redraw happens immediately here and the refetch
    /// that tops the buffer back up runs behind it.
    func setVisibleCount(_ count: Int) {
        let wanted = Swift.max(1, count)
        guard wanted != visibleCount else { return }
        visibleCount = wanted
    }

    /// Candles to request for a visible window of `count`.
    ///
    /// Only the count-based sources can buy older history this way — see
    /// `DataSourceType.fetchesByCount`.
    private func fetchCount(for count: Int) -> Int {
        source.fetchesByCount ? count + Indicator.warmupHeadroom : count
    }

    /// Candles of history the overlays currently on need before the visible window.
    ///
    /// `Indicator.warmupHeadroom` is sized for the longest EMA on offer and fetched
    /// regardless; this is what's actually needed right now. CoinGecko has to choose
    /// between a window with real highs and lows and a deeper one without, and the
    /// answer turns on how far back this chart genuinely reads — a 20-period EMA and
    /// a 200-period EMA don't want the same window.
    private var indicatorWarmup: Int {
        var needed = 0
        if showEMA { needed = Swift.max(needed, emaPeriod) }
        if showRSI { needed = Swift.max(needed, RSI.period) }
        if showBollinger { needed = Swift.max(needed, Indicator.bollingerPeriod) }
        if showTrendFlips { needed = Swift.max(needed, Indicator.supertrendPeriod) }
        return needed
    }

    func fetchData(for range: TimeRange, count: Int, silent: Bool = false) async {
        // Silent refresh: if a fetch is already running let it finish.
        if silent, isFetching {
            return
        }

        visibleCount = Swift.max(1, count)
        let count = fetchCount(for: count)

        fetchTask?.cancel()
        fetchGeneration += 1
        let generation = fetchGeneration

        errorMessage = nil
        isFetching = true

        let hadData = !klineData.isEmpty
        let isSlowSource = source != .binance

        // Cache-first for slow sources: show stale data instantly.
        if !hadData, isSlowSource {
            if let cached = await api.getCachedKlines(symbol: apiSymbol, interval: range.binanceInterval, count: count) {
                klineData = cached
                currentPrice = cached.last?.closePrice
            }
        }

        fetchTask = Task { [weak self] in
            guard let self else { return }

            do {
                if isSlowSource,
                   let cgService = api as? CoinGeckoAPIService
                {
                    try await self.fetchStaged(
                        cgService: cgService,
                        range: range,
                        count: count,
                        generation: generation
                    )
                } else if let pmService = api as? PolymarketService {
                    // Polymarket needs the whole TimeRange, not the interval token:
                    // that token maps 1D and 3M both onto "1d".
                    if pmSeries.count > 1 {
                        try await self.fetchPmMultiSeries(
                            pmService: pmService,
                            range: range,
                            count: count,
                            generation: generation
                        )
                        return
                    } else {
                        let tokenID = pmSeries.first?.tokenID ?? self.apiSymbol
                        let data = try await pmService.fetchPrices(
                            tokenID: tokenID,
                            range: range,
                            count: count
                        )
                        guard !Task.isCancelled else { return }
                        guard fetchGeneration == generation else { return }
                        klineData = data
                        currentPrice = data.last?.closePrice
                    }
                } else {
                    let data = try await self.api.fetchKlines(
                        symbol: self.apiSymbol,
                        interval: range.binanceInterval,
                        limit: count
                    )
                    guard !Task.isCancelled else { return }
                    guard fetchGeneration == generation else { return }
                    klineData = data
                    currentPrice = data.last?.closePrice
                }

                guard !Task.isCancelled else { return }
                guard fetchGeneration == generation else { return }
                lastUpdated = Date()
                errorMessage = nil
            } catch is CancellationError {
                return
            } catch {
                guard !Task.isCancelled else { return }
                guard fetchGeneration == generation else { return }
                errorMessage = error.localizedDescription
            }

            // Only touch flags if this is still the current generation.
            guard fetchGeneration == generation else { return }
            isFetching = false
        }
    }

    /// Fetch all Polymarket series in parallel and update `pmSeriesData`.
    ///
    /// Individual series failures are silently ignored — the chart shows whatever
    /// data arrived. Sets `lastUpdated` on any partial or full success.
    private func fetchPmMultiSeries(
        pmService: PolymarketService,
        range: TimeRange,
        count: Int,
        generation: Int
    ) async throws {
        let series = pmSeries
        let results: [(String, [KlineData])] = await withTaskGroup(of: (String, [KlineData]).self) { group in
            for s in series {
                group.addTask {
                    let data = (try? await pmService.fetchPrices(tokenID: s.tokenID, range: range, count: count)) ?? []
                    return (s.tokenID, data)
                }
            }
            var collected: [(String, [KlineData])] = []
            for await pair in group { collected.append(pair) }
            return collected
        }

        guard !Task.isCancelled, fetchGeneration == generation else { return }

        pmSeriesData = Dictionary(uniqueKeysWithValues: results)
        syncPrimaryKlineData()
        lastUpdated = Date()
        errorMessage = nil
        isFetching = false
    }

    /// Consume staged kline data from CoinGecko, rendering each batch as it lands
    /// instead of waiting for the full window.
    ///
    /// Batches arrive coarse-to-exact: cached candles first (instant), then a
    /// 1-day probe if the chart was blank, then the requested window. Each batch
    /// replaces the previous one, so the chart fills in rather than sitting empty
    /// behind the rate limiter.
    private func fetchStaged(
        cgService: CoinGeckoAPIService,
        range: TimeRange,
        count: Int,
        generation: Int
    ) async throws {
        let stream = cgService.fetchKlinesStaged(
            symbol: apiSymbol,
            interval: range.binanceInterval,
            limit: count,
            requiredCount: visibleCount + indicatorWarmup,
            needsFirstPaint: klineData.isEmpty
        )

        for try await batch in stream {
            guard !Task.isCancelled else { return }
            // A newer fetch (zoom, timeframe, ticker change) owns the chart now.
            guard fetchGeneration == generation else { return }
            guard !batch.isEmpty else { continue }

            klineData = batch
            if let last = batch.last {
                currentPrice = last.closePrice
            }
            // Partial batches still count as a successful render.
            lastUpdated = Date()
        }
    }
}
