import Foundation

/// Connects chart view models to Pine script alert delivery: routes what a script raises on a live
/// bar, and keeps each subscription honest when its chart, script or symbol changes under it.
@MainActor
final class PineAlertCoordinator {
    static let shared = PineAlertCoordinator()

    let store: PineAlertStore
    private let dispatcher: PineAlertDispatcher
    private var frequencyGuard: PineAlertFrequencyGuard

    private struct WeakChart {
        weak var chart: ChartViewModel?
    }
    private var attached: [WeakChart] = []

    init(store: PineAlertStore = .shared, dispatcher: PineAlertDispatcher = PineAlertDispatcher()) {
        self.store = store
        self.dispatcher = dispatcher
        frequencyGuard = PineAlertFrequencyGuard(seeded: store.recentDedupeKeys())
    }

    /// Starts routing `chart`'s alerts. Safe to call again for a chart already attached.
    func attach(_ chart: ChartViewModel) {
        attached.removeAll { $0.chart == nil }
        if !attached.contains(where: { $0.chart === chart }) { attached.append(WeakChart(chart: chart)) }
        chart.pineAlertHandler = { [weak self, weak chart] events, barID, instanceID in
            guard let self, let chart else { return }
            self.ingest(
                events: events, barID: barID, chartID: chart.chartID, instanceID: instanceID,
                sourceHash: instanceID.flatMap { chart.pineInstanceSourceHashes[$0] })
        }
        chart.pineContextHandler = { [weak self, weak chart] dataset, hashes in
            guard let self, let chart else { return }
            self.contextChanged(chartID: chart.chartID, dataset: dataset, hashes: hashes)
        }
        chart.pineInstanceRemovedHandler = { [weak self, weak chart] instanceID in
            guard let self, let chart else { return }
            self.instanceRemoved(chartID: chart.chartID, instanceID: instanceID)
        }
        contextChanged(chartID: chart.chartID, dataset: chart.pineAlertDataset, hashes: chart.pineInstanceSourceHashes)
    }

    /// The attached chart with this id, when its tab is open.
    func chart(withID chartID: UUID) -> ChartViewModel? {
        attached.lazy.compactMap(\.chart).first { $0.chartID == chartID }
    }

    // MARK: Routing

    func ingest(
        events: [PineAlertEvent], barID: PineBarID?, chartID: UUID, instanceID: UUID?, sourceHash: String?
    ) {
        let routed = PineAlertRouter.route(
            events: events, barID: barID, chartID: chartID, instanceID: instanceID, sourceHash: sourceHash,
            subscriptions: store.subscriptions, guard: &frequencyGuard)
        for item in routed where store.record(item.notification, dedupeKey: item.dedupeKey) {
            let notification = item.notification
            Task { [dispatcher] in await dispatcher.dispatch(notification) }
        }
    }

    // MARK: Subscription lifecycle

    /// A chart recalculated its scripts, or one learned its source. A source that no longer matches
    /// what a subscription was created for pauses it, and stays paused until the user re-arms.
    /// `hashes` holds only instances whose source is known: an absent one is still loading, not changed.
    func contextChanged(chartID: UUID, dataset: PineDatasetKey, hashes: [UUID: String]) {
        store.setDataset(dataset, forChart: chartID)
        for subscription in store.subscriptions(forChart: chartID) where subscription.state == .active {
            guard let instanceID = subscription.instanceID, let hash = hashes[instanceID],
                hash != subscription.sourceHash
            else { continue }
            store.update(subscription.id) { $0.state = .scriptChanged }
        }
    }

    /// An indicator was taken off the chart: its alerts, and what they delivered, go with it.
    func instanceRemoved(chartID: UUID, instanceID: UUID) {
        for subscription in store.subscriptions(forChart: chartID) where subscription.instanceID == instanceID {
            remove(subscription: subscription.id)
        }
    }

    func chartRemoved(chartID: UUID) {
        store.removeSubscriptions(forChart: chartID)
    }

    /// The scripts folder changed: a subscription whose saved script is gone can no longer be trusted.
    func scriptsChanged() async {
        for subscription in store.subscriptions {
            guard let scriptID = subscription.scriptID, subscription.state != .scriptDeleted else { continue }
            let exists = (try? await ScriptStore.shared.script(id: scriptID)) != nil
            if !exists { store.update(subscription.id) { $0.state = .scriptDeleted } }
        }
    }

    /// Creates an active subscription for one of `chart`'s applied indicators, or nil when it is gone
    /// or has not resolved its source yet.
    @discardableResult
    func subscribe(
        _ chart: ChartViewModel, instanceID: UUID, scriptName: String, note: String
    ) -> PineAlertSubscription? {
        guard let instance = chart.scriptInstances.first(where: { $0.id == instanceID }),
            let hash = chart.pineInstanceSourceHashes[instanceID]
        else { return nil }
        let dataset = chart.pineAlertDataset
        let subscription = PineAlertSubscription(
            chartID: chart.chartID, instanceID: instanceID, scriptID: instance.scriptID, scriptName: scriptName,
            symbolKey: dataset.symbolKey, timeframe: dataset.timeframe, sourceHash: hash, note: note)
        store.add(subscription)
        Task { await AlertStore.shared.requestNotificationAuthorizationIfNeeded() }
        return subscription
    }

    /// Whether `subscription` can be armed now: its chart is open and still runs its indicator.
    func canRearm(_ subscription: PineAlertSubscription) -> Bool {
        sourceHash(of: subscription) != nil
    }

    private func sourceHash(of subscription: PineAlertSubscription) -> String? {
        guard let instanceID = subscription.instanceID,
            let chart = chart(withID: subscription.chartID),
            chart.scriptInstances.contains(where: { $0.id == instanceID })
        else { return nil }
        return chart.pineInstanceSourceHashes[instanceID]
    }

    func pause(subscription id: UUID) {
        store.update(id) { $0.state = .paused }
    }

    /// Arms a subscription again, re-pinning it to its chart's current script and market and
    /// forgetting what it delivered, so it starts clean. False when its chart is not open.
    @discardableResult
    func rearm(subscription id: UUID) -> Bool {
        guard let subscription = store.subscription(id: id), let instanceID = subscription.instanceID,
            let chart = chart(withID: subscription.chartID),
            let scriptID = chart.scriptInstances.first(where: { $0.id == instanceID })?.scriptID,
            let hash = sourceHash(of: subscription)
        else { return false }
        let dataset = chart.pineAlertDataset
        frequencyGuard.reset(subscription: id)
        store.clearDedupeKeys(subscriptionID: id)
        store.update(id) {
            $0.scriptID = scriptID
            $0.sourceHash = hash
            $0.symbolKey = dataset.symbolKey
            $0.timeframe = dataset.timeframe
            $0.createdAt = Date()
            $0.state = .active
        }
        Task { await AlertStore.shared.requestNotificationAuthorizationIfNeeded() }
        return true
    }

    func remove(subscription id: UUID) {
        frequencyGuard.reset(subscription: id)
        store.remove(id: id)
    }
}
