import Foundation

enum ChartTabKind: String, Codable {
    case charts
    case portfolio
}

/// One tab's worth of chart state — the unit `TabsStore` persists and
/// `ContentViewModel` hydrates from.
///
/// Everything here used to be app-global: the ticker list lived in `tickers.json`
/// and the timeframe/layout/zoom lived only in memory on the single
/// `ContentViewModel`. Making it a value keyed by `id` is what lets several
/// windows hold independent chart sets.
struct ChartTab: Identifiable, Codable, Equatable {
    let id: UUID
    /// Tab label — `UI.unnamedView` until a saved view is loaded or the user renames it.
    var name: String
    /// The saved view this tab was last loaded from, if any.
    var savedViewID: UUID?
    var tickerConfigs: [TickerConfig]
    var chartColumns: [ChartColumn]?
    var timeRange: TimeRange
    var layoutMode: LayoutMode
    var candleCount: Int
    var replaySession: ReplaySession?
    var kind: ChartTabKind

    init(
        id: UUID = UUID(),
        name: String = UI.unnamedView,
        savedViewID: UUID? = nil,
        tickerConfigs: [TickerConfig] = [],
        chartColumns: [ChartColumn]? = nil,
        timeRange: TimeRange = .oneDay,
        layoutMode: LayoutMode = .vertical,
        candleCount: Int? = nil,
        replaySession: ReplaySession? = nil,
        kind: ChartTabKind = .charts
    ) {
        self.id = id
        self.name = name
        self.savedViewID = savedViewID
        self.tickerConfigs = tickerConfigs
        self.chartColumns = chartColumns
        self.timeRange = timeRange
        self.layoutMode = layoutMode
        self.candleCount = candleCount ?? timeRange.dataPointLimit
        self.replaySession = replaySession
        self.kind = kind
    }

    private enum CodingKeys: String, CodingKey {
        case id, name, savedViewID, tickerConfigs, chartColumns, timeRange, layoutMode, candleCount, replaySession, kind
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        id = try values.decode(UUID.self, forKey: .id)
        name = try values.decode(String.self, forKey: .name)
        savedViewID = try values.decodeIfPresent(UUID.self, forKey: .savedViewID)
        tickerConfigs = try values.decode([TickerConfig].self, forKey: .tickerConfigs)
        chartColumns = try values.decodeIfPresent([ChartColumn].self, forKey: .chartColumns)
        timeRange = try values.decode(TimeRange.self, forKey: .timeRange)
        layoutMode = try values.decode(LayoutMode.self, forKey: .layoutMode)
        candleCount = try values.decode(Int.self, forKey: .candleCount)
        replaySession = try values.decodeIfPresent(ReplaySession.self, forKey: .replaySession)
        kind = try values.decodeIfPresent(ChartTabKind.self, forKey: .kind) ?? .charts
    }
}

/// What lands in `tabs.json`: every tab plus how the tabs were distributed
/// across windows at quit.
///
/// `windowGroups` is read back out of AppKit at save time rather than maintained
/// as we go — the user reorders, detaches, and merges tabs through the system
/// tab bar, so `NSWindow.tabGroup` is the only authority on the current layout.
struct TabsSnapshot: Codable {
    var tabs: [ChartTab]
    var windowGroups: [[UUID]]

    static let empty = TabsSnapshot(tabs: [], windowGroups: [])
}
