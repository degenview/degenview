import Foundation

struct SavedView: Identifiable, Codable, Equatable {
    var id = UUID()
    var name: String
    var tickers: [String]
    var timeRange: TimeRange
    var createdAt: Date
    /// Per-ticker data source configs, in the order of `tickers`.
    var tickerConfigs: [TickerConfig]
    /// Explicit grid membership and ordering. Nil means the default two-column layout.
    var chartColumns: [ChartColumn]? = nil
    /// Zoom level — candle count at time of save.
    var candleCount: Int
    /// When this layout was last opened or saved. Nil for views that predate recency tracking;
    /// they stay out of "Recently used" until opened.
    var lastOpenedAt: Date? = nil
    /// Whether changes to this layout are written back without asking. Nil = off.
    var autosave: Bool? = nil

    static func == (lhs: SavedView, rhs: SavedView) -> Bool {
        lhs.id == rhs.id
    }
}
