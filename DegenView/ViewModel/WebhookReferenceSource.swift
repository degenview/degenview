import Foundation

/// Finds, and clears, the alerts that point at a webhook endpoint, so deleting an endpoint can warn
/// first and then remove its id from every alert instead of leaving dangling references.
@MainActor
protocol WebhookReferenceSource {
    /// How many alerts (price alerts and Pine alert subscriptions) reference `endpointID`.
    func referenceCount(to endpointID: UUID) -> Int
    /// Removes `endpointID` from every alert that references it.
    func removeReferences(to endpointID: UUID)
}

/// The live alert stores. Price alerts go back through `AlertStore.save`, i.e. the same
/// `alert_command` queue every other price-alert edit uses, so the runtime owner applies the change.
@MainActor
struct AlertWebhookReferences: WebhookReferenceSource {
    var priceAlerts: () -> [PriceAlert] = { AlertStore.shared.alerts }
    var savePriceAlert: (PriceAlert) -> Void = { AlertStore.shared.save($0) }
    var pineStore: PineAlertStore = .shared

    func referenceCount(to endpointID: UUID) -> Int {
        priceAlerts().filter { $0.webhookEndpointIDs.contains(endpointID) }.count
            + pineStore.subscriptions.filter { $0.webhookEndpointIDs.contains(endpointID) }.count
    }

    func removeReferences(to endpointID: UUID) {
        for var alert in priceAlerts() where alert.webhookEndpointIDs.contains(endpointID) {
            alert.webhookEndpointIDs.removeAll { $0 == endpointID }
            alert.updatedAt = Date()
            savePriceAlert(alert)
        }
        for subscription in pineStore.subscriptions where subscription.webhookEndpointIDs.contains(endpointID) {
            pineStore.update(subscription.id) { $0.webhookEndpointIDs.removeAll { $0 == endpointID } }
        }
    }
}
