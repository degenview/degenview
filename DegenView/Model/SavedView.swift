import Foundation

struct SavedView: Identifiable, Codable, Equatable {
    var id = UUID()
    var name: String
    var tickers: [String]
    var timeRange: TimeRange
    var layoutMode: LayoutMode
    var createdAt: Date
    /// Per-ticker data source configs, in the order of `tickers`.
    var tickerConfigs: [TickerConfig]
    /// Explicit grid membership and ordering. Nil means the default two-column layout.
    var chartColumns: [ChartColumn]? = nil
    /// Zoom level — candle count at time of save.
    var candleCount: Int

    static func == (lhs: SavedView, rhs: SavedView) -> Bool {
        lhs.id == rhs.id
    }
}
