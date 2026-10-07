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
    /// Webhook endpoints (`WebhookEndpoint.id`) this alert also posts to. Ids, never URLs, so
    /// renaming or disabling an endpoint needs no change here.
    var webhookEndpointIDs: [UUID] = []

    var isActive: Bool { state == .active }

    /// Whether `dataset` is the market this subscription was created for.
    func watches(_ dataset: PineDatasetKey) -> Bool {
        dataset.symbolKey == symbolKey && dataset.timeframe == timeframe
    }
}

extension PineAlertSubscription {
    private enum CodingKeys: String, CodingKey {
        case id, chartID, instanceID, scriptID, scriptName, symbolKey, timeframe, sourceHash, note, state, createdAt
        case webhookEndpointIDs
    }

    /// Alerts saved before webhooks existed have no `webhookEndpointIDs`.
    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(UUID.self, forKey: .id)
        chartID = try c.decode(UUID.self, forKey: .chartID)
        instanceID = try c.decodeIfPresent(UUID.self, forKey: .instanceID)
        scriptID = try c.decodeIfPresent(UUID.self, forKey: .scriptID)
        scriptName = try c.decode(String.self, forKey: .scriptName)
        symbolKey = try c.decode(String.self, forKey: .symbolKey)
        timeframe = try c.decode(String.self, forKey: .timeframe)
        sourceHash = try c.decode(String.self, forKey: .sourceHash)
        note = try c.decode(String.self, forKey: .note)
        state = try c.decode(State.self, forKey: .state)
        createdAt = try c.decode(Date.self, forKey: .createdAt)
        webhookEndpointIDs = try c.decodeIfPresent([UUID].self, forKey: .webhookEndpointIDs) ?? []
    }
}
