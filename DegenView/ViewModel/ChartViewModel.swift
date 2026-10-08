import Combine
import Foundation
import SwiftUI

@MainActor
final class ChartViewModel: ObservableObject {
    @Published var ticker: String
    @Published var source: DataSourceType

    /// Human-readable label, for sources whose `ticker` is an opaque identifier.
    /// A Polymarket CLOB token id is 77 digits (and a Kalshi ticker is opaque), so the market
    /// question rides along.
    @Published var displayName: String?
    @Published var portfolioChart: PortfolioChartConfig?
    @Published var coinMarketCapChart: CoinMarketCapChartConfig?
    @Published var bitcoinPowerLaw: BitcoinPowerLawConfig?
    @Published private(set) var powerLawHistory: [BitcoinDailyClose] = []
    @Published private(set) var powerLawWarning: String?
    @Published private(set) var powerLawXZoom = 1.0

    var isPortfolioChart: Bool { portfolioChart != nil }
    var isBitcoinPowerLaw: Bool { bitcoinPowerLaw != nil }

    func adjustPowerLawXZoom(scrollingUp: Bool) {
        let factor = scrollingUp ? 1.12 : 1 / 1.12
        powerLawXZoom = (powerLawXZoom * factor).clamped(to: 1...20)
    }

    /// The market this chart shows, as a string. Follows `updateTicker`, so it never names a market
    /// the chart has left; `chartID` is the identity that survives a switch.
    var uniqueID: String { "\(ticker)_\(source.rawValue)" }

    /// The market this chart shows.
    var instrumentID: InstrumentID { InstrumentID(source: source, symbol: ticker) }

    /// Whether this chart already shows `instrument`. Compared by the id each provider's API takes, so a
    /// bare Binance `BTC` and `BTCUSDT` are the same market.
    func shows(_ instrument: InstrumentID) -> Bool {
        instrumentID.isSameMarket(as: instrument)
    }

    private var api: TickerDataSource

    /// Where a market switch finds the provider for its new source. Tests substitute a stub.
    var serviceResolver: (DataSourceType) -> TickerDataSource = { DataSourceFactory.shared.service(for: $0) }

    /// What the card header and settings sheet call this chart.
    ///
    /// Exchange pairs read `BASE/QUOTE` whichever id the exchange uses (`BTCUSDT`, `BTC-USD`).
    /// Only the label changes — `ticker` stays the identity everything else is keyed by.
    var title: String {
        if let displayName, !displayName.isEmpty { return displayName }
        return marketPair?.display ?? ticker.uppercased()
    }

    /// The pair an exchange chart trades. The persisted ticker is read as-is, so `ETHBTC` stays
    /// `ETH/BTC`; only a bare asset (`BTC` on Binance) goes through `apiSymbol`, which supplies the
    /// quote it will fetch against.
    var marketPair: MarketSymbol? {
        // A CoinGecko chart stores a coin id ("bitcoin"); its symbol has to be looked up.
        if source == .coingecko { return coinSymbol.map(MarketSymbol.coinGecko(symbol:)) }
        return MarketSymbol(ticker: ticker, source: source) ?? MarketSymbol(ticker: apiSymbol, source: source)
    }

    /// Ticker symbol behind a CoinGecko coin id, once `resolveCoinSymbol()` has found it.
    /// Until then the header keeps showing the id.
    @Published var coinSymbol: String?

    /// Look up `coinSymbol` for a CoinGecko chart. Independent of icon caching, so charts saved
    /// before this existed are labelled too.
    func resolveCoinSymbol() async {
        guard source == .coingecko, coinSymbol == nil else { return }
        let id = ticker
        let symbol = await IconResolver.shared.symbol(forCoinID: id)
        // The chart may have been pointed at another coin while the lookup ran.
        guard source == .coingecko, ticker == id else { return }
        coinSymbol = symbol
    }

    /// The symbol used for API calls — source-dependent.
    var apiSymbol: String {
        // For every source but Binance and Coinbase the ticker IS the fullSymbol (coin ID,
        // pair address, CLOB token id, or Kalshi "SERIES/MARKET").
        InstrumentID(source: source, symbol: ticker).apiSymbol
    }

    /// Base asset symbol for icon lookup (strips quote currency suffixes).
    var baseSymbol: String {
        switch source {
        case .binance, .coinbase:
            return marketPair?.base ?? ticker.uppercased()
        case .coingecko, .dexscreener:
            let parts = ticker.components(separatedBy: "/")
            return parts.first?.uppercased() ?? ticker.uppercased()
        case .polymarket, .kalshi:
            // The ticker is an opaque market id; the monogram fallback needs the question.
            return title
        case .alpaca:
            return ticker.uppercased()
        case .coinMarketCap:
            return "CMC"
        }
    }

    /// Whether prices read as USD or as probabilities.
    var priceScale: PriceScale { source.priceScale }

    /// Prediction markets report one price per timestamp, so they draw as a line.
    var usesLineChart: Bool { source.isPredictionMarket }

    /// Identity of the icon currently wanted: the market's own key, so the card never keeps
    /// showing a previous coin's artwork after a switch.
    var iconKey: String { "\(source.rawValue):\(ticker)" }

