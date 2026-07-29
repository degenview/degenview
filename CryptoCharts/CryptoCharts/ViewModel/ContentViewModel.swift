import Foundation
import SwiftUI

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
    @Published var useLogScale = false {
        didSet { markChanged() }
    }
    @Published var layoutMode: LayoutMode = .vertical {
        didSet { markChanged() }
    }
    @Published var isRefreshing = false

    /// Minimum candles to show at max zoom-in.
    private let minCandles = 10
    /// Maximum candles to show at max zoom-out.
    private let maxCandles = 500

    @Published var savedViews: [SavedView] = []
    @Published var currentViewName = "Unnamed"
    @Published var hasUnsavedChanges = false

    private var currentViewID: UUID?
    private var isApplyingView = false

    private let api: BinanceAPIServiceProtocol
    private let tickerStore: TickerStore
    private let viewStore = ViewStore()
    private var refreshTimer: Timer?
    private let wsService = BinanceWebSocketService()

    init(api: BinanceAPIServiceProtocol = BinanceAPIService(), tickerStore: TickerStore = TickerStore()) {
        self.api = api
        self.tickerStore = tickerStore
        self.savedViews = viewStore.load()
    }

    /// Load persisted tickers, create chart view models, and start auto-refresh.
    func loadTickers() {
        let tickers = tickerStore.load()
        chartViewModels = tickers.map { ChartViewModel(ticker: $0, api: api) }
        Task {
            for vm in chartViewModels {
                await vm.fetchData(for: selectedTimeRange, count: candleCount)
                try? await Task.sleep(nanoseconds: 100_000_000)
            }
            connectWebSocket()
            restoreLastView()
        }
        startAutoRefresh()
    }

    /// Restore the last-used saved view if available.
    private func restoreLastView() {
        guard let idString = UserDefaults.standard.string(forKey: "lastViewID"),
              let id = UUID(uuidString: idString),
              let view = savedViews.first(where: { $0.id == id })
        else { return }
        loadView(view)
    }

    private func saveLastViewID(_ id: UUID?) {
        if let id {
            UserDefaults.standard.set(id.uuidString, forKey: "lastViewID")
        } else {
            UserDefaults.standard.removeObject(forKey: "lastViewID")
        }
    }

    /// Refresh all charts every 5 seconds. Cache prevents redundant API calls.
    func startAutoRefresh() {
        stopAutoRefresh()
        refreshTimer = Timer.scheduledTimer(withTimeInterval: 5, repeats: true) { [weak self] _ in
            guard let self else { return }
            Task { @MainActor in
                self.refetchAll()
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

    /// Open WebSocket streams for all current tickers at the current interval.
    private func connectWebSocket() {
        let symbols = chartViewModels.map { $0.pair.lowercased() }
        let interval = selectedTimeRange.binanceInterval

        wsService.connect(symbols: symbols, interval: interval) { [weak self] symbol, kline in
            self?.chartViewModels
                .first(where: { $0.pair.uppercased() == symbol.uppercased() })?
                .applyKlineUpdate(kline)
        }
    }

    /// Adjust candle count by a delta. Clamped to [minCandles, maxCandles].
    /// Positive delta = zoom in (fewer candles), negative = zoom out (more candles).
    func adjustCandleCount(by delta: Int) {
        let newCount = (candleCount - delta).clamped(to: minCandles...maxCandles)
        guard newCount != candleCount else { return }
        candleCount = newCount
        markChanged()
        refetchAll()
    }

    private var refetchTask: Task<Void, Never>?

    private func refetchAll() {
        refetchTask?.cancel()
        refetchTask = Task { [weak self] in
            guard let self else { return }
            self.isRefreshing = true
            defer { self.isRefreshing = false }

            let range = self.selectedTimeRange
            let count = self.candleCount
            for vm in self.chartViewModels {
                guard !Task.isCancelled else { return }
                await vm.fetchData(for: range, count: count)
            }
        }
    }

    /// Validate and add a new ticker.
    func addTicker(_ raw: String) async throws {
        let normalized = normalizeTicker(raw)

        guard !chartViewModels.contains(where: { $0.ticker.uppercased() == normalized.uppercased() }) else {
            throw TickerError.duplicate(normalized)
        }

        let isValid = try await api.validateSymbol(normalized)
        guard isValid else {
            throw TickerError.invalidSymbol(normalized)
        }

        let vm = ChartViewModel(ticker: normalized, api: api)
        chartViewModels.append(vm)
        tickerStore.save(chartViewModels.map { $0.ticker })

        await vm.fetchData(for: selectedTimeRange, count: candleCount)
        markChanged()
        connectWebSocket()
    }

    /// Remove a ticker and persist the change.
    func removeTicker(_ vm: ChartViewModel) {
        chartViewModels.removeAll { $0.ticker == vm.ticker }
        tickerStore.save(chartViewModels.map { $0.ticker })
        markChanged()
        connectWebSocket()
    }

    /// Reorder tickers via drag-and-drop.
    func moveTicker(from source: IndexSet, to destination: Int) {
        chartViewModels.move(fromOffsets: source, toOffset: destination)
        tickerStore.save(chartViewModels.map { $0.ticker })
        markChanged()
    }

    // MARK: - Saved Views

    /// Save the current state. Updates existing view if already named, otherwise creates new.
    func saveCurrentView(name: String) {
        let view = SavedView(
            id: currentViewID ?? UUID(),
            name: name,
            tickers: chartViewModels.map { $0.ticker },
            timeRange: selectedTimeRange,
            useLogScale: useLogScale,
            layoutMode: layoutMode,
            createdAt: Date()
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
        guard let id = currentViewID else {
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
        candleCount = view.timeRange.dataPointLimit
        useLogScale = view.useLogScale
        layoutMode = view.layoutMode
        chartViewModels = view.tickers.map { ChartViewModel(ticker: $0, api: api) }
        tickerStore.save(view.tickers)

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
            currentViewName = "Unnamed"
            currentViewID = nil
            hasUnsavedChanges = false
            saveLastViewID(nil)
        }
    }

    /// Mark current view as having unsaved changes (unless applying a loaded view).
    private func markChanged() {
        guard !isApplyingView else { return }
        hasUnsavedChanges = true
    }

    /// Normalize user input to a Binance pair.
    private func normalizeTicker(_ raw: String) -> String {
        let uppercased = raw.trimmingCharacters(in: .whitespaces).uppercased()
        if uppercased.hasSuffix("USDT") || uppercased.hasSuffix("USDC") ||
           uppercased.hasSuffix("BUSD") {
            return uppercased
        }
        return "\(uppercased)USDT"
    }
}

extension Comparable {
    func clamped(to range: ClosedRange<Self>) -> Self {
        min(max(self, range.lowerBound), range.upperBound)
    }
}

enum TickerError: LocalizedError {
    case duplicate(String)
    case invalidSymbol(String)

    var errorDescription: String? {
        switch self {
        case .duplicate(let ticker):
            return "\"\(ticker)\" is already in your list"
        case .invalidSymbol(let ticker):
            return "\"\(ticker)\" is not a valid trading pair on Binance"
        }
    }
}
