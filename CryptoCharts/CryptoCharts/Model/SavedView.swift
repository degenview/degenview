import Foundation

struct SavedView: Identifiable, Codable, Equatable {
    var id = UUID()
    var name: String
    var tickers: [String]
    var timeRange: TimeRange
    var layoutMode: LayoutMode
    var createdAt: Date
    /// Per-ticker data source configs. When nil (legacy data), tickers map to .binance.
    var tickerConfigs: [TickerConfig]?

    static func == (lhs: SavedView, rhs: SavedView) -> Bool {
        lhs.id == rhs.id
    }

    /// Resolved configs, falling back to .binance for legacy data.
    var resolvedConfigs: [TickerConfig] {
        if let configs = tickerConfigs, configs.count == tickers.count {
            return configs
        }
        return tickers.map { TickerConfig(symbol: $0, source: .binance) }
    }
}
