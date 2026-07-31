import Foundation
import SwiftUI

@MainActor
final class ChartViewModel: ObservableObject {
    @Published var ticker: String
    @Published var source: DataSourceType

    /// Unique identifier — derived from initial ticker+source, stable across updates.
    let uniqueID: String

    private var api: TickerDataSource

    /// The symbol used for API calls — source-dependent.
    var apiSymbol: String {
        switch source {
        case .binance:
            let upper = ticker.uppercased()
            if upper.hasSuffix("USDT") || upper.hasSuffix("USDC") || upper.hasSuffix("BUSD") {
                return upper
            }
            return "\(upper)USDT"
        case .coingecko, .dexscreener:
            // ticker IS the fullSymbol (coin ID or pair address) from search result
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
        }
    }

    /// Identity of the icon currently wanted. `uniqueID` deliberately survives
    /// `updateTicker`, so it can't drive the icon lookup — the card would keep
    /// showing the previous coin's artwork.
    var iconKey: String { "\(source.rawValue):\(ticker)" }

    @Published var klineData: [KlineData] = []
    @Published var errorMessage: String?
    @Published var lastUpdated: Date?
    @Published var currentPrice: Double?

    // MARK: - Chart appearance settings

    @Published var bullishColor: Color = .green
    @Published var bearishColor: Color = .red
    @Published var yAxisDecimalPlaces: Int? = nil  // nil = auto-detect

    private var fetchTask: Task<Void, Never>?
    private var fetchGeneration = 0
    private var isFetching = false

    var priceChangePercent: Double? {
        klineData.priceChangePercent
    }

    var priceChangeIsPositive: Bool {
        guard let change = priceChangePercent else { return true }
        return change >= 0
    }

    init(ticker: String, source: DataSourceType = .binance, api: TickerDataSource? = nil) {
        self.ticker = ticker
        self.source = source
        self.uniqueID = "\(ticker)_\(source.rawValue)"
        self.api = api ?? DataSourceFactory.shared.service(for: source)
    }

    /// Apply persisted chart settings from a TickerConfig.
    func applyConfig(_ config: TickerConfig) {
        if let hex = config.bullishColorHex { bullishColor = Color(hex: hex) }
        if let hex = config.bearishColorHex { bearishColor = Color(hex: hex) }
        yAxisDecimalPlaces = config.yAxisDecimalPlaces
    }

    /// Update ticker symbol and/or source, re-fetch data.
    func updateTicker(symbol: String, source: DataSourceType) {
        ticker = symbol
        self.source = source
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
        }
        // New candle (openTime > last.openTime): skip — REST fetch adds it within 5 s

        if currentPrice != kline.closePrice {
            currentPrice = kline.closePrice
        }
    }

    func fetchData(for range: TimeRange, count: Int, silent: Bool = false) async {
        // Silent refresh: if a fetch is already running let it finish.
        if silent, isFetching {
            return
        }

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
