import AppKit
import Foundation

/// Drives the Script Manager's preview chart: one `ChartViewModel` that runs whichever script is
/// being edited against a live market, plus the market, timeframe and input values the user tries.
///
/// It stands in for the parts of `ContentViewModel` a bare `ChartViewModel` lacks — the fetch,
/// the five-second refresh, the live streams and the hidden-window suspension — and never attaches
/// the alert coordinator, so a preview cannot raise alerts.
@MainActor
final class ScriptPreviewViewModel: ObservableObject {
    let chart: ChartViewModel

    @Published private(set) var market: PreviewMarket
    @Published private(set) var recents: [PreviewMarket]
    @Published private(set) var timeRange: TimeRange
    /// Candles on screen; scroll-zoom and the header's zoom buttons change it.
    @Published private(set) var candleCount: Int
    @Published private(set) var inputSchema = PineInputSchema()
    /// Overrides the user has set, by input id. Anything absent runs at its default.
    @Published private(set) var inputValues: [String: PineInputValue] = [:]
    /// Why the script on screen can't run on a chart, when it can't.
    @Published private(set) var unsupportedReason: String?
    /// The latest source didn't compile, so the chart still shows the last one that did.
    @Published private(set) var isShowingStaleOutput = false

    private let defaults: UserDefaults
    private let inputsStore: ScriptPreviewInputsStore
    private let feed: ChartLiveFeed
    private let refreshInterval: TimeInterval
    private let applyDelay: Duration
    private let scrollZoom: ScrollZoomMonitor
    private let axisDrag = PriceAxisDragMonitor()

    private var scriptID: UUID?
    private var latestSource: String?
    private var appliedSource: String?
    private var applyTask: Task<Void, Never>?
    private var persistTask: Task<Void, Never>?
    private var fetchTask: Task<Void, Never>?
    private var zoomTask: Task<Void, Never>?
    private var refreshTimer: Timer?
    private var observers: [NSObjectProtocol] = []
    private var isWindowVisible = true
    private var isPaneVisible = false

    static let marketKey = "scriptPreview.market"
    static let recentsKey = "scriptPreview.recents"
    static let timeRangeKey = "scriptPreview.timeRange"
    static let candleCountKey = "scriptPreview.candleCount"
    static let yZoomKey = "scriptPreview.yZoom"
    static let recentsLimit = 5

    init(
        chart: ChartViewModel? = nil,
        defaults: UserDefaults = .standard,
        inputsStore: ScriptPreviewInputsStore? = nil,
        feed: ChartLiveFeed? = nil,
        refreshInterval: TimeInterval = Timeout.autoRefresh,
        applyDelay: Duration = .milliseconds(350)
    ) {
        self.defaults = defaults
        self.inputsStore = inputsStore ?? .shared
        self.feed = feed ?? ChartLiveFeed()
        self.refreshInterval = refreshInterval
        self.applyDelay = applyDelay
        self.scrollZoom = ScrollZoomMonitor()

        let saved = Self.load(PreviewMarket.self, key: Self.marketKey, from: defaults)
        let market = saved.flatMap { PreviewMarket.isSupported($0.source) ? $0 : nil } ?? .default
        let range = defaults.string(forKey: Self.timeRangeKey).flatMap(TimeRange.init(rawValue:)) ?? .oneDay
        self.market = market
        self.timeRange = range
        self.candleCount =
            (defaults.object(forKey: Self.candleCountKey) as? Int)?
            .clamped(to: Candle.minCandles...Candle.maxCandles) ?? range.dataPointLimit
        self.recents =
            (Self.load([PreviewMarket].self, key: Self.recentsKey, from: defaults) ?? [])
            .filter { PreviewMarket.isSupported($0.source) }
        self.chart = chart ?? ChartViewModel(ticker: market.ticker, source: market.source)
        self.chart.setVisibleCount(candleCount)
        if let yZoom = defaults.object(forKey: Self.yZoomKey) as? Double, yZoom.isFinite {
            self.chart.yZoom = yZoom.clamped(to: PriceZoom.minFactor...PriceZoom.maxFactor)
        }

        scrollZoom.onScroll = { [weak self] steps in self?.zoom(steps: steps) }
        axisDrag.onChangeEnded = { [weak self] in
            guard let self else { return }
            defaults.set(self.chart.yZoom, forKey: Self.yZoomKey)
        }
    }

    // MARK: - Script

