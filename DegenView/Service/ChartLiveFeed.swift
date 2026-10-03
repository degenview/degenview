import Foundation

/// Opens each provider's live stream for a set of charts and routes what arrives to the chart it
/// belongs to. One feed per host: a chart tab, or the Script Manager's preview.
///
/// Only Binance, Coinbase and Alpaca stream. The rest refresh over REST, which the host drives.
@MainActor
final class ChartLiveFeed {
    private let binance = BinanceWebSocketService()
    private let coinbase = CoinbaseWebSocketService()
    private let alpaca = AlpacaWebSocketService()

    /// Reconnects every stream for `charts` at `range`, or closes them all when `active` is false.
    /// Call it again whenever the charts, their symbols or the range change.
    func update(charts: [ChartViewModel], range: TimeRange, active: Bool) {
        // A hidden host has nothing to draw a tick onto.
        guard active else {
            disconnect()
            return
        }

        let interval = range.binanceInterval
        let binanceCharts = charts.filter { $0.source == .binance }

        // Binance streams no quarterly or yearly klines, and its monthly ones would land in the
        // wrong candle. Those candles move slowly; the five-second REST refresh keeps them current.
        if binanceCharts.isEmpty || KlineData.monthlyFold(for: interval) != nil {
            binance.disconnect()
        } else {
            binance.connect(
                symbols: binanceCharts.map { $0.apiSymbol.lowercased() }, interval: interval,
                onUpdate: { symbol, kline in
                    binanceCharts.first { $0.apiSymbol.uppercased() == symbol.uppercased() }?
                        .applyKlineUpdate(kline)
                },
                onBookTicker: { symbol, bid, ask in
                    binanceCharts.first { $0.apiSymbol.uppercased() == symbol.uppercased() }?
                        .liveQuote.apply(bid: bid, ask: ask)
                })
        }

        let coinbaseCharts = charts.filter { $0.source == .coinbase }
        if coinbaseCharts.isEmpty {
            coinbase.disconnect()
        } else if let plan = CoinbaseGranularity(interval: interval) {
            // The socket is interval-agnostic; the plan only decides which candle a trade lands in.
            coinbase.connect(products: coinbaseCharts.map(\.apiSymbol)) { tick in
                coinbaseCharts.first { $0.apiSymbol.uppercased() == tick.productID }?
                    .applyTick(tick, plan: plan)
            }
        }

        let stockCharts = charts.filter { $0.source == .alpaca }
        if stockCharts.isEmpty || !AlpacaCredentialsStore.isConfigured {
            alpaca.disconnect()
        } else {
            alpaca.connect(symbols: stockCharts.map(\.apiSymbol)) { symbol, kline in
                stockCharts.first { $0.apiSymbol.uppercased() == symbol.uppercased() }?
                    .applyLiveBar(kline, candleDuration: range.binanceIntervalSeconds)
            }
        }
    }

    func disconnect() {
        binance.disconnect()
        coinbase.disconnect()
        alpaca.disconnect()
    }
}
