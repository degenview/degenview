import Foundation
import SwiftUI

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
        case .coingecko, .dexscreener, .polymarket:
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

    /// Every enabled indicator, computed over the full buffer and trimmed to the
    /// visible tail so warm-up happens off screen.
    var indicators: IndicatorSeries {
        IndicatorSeries.make(
            candles: klineData,
            visibleCount: visibleCount,
            showRSI: showRSI,
            showEMA: showEMA,
            emaPeriod: emaPeriod,
            showBollinger: showBollinger
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

    /// First click of a line in progress, and the rubber-band end that follows the
    /// pointer until the second click lands.
    @Published private(set) var draftStart: TrendAnchor?
    @Published private(set) var draftEnd: TrendAnchor?

    @Published var selectedLineID: UUID?

    var trendDraft: (start: TrendAnchor, end: TrendAnchor)? {
        guard let draftStart, let draftEnd else { return nil }
        return (draftStart, draftEnd)
    }

    var hasDraft: Bool { draftStart != nil }

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
        trendLines = config.trendLines ?? []
        if let name = config.displayName { displayName = name }
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

    @discardableResult
    func removeSelectedLine() -> Bool {
        guard let id = selectedLineID,
              let index = trendLines.firstIndex(where: { $0.id == id }) else { return false }
        trendLines.remove(at: index)
        selectedLineID = nil
        return true
    }

    /// Update ticker symbol and/or source, re-fetch data.
    func updateTicker(symbol: String, source: DataSourceType, displayName: String? = nil) {
        ticker = symbol
        self.source = source
        self.displayName = displayName
        api = DataSourceFactory.shared.service(for: source)
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
                    let data = try await pmService.fetchPrices(
                        tokenID: self.apiSymbol,
                        range: range,
                        count: count
                    )
                    guard !Task.isCancelled else { return }
                    guard fetchGeneration == generation else { return }
                    klineData = data
                    currentPrice = data.last?.closePrice
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
