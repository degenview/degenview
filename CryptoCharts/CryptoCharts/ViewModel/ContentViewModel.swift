import Foundation
import SwiftUI
import AppKit

enum LayoutMode: String, CaseIterable, Codable {
    case vertical
    case grid

    var icon: String {
        switch self {
        case .vertical: return "rectangle.stack"
        case .grid:     return "rectangle.grid.1x2"
        }
    }

    var next: LayoutMode {
        switch self {
        case .vertical: return .grid
        case .grid:     return .vertical
        }
    }
}

@MainActor
final class ContentViewModel: ObservableObject {
    @Published var chartViewModels: [ChartViewModel] = []
    @Published var selectedTimeRange: TimeRange = .oneDay {
        didSet {
            // Reset candle count to default when timeframe changes
            if oldValue != selectedTimeRange {
                candleCount = selectedTimeRange.dataPointLimit
            }
        }
    }
    @Published var candleCount: Int = TimeRange.oneDay.dataPointLimit
    @Published var layoutMode: LayoutMode = .vertical {
        didSet { markChanged() }
    }
    @Published var isRefreshing = false

    @Published var savedViews: [SavedView] = []
    @Published var currentViewName = UI.unnamedView
    @Published var hasUnsavedChanges = false

    private var currentViewID: UUID?
    private var isApplyingView = false

    private let api: BinanceAPIService
    private let tickerStore = JSONStore<[TickerConfig]>(filename: "tickers.json")
    private let viewStore = JSONStore<[SavedView]>(filename: "views.json")
    private var refreshTimer: Timer?
    private let wsService = BinanceWebSocketService()

    private var scrollMonitor: Any?

    init(api: BinanceAPIService = BinanceAPIService()) {
        self.api = api
        self.savedViews = viewStore.load() ?? []

        scrollMonitor = NSEvent.addLocalMonitorForEvents(matching: .scrollWheel) { [weak self] event in
            guard let self, !self.chartViewModels.isEmpty else { return event }
            let step = max(1, Int(Double(self.candleCount) * Candle.zoomStepFraction))
            if event.scrollingDeltaY > 0 {
                self.adjustCandleCount(by: step)
            } else if event.scrollingDeltaY < 0 {
                self.adjustCandleCount(by: -step)
            }
            return event
        }
    }

    deinit {
        if let m = scrollMonitor { NSEvent.removeMonitor(m) }
    }

    /// Load persisted tickers, create chart view models, and start auto-refresh.
    func loadTickers() {
        let configs = tickerStore.load() ?? loadLegacyTickers()
        chartViewModels = configs.map { config in
            let vm = ChartViewModel(ticker: config.symbol, source: config.source)
            vm.applyConfig(config)
            return vm
        }
        let didRestore = restoreLastView()
        if !didRestore {
            Task {
                for vm in chartViewModels {
                    await vm.fetchData(for: selectedTimeRange, count: candleCount)
                    try? await Task.sleep(nanoseconds: Timeout.fetchStaggerNS)
                }
                connectWebSocket()
            }
        }
        // loadView() handles refetchAll() + connectWebSocket() for restored views
        startAutoRefresh()
    }

    /// Migrate legacy [String] format to [TickerConfig] once, then save.
    private func loadLegacyTickers() -> [TickerConfig] {
        guard let data = try? Data(contentsOf: AppSupport.directory.appendingPathComponent("tickers.json")),
              let strings = try? JSONDecoder().decode([String].self, from: data)
        else { return [] }
#if DEBUG
        print("[TickerStore] Migrated \(strings.count) legacy tickers to .binance")
#endif
        let configs = strings.map { TickerConfig(symbol: $0, source: .binance) }
        tickerStore.save(configs)
        return configs
    }

    /// Restore the last-used saved view if available.
    @discardableResult
    private func restoreLastView() -> Bool {
        guard let idString = UserDefaults.standard.string(forKey: "lastViewID"),
              let id = UUID(uuidString: idString),
              let view = savedViews.first(where: { $0.id == id })
        else { return false }
        loadView(view)
        return true
    }

