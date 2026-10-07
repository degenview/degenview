import Foundation

/// Sends the webhooks of triggered price alerts. Owned by `AlertRuntimeHost`, so it only ever runs
/// in the process that holds `alert_runtime.lock` (the login-item agent, or the app when the agent
/// is not running). `AlertStore` and the GUI never call it: there is no second sender.
///
/// It reads the persisted snapshot after the engine has already committed the trigger, so a slow or
/// failing endpoint cannot affect evaluation, and a failed delivery never changes the trigger.
actor PriceAlertWebhookDispatcher {
    /// What an alert with a webhook but no message of its own posts.
    static let defaultTemplate = AlertMessageRenderer.defaultPriceAlertTemplate
    /// A trigger older than this is never sent. If an owner died with a webhook unsent, the next
    /// owner must not post a stale "BUY" a minute later.
    static let maximumEventAge: TimeInterval = 60

    static let shared = PriceAlertWebhookDispatcher()

    private let database: AppDatabase
    private let service: WebhookDeliveryService
    private let now: @Sendable () -> Date
    /// "event|endpoint" pairs already handled by this process, so a busy tick does not hit the
    /// database again for the same pair. The database claim is what makes it exactly-once across
    /// processes.
    private var handled: Set<String> = []

    init(
        database: AppDatabase = .shared,
        service: WebhookDeliveryService = .shared,
        now: @escaping @Sendable () -> Date = { Date() }
    ) {
        self.database = database
        self.service = service
        self.now = now
    }

    /// Starts one delivery per (trigger, endpoint) that has not been sent yet and returns their
    /// tasks without waiting for them. Callers that need completion (tests) await the tasks.
    @discardableResult
    func dispatch(snapshot: AlertPersistenceSnapshot) -> [Task<Void, Never>] {
        guard snapshot.settings.deliveryEnabled else { return [] }
        let current = now()
        let alertsByID = Dictionary(snapshot.alerts.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
        let work: [(event: AlertTriggerEvent, alert: PriceAlert)] = snapshot.history.compactMap { event in
            guard event.origin == .live, current.timeIntervalSince(event.timestamp) <= Self.maximumEventAge,
                let alert = alertsByID[event.alertID], !alert.webhookEndpointIDs.isEmpty
            else { return nil }
            return (event, alert)
        }
        guard !work.isEmpty else { return [] }

        // An unreadable endpoint table means "unknown", not "no endpoints": skip, and try again on
        // the next tick while the trigger is still fresh.
        guard let endpoints = try? database.webhookEndpoints() else { return [] }
        let byID = Dictionary(endpoints.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })

        var tasks: [Task<Void, Never>] = []
        for (event, alert) in work {
            let message = Self.message(for: event, alert: alert, now: current)
            for endpointID in alert.webhookEndpointIDs {
                // A deleted endpoint is a dangling id, and a disabled one is a switch the user set:
                // neither is attempted or recorded.
                guard let endpoint = byID[endpointID], endpoint.isEnabled else { continue }
                guard handled.insert("\(event.id.uuidString)|\(endpointID.uuidString)").inserted else { continue }
                guard
                    let claim = database.claimWebhookDelivery(
                        endpointID: endpointID, source: .price, eventID: event.id, now: current)
                else { continue }
                let database = database
                let service = service
                tasks.append(
                    Task.detached {
                        let result = await service.deliver(message: message, to: endpoint, mode: .production)
                        database.finishWebhookDelivery(id: claim, result: result)
                    })
            }
        }
        return tasks
    }

    /// The rendered body for a trigger, from the candle it was read from.
    static func message(for event: AlertTriggerEvent, alert: PriceAlert, now: Date) -> String {
        let template = alert.webhookMessage.flatMap { $0.isEmpty ? nil : $0 } ?? defaultTemplate
        return AlertMessageRenderer.render(template, context: context(for: event, now: now))
    }

    static func context(for event: AlertTriggerEvent, now: Date) -> AlertMessageContext {
        let asset = event.asset
        var context = AlertMessageContext(
            ticker: asset.metadata["apiSymbol"] ?? asset.symbol, exchange: asset.source.displayName, now: now)
        if let candle = event.candle {
            context.open = candle.open
            context.high = candle.high
            context.low = candle.low
            context.close = candle.close
            context.volume = candle.volume
            context.time = candle.openTime
            context.interval = AlertMessageRenderer.tradingViewInterval(candle.interval)
        }
        return context
    }
}
