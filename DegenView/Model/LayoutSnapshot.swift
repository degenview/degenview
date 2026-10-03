import Foundation

/// The part of a tab that a saved layout owns, reduced to what decides "has this changed?".
///
/// Comparing two snapshots is the whole dirty check, so it holds only persisted configuration:
/// no prices, drawings, replay, or tool state. Zoom (`candleCount`) is left out on purpose —
/// scrolling the chart shouldn't ask the user to save.
struct LayoutSnapshot: Equatable {
    var timeRange: TimeRange
    var configs: [TickerConfig]
    /// Chart ids per column. `ChartColumn.id` is regenerated whenever the default grid is
    /// resolved, so only membership and order are comparable.
    var columns: [[UUID]]

    static let empty = LayoutSnapshot(timeRange: .oneDay, configs: [], columns: [[UUID]]())

    init(timeRange: TimeRange, configs: [TickerConfig], columns: [[UUID]]) {
        self.timeRange = timeRange
        self.configs = configs
        self.columns = columns
    }

    init(timeRange: TimeRange, configs: [TickerConfig], columns: [ChartColumn]) {
        self.init(timeRange: timeRange, configs: configs, columns: columns.map(\.chartIDs))
    }

    /// The snapshot a tab restored from `view` should produce, grid resolved the way the tab resolves it.
    init(view: SavedView) {
        let resolved = ChartColumn.resolved(view.chartColumns, chartIDs: view.tickerConfigs.map(\.chartID))
        self.init(timeRange: view.timeRange, configs: view.tickerConfigs, columns: resolved)
    }
}