    private func saveLastViewID(_ id: UUID?) {
        if let id {
            UserDefaults.standard.set(id.uuidString, forKey: "lastViewID")
        } else {
            UserDefaults.standard.removeObject(forKey: "lastViewID")
        }
    }

    /// Refresh all charts every 5 seconds. Cache prevents redundant API calls.
    /// No loading indicator — silent background refresh.
    func startAutoRefresh() {
        stopAutoRefresh()
        refreshTimer = Timer.scheduledTimer(withTimeInterval: Timeout.autoRefresh, repeats: true) { [weak self] _ in
            guard let self else { return }
            Task { @MainActor in
                await self.refetchAllSilent()
            }
        }
    }

    func stopAutoRefresh() {
        refreshTimer?.invalidate()
        refreshTimer = nil
        wsService.disconnect()
    }

    /// Set global timeframe and refetch all charts.
    func setTimeRange(_ range: TimeRange) {
        selectedTimeRange = range
        markChanged()
        refetchAll()
        connectWebSocket()
    }

    /// Open WebSocket streams for Binance tickers only.
    private func connectWebSocket() {
        let binanceVMs = chartViewModels.filter { $0.source == .binance }
        let symbols = binanceVMs.map { $0.apiSymbol.lowercased() }
        let interval = selectedTimeRange.binanceInterval

        guard !symbols.isEmpty else {
            wsService.disconnect()
            return
        }

        wsService.connect(symbols: symbols, interval: interval) { [weak self] symbol, kline in
            self?.chartViewModels
                .first(where: { $0.source == .binance && $0.apiSymbol.uppercased() == symbol.uppercased() })?
                .applyKlineUpdate(kline)
        }
    }

    /// Adjust candle count by a delta. Clamped to [min, max].
    /// Positive delta = zoom in (fewer candles), negative = zoom out (more candles).
    func adjustCandleCount(by delta: Int) {
        let newCount = (candleCount - delta).clamped(to: Candle.minCandles...Candle.maxCandles)
        guard newCount != candleCount else { return }
        candleCount = newCount
        markChanged()
        refetchAll()
    }

    private var refetchTask: Task<Void, Never>?

    /// Refetch with loading indicator (user-initiated: zoom, time range change).
    private func refetchAll() {
        refetchTask?.cancel()
        refetchTask = Task { [weak self] in
            guard let self else { return }
            self.isRefreshing = true
            defer { self.isRefreshing = false }
            await self.refetchAllVMs()
        }
    }

    /// Refetch without loading indicator (auto-refresh timer).
    private func refetchAllSilent() async {
        refetchTask?.cancel()
        let task = Task { [weak self] in
            guard let self else { return }
            await self.refetchAllVMs()
        }
        refetchTask = task
        await task.value
    }

    private func refetchAllVMs() async {
        let range = self.selectedTimeRange
        let count = self.candleCount
        for vm in self.chartViewModels {
            guard !Task.isCancelled else { return }
            await vm.fetchData(for: range, count: count)
        }
    }

    /// Add a ticker with a chosen data source.
    func addTicker(symbol: String, source: DataSourceType) async throws {
        // Duplicate check: same symbol + same source
        guard !chartViewModels.contains(where: {
            $0.ticker.uppercased() == symbol.uppercased() && $0.source == source
        }) else {
            throw TickerError.duplicate(symbol)
        }

        let vm = ChartViewModel(ticker: symbol, source: source)
        chartViewModels.append(vm)
        saveTickerConfigs()

        await vm.fetchData(for: selectedTimeRange, count: candleCount)
        markChanged()
        connectWebSocket()
    }

    /// Remove a ticker and persist the change.
    func removeTicker(_ vm: ChartViewModel) {
        chartViewModels.removeAll { $0.uniqueID == vm.uniqueID }
        persistTickers()
        markChanged()
        connectWebSocket()
    }

