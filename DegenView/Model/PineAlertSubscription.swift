import Foundation

/// A user's request to be notified when one chart's applied Pine script raises `alert()` events.
///
/// Bound to the chart, symbol, timeframe and the exact source that was applied, so a different
/// script or a chart showing another market never fires it by accident.
struct PineAlertSubscription: Codable, Identifiable, Equatable, Sendable {
    enum State: String, Codable, Sendable {
        case active
        /// Switched off by the user.
        case paused
        /// The chart's script no longer matches `sourceHash`. Silent until the user re-arms.
        case scriptChanged
        /// The saved script it was created from no longer exists.
        case scriptDeleted
    }

    var id = UUID()
    var chartID: UUID
    /// The applied indicator (`ChartScriptInstance.id`) this alert watches. Two instances of one
    /// script, with different inputs, are separate alerts.
    var instanceID: UUID?
    var scriptID: UUID?
    var scriptName: String
    /// `"<source>:<ticker>"`, matching `PineDatasetKey.symbolKey`.
    var symbolKey: String
    var timeframe: String
    var sourceHash: String
    var note = ""
    var state = State.active
    var createdAt = Date()

    var isActive: Bool { state == .active }

    /// Whether `dataset` is the market this subscription was created for.
    func watches(_ dataset: PineDatasetKey) -> Bool {
        dataset.symbolKey == symbolKey && dataset.timeframe == timeframe
    }
}
