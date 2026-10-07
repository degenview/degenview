import Foundation

/// Posts an admitted Pine alert to the webhook endpoints its subscription selected.
///
/// It sits behind `PineAlertDispatcher` like the notification and banner channels, so it only ever
/// sees alerts that already passed the router, the frequency guard and the persisted dedupe key.
/// It adds no frequency logic of its own and never runs for historical bars. The message is the
/// script's final text, sent as is.
struct WebhookPineAlertChannel: PineAlertChannel {
    let name = "webhook"

    private let database: AppDatabase
    private let service: WebhookDeliveryService

    init(database: AppDatabase = .shared, service: WebhookDeliveryService = .shared) {
        self.database = database
        self.service = service
    }

    func deliver(_ notification: PineAlertNotification) async throws {
        // No endpoints selected is the normal case: return before touching the database.
        guard let selected = notification.webhookEndpointIDs, !selected.isEmpty else { return }
        // An unreadable table throws (the dispatcher logs it) instead of reading as "no endpoints".
        let endpoints = try database.webhookEndpoints()

        var targets: [(endpoint: WebhookEndpoint, claim: UUID)] = []
        for id in Set(selected) {
            // Deleted ids dangle harmlessly; disabled endpoints are the user's global off switch.
            guard let endpoint = endpoints.first(where: { $0.id == id }), endpoint.isEnabled else { continue }
            // One row per (alert, endpoint) before the request goes out; nil means already handled.
            guard
                let claim = database.claimWebhookDelivery(
                    endpointID: id, source: .pine, eventID: notification.id, now: Date())
            else { continue }
            targets.append((endpoint, claim))
        }
        guard !targets.isEmpty else { return }

        let message = notification.message
        let database = database
        let service = service
        await withTaskGroup(of: Void.self) { group in
            for (endpoint, claim) in targets {
                group.addTask {
                    let result = await service.deliver(message: message, to: endpoint, mode: .production)
                    database.finishWebhookDelivery(id: claim, result: result)
                }
            }
        }
    }
}
