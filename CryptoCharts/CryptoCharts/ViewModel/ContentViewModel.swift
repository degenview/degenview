import Foundation
import SwiftUI

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
    @Published var useLogScale = false

    /// Minimum candles to show at max zoom-in.
    private let minCandles = 10
    /// Maximum candles to show at max zoom-out.
    private let maxCandles = 500

    private let api: BinanceAPIServiceProtocol
    private let tickerStore: TickerStore
    private var refreshTimer: Timer?
    private let wsService = BinanceWebSocketService()

    init(api: BinanceAPIServiceProtocol = BinanceAPIService(), tickerStore: TickerStore = TickerStore()) {
        self.api = api
        self.tickerStore = tickerStore
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
        }
        startAutoRefresh()
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
        refetchAll()
    }

    private var refetchTask: Task<Void, Never>?

    private func refetchAll() {
        refetchTask?.cancel()
        refetchTask = Task { [weak self] in
            guard let self else { return }
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
        connectWebSocket()
    }

    /// Remove a ticker and persist the change.
    func removeTicker(_ vm: ChartViewModel) {
        chartViewModels.removeAll { $0.ticker == vm.ticker }
        tickerStore.save(chartViewModels.map { $0.ticker })
        connectWebSocket()
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
