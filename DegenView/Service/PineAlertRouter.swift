import Foundation

/// Turns the alerts a chart's script raised into deliverable notifications. Pure: the caller owns
/// the subscriptions and the guard, so every rule is testable without a chart or a database.
enum PineAlertRouter {
    struct Routed: Equatable, Sendable {
        var notification: PineAlertNotification
        /// Unique per call site, bar and mode; nil for `freq_all`. See `PineAlertFrequencyGuard`.
        var dedupeKey: String?
    }

    /// - Parameters:
    ///   - events: what `PineExecutionUpdate.alerts` carried.
    ///   - barID: the update's bar; its dataset is the market the script ran on.
    ///   - sourceHash: hash of the source the chart has applied now.
    static func route(
        events: [PineAlertEvent], barID: PineBarID?, chartID: UUID, sourceHash: String?,
        subscriptions: [PineAlertSubscription], guard frequencyGuard: inout PineAlertFrequencyGuard,
        now: Date = Date()
    ) -> [Routed] {
        guard let dataset = barID?.dataset, let sourceHash else { return [] }
        let live = events.filter(\.isRealtime)
        guard !live.isEmpty else { return [] }

        var routed: [Routed] = []
        for subscription in subscriptions
        where subscription.isActive && subscription.chartID == chartID && subscription.watches(dataset)
            && subscription.sourceHash == sourceHash
        {
            let barLength = TimeRange(rawValue: subscription.timeframe)?.binanceIntervalSeconds ?? 0
            for event in live {
                // A bar that ended before the subscription existed is history the user never asked about.
                guard event.time.addingTimeInterval(barLength) > subscription.createdAt else { continue }
                guard frequencyGuard.admit(event, subscription: subscription.id) else { continue }
                routed.append(
                    Routed(
                        notification: PineAlertNotification(
                            subscriptionID: subscription.id, scriptName: subscription.scriptName,
                            chartID: chartID, symbolKey: subscription.symbolKey, timeframe: subscription.timeframe,
                            barTime: event.time, message: event.message, frequency: event.frequency,
                            triggeredAt: now, isConfirmed: event.isConfirmed),
                        dedupeKey: PineAlertFrequencyGuard.dedupeKey(subscription: subscription.id, event: event)))
            }
        }
        return routed
    }
}