    /// Points the preview at another script. Its saved inputs load now; its source arrives through
    /// `sourceChanged`, once the editor has read it. Called again when the same script's type
    /// changes, which can turn it into (or out of) a library.
    func bind(scriptID: UUID, type: ScriptType) {
        let reason = type == .library ? "Libraries export functions — they don't draw on a chart." : nil
        guard scriptID != self.scriptID else {
            setUnsupportedReason(reason)
            return
        }
        applyTask?.cancel()
        persistTask?.cancel()
        self.scriptID = scriptID
        latestSource = nil
        appliedSource = nil
        isShowingStaleOutput = false
        inputSchema = PineInputSchema()
        inputValues = inputsStore.inputs(for: scriptID)
        chart.unloadPineScript()
        setUnsupportedReason(reason)
    }

    private func setUnsupportedReason(_ reason: String?) {
        guard reason != unsupportedReason else {
            refreshActivity()
            return
        }
        unsupportedReason = reason
        if reason != nil {
            applyTask?.cancel()
            appliedSource = nil
            isShowingStaleOutput = false
            inputSchema = PineInputSchema()
            chart.unloadPineScript()
        } else if let source = latestSource {
            sourceChanged(source)
        }
        refreshActivity()
    }

    /// The editor's text changed. Applies it once typing pauses, if it compiles.
    func sourceChanged(_ source: String) {
        latestSource = source
        guard unsupportedReason == nil, !source.isEmpty else { return }
        applyTask?.cancel()
        // Back to the text the chart already shows (an error undone): nothing is stale any more.
        if source == appliedSource {
            isShowingStaleOutput = false
            return
        }
        let delay = applyDelay
        applyTask = Task { [weak self] in
            // The first source after opening a script applies at once; later edits wait for a pause.
            if self?.appliedSource != nil { try? await Task.sleep(for: delay) }
            guard !Task.isCancelled else { return }
            self?.applyLatest()
        }
    }

    private func applyLatest() {
        guard let source = latestSource, source != appliedSource else { return }
        let compiled = PineCompiler.compile(source: source, libraries: PineLibraryRegistry.shared)
        guard compiled.isValid else {
            // The engine keeps the last valid output on screen; say so.
            isShowingStaleOutput = appliedSource != nil
            return
        }
        isShowingStaleOutput = false
        appliedSource = source
        inputSchema = compiled.inputSchema
        let pruned = Self.pruned(inputValues, to: compiled.inputSchema)
        if pruned != inputValues {
            inputValues = pruned
            persistInputs()
        }
        chart.loadPineScript(source: source, inputs: inputValues)
    }

