import Foundation
import SwiftUI

@MainActor
final class ChartViewModel: ObservableObject {
    let ticker: String
    private let api: BinanceAPIServiceProtocol

    var pair: String {
        ticker.uppercased().hasSuffix("USDT") ? ticker.uppercased() : "\(ticker.uppercased())USDT"
    }

    @Published var klineData: [KlineData] = []
    @Published var isLoading = false
    @Published var errorMessage: String?
    @Published var lastUpdated: Date?
    @Published var currentPrice: Double?

    private var fetchTask: Task<Void, Never>?

    var priceChangePercent: Double? {
        klineData.priceChangePercent
    }

    var priceChangeIsPositive: Bool {
        guard let change = priceChangePercent else { return true }
        return change >= 0
    }

    init(ticker: String, api: BinanceAPIServiceProtocol = BinanceAPIService()) {
        self.ticker = ticker
        self.api = api
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

    func fetchData(for range: TimeRange, count: Int) async {
        fetchTask?.cancel()
        errorMessage = nil
        isLoading = true

        fetchTask = Task { [weak self] in
            guard let self else { return }

            do {
                let data = try await self.api.fetchKlines(
                    symbol: self.pair,
                    interval: range.binanceInterval,
                    limit: count
                )
                guard !Task.isCancelled else { return }

                self.klineData = data
                self.currentPrice = data.last?.closePrice
                self.lastUpdated = Date()
                self.errorMessage = nil
            } catch is CancellationError {
                // Superseded by a newer request — ignore
            } catch {
                guard !Task.isCancelled else { return }
                self.errorMessage = error.localizedDescription
            }

            guard !Task.isCancelled else { return }
            self.isLoading = false
        }
    }
}
