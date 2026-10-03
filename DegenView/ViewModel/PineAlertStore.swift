import Foundation

/// The user's Pine script alert subscriptions and the history of what was delivered.
@MainActor
final class PineAlertStore: ObservableObject {
    static let shared = PineAlertStore()

    @Published private(set) var subscriptions: [PineAlertSubscription]
    @Published private(set) var history: [PineAlertNotification]
    /// The alert `GlobalAlertBanner` shows now.
    @Published var banner: PineAlertNotification?
    /// The market each open chart's script is running on, for telling a subscription whose chart
    /// now shows another symbol from one that is live.
    @Published private(set) var chartDatasets: [UUID: PineDatasetKey] = [:]

    private let database: AppDatabase

    init(database: AppDatabase = .shared) {
        self.database = database
        subscriptions = database.pineAlertSubscriptions()
        history = database.recentPineAlertEvents()
    }

    /// Dedupe keys of recent deliveries, to seed a fresh `PineAlertFrequencyGuard`.
    func recentDedupeKeys() -> [String] { database.recentPineAlertDedupeKeys() }

    func subscriptions(forChart chartID: UUID) -> [PineAlertSubscription] {
        subscriptions.filter { $0.chartID == chartID }
    }

    /// The alert a chart already has for `scriptID` on `dataset`, in any state: a paused or
    /// script-changed one is re-armed, not replaced. An active one wins over older ones.
    func subscription(forChart chartID: UUID, scriptID: UUID?, dataset: PineDatasetKey) -> PineAlertSubscription? {
        let matches = subscriptions(forChart: chartID).filter { $0.scriptID == scriptID && $0.watches(dataset) }
        return matches.first(where: \.isActive) ?? matches.max { $0.createdAt < $1.createdAt }
    }

    func subscription(id: UUID) -> PineAlertSubscription? {
        subscriptions.first { $0.id == id }
    }

    func add(_ subscription: PineAlertSubscription) {
        subscriptions.append(subscription)
        saveSubscriptions()
    }

    func update(_ id: UUID, _ change: (inout PineAlertSubscription) -> Void) {
        guard let index = subscriptions.firstIndex(where: { $0.id == id }) else { return }
        var copy = subscriptions[index]
        change(&copy)
        guard copy != subscriptions[index] else { return }
        subscriptions[index] = copy
        saveSubscriptions()
    }

    func remove(id: UUID) {
        subscriptions.removeAll { $0.id == id }
        saveSubscriptions()
        database.deletePineAlertEvents(subscriptionID: id)
        history.removeAll { $0.subscriptionID == id }
    }

    func removeSubscriptions(forChart chartID: UUID) {
        for subscription in subscriptions(forChart: chartID) { remove(id: subscription.id) }
        chartDatasets[chartID] = nil
    }

    func setDataset(_ dataset: PineDatasetKey?, forChart chartID: UUID) {
        guard chartDatasets[chartID] != dataset else { return }
        chartDatasets[chartID] = dataset
    }

    /// Stores a delivery. False when its dedupe key was recorded before, so it must not be delivered.
    func record(_ notification: PineAlertNotification, dedupeKey: String?) -> Bool {
        guard database.insertPineAlertEvent(notification, dedupeKey: dedupeKey) else { return false }
        history.insert(notification, at: 0)
        if history.count > AppDatabase.pineAlertHistoryLimit {
            history.removeLast(history.count - AppDatabase.pineAlertHistoryLimit)
        }
        return true
    }

    func clearDedupeKeys(subscriptionID: UUID) {
        database.clearPineAlertDedupeKeys(subscriptionID: subscriptionID)
    }

    private func saveSubscriptions() {
        database.savePineAlertSubscriptions(subscriptions)
    }
}