    /// Keeps the overrides that still name an input of the same kind, so a rename or a type change
    /// in the source falls back to the default instead of feeding the script a mismatched value.
    static func pruned(_ values: [String: PineInputValue], to schema: PineInputSchema) -> [String: PineInputValue] {
        let definitions = Dictionary(schema.inputs.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
        return values.filter { id, value in
            guard let definition = definitions[id] else { return false }
            return sameKind(value, definition.defaultValue)
        }
    }

    private static func sameKind(_ lhs: PineInputValue, _ rhs: PineInputValue) -> Bool {
        switch (lhs, rhs) {
        case (.int, .int), (.float, .float), (.bool, .bool), (.string, .string), (.color, .color),
            (.source, .source):
            return true
        default:
            return false
        }
    }

    // MARK: - Inputs

    func setInput(_ value: PineInputValue, id: String) {
        inputValues[id] = value
        chart.setPineInput(value, id: id)
        persistInputs()
    }

    func resetInputs() {
        guard !inputValues.isEmpty else { return }
        inputValues = [:]
        persistInputs()
        guard let source = appliedSource else { return }
        chart.loadPineScript(source: source, inputs: [:])
    }

    private func persistInputs() {
        guard let scriptID else { return }
        persistTask?.cancel()
        persistTask = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(500))
            guard !Task.isCancelled, let self else { return }
            inputsStore.setInputs(inputValues, for: scriptID)
        }
    }

    /// Forgets saved inputs of scripts that were deleted.
    func prune(keeping scriptIDs: Set<UUID>) {
        inputsStore.prune(keeping: scriptIDs)
    }

    // MARK: - Market

    func selectMarket(_ newMarket: PreviewMarket) {
        guard PreviewMarket.isSupported(newMarket.source), newMarket != market else { return }
        market = newMarket
        recents = Array(([newMarket] + recents.filter { $0 != newMarket }).prefix(Self.recentsLimit))
        Self.save(market, key: Self.marketKey, to: defaults)
        Self.save(recents, key: Self.recentsKey, to: defaults)
        chart.updateTicker(symbol: newMarket.ticker, source: newMarket.source)
        reload()
    }

    func setTimeRange(_ range: TimeRange) {
        guard range != timeRange else { return }
        timeRange = range
        candleCount = range.dataPointLimit
        chart.setVisibleCount(candleCount)
        defaults.set(range.rawValue, forKey: Self.timeRangeKey)
        defaults.set(candleCount, forKey: Self.candleCountKey)
        reload()
    }

    func retry() { reload() }

    // MARK: - Zoom

    /// How many candles one zoom step adds or removes.
    var zoomStep: Int { max(1, Int(Double(candleCount) * Candle.zoomStepFraction)) }

    /// Positive steps zoom in (fewer candles), negative zoom out. Clamped to the chart's limits.
    func zoom(steps: Int) {
        let newCount = (candleCount - steps * zoomStep).clamped(to: Candle.minCandles...Candle.maxCandles)
        guard newCount != candleCount else { return }
        candleCount = newCount
        defaults.set(newCount, forKey: Self.candleCountKey)
        // Redraw from the buffer straight away; the refetch only tops it back up.
        chart.setVisibleCount(newCount)
        zoomTask?.cancel()
        zoomTask = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(150))
            guard !Task.isCancelled, let self, isActive else { return }
            await chart.fetchData(for: timeRange, count: candleCount, silent: true)
        }
    }

    // MARK: - Lifecycle

    /// Whether anything should be running: the preview is on screen, in a window the user can see,
    /// with a script that can run.
    var isActive: Bool { isWindowVisible && isPaneVisible && unsupportedReason == nil }

    /// The preview pane appeared or went away (hidden by the user, or no script selected).
    func setPaneVisible(_ visible: Bool) {
        guard visible != isPaneVisible else { return }
        isPaneVisible = visible
        refreshActivity()
    }

    /// A script's workspace left the screen. Ignored once another script's workspace has bound —
    /// SwiftUI may run the new workspace's `onAppear` before the old one's `onDisappear`, and
    /// hiding the pane then would leave the preview stopped with nothing to restart it.
    func paneDisappeared(scriptID: UUID) {
        guard scriptID == self.scriptID else { return }
        setPaneVisible(false)
    }

    /// Follows the hosting window's occlusion, like a chart tab: a hidden Script Manager stops
    /// polling and closes its streams, and a closed one releases its monitors.
    func attach(to window: NSWindow) {
        scrollZoom.window = window
        axisDrag.window = window
        axisDrag.install()
        observers.forEach(NotificationCenter.default.removeObserver)
        observers = [
            NotificationCenter.default.addObserver(
                forName: NSWindow.didChangeOcclusionStateNotification, object: window, queue: .main
            ) { [weak self, weak window] _ in
                guard let window else { return }
                let visible = window.occlusionState.contains(.visible)
                Task { @MainActor [weak self] in self?.setWindowVisible(visible) }
            },
            NotificationCenter.default.addObserver(
                forName: NSWindow.willCloseNotification, object: window, queue: .main
            ) { [weak self] _ in
                Task { @MainActor [weak self] in self?.setWindowVisible(false) }
            },
        ]
        setWindowVisible(window.occlusionState.contains(.visible))
    }

    /// The region of the pane that scroll-wheel zoom responds to.
    func registerZoomRegion(_ view: NSView) {
        scrollZoom.region = view
    }

    /// The price-axis gutter beside the candles: dragging it scales them, like on a chart tab.
    func registerAxisRegion(_ view: NSView) {
        axisDrag.register(view, for: chart)
    }

    func setWindowVisible(_ visible: Bool) {
        guard visible != isWindowVisible else { return }
        isWindowVisible = visible
        refreshActivity()
    }

    private func refreshActivity() {
        guard isActive else {
            stopRefreshing()
            return
        }
        // Already running: a script switch or a layout change needs no new fetch.
        guard refreshTimer == nil else { return }
        startRefreshing()
        reload()
    }

    private func reload() {
        guard isActive else { return }
        fetchTask?.cancel()
        let range = timeRange
        let count = candleCount
        fetchTask = Task { [weak self] in
            guard let self else { return }
            await chart.fetchData(for: range, count: count)
        }
        feed.update(charts: [chart], range: range, active: true)
    }

    private func startRefreshing() {
        guard refreshTimer == nil else { return }
        refreshTimer = Timer.scheduledTimer(withTimeInterval: refreshInterval, repeats: true) { [weak self] _ in
            Task { @MainActor [weak self] in
                guard let self, isActive else { return }
                await chart.fetchData(for: timeRange, count: candleCount, silent: true)
            }
        }
    }

    private func stopRefreshing() {
        refreshTimer?.invalidate()
        refreshTimer = nil
        fetchTask?.cancel()
        zoomTask?.cancel()
        feed.disconnect()
    }

    // MARK: - Defaults

    private static func load<T: Decodable>(_ type: T.Type, key: String, from defaults: UserDefaults) -> T? {
        defaults.data(forKey: key).flatMap { try? JSONDecoder().decode(type, from: $0) }
    }

    private static func save<T: Encodable>(_ value: T, key: String, to defaults: UserDefaults) {
        defaults.set(try? JSONEncoder().encode(value), forKey: key)
    }
}