    /// All tradable choices for multi-outcome prediction-market events. Empty for single-choice
    /// markets and all other sources.
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
            let gated: [KlineData]
            if let replayTimestamp {
                gated = Array(data.prefix(Self.upperBound(of: replayTimestamp, in: data)))
            } else {
                gated = data
            }
            return (s.tokenID, s.label, gated, pmColor(for: s.tokenID))
        }
    }

    /// Highest currently visible prediction-market choice, used by the card subtitle.
    var leadingMarketChoice: (label: String, price: Double)? {
        guard source.isPredictionMarket else { return nil }
        if pmSeries.count > 1 {
            return pmVisibleSeries.compactMap { series in
                series.data.last.map { (label: series.label, price: $0.closePrice) }
            }.max { $0.price < $1.price }
        }
        guard let label = pmSeries.first?.label, let price = displayedPrice else { return nil }
        return (label, price)
    }

    /// Toggle a prediction-market series on/off by its market id and refresh the primary data.
    func togglePmSeries(_ tokenID: String) {
        guard let i = pmSeries.firstIndex(where: { $0.tokenID == tokenID }) else { return }
        pmSeries[i].enabled.toggle()
        syncPrimaryKlineData()
    }

    /// Keep `klineData` / `currentPrice` in sync with the first enabled PM series.
    private func syncPrimaryKlineData() {
        guard !pmSeries.isEmpty else { return }
        if let first = pmSeries.first(where: \.enabled),
            let data = pmSeriesData[first.tokenID], !data.isEmpty
        {
            klineData = data
            currentPrice = data.last?.closePrice
        }
    }

    /// Everything fetched, including the warm-up candles that sit off the left edge.
    /// Read `visibleKlines` for what the chart actually draws.
    @Published var klineData: [KlineData] = []

    /// The tab's authoritative replay clock. The canonical fetched series remains
    /// untouched; every chart/indicator/interaction consumer reads through this gate.
    private var granularReplayCache = GranularReplayCache()
    @Published private(set) var replayTimestamp: Date?
    @Published private(set) var replaySelectionTimestamp: Date?
    @Published private(set) var granularReplayData: [KlineData] = []
    private(set) var granularReplayInterval: ReplayInterval?

    var replayKlines: [KlineData] {
        guard let replayTimestamp else { return klineData }
        guard let interval = granularReplayInterval,
            let sourceSeconds = interval.seconds,
            !granularReplayData.isEmpty
        else { return Array(klineData.prefix(Self.upperBound(of: replayTimestamp, in: klineData))) }
        return aggregatedReplayKlines(through: replayTimestamp, sourceSeconds: sourceSeconds)
    }

    /// How many of the newest candles are on screen. The rest are warm-up for
    /// period-based indicators, and slack that lets a zoom redraw without refetching.
    @Published private(set) var visibleCount: Int = TimeRange.oneDay.dataPointLimit

    /// The candles the chart draws — the newest `visibleCount` of the buffer.
    var visibleKlines: [KlineData] {
        let available = replayKlines
        guard visibleCount > 0, available.count > visibleCount else { return available }
        return Array(available.suffix(visibleCount))
    }

    @Published var errorMessage: String?
    @Published var lastUpdated: Date?
    @Published var currentPrice: Double?
    /// Best bid and ask from the exchange stream (Binance, Coinbase); empty for other sources.
    let liveQuote = ChartLiveQuote()
    @Published private(set) var cmcAltcoinLatest: AltcoinSeasonLatest?
    @Published private(set) var cmcAltcoinHistory: [AltcoinSeasonHistoricalPoint] = []
    @Published private(set) var cmcFearGreedLatest: FearAndGreedLatest?
    @Published private(set) var cmcFearGreedHistory: [FearAndGreedHistoricalPoint] = []

    // MARK: - Chart appearance settings

    @Published var bullishColor: Color = .green
    @Published var bearishColor: Color = .red
    @Published var yAxisDecimalPlaces: Int? = nil  // nil = auto-detect

    /// Turnover bars under the candles. Off by default — only Binance reports the
    /// quote volume they're drawn from.
    @Published var showVolume: Bool = false {
        didSet { if !showVolume { hiddenBuiltIns.remove(.volume) } }
    }

    /// RSI line across the bottom of the plot. Off by default. Computed from closes,
    /// so unlike volume it works on every source.
    @Published var showRSI: Bool = false {
        didSet { if !showRSI { hiddenBuiltIns.remove(.rsi) } }
    }

    /// Price-scale overlays: an EMA at `emaPeriod`, and Bollinger bands.
    @Published var showEMA: Bool = false {
        didSet { if !showEMA { hiddenBuiltIns.remove(.ema) } }
    }
    @Published var emaPeriod: Int = Indicator.emaDefaultPeriod
    @Published var showBollinger: Bool = false {
        didSet { if !showBollinger { hiddenBuiltIns.remove(.bollinger) } }
    }
    @Published var showTrendFlips: Bool = false {
        didSet { if !showTrendFlips { hiddenBuiltIns.remove(.trendFlips) } }
    }

    /// Applied built-ins that are switched off without being removed (the chip's eye). A flag turned
    /// off drops its entry, so re-adding an indicator always shows it.
    @Published var hiddenBuiltIns: Set<BuiltInIndicator> = []

    // MARK: - Pine script

    @Published var pineConfiguration: PineConfiguration?
    @Published var scriptInstances: [ChartScriptInstance] = []
    var chartID = UUID()
    @Published private(set) var pineOutput: PineVisualOutput = .empty
    @Published private(set) var pineDiagnostics: [PineDiagnostic] = []
    @Published private(set) var pineStatus = "No script applied"
    /// Resolves `chart.fg_color` / `chart.bg_color`; set by the card from its color scheme.
    private(set) var pineTheme: PineChartTheme = .dark
    /// Feeds the chart's Pine host, one operation at a time and in order.
    private var pineFeed: AsyncStream<PineFeedOperation>.Continuation?
    private var pineFeedTask: Task<Void, Never>?
    private var pineGeneration = 0
    /// The dataset the Pine host currently holds. Live updates are fed only while it matches the
    /// chart's requested symbol and timeframe, so a tick for a new timeframe cannot land on old bars.
    private var pineDataset: PineDatasetKey?
    /// The timeframe most recently requested from `fetchData`.
    private var requestedRange: TimeRange?
    /// Called on the main actor with alerts a script raised on a live bar, and the applied instance
    /// that raised them (nil for the single-script preview path). Historical calculation and loading
    /// never call it.
    var pineAlertHandler: (([PineAlertEvent], PineBarID?, UUID?) -> Void)?
    /// Called on the main actor whenever the applied scripts are recalculated from scratch or learn
    /// their source: with the market they now run on and the source hash of every instance that has
    /// one. An instance whose source is not resolved yet is absent, never "changed".
    var pineContextHandler: ((PineDatasetKey, [UUID: String]) -> Void)?
    /// Called on the main actor with an applied instance's id once it is removed from the chart.
    var pineInstanceRemovedHandler: ((UUID) -> Void)?

    /// Every enabled indicator, computed over the full buffer and trimmed to the
    /// visible tail so warm-up happens off screen.
    var indicators: IndicatorSeries {
        IndicatorSeries.make(
            candles: replayKlines,
            visibleCount: visibleCount,
            showRSI: isDrawn(.rsi),
            showEMA: isDrawn(.ema),
            emaPeriod: emaPeriod,
            showBollinger: isDrawn(.bollinger),
            showTrendFlips: isDrawn(.trendFlips)
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
    @Published var fibonacciRetracements: [FibonacciRetracementDrawing] = []
    var drawingUndoCoordinator: DrawingUndoCoordinator?
    let drawingStore: DrawingStore
    private var fibonacciSettingsOriginal: FibonacciRetracementDrawing?
    private var drawingStoreSubscription: AnyCancellable?
    private var fibonacciStoreSubscription: AnyCancellable?
    private var brushStoreSubscription: AnyCancellable?

    /// First click of a line in progress, and the rubber-band end that follows the
    /// pointer until the second click lands.
    @Published private(set) var draftStart: TrendAnchor?
    @Published private(set) var draftEnd: TrendAnchor?

    @Published var selectedLineID: UUID?
    /// The selected line whose appearance popover is open. Kept separate from
    /// selection so grabbing an endpoint does not open a popover under the drag.
    @Published var editingLineID: UUID?
    @Published var selectedFibonacciID: UUID?
    @Published var editingFibonacciID: UUID?
    @Published private(set) var fibonacciDraftStart: TrendAnchor?
    @Published private(set) var fibonacciDraftEnd: TrendAnchor?

    var fibonacciDraft: (start: TrendAnchor, end: TrendAnchor)? {
        guard let fibonacciDraftStart, let fibonacciDraftEnd else { return nil }
        return (fibonacciDraftStart, fibonacciDraftEnd)
    }

    var hasFibonacciDraft: Bool { fibonacciDraftStart != nil }

    // MARK: - Brush

    /// Freehand strokes on this chart, anchored to time and price.
    @Published var brushes: [BrushDrawing] = []
    @Published var selectedBrushID: UUID?
    /// The stroke whose style editor is open. Separate from selection so grabbing a
    /// stroke to move it does not open an editor under the drag.
    @Published var editingBrushID: UUID?
    @Published var hoveredBrushID: UUID?
    /// The stroke being drawn right now. Never persisted until the pointer is released.
    @Published var brushDraft: BrushDraft?
    /// Screen position of the last kept sample, so the next one is judged by distance.
    var lastBrushSample: CGPoint?
    var brushSettingsOriginal: BrushDrawing?

    var hasBrushDraft: Bool { brushDraft != nil }

    var brushOverlay: BrushOverlayState {
        BrushOverlayState(
            strokes: brushes, draft: brushDraft, selectedID: selectedBrushID, hoveredID: hoveredBrushID)
    }

    var trendDraft: (start: TrendAnchor, end: TrendAnchor)? {
        guard let draftStart, let draftEnd else { return nil }
        return (draftStart, draftEnd)
    }

    var hasDraft: Bool { draftStart != nil }

    // MARK: - Ruler

    /// Measuring rectangles on this chart. Never persisted, never restored and never in
    /// drawing history — a ruler answers a question and goes away with the tool.
    @Published private(set) var rulers: [RulerRect] = []

    /// First corner of a rectangle in progress, and the opposite corner that follows the
    /// pointer until it is finished.
    @Published private(set) var rulerDraftStart: TrendAnchor?
    @Published private(set) var rulerDraftEnd: TrendAnchor?

    /// The ruler showing its handles because it was clicked, and the part of a ruler the
    /// pointer is over right now (which drives the highlight and the cursor).
    @Published private(set) var selectedRulerID: UUID?
    @Published private(set) var hoveredRuler: RulerHit?

    var rulerDraft: (start: TrendAnchor, end: TrendAnchor)? {
        guard let rulerDraftStart, let rulerDraftEnd else { return nil }
        return (rulerDraftStart, rulerDraftEnd)
    }

    var hasRulerDraft: Bool { rulerDraftStart != nil }

    var rulerOverlay: RulerOverlayState {
        RulerOverlayState(
            rects: rulers, draft: rulerDraft, selectedID: selectedRulerID, hover: hoveredRuler)
    }

    private var fetchTask: Task<Void, Never>?
    private var fetchGeneration = 0
    @Published private(set) var isFetching = false

    /// Change across the *visible* window — the warm-up candles are off screen, so
    /// counting them would report a move the user can't see.
    var priceChangePercent: Double? {
        visibleKlines.priceChangePercent
    }

    var priceChangeAmount: Double? {
        visibleKlines.priceChangeAmount
    }

    var displayedPrice: Double? {
        replayTimestamp == nil ? currentPrice : replayKlines.last?.closePrice
    }

    func applyReplayTimestamp(_ timestamp: Date?) {
        let changed = replayTimestamp != timestamp
        replayTimestamp = timestamp
        // Replay recalculates as pure history; leaving it goes back to the live feed.
        if changed, pineConfiguration?.appliedSource?.isEmpty == false { reevaluatePine() }
        if changed, !scriptInstances.isEmpty { reevaluateAllPineInstances() }
    }

    func applyReplaySelectionTimestamp(_ timestamp: Date?) {
        replaySelectionTimestamp = timestamp
    }

    var supportedReplayIntervals: [ReplayInterval] {
        guard let source = api as? GranularReplayDataSource else { return [.automatic, .chartBar] }
        let span: TimeInterval = {
            guard let first = klineData.first, let last = klineData.last else { return 0 }
            return last.openTime.timeIntervalSince(first.openTime) + currentReplayChartSeconds
        }()
        return source.supportedReplayIntervals(chartInterval: currentReplayChartInterval).filter {
            interval in
            guard let seconds = interval.seconds else { return true }
            return span <= 0 || span / seconds <= 100_000
        }
    }

    /// The chart bar a replay beginning at `date` starts on (the first bar when `date` is earlier).
    private func replayFetchStart(from date: Date?) -> Date? {
        guard let first = klineData.first?.openTime else { return nil }
        guard let date else { return first }
        return klineData.last(where: { $0.openTime <= date })?.openTime ?? first
    }

    func resolvedReplayInterval(_ requested: ReplayInterval, from date: Date? = nil) -> ReplayInterval {
        guard requested == .automatic else {
            return supportedReplayIntervals.contains(requested) ? requested : .chartBar
        }
        let concrete = supportedReplayIntervals.compactMap {
            interval -> (ReplayInterval, TimeInterval)? in
            guard let seconds = interval.seconds else { return nil }
            return (interval, seconds)
        }.sorted { $0.1 < $1.1 }
        guard let first = replayFetchStart(from: date), let last = klineData.last else { return .chartBar }
        let span = last.openTime.timeIntervalSince(first) + currentReplayChartSeconds
        return concrete.first(where: { span / $0.1 <= 100_000 })?.0 ?? concrete.last?.0 ?? .chartBar
    }

    /// Loads the fine-grained history a replay steps through, from the chart bar holding `date`
    /// (the whole chart when nil). Bars before that point are never stepped through, so they
    /// are not downloaded; they show as the chart's own candles.
    func loadGranularReplayData(interval requested: ReplayInterval, from date: Date? = nil) async throws
        -> ReplayInterval
    {
        let interval = resolvedReplayInterval(requested, from: date)
        guard interval != .chartBar,
            let source = api as? GranularReplayDataSource,
            let start = replayFetchStart(from: date),
            let last = klineData.last
        else {
            granularReplayData = []
            granularReplayInterval = nil
            return .chartBar
        }
        let end = last.openTime.addingTimeInterval(currentReplayChartSeconds)
        let key = GranularReplayCache.Key(symbol: apiSymbol, interval: interval)
        let data: [KlineData]
        if let cached = granularReplayCache.slice(key, start: start, end: end) {
            data = cached
        } else {
            data = try await source.fetchReplayKlines(
                symbol: apiSymbol, interval: interval, start: start, end: end, maximumCount: 100_000)
            granularReplayCache.store(key, start: start, end: end, data: data)
        }
        guard !data.isEmpty else {
            granularReplayData = []
            granularReplayInterval = nil
            return .chartBar
        }
        granularReplayData = data
        granularReplayInterval = interval
        return interval
    }

    func clearGranularReplayData() {
        granularReplayData = []
        granularReplayInterval = nil
    }

    func replayTimeline(fallbackToChartBars: Bool = true) -> [Date] {
        if let seconds = granularReplayInterval?.seconds, !granularReplayData.isEmpty {
            return granularReplayData.map { $0.openTime.addingTimeInterval(seconds) }
        }
        return fallbackToChartBars ? klineData.map(\.openTime) : []
    }

    func replayBarEnd(containing date: Date) -> Date {
        guard let index = klineData.lastIndex(where: { $0.openTime <= date }) else { return date }
        if index + 1 < klineData.count { return klineData[index + 1].openTime }
        return klineData[index].openTime.addingTimeInterval(currentReplayChartSeconds)
    }

    private var currentReplayChartInterval: String {
        // The owning ContentViewModel passes the same selected TimeRange used to fetch;
        // fetched bar spacing is more reliable for fallback ranges with shared tokens.
        guard klineData.count > 1 else { return "1d" }
        let spacing = klineData[1].openTime.timeIntervalSince(klineData[0].openTime)
        if spacing <= 3_600 { return "1h" }
        if spacing <= 86_400 { return "1d" }
        if spacing <= 604_800 { return "1w" }
        return "1M"
    }

    private var currentReplayChartSeconds: TimeInterval {
        guard klineData.count > 1 else { return 86_400 }
        return max(60, klineData[1].openTime.timeIntervalSince(klineData[0].openTime))
    }

    private func aggregatedReplayKlines(through timestamp: Date, sourceSeconds: TimeInterval)
        -> [KlineData]
    {
        var output: [KlineData] = []
        let canonicalEnd = Self.upperBound(of: timestamp, in: klineData)
        output.reserveCapacity(canonicalEnd)

        for index in 0..<canonicalEnd {
            let candle = klineData[index]
            let bucketEnd =
                index + 1 < klineData.count
                ? klineData[index + 1].openTime
                : candle.openTime.addingTimeInterval(currentReplayChartSeconds)
            let observedEnd = min(timestamp, bucketEnd)
            let startIndex = Self.lowerBound(of: candle.openTime, in: granularReplayData)
            let endIndex = Self.upperBound(
                of: observedEnd.addingTimeInterval(-sourceSeconds),
                in: granularReplayData
            )
            if startIndex < endIndex,
                let aggregate = ReplayEngine.aggregate(
                    granularReplayData[startIndex..<endIndex],
                    bucketStart: candle.openTime
                )
            {
                output.append(aggregate)
            } else if bucketEnd <= timestamp {
                // The displayed provider already confirmed this completed bucket.
                // This covers session gaps without exposing a still-forming candle.
                output.append(candle)
            }
        }
        return output
    }

    func replayDate(nearestTo point: CGPoint, in plot: ChartPlot) -> Date? {
        let points = visibleKlines
        guard !points.isEmpty else { return nil }
        let fractional = plot.fractionalIndex(
            forX: point.x, slotWidth: plot.slotWidth(forCount: points.count))
        let index = Int(fractional.rounded()).clamped(to: 0...(points.count - 1))
        return points[index].openTime
    }

    private static func upperBound(of timestamp: Date, in candles: [KlineData]) -> Int {
        var low = 0
        var high = candles.count
        while low < high {
            let mid = (low + high) / 2
            if candles[mid].openTime <= timestamp { low = mid + 1 } else { high = mid }
        }
        return low
    }

    private static func lowerBound(of timestamp: Date, in candles: [KlineData]) -> Int {
        var low = 0
        var high = candles.count
        while low < high {
            let mid = (low + high) / 2
            if candles[mid].openTime < timestamp { low = mid + 1 } else { high = mid }
        }
        return low
    }

    var priceChangeIsPositive: Bool {
        guard let change = priceChangePercent else { return true }
        return change >= 0
    }

    init(
        ticker: String, source: DataSourceType = .binance, displayName: String? = nil,
        api: TickerDataSource? = nil, drawingStore: DrawingStore? = nil
    ) {
        self.ticker = ticker
        self.source = source
        self.displayName = displayName
        self.api = api ?? DataSourceFactory.shared.service(for: source)
        self.drawingStore = drawingStore ?? .shared
        observeDrawings()
    }

    /// Apply persisted chart settings from a TickerConfig.
    func applyConfig(_ config: TickerConfig) {
        chartID = config.chartID
        scriptInstances = Self.migratedPineInstances(config)
        portfolioChart = config.portfolioChart
        coinMarketCapChart = config.coinMarketCapChart
        bitcoinPowerLaw = config.bitcoinPowerLaw
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
        hiddenBuiltIns = Set((config.hiddenIndicators ?? []).compactMap(BuiltInIndicator.init(rawValue:)))
        trendLines = drawingStore.lines(ticker: ticker, source: source)
        fibonacciRetracements = drawingStore.fibs(ticker: ticker, source: source)
        brushes = drawingStore.brushes(ticker: ticker, source: source)
        if let name = config.displayName { displayName = name }
        if let series = config.pmSeries, !series.isEmpty { pmSeries = series }
        hydratePineInstances()
    }

    /// `scriptInstances` from `config.scripts` when present (the modern shape); otherwise,
    /// one `legacySource`-carrying instance synthesized from the pre-instance
    /// `TickerConfig.pine` shape (raw applied source text, no `ScriptStore` entry to resolve
    /// it from). `config.pine` is never written back by this path — it exists purely so an
    /// old persisted chart keeps working after decode.
    /// Not `private`: exercised directly by `ChartScriptInstanceDecodeTests`. `nonisolated`
    /// because it's a pure function of its argument — no actor state involved.
    nonisolated static func migratedPineInstances(_ config: TickerConfig) -> [ChartScriptInstance] {
        guard config.scripts.isEmpty else { return config.scripts }
        guard let pine = config.pine, let source = pine.appliedSource, !source.isEmpty else { return [] }
        return [
            ChartScriptInstance(
                scriptID: UUID(), loadedRevisionID: UUID(), inputs: pine.inputs, isVisible: true,
                updateStatus: .current, legacySource: source)
        ]
    }

    func updatePineDraft(_ source: String) {
        if pineConfiguration == nil { pineConfiguration = PineConfiguration() }
        pineConfiguration?.draftSource = source
    }

    @discardableResult func applyPineDraft() -> Bool {
        guard var config = pineConfiguration else { return false }
        let compiled = PineCompiler.compile(source: config.draftSource, libraries: PineLibraryRegistry.shared)
        pineDiagnostics = compiled.diagnostics
        guard compiled.isValid else {
            pineStatus = "Compile failed — last valid output remains active"
            return false
        }
        config.appliedSource = config.draftSource
        let validIDs = Set(compiled.inputSchema.inputs.map(\.id))
        config.inputs = config.inputs.filter { validIDs.contains($0.key) }
        pineConfiguration = config
        pineStatus = "Evaluating…"
        reevaluatePine(compiled: compiled)
        return true
    }

    /// Replaces the running script with `source`. Returns false when it does not compile, in which
    /// case the previous output stays active.
    @discardableResult func loadPineScript(source: String, inputs: [String: PineInputValue] = [:]) -> Bool {
        pineConfiguration = PineConfiguration(
            draftSource: source, appliedSource: pineConfiguration?.appliedSource, inputs: inputs)
        return applyPineDraft()
    }

    /// Stops the running script and forgets its source and inputs.
    func unloadPineScript() {
        let removed = scriptInstances.map(\.id)
        scriptInstances = []
        removed.forEach { pineInstanceRemovedHandler?($0) }
        pineConfiguration = nil
        pineDiagnostics = []
        pineStatus = "No script applied"
        reevaluatePine()
        reevaluateAllPineInstances()
    }

    func setPineInput(_ value: PineInputValue, id: String) {
        pineConfiguration?.inputs[id] = value
        reevaluatePine()
    }

    func setPineTheme(_ theme: PineChartTheme) {
        guard theme != pineTheme else { return }
        pineTheme = theme
        reevaluatePine()
    }

    /// `overlay=false` scripts get their own pane under the candles; the line chart draws no scripts.
    var showsPinePane: Bool { !usesLineChart && !pineOutput.overlay }

    /// Height of that pane out of a chart area `chartHeight` tall — the candles keep the rest.
    func pinePaneHeight(forChartHeight chartHeight: CGFloat) -> CGFloat {
        showsPinePane ? (chartHeight * 0.3).rounded() : 0
    }

    /// The market the applied script runs on.
    var pineAlertDataset: PineDatasetKey { pineDataset(for: requestedRange ?? .oneDay) }

    /// Hash of the applied script's source, nil when none is applied.
    var appliedSourceHash: String? {
        guard let source = pineConfiguration?.appliedSource, !source.isEmpty else { return nil }
        return ScriptSourceHash.sha256(source)
    }

    /// The dataset a fetch for `range` fills.
    private func pineDataset(for range: TimeRange) -> PineDatasetKey {
        PineDatasetKey(symbolKey: "\(source.rawValue):\(ticker)", timeframe: range.rawValue)
    }

    /// Recalculates the applied script from scratch over the current bars: on a config, script, input,
    /// theme, symbol or timeframe change, or when the feed can no longer be reconciled. A fresh host
    /// means no state survives from the previous program or dataset.
    func reevaluatePine(compiled supplied: PineCompiledProgram? = nil) {
        stopPineFeed()
        pineGeneration += 1
        let generation = pineGeneration
        pineContextHandler?(pineAlertDataset, [:])
        guard let config = pineConfiguration, let source = config.appliedSource, !source.isEmpty else {
            pineOutput = .empty
            return
        }
        let bars = replayKlines
        let live = replayTimestamp == nil
        let dataset = pineDataset(for: requestedRange ?? .oneDay)
        let inputs = config.inputs
        let theme = pineTheme
        let symbol = PineSymbolInfo(
            ticker: ticker, tickerID: dataset.symbolKey,
            type: self.source == .alpaca ? "stock" : self.source.isPredictionMarket ? "prediction" : "crypto")
        let securityChart = PineSecurityTarget.Chart(
            tickerID: dataset.symbolKey, source: self.source, apiSymbol: apiSymbol)
        let (operations, feed) = AsyncStream.makeStream(of: PineFeedOperation.self)
        pineFeed = feed
        pineDataset = dataset
        pineFeedTask = Task.detached(priority: .userInitiated) { [weak self] in
            var compiled = supplied
            var host: PineExecutionHost?
            var liveSecurity: PineSecurityDataProvider?
            for await operation in operations {
                if Task.isCancelled { return }
                var outcome: PineExecutionOutcome
                switch operation {
                case .rebuild:
                    let program = compiled ?? PineCompiler.compile(source: source, libraries: PineLibraryRegistry.shared)
                    compiled = program
                    guard program.isValid else {
                        await self?.applyPine(program: program, outcome: nil, generation: generation)
                        host = nil
                        continue
                    }
                    liveSecurity = await PineSecurityFeed.prepare(
                        program: program, inputs: inputs, theme: theme, symbol: symbol, chart: securityChart,
                        bars: bars)
                    let fresh = PineExecutionHost(
                        program: program, dataset: dataset, inputs: inputs, theme: theme, symbol: symbol,
                        securityData: liveSecurity)
                    host = fresh
                    outcome = await fresh.rebuild(bars: bars, live: live)
                case .ingest(let update):
                    guard let host else { continue }
                    outcome = await host.ingest(update)
                case .sync(let snapshot):
                    guard let host else { continue }
                    // Intrabars for `request.security_lower_tf` follow the market with the chart's own refresh.
                    await (liveSecurity as? PineIntrabarSeries)?.refresh()
                    outcome = await host.sync(snapshot: snapshot)
                }
                guard let program = compiled else { continue }
                await self?.applyPine(program: program, outcome: outcome, generation: generation)
            }
        }
        feed.yield(.rebuild)
    }

    private func stopPineFeed() {
        pineFeed?.finish()
        pineFeedTask?.cancel()
        pineFeed = nil
        pineFeedTask = nil
    }

    /// Whether live updates may reach the Pine host right now.
    private var pineAcceptsLiveUpdates: Bool {
        guard pineFeed != nil, replayTimestamp == nil, let requestedRange else { return false }
        return pineDataset == pineDataset(for: requestedRange)
    }

    /// Feeds a live candle observation into the applied script's execution pipeline.
    private func feedPine(_ bar: KlineData, origin: PineMarketUpdate.Origin) {
        guard pineAcceptsLiveUpdates else { return }
        pineFeed?.yield(.ingest(PineMarketUpdate(bar: bar, origin: origin)))
    }

    /// After a fetch: reconcile a running script with the refreshed bars, or rebuild when the symbol,
    /// timeframe or script changed under it.
    private func syncPine(range: TimeRange) {
        guard let source = pineConfiguration?.appliedSource, !source.isEmpty else { return }
        guard replayTimestamp == nil, pineFeed != nil, pineDataset == pineDataset(for: range) else {
            reevaluatePine()
            return
        }
        pineFeed?.yield(.sync(klineData))
    }

    private func applyPine(program: PineCompiledProgram, outcome: PineExecutionOutcome?, generation: Int) {
        guard pineGeneration == generation else { return }
        guard let outcome else {
            pineDiagnostics = program.diagnostics
            pineStatus = "Compile failed"
            return
        }
        switch outcome {
        case .updated(let update):
            pineOutput = update.output
            pineDiagnostics = program.diagnostics
            let live = update.executions.contains { $0.isRealtime }
            pineStatus = "Applied \(program.declaration.title) · \(update.output.barCount) bars\(live ? " · live" : "")"
            if !update.alerts.isEmpty { pineAlertHandler?(update.alerts, update.barID, nil) }
        case .unchanged:
            break
        case .needsRebuild:
            reevaluatePine(compiled: program)
        case .failed(let diagnostic):
            pineDiagnostics = [diagnostic]
            pineStatus = "Runtime failed — last valid output remains active"
        }
    }


    // MARK: - Pine instances (multi-script)

    /// Latest result per applied instance, keyed by `ChartScriptInstance.id`.
    @Published private(set) var pineResults: [UUID: PineInstanceResult] = [:]

    /// Execution plumbing per instance — mirrors the singular `pineFeed`/`pineFeedTask`/
    /// `pineGeneration`/`pineDataset` above, but one per `scriptInstances` entry.
    private var pineRuntimes: [UUID: PineInstanceRuntime] = [:]

    /// Seeds the multi-instance pipeline from `scriptInstances` right after it's hydrated
    /// from a decoded/migrated `TickerConfig`.
    private func hydratePineInstances() {
        reevaluateAllPineInstances()
    }

    /// Appends a new instance running `source` at the end of `scriptInstances`. Returns its
    /// id, or nil when `source` fails to compile — nothing is added, existing instances are
    /// untouched.
    @discardableResult
    func addPineInstance(
        scriptID: UUID, revisionID: UUID, source: String, inputs: [String: PineInputValue] = [:]
    ) -> UUID? {
        let compiled = PineCompiler.compile(source: source, libraries: PineLibraryRegistry.shared)
        guard compiled.isValid else { return nil }
        let instance = ChartScriptInstance(scriptID: scriptID, loadedRevisionID: revisionID, inputs: inputs)
        scriptInstances.append(instance)
        reevaluatePineInstance(instance.id, source: source, compiled: compiled)
        return instance.id
    }

    /// Stops and forgets one instance — the only way its state actually goes away. Hiding
    /// keeps everything; this discards it.
    func removePineInstance(_ id: UUID) {
        pineRuntimes[id]?.stop()
        pineRuntimes.removeValue(forKey: id)
        pineResults.removeValue(forKey: id)
        scriptInstances.removeAll { $0.id == id }
        pineInstanceRemovedHandler?(id)
        pineContextHandler?(pineAlertDataset, pineInstanceSourceHashes)
    }

    /// Suppresses rendering without discarding config or runtime state.
    func setPineInstanceVisible(_ id: UUID, isVisible: Bool) {
        guard let index = scriptInstances.firstIndex(where: { $0.id == id }) else { return }
        scriptInstances[index].isVisible = isVisible
        // No rebuild — rendering reads `isVisible` directly; the runtime keeps ticking.
    }

    /// Replaces one instance's inputs and rebuilds just that instance.
    func setPineInstanceInputs(_ id: UUID, inputs: [String: PineInputValue]) {
        guard let index = scriptInstances.firstIndex(where: { $0.id == id }) else { return }
        scriptInstances[index].inputs = inputs
        reevaluatePineInstance(id)
    }

    /// Simple reorder for the legend/settings list — not full drag-reorder.
    func movePineInstance(_ id: UUID, toIndex newIndex: Int) {
        guard let from = scriptInstances.firstIndex(where: { $0.id == id }) else { return }
        let instance = scriptInstances.remove(at: from)
        scriptInstances.insert(instance, at: min(max(newIndex, 0), scriptInstances.count))
    }

    /// Test-only visibility into one instance's generation, to assert an unrelated instance
    /// never rebuilds.
    func pineGeneration(forInstance id: UUID) -> Int? { pineRuntimes[id]?.generation }

    /// Hash of each applied instance's resolved source. An instance still resolving its script is
    /// absent, so a subscription is never judged against a source that is not known yet.
    var pineInstanceSourceHashes: [UUID: String] {
        var hashes: [UUID: String] = [:]
        for instance in scriptInstances {
            if let resolved = pineRuntimes[instance.id]?.resolvedSource, !resolved.isEmpty {
                hashes[instance.id] = ScriptSourceHash.sha256(resolved)
            }
        }
        return hashes
    }

    /// The source one applied instance is running, once resolved from the script library.
    func pineInstanceResolvedSource(_ id: UUID) -> String? {
        pineRuntimes[id]?.resolvedSource
    }

    /// Recalculates one instance from scratch: on an input, theme, symbol or timeframe change
    /// for it, or right after it's added. A fresh host means no state survives from the
    /// previous program or dataset.
    private func reevaluatePineInstance(
        _ id: UUID, source suppliedSource: String? = nil, compiled supplied: PineCompiledProgram? = nil
    ) {
        guard let instance = scriptInstances.first(where: { $0.id == id }) else { return }
        let runtime = pineRuntimes[id] ?? {
            let created = PineInstanceRuntime(instanceID: id)
            pineRuntimes[id] = created
            return created
        }()
        runtime.stop()
        runtime.generation += 1
        let generation = runtime.generation
        // Captured on the main actor: an input-only rebuild reuses the source already
        // resolved for this instance rather than re-touching ScriptStore every time.
        let cachedResolvedSource = runtime.resolvedSource

        let bars = replayKlines
        let live = replayTimestamp == nil
        let dataset = pineDataset(for: requestedRange ?? .oneDay)
        let inputs = instance.inputs
        let theme = pineTheme
        let symbol = PineSymbolInfo(
            ticker: ticker, tickerID: dataset.symbolKey,
            type: source == .alpaca ? "stock" : source.isPredictionMarket ? "prediction" : "crypto")
        let securityChart = PineSecurityTarget.Chart(
            tickerID: dataset.symbolKey, source: source, apiSymbol: apiSymbol)
        let (operations, feed) = AsyncStream.makeStream(of: PineFeedOperation.self)
        runtime.feed = feed
        runtime.dataset = dataset

        runtime.feedTask = Task.detached(priority: .userInitiated) { [weak self] in
            var compiled = supplied
            var host: PineExecutionHost?
            var liveSecurity: PineSecurityDataProvider?
            let resolvedSource: String
            if let suppliedSource {
                resolvedSource = suppliedSource
            } else if let legacy = instance.legacySource {
                resolvedSource = legacy
            } else if let cachedResolvedSource {
                resolvedSource = cachedResolvedSource
            } else if let resolved = try? await ScriptStore.shared.resolvedSource(
                scriptID: instance.scriptID, revisionID: instance.loadedRevisionID)
            {
                resolvedSource = resolved.source
                await self?.markPineInstanceUpdateStatus(id, isLatest: resolved.isLatest)
            } else {
                await self?.markPineInstanceScriptMissing(id, generation: generation)
                return
            }
            await self?.cachePineInstanceResolvedSource(id, resolvedSource)
            for await operation in operations {
                if Task.isCancelled { return }
                var outcome: PineExecutionOutcome
                switch operation {
                case .rebuild:
                    let program = compiled ?? PineCompiler.compile(source: resolvedSource, libraries: PineLibraryRegistry.shared)
                    compiled = program
                    guard program.isValid else {
                        await self?.applyPineInstance(id, program: program, outcome: nil, generation: generation)
                        host = nil
                        continue
                    }
                    liveSecurity = await PineSecurityFeed.prepare(
                        program: program, inputs: inputs, theme: theme, symbol: symbol, chart: securityChart,
                        bars: bars)
                    let fresh = PineExecutionHost(
                        program: program, dataset: dataset, inputs: inputs, theme: theme, symbol: symbol,
                        securityData: liveSecurity)
                    host = fresh
                    outcome = await fresh.rebuild(bars: bars, live: live)
                case .ingest(let update):
                    guard let host else { continue }
                    outcome = await host.ingest(update)
                case .sync(let snapshot):
                    guard let host else { continue }
                    // Intrabars for `request.security_lower_tf` follow the market with the chart's own refresh.
                    await (liveSecurity as? PineIntrabarSeries)?.refresh()
                    outcome = await host.sync(snapshot: snapshot)
                }
                guard let program = compiled else { continue }
                await self?.applyPineInstance(id, program: program, outcome: outcome, generation: generation)
            }
        }
        feed.yield(.rebuild)
    }

    private func cachePineInstanceResolvedSource(_ id: UUID, _ source: String) {
        pineRuntimes[id]?.resolvedSource = source
        pineContextHandler?(pineAlertDataset, pineInstanceSourceHashes)
    }

    private func markPineInstanceUpdateStatus(_ id: UUID, isLatest: Bool) {
        guard let index = scriptInstances.firstIndex(where: { $0.id == id }) else { return }
        scriptInstances[index].updateStatus = isLatest ? .current : .available
    }

    private func markPineInstanceScriptMissing(_ id: UUID, generation: Int) {
        guard pineRuntimes[id]?.generation == generation,
            let index = scriptInstances.firstIndex(where: { $0.id == id })
        else { return }
        scriptInstances[index].updateStatus = .missing
        pineResults[id] = PineInstanceResult(state: .failed("Script no longer exists"))
    }

    private func applyPineInstance(
        _ instanceID: UUID, program: PineCompiledProgram, outcome: PineExecutionOutcome?, generation: Int
    ) {
        guard pineRuntimes[instanceID]?.generation == generation else { return }
        guard let outcome else {
            var result = pineResults[instanceID] ?? PineInstanceResult()
            result.diagnostics = program.diagnostics
            result.state = .failed("Compile failed")
            pineResults[instanceID] = result
            return
        }
        switch outcome {
        case .updated(let update):
            var result = PineInstanceResult()
            result.output = update.output
            result.declaration = program.declaration
            result.inputSchema = program.inputSchema
            result.diagnostics = program.diagnostics
            result.state = .ready
            result.isLive = update.executions.contains { $0.isRealtime }
            result.alertCallCount = program.alertCallSites.count
            pineResults[instanceID] = result
            if !update.alerts.isEmpty { pineAlertHandler?(update.alerts, update.barID, instanceID) }
        case .unchanged:
            break
        case .needsRebuild:
            reevaluatePineInstance(instanceID, compiled: program)
        case .failed(let diagnostic):
            var result = pineResults[instanceID] ?? PineInstanceResult()
            result.diagnostics = [diagnostic]
            result.state = .failed("Runtime failed — last valid output remains active")
            pineResults[instanceID] = result
        }
    }

    /// Feeds one observation into every instance's own pipeline. History is fetched once by
    /// the chart; this is the fan-out point — each instance processes it independently.
    private func feedPineAllInstances(_ bar: KlineData, origin: PineMarketUpdate.Origin) {
        guard replayTimestamp == nil, let requestedRange else { return }
        let current = pineDataset(for: requestedRange)
        for instance in scriptInstances {
            guard pineRuntimes[instance.id]?.dataset == current else { continue }
            pineRuntimes[instance.id]?.feed?.yield(.ingest(PineMarketUpdate(bar: bar, origin: origin)))
        }
    }

    /// After a fetch: reconcile every running instance with the refreshed bars, or rebuild
    /// an instance whose feed can no longer be reconciled (symbol/timeframe changed under it).
    private func syncPineAllInstances(range: TimeRange) {
        let dataset = pineDataset(for: range)
        for instance in scriptInstances {
            guard replayTimestamp == nil, let runtime = pineRuntimes[instance.id], runtime.dataset == dataset else {
                reevaluatePineInstance(instance.id)
                continue
            }
            runtime.feed?.yield(.sync(klineData))
        }
    }

    /// Symbol/timeframe/theme/replay change: every instance is stale, all rebuild against the
    /// same new authoritative history.
    private func reevaluateAllPineInstances() {
        pineContextHandler?(pineAlertDataset, pineInstanceSourceHashes)
        for instance in scriptInstances { reevaluatePineInstance(instance.id) }
    }

    // MARK: - Pine instance rendering

    /// Every visible instance's latest output, in `scriptInstances` order — later instances
    /// composite on top, matching TradingView's "last added on top".
    var visiblePineOutputs: [PineVisualOutput] {
        scriptInstances.filter(\.isVisible).compactMap { pineResults[$0.id]?.output }
    }

    /// The subset that draws in its own pane under the candles (`overlay == false`).
    var panePineOutputs: [PineVisualOutput] { visiblePineOutputs.filter { !$0.overlay } }

    /// `overlay=false` instances get their own stacked panes; the line chart draws no scripts
    /// (unchanged rule).
    var showsPinePanes: Bool { !usesLineChart && !panePineOutputs.isEmpty }

    /// Shared budget for every stacked pane: 30% of `chartHeight`, or 60pt per pane if that's
    /// more — matches today's single-pane behavior exactly at one pane, and keeps panes
    /// legible at higher N rather than shrinking the candle area unboundedly.
    func pinePanesHeight(forChartHeight chartHeight: CGFloat) -> CGFloat {
        let count = panePineOutputs.count
        guard count > 0 else { return 0 }
        return max(chartHeight * 0.3, CGFloat(count) * 60).rounded()
    }

    /// One pane's height out of the shared budget.
    func pinePaneHeight(forChartHeight chartHeight: CGFloat, count: Int) -> CGFloat {
        guard count > 0 else { return 0 }
        return (pinePanesHeight(forChartHeight: chartHeight) / CGFloat(count)).rounded()
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
            yAxisDecimalPlaces: yAxisDecimalPlaces,
            // The candle chart draws overlay scripts and makes room for what they draw; the line chart draws none.
            scriptExtent: usesLineChart
                ? .none
                : PineChartLayer.overlayExtent(of: visiblePineOutputs, candles: visibleKlines, style: .default)
        )
    }

    /// Slots the candle chart reserves right of the last candle for what `outputs` draw there.
    func pineFutureSlots(of outputs: [PineVisualOutput]) -> Int {
        usesLineChart
            ? 0 : PineChartLayer.overlayExtent(of: outputs, candles: visibleKlines, style: .default).futureBars
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

    func snappedAnchor(at point: CGPoint, in plot: ChartPlot, strong: Bool) -> TrendAnchor {
        var result = anchor(at: point, in: plot)
        let points = replayKlines.suffix(visibleKlines.count)
        guard !points.isEmpty else { return result }
        let index = Int(
            plot.fractionalIndex(forX: point.x, slotWidth: plot.slotWidth(forCount: points.count)).rounded()
        ).clamped(to: 0...(points.count - 1))
        let candle = points[points.index(points.startIndex, offsetBy: index)]
        let candidates = [candle.openPrice, candle.highPrice, candle.lowPrice, candle.closePrice]
        guard let nearest = candidates.min(by: { abs(plot.y(for: $0) - point.y) < abs(plot.y(for: $1) - point.y) })
        else { return result }
        if strong || abs(plot.y(for: nearest) - point.y) <= Drawing.hitTolerance {
            result.date = candle.openTime
            result.price = nearest
        }
        return result
    }

    func fibonacciHandleHit(at point: CGPoint, in plot: ChartPlot) -> (id: UUID, isStart: Bool)? {
        let points = visibleKlines
        guard !points.isEmpty else { return nil }
        let slot = plot.slotWidth(forCount: points.count)
        for fib in fibonacciRetracements.reversed() where !fib.isHidden {
            for (anchor, isStart) in [(fib.point1, true), (fib.point2, false)] {
                let position = plot.position(of: anchor, points: points, slotWidth: slot)
                if hypot(position.x - point.x, position.y - point.y) <= Drawing.hitTolerance {
                    return (fib.id, isStart)
                }
            }
        }
        return nil
    }

    func fibonacciHit(at point: CGPoint, in plot: ChartPlot) -> UUID? {
        let points = visibleKlines
        guard !points.isEmpty else { return nil }
        let slot = plot.slotWidth(forCount: points.count)
        for fib in fibonacciRetracements.reversed() where !fib.isHidden {
            let first = plot.position(of: fib.point1, points: points, slotWidth: slot)
            let second = plot.position(of: fib.point2, points: points, slotWidth: slot)
            let left = fib.style.extendLeft ? plot.plotRect.minX : min(first.x, second.x)
            let right = fib.style.extendRight ? plot.plotRect.maxX : max(first.x, second.x)
            for (_, price) in FibonacciCalculator.prices(for: fib) {
                let y = plot.y(for: price)
                if point.x >= left - Drawing.hitTolerance, point.x <= right + Drawing.hitTolerance,
                    abs(point.y - y) <= Drawing.hitTolerance
                {
                    return fib.id
                }
            }
            if Self.distance(from: point, toSegmentFrom: first, to: second) <= Drawing.hitTolerance {
                return fib.id
            }
        }
        return nil
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
    private static func distance(from point: CGPoint, toSegmentFrom start: CGPoint, to end: CGPoint)
        -> CGFloat
    {
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

        let line = TrendLine(start: start, end: anchor)
        let index = trendLines.count
        trendLines.append(line)
        persistTrendLines()
        drawingUndoCoordinator?.recordLine(
            instrument: drawingInstrument, before: nil, beforeIndex: index, after: line,
            afterIndex: index, actionName: "Add Trend Line")
        draftStart = nil
        draftEnd = nil
        return true
    }

    func cancelDraft() {
        draftStart = nil
        draftEnd = nil
    }

    func beginFibonacciDraft(at anchor: TrendAnchor) {
        fibonacciDraftStart = anchor
        fibonacciDraftEnd = anchor
        selectedFibonacciID = nil
    }

    func updateFibonacciDraft(to anchor: TrendAnchor) {
        guard fibonacciDraftStart != nil else { return }
        fibonacciDraftEnd = anchor
    }

    @discardableResult
    func commitFibonacciDraft(at anchor: TrendAnchor, in plot: ChartPlot) -> Bool {
        guard let start = fibonacciDraftStart else { return false }
        let slot = plot.slotWidth(forCount: visibleKlines.count)
        let from = plot.position(of: start, points: visibleKlines, slotWidth: slot)
        let to = plot.position(of: anchor, points: visibleKlines, slotWidth: slot)
        guard hypot(to.x - from.x, to.y - from.y) >= Drawing.hitTolerance else { return false }
        let drawing = FibonacciRetracementDrawing(point1: start, point2: anchor)
        let index = fibonacciRetracements.count
        fibonacciRetracements.append(drawing)
        cancelFibonacciDraft()
        persistFibonacciRetracements()
        drawingUndoCoordinator?.recordFibonacci(
            instrument: drawingInstrument, before: nil, beforeIndex: index, after: drawing,
            afterIndex: index, actionName: "Add Fibonacci Retracement")
        return true
    }

    func cancelFibonacciDraft() {
        fibonacciDraftStart = nil
        fibonacciDraftEnd = nil
    }

    func moveFibonacciAnchor(id: UUID, isStart: Bool, to anchor: TrendAnchor) {
        guard let index = fibonacciRetracements.firstIndex(where: { $0.id == id }),
            !fibonacciRetracements[index].isLocked
        else { return }
        if isStart {
            fibonacciRetracements[index].point1 = anchor
        } else {
            fibonacciRetracements[index].point2 = anchor
        }
        fibonacciRetracements[index].updatedAt = Date()
    }

    func translateFibonacci(
        id: UUID, original: FibonacciRetracementDrawing, from start: TrendAnchor, to current: TrendAnchor
    ) {
        guard let index = fibonacciRetracements.firstIndex(where: { $0.id == id }), !original.isLocked else { return }
        let timeDelta = current.date.timeIntervalSince(start.date)
        let priceDelta = current.price - start.price
        fibonacciRetracements[index].point1 = TrendAnchor(
            date: original.point1.date.addingTimeInterval(timeDelta), price: original.point1.price + priceDelta)
        fibonacciRetracements[index].point2 = TrendAnchor(
            date: original.point2.date.addingTimeInterval(timeDelta), price: original.point2.price + priceDelta)
        fibonacciRetracements[index].updatedAt = Date()
    }

    func updateFibonacci(_ drawing: FibonacciRetracementDrawing, recordUndo: Bool = true) {
        guard drawing.levels.count <= FibonacciDefaults.maximumLevelCount,
            let index = fibonacciRetracements.firstIndex(where: { $0.id == drawing.id })
        else { return }
        let previous = fibonacciRetracements[index]
        fibonacciRetracements[index] = drawing
        persistFibonacciRetracements()
        if recordUndo {
            drawingUndoCoordinator?.recordFibonacci(
                instrument: drawingInstrument, before: previous, beforeIndex: index, after: drawing,
                afterIndex: index, actionName: "Edit Fibonacci Retracement")
        }
    }

    func persistFibonacciRetracements() {
        drawingStore.save(fibonacciRetracements, ticker: ticker, source: source)
    }

    func beginFibonacciSettingsEdit(id: UUID) {
        fibonacciSettingsOriginal = fibonacciRetracements.first { $0.id == id }
    }

    func endFibonacciSettingsEdit(id: UUID) {
        defer { fibonacciSettingsOriginal = nil }
        guard let before = fibonacciSettingsOriginal,
            let index = fibonacciRetracements.firstIndex(where: { $0.id == id })
        else { return }
        drawingUndoCoordinator?.recordFibonacci(
            instrument: drawingInstrument, before: before, beforeIndex: index,
            after: fibonacciRetracements[index], afterIndex: index,
            actionName: "Edit Fibonacci Retracement")
    }

    func commitFibonacciDrag(original: FibonacciRetracementDrawing, actionName: String) {
        guard let index = fibonacciRetracements.firstIndex(where: { $0.id == original.id }) else { return }
        persistFibonacciRetracements()
        drawingUndoCoordinator?.recordFibonacci(
            instrument: drawingInstrument, before: original, beforeIndex: index,
            after: fibonacciRetracements[index], afterIndex: index, actionName: actionName)
    }

    @discardableResult
    func removeSelectedFibonacci() -> Bool {
        guard let id = selectedFibonacciID else { return false }
        return removeFibonacci(id: id)
    }

    @discardableResult
    func removeFibonacci(id: UUID) -> Bool {
        guard let index = fibonacciRetracements.firstIndex(where: { $0.id == id }) else { return false }
        let drawing = fibonacciRetracements.remove(at: index)
        if selectedFibonacciID == id { selectedFibonacciID = nil }
        if editingFibonacciID == id { editingFibonacciID = nil }
        persistFibonacciRetracements()
        drawingUndoCoordinator?.recordFibonacci(
            instrument: drawingInstrument, before: drawing, beforeIndex: index, after: nil,
            afterIndex: index, actionName: "Delete Fibonacci Retracement")
        return true
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
        drawingStore.save(trendLines, ticker: ticker, source: source)
    }

    func commitTrendLineDrag(original: TrendLine) {
        guard let index = trendLines.firstIndex(where: { $0.id == original.id }) else { return }
        persistTrendLines()
        drawingUndoCoordinator?.recordLine(
            instrument: drawingInstrument, before: original, beforeIndex: index,
            after: trendLines[index], afterIndex: index, actionName: "Move Trend Line")
    }

    func setColor(_ color: TrendLineColor, for lineID: UUID) {
        guard let index = trendLines.firstIndex(where: { $0.id == lineID }) else { return }
        let previous = trendLines[index]
        trendLines[index].color = color
        persistTrendLines()
        drawingUndoCoordinator?.recordLine(
            instrument: drawingInstrument, before: previous, beforeIndex: index,
            after: trendLines[index], afterIndex: index, actionName: "Change Trend Line Color")
    }

    func setThickness(_ thickness: TrendLineThickness, for lineID: UUID) {
        guard let index = trendLines.firstIndex(where: { $0.id == lineID }) else { return }
        let previous = trendLines[index]
        trendLines[index].thickness = thickness
        persistTrendLines()
        drawingUndoCoordinator?.recordLine(
            instrument: drawingInstrument, before: previous, beforeIndex: index,
            after: trendLines[index], afterIndex: index, actionName: "Change Trend Line Thickness")
    }

    @discardableResult
    func removeLine(id: UUID) -> Bool {
        guard let index = trendLines.firstIndex(where: { $0.id == id }) else { return false }
        let line = trendLines.remove(at: index)
        if selectedLineID == id { selectedLineID = nil }
        if editingLineID == id { editingLineID = nil }
        persistTrendLines()
        drawingUndoCoordinator?.recordLine(
            instrument: drawingInstrument, before: line, beforeIndex: index, after: nil,
            afterIndex: index, actionName: "Delete Trend Line")
        return true
    }

    @discardableResult
    func removeSelectedLine() -> Bool {
        guard let id = selectedLineID else { return false }
        return removeLine(id: id)
    }

    // MARK: - Drawing a ruler

    /// First click, or the press of a drag: pin one corner. The rectangle tracks the
    /// pointer from here until it is finished.
    func beginRulerDraft(at anchor: TrendAnchor) {
        rulerDraftStart = anchor
        rulerDraftEnd = anchor
        selectedRulerID = nil
        hoveredRuler = nil
    }

    func updateRulerDraft(to anchor: TrendAnchor) {
        guard rulerDraftStart != nil else { return }
        rulerDraftEnd = anchor
    }

    /// Finishes the rectangle and selects it. Rejects an end that landed back on the first
    /// corner — a rectangle with no area would report 0% over one candle — and leaves the
    /// draft open so the next click can still finish it.
    @discardableResult
    func commitRulerDraft(at anchor: TrendAnchor, in plot: ChartPlot) -> Bool {
        guard let start = rulerDraftStart else { return false }
        let points = visibleKlines
        let slot = plot.slotWidth(forCount: points.count)
        let from = plot.position(of: start, points: points, slotWidth: slot)
        let to = plot.position(of: anchor, points: points, slotWidth: slot)
        guard hypot(to.x - from.x, to.y - from.y) >= Drawing.hitTolerance else { return false }

        let ruler = RulerRect(start: start, end: anchor)
        rulers.append(ruler)
        selectedRulerID = ruler.id
        rulerDraftStart = nil
        rulerDraftEnd = nil
        return true
    }

    func cancelRulerDraft() {
        rulerDraftStart = nil
        rulerDraftEnd = nil
    }

    // MARK: - Editing a ruler

    /// The ruler part under `point`: a corner within reach first, then an edge. Newest
    /// first, matching what the renderer draws on top. The interior is not a hit, so a
    /// measurement can start inside an existing one.
    func rulerHit(at point: CGPoint, in plot: ChartPlot) -> RulerHit? {
        let points = visibleKlines
        guard !points.isEmpty else { return nil }
        let slot = plot.slotWidth(forCount: points.count)

        for ruler in rulers.reversed() {
            let from = plot.position(of: ruler.start, points: points, slotWidth: slot)
            let to = plot.position(of: ruler.end, points: points, slotWidth: slot)
            let box = CGRect(
                x: min(from.x, to.x), y: min(from.y, to.y),
                width: abs(to.x - from.x), height: abs(to.y - from.y))

            for corner in RulerCorner.allCases {
                let center = CGPoint(
                    x: corner.isLeft ? box.minX : box.maxX,
                    y: corner.isTop ? box.minY : box.maxY)
                if hypot(center.x - point.x, center.y - point.y) <= Drawing.rulerCornerReach {
                    return RulerHit(id: ruler.id, part: .corner(corner))
                }
            }

            let band = Drawing.rulerEdgeBand
            let outer = box.insetBy(dx: -band, dy: -band)
            let inner = box.insetBy(dx: band, dy: band)
            if outer.contains(point), !inner.contains(point) {
                return RulerHit(id: ruler.id, part: .edge)
            }
        }
        return nil
    }

    /// Only publishes a change, so a pointer wandering over empty chart doesn't redraw
    /// the canvas on every mouse move.
    func setRulerHover(_ hit: RulerHit?) {
        guard hoveredRuler != hit else { return }
        hoveredRuler = hit
    }

    func selectRuler(_ id: UUID?) {
        guard selectedRulerID != id else { return }
        selectedRulerID = id
    }

    /// Drags `corner` of `original` to `anchor`. Always computed from the rectangle as it
    /// was when the drag began, so crossing over the opposite corner and coming back
    /// restores it exactly.
    func moveRulerCorner(original: RulerRect, corner: RulerCorner, to anchor: TrendAnchor) {
        replaceRuler(original.resized(corner: corner, to: anchor))
    }

    /// Moves the whole rectangle by how far the pointer has travelled since the drag began.
    func translateRuler(original: RulerRect, from start: TrendAnchor, to current: TrendAnchor) {
        replaceRuler(
            original.translated(
                by: current.date.timeIntervalSince(start.date),
                price: current.price - start.price))
    }

    private func replaceRuler(_ ruler: RulerRect) {
        guard let index = rulers.firstIndex(where: { $0.id == ruler.id }) else { return }
        rulers[index] = ruler
    }

    @discardableResult
    func removeSelectedRuler() -> Bool {
        guard let id = selectedRulerID, rulers.contains(where: { $0.id == id }) else { return false }
        rulers.removeAll { $0.id == id }
        selectedRulerID = nil
        hoveredRuler = nil
        return true
    }

    /// Put every measurement on this chart away. Reports whether there was anything to
    /// clear, so Esc and Delete can tell a dismissal from a step out of the tool.
    @discardableResult
    func clearRulers() -> Bool {
        selectedRulerID = nil
        hoveredRuler = nil
        guard !rulers.isEmpty else { return false }
        rulers.removeAll()
        return true
    }

    /// Point this chart at another market. The one safe in-place switch: everything tied to the old
    /// market is cancelled or cleared, so nothing it started can land on the new one, while the
    /// chart's own identity (`chartID`, its place in the grid), timeframe, colours and
    /// indicators stay. The caller refetches.
    ///
    /// Drawings are not carried over: they belong to a market in `DrawingStore`, so the new market
    /// shows its own and the old market's come back when the chart returns to it.
    func updateTicker(
        symbol: String, source: DataSourceType, displayName: String? = nil,
        pmSeries: [PmSeriesConfig]? = nil
    ) {
        // A response or refresh the old market started finds its generation gone and is dropped.
        fetchTask?.cancel()
        fetchTask = nil
        fetchGeneration += 1
        isFetching = false
        errorMessage = nil
        lastUpdated = nil
        clearGranularReplayData()
        ticker = symbol
        self.source = source
        self.displayName = displayName
        coinSymbol = nil
        self.pmSeries = pmSeries ?? []
        self.pmSeriesData = [:]
        klineData = []
        currentPrice = nil
        liveQuote.clear()
        // A new instrument shares no series with the old one; the next fetch rebuilds the script.
        stopPineFeed()
        pineDataset = nil
        pineOutput = .empty
        pineResults = [:]
        for runtime in pineRuntimes.values { runtime.stop() }
        api = serviceResolver(source)
        trendLines = drawingStore.lines(ticker: ticker, source: source)
        fibonacciRetracements = drawingStore.fibs(ticker: ticker, source: source)
        brushes = drawingStore.brushes(ticker: ticker, source: source)
        resetTransientDrawingState()
        applyCapabilityFallback()
    }

    /// Selections, drafts and measurements belong to the market they were made on.
    private func resetTransientDrawingState() {
        cancelDraft()
        cancelFibonacciDraft()
        resetBrushState()
        selectedLineID = nil
        editingLineID = nil
        selectedFibonacciID = nil
        editingFibonacciID = nil
        clearRulers()
    }

    /// Settings that mean nothing for the new market are switched off, never reinterpreted: volume
    /// bars need a source that reports turnover, and a fixed price precision chosen for one asset
    /// would misread another, so it returns to automatic.
    private func applyCapabilityFallback() {
        if usesLineChart || !source.providesVolume { showVolume = false }
        yAxisDecimalPlaces = nil
    }

    private func observeDrawings() {
        drawingStoreSubscription = drawingStore.$linesByInstrument
            .sink { [weak self] allLines in
                guard let self else { return }
                let key = self.drawingStore.key(ticker: self.ticker, source: self.source)
                let sharedLines = allLines[key] ?? []
                if self.trendLines != sharedLines { self.trendLines = sharedLines }
                if let id = self.selectedLineID, !sharedLines.contains(where: { $0.id == id }) {
                    self.selectedLineID = nil
                    self.editingLineID = nil
                }
            }
        fibonacciStoreSubscription = drawingStore.$fibsByInstrument
            .sink { [weak self] allFibs in
                guard let self else { return }
                let key = self.drawingStore.key(ticker: self.ticker, source: self.source)
                let sharedFibs = allFibs[key] ?? []
                if self.fibonacciRetracements != sharedFibs { self.fibonacciRetracements = sharedFibs }
                if let id = self.selectedFibonacciID, !sharedFibs.contains(where: { $0.id == id }) {
                    self.selectedFibonacciID = nil
                    self.editingFibonacciID = nil
                }
            }
        brushStoreSubscription = drawingStore.$brushesByInstrument
            .sink { [weak self] allBrushes in
                guard let self else { return }
                let key = self.drawingStore.key(ticker: self.ticker, source: self.source)
                let sharedBrushes = allBrushes[key] ?? []
                if self.brushes != sharedBrushes { self.brushes = sharedBrushes }
                if let id = self.selectedBrushID, !sharedBrushes.contains(where: { $0.id == id }) {
                    self.selectedBrushID = nil
                    self.editingBrushID = nil
                }
                if let id = self.hoveredBrushID, !sharedBrushes.contains(where: { $0.id == id }) {
                    self.hoveredBrushID = nil
                }
            }
    }

    var drawingInstrument: String {
        drawingStore.key(ticker: ticker, source: source)
    }

    /// Merge a WebSocket kline tick into the current dataset.
    /// - Updates the in-progress (rightmost) candle in-place when the openTime matches.
    /// - Appends a candle that opens after the last one; the REST refresh later reconciles the buffer.
    /// - Forwards the raw tick to the Pine pipeline, which owns bar identity and the close flag: the
    ///   REST refresh replaces `klineData`, so `isClosed` cannot be read back from it.
    func applyKlineUpdate(_ kline: KlineData) {
        guard let last = klineData.last else { return }

        if last.openTime == kline.openTime {
            klineData[klineData.count - 1].closePrice = kline.closePrice
            klineData[klineData.count - 1].highPrice = kline.highPrice
            klineData[klineData.count - 1].lowPrice = kline.lowPrice
            klineData[klineData.count - 1].volume = kline.volume
            klineData[klineData.count - 1].quoteVolume = kline.quoteVolume
            klineData[klineData.count - 1].isClosed = kline.isClosed
        } else if last.openTime < kline.openTime {
            // Preserve the close/new-bar transition immediately; the REST refresh later
            // reconciles the buffer but no live execution is lost in the meantime.
            klineData.append(kline)
        }

        if currentPrice != kline.closePrice {
            currentPrice = kline.closePrice
        }
        feedPine(kline, origin: .stream)
        feedPineAllInstances(kline, origin: .stream)
    }

    /// Fold a completed lower-resolution live bar into the current displayed candle.
    /// Alpaca's free stream emits minute bars even when the chart is hourly or daily.
    func applyLiveBar(_ bar: KlineData, candleDuration: TimeInterval) {
        guard !klineData.isEmpty else { return }
        let index = klineData.count - 1
        let last = klineData[index]
        guard bar.openTime >= last.openTime,
            bar.openTime < last.openTime.addingTimeInterval(candleDuration)
        else { return }

        klineData[index].highPrice = max(last.highPrice, bar.highPrice)
        klineData[index].lowPrice = min(last.lowPrice, bar.lowPrice)
        klineData[index].closePrice = bar.closePrice
        klineData[index].volume += bar.volume
        klineData[index].quoteVolume += bar.quoteVolume
        currentPrice = bar.closePrice
        feedPine(klineData[index], origin: .stream)
        feedPineAllInstances(klineData[index], origin: .stream)
    }

    /// Fold one Coinbase trade into the live candle.
    ///
    /// Coinbase has no candle stream, so the candle is built here: the REST fetch supplies the
    /// open and the history, each trade moves the high, low and close, and a trade past the
    /// candle's end opens the next one. The REST refresh replaces the buffer every few seconds,
    /// so volume summed from trades only has to hold until then.
    func applyTick(_ tick: CoinbaseTick, plan: CoinbaseGranularity) {
        liveQuote.apply(bid: tick.bestBid, ask: tick.bestAsk)
        guard let last = klineData.last else { return }

        let bucket = plan.bucketStart(of: tick.time)
        guard bucket >= last.openTime else { return }

        let notional = tick.price * tick.size
        let index = klineData.count - 1
        if bucket == last.openTime {
            klineData[index].highPrice = max(last.highPrice, tick.price)
            klineData[index].lowPrice = min(last.lowPrice, tick.price)
            klineData[index].closePrice = tick.price
            klineData[index].volume += tick.size
            klineData[index].quoteVolume += notional
            feedPine(klineData[index], origin: .stream)
            feedPineAllInstances(klineData[index], origin: .stream)
        } else {
            // Same hand-off Binance's closing kline makes: the finished candle is flagged
            // before the first trade of the next one arrives.
            klineData[index].isClosed = true
            feedPine(klineData[index], origin: .stream)
            feedPineAllInstances(klineData[index], origin: .stream)
            let opened = KlineData(
                openTime: bucket, openPrice: tick.price, highPrice: tick.price, lowPrice: tick.price,
                closePrice: tick.price, volume: tick.size, quoteVolume: notional)
            klineData.append(opened)
            feedPine(opened, origin: .stream)
            feedPineAllInstances(opened, origin: .stream)
        }

        if currentPrice != tick.price {
            currentPrice = tick.price
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
        if showTrendFlips { needed = Swift.max(needed, Indicator.supertrendPeriod) }
        return needed
    }

    func fetchData(for range: TimeRange, count: Int, silent: Bool = false) async {
        guard !isPortfolioChart else { return }
        if isBitcoinPowerLaw { return }
        if coinMarketCapChart != nil {
            await fetchCoinMarketCap(force: !silent)
            return
        }
        // Silent refresh: if a fetch is already running let it finish.
        if silent, isFetching {
            return
        }

        // A prediction market draws a price line, not candles: its 3M and 1Y stay three months and
        // a year of points however few quarterly or yearly candles the tab asks the others for.
        let shownCount = source.isPredictionMarket ? Swift.max(count, range.lineChartPointCount) : count
        visibleCount = Swift.max(1, shownCount)
        requestedRange = range
        let count = fetchCount(for: shownCount)

        fetchTask?.cancel()
        fetchGeneration += 1
        let generation = fetchGeneration

        errorMessage = nil
        isFetching = true

        let hadData = !klineData.isEmpty
        let isSlowSource = source != .binance && source != .coinbase

        // Cache-first for slow sources: show stale data instantly.
        if !hadData, isSlowSource {
            if let cached = await api.getCachedKlines(
                symbol: apiSymbol, interval: range.binanceInterval, count: count)
            {
                klineData = cached
                currentPrice = cached.last?.closePrice
            }
        }

        let task = Task { [weak self] in
            guard let self else { return }
            defer {
                if fetchGeneration == generation {
                    isFetching = false
                }
            }

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
                } else if let pmService = api as? PredictionMarketDataSource {
                    // Prediction markets need the whole TimeRange, not the interval token:
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
                            marketID: tokenID,
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

        }
        fetchTask = task

        await withTaskCancellationHandler {
            await task.value
        } onCancel: {
            task.cancel()
        }

        if fetchGeneration == generation {
            fetchTask = nil
            syncPine(range: range)
            syncPineAllInstances(range: range)
        }
    }

    func fetchPowerLaw(force: Bool = false) async {
        guard isBitcoinPowerLaw else { return }
        if let cached = await BitcoinHistoryService.shared.cachedHistory() {
            powerLawHistory = cached.closes
        }
        isFetching = powerLawHistory.isEmpty
        errorMessage = nil
        powerLawWarning = nil
        do {
            let refreshed = try await BitcoinHistoryService.shared.refresh(force: force)
            powerLawHistory = refreshed.closes
            lastUpdated = refreshed.fetchedAt
            if Date().timeIntervalSince(refreshed.fetchedAt) >= BitcoinHistoryService.cacheLifetime {
                powerLawWarning = "Showing cached data; refresh failed."
            }
        } catch {
            if powerLawHistory.isEmpty {
                errorMessage = error.localizedDescription
            } else {
                powerLawWarning = "Showing cached data; refresh failed: \(error.localizedDescription)"
            }
        }
        isFetching = false
    }

    func fetchCoinMarketCap(force: Bool = false) async {
        guard let config = coinMarketCapChart else { return }
        fetchTask?.cancel()
        fetchGeneration += 1
        let generation = fetchGeneration
        errorMessage = nil
        isFetching = true
        let task = Task { [weak self] in
            guard let self else { return }
            defer { if self.fetchGeneration == generation { self.isFetching = false } }
            do {
                switch config.type {
                case .altcoinSeasonLatest:
                    let value = try await CoinMarketCapDataProvider.shared.altcoinLatest(force: force)
                    guard self.fetchGeneration == generation else { return }
                    self.cmcAltcoinLatest = value
                    self.lastUpdated = CMCDateParser.parse(value.snapshotTime)
                case .altcoinSeasonHistorical:
                    let value = try await CoinMarketCapDataProvider.shared.altcoinHistorical(
                        config.altcoinRange, force: force)
                    guard self.fetchGeneration == generation,
                        self.coinMarketCapChart?.altcoinRange == config.altcoinRange
                    else { return }
                    self.cmcAltcoinHistory = value.points.sorted {
                        (CMCDateParser.parse($0.timestamp) ?? .distantPast)
                            < (CMCDateParser.parse($1.timestamp) ?? .distantPast)
                    }
                    self.lastUpdated = self.cmcAltcoinHistory.last.flatMap { CMCDateParser.parse($0.timestamp) }
                case .fearAndGreedLatest:
                    let value = try await CoinMarketCapDataProvider.shared.fearGreedLatest(force: force)
                    guard self.fetchGeneration == generation else { return }
                    self.cmcFearGreedLatest = value
                    self.lastUpdated = CMCDateParser.parse(value.updateTime)
                case .fearAndGreedHistorical:
                    let value = try await CoinMarketCapDataProvider.shared.fearGreedHistorical(
                        config.fearGreedRange, force: force)
                    guard self.fetchGeneration == generation,
                        self.coinMarketCapChart?.fearGreedRange == config.fearGreedRange
                    else { return }
                    self.cmcFearGreedHistory = value
                    self.lastUpdated = value.last.flatMap { CMCDateParser.parse($0.timestamp) }
                }
            } catch is CancellationError { return } catch {
                if self.fetchGeneration == generation { self.errorMessage = error.localizedDescription }
            }
        }
        fetchTask = task
        await task.value
    }

    func updateCMCConfig(_ update: (inout CoinMarketCapChartConfig) -> Void) {
        guard var config = coinMarketCapChart else { return }
        update(&config)
        coinMarketCapChart = config
        Task { await fetchCoinMarketCap(force: false) }
    }

    /// Fetch all prediction-market series in parallel and update `pmSeriesData`.
    ///
    /// Individual series failures are silently ignored — the chart shows whatever
    /// data arrived. Sets `lastUpdated` on any partial or full success.
    private func fetchPmMultiSeries(
        pmService: PredictionMarketDataSource,
        range: TimeRange,
        count: Int,
        generation: Int
    ) async throws {
        let series = pmSeries
        let results: [(String, [KlineData])] = await withTaskGroup(of: (String, [KlineData]).self) {
            group in
            for s in series {
                group.addTask {
                    let data =
                        (try? await pmService.fetchPrices(marketID: s.tokenID, range: range, count: count)) ?? []
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