    /// Update a chart's ticker symbol and/or source, then refetch.
    func updateTicker(_ vm: ChartViewModel, symbol: String, source: DataSourceType) {
        vm.updateTicker(symbol: symbol, source: source)
        persistTickers()
        markChanged()
        Task {
            await vm.fetchData(for: selectedTimeRange, count: candleCount)
        }
        connectWebSocket()
    }

    /// Reorder tickers via drag-and-drop.
    func moveTicker(from source: IndexSet, to destination: Int) {
        chartViewModels.move(fromOffsets: source, toOffset: destination)
        persistTickers()
        markChanged()
    }

    private func makeTickerConfigs() -> [TickerConfig] {
        chartViewModels.map { vm in
            TickerConfig(
                symbol: vm.ticker,
                source: vm.source,
                bullishColorHex: vm.bullishColor.hexString,
                bearishColorHex: vm.bearishColor.hexString,
                yAxisDecimalPlaces: vm.yAxisDecimalPlaces
            )
        }
    }

    private func persistTickers() {
        tickerStore.save(makeTickerConfigs())
    }

    private func saveTickerConfigs() {
        tickerStore.save(makeTickerConfigs())
    }

    // MARK: - Saved Views

    /// Save the current state. Updates existing view if already named, otherwise creates new.
    func saveCurrentView(name: String) {
        let configs = makeTickerConfigs()
        let view = SavedView(
            id: currentViewID ?? UUID(),
            name: name,
            tickers: chartViewModels.map { $0.ticker },
            timeRange: selectedTimeRange,
            layoutMode: layoutMode,
            createdAt: Date(),
            tickerConfigs: configs,
            candleCount: candleCount
        )
        savedViews.removeAll { $0.id == view.id }
        savedViews.append(view)
        viewStore.save(savedViews)

        currentViewName = name
        currentViewID = view.id
        hasUnsavedChanges = false
        saveLastViewID(view.id)
    }

    /// Save changes to the current named view without prompting.
    func saveChanges() {
        guard currentViewID != nil else {
            // No saved view yet — treat as new save; caller should prompt for name
            return
        }
        saveCurrentView(name: currentViewName)
    }

    /// Apply a saved view, replacing all current state.
    func loadView(_ view: SavedView) {
        isApplyingView = true
        defer { isApplyingView = false }

        chartViewModels.removeAll()
        selectedTimeRange = view.timeRange
        candleCount = view.candleCount ?? view.timeRange.dataPointLimit
        layoutMode = view.layoutMode

        let configs = view.resolvedConfigs
        chartViewModels = configs.map { config in
            let vm = ChartViewModel(ticker: config.symbol, source: config.source)
            vm.applyConfig(config)
            return vm
        }
        tickerStore.save(configs)

        currentViewName = view.name
        currentViewID = view.id
        hasUnsavedChanges = false
        saveLastViewID(view.id)

        refetchAll()
        connectWebSocket()
    }

    /// Delete a saved view.
    func deleteView(_ view: SavedView) {
        savedViews.removeAll { $0.id == view.id }
        viewStore.save(savedViews)
        if currentViewID == view.id {
            currentViewName = UI.unnamedView
            currentViewID = nil
            hasUnsavedChanges = false
            saveLastViewID(nil)
        }
    }

    /// Persist chart appearance settings (colors, decimals) to disk.
    func persistChartSettings() {
        persistTickers()
        markChanged()
    }

    /// Mark current view as having unsaved changes (unless applying a loaded view).
    private func markChanged() {
        guard !isApplyingView else { return }
        hasUnsavedChanges = true
    }

}

enum TickerError: LocalizedError {
    case duplicate(String)

    var errorDescription: String? {
        switch self {
        case .duplicate(let ticker):
            return "\"\(ticker)\" is already in your list"
        }
    }
}
