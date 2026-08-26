import Foundation

protocol AlertSnapshotRepository: Sendable {
    func load() async -> AlertPersistenceSnapshot?
    func save(_ snapshot: AlertPersistenceSnapshot) async
}

actor LocalAlertRepository: AlertSnapshotRepository {
    private let persistence = AlertRuntimePersistence.shared
    func load() -> AlertPersistenceSnapshot? { persistence.loadSnapshot() }
    func save(_ snapshot: AlertPersistenceSnapshot) { persistence.saveSnapshot(snapshot) }
}

actor LocalPriceAlertEngine {
    private let repository: any AlertSnapshotRepository
    private var snapshot: AlertPersistenceSnapshot
    private var index: [String: Set<UUID>] = [:]

    init(repository: any AlertSnapshotRepository = LocalAlertRepository()) async {
        self.repository = repository
        let loaded = await repository.load()
        if let loaded, loaded.schemaVersion <= AlertPersistenceSnapshot.currentSchemaVersion {
            snapshot = loaded
        } else {
            snapshot = AlertPersistenceSnapshot()
        }
        for alert in snapshot.alerts where alert.state == .active {
            index[alert.asset.key, default: []].insert(alert.id)
        }
    }

    func currentSnapshot() -> AlertPersistenceSnapshot { snapshot }
    func replaceHealth(_ health: AlertRuntimeHealth) async {
        snapshot.health = health
        await persist()
    }
    func markDelivery(eventID: UUID, record: AlertDeliveryRecord) async {
        guard let i = snapshot.history.firstIndex(where: { $0.id == eventID }) else { return }
        snapshot.history[i].delivery = record
        await persist()
    }
    func apply(_ command: AlertRuntimeCommand) async {
        guard !snapshot.processedCommandIDs.contains(command.id) else { return }
        switch command.payload {
        case .save(let alert, let baseline): await saveAlert(alert, baseline: baseline)
        case .setState(let id, let state, let baseline): await setState(id, state: state, baseline: baseline)
        case .delete(let id): await delete(id)
        case .clearHistory: await clearHistory()
        case .updateSettings(let settings): await updateSettings(settings)
        case .requestNotificationAuthorization: break
        }
        snapshot.processedCommandIDs.append(command.id)
        if snapshot.processedCommandIDs.count > 1_000 {
            snapshot.processedCommandIDs.removeFirst(snapshot.processedCommandIDs.count - 1_000)
        }
        await persist()
    }
    func activeAssets() -> [PortfolioAsset] {
        Array(
            MarketQuoteCoordinator.assetsByKey(
                snapshot.alerts.lazy.filter { $0.state == .active }.map(\.asset)
            ).values)
    }

    func saveAlert(_ alert: PriceAlert, baseline: Decimal?) async {
        var value = alert
        value.updatedAt = Date()
        value.previousValue = baseline
        value.lastFingerprint = nil
        if case .unsupported = value.condition {
            value.state = .unsupported
            value.armed = false
        } else if let baseline, let target = value.condition.target {
            value.armed = value.condition.crossesAbove ? baseline < target : baseline > target
        }
        if let old = snapshot.alerts.firstIndex(where: { $0.id == value.id }) {
            snapshot.alerts[old] = value
        } else {
            snapshot.alerts.append(value)
        }
        rebuildIndex()
        await persist()
    }

    func setState(_ id: UUID, state: AlertState, baseline: Decimal?) async {
        guard let i = snapshot.alerts.firstIndex(where: { $0.id == id }) else { return }
        snapshot.alerts[i].state = state
        snapshot.alerts[i].updatedAt = Date()
        if state == .active {
            snapshot.alerts[i].previousValue = baseline
            if let baseline, let target = snapshot.alerts[i].condition.target {
                snapshot.alerts[i].armed =
                    snapshot.alerts[i].condition.crossesAbove ? baseline < target : baseline > target
            }
        }
        rebuildIndex()
        await persist()
    }

    func delete(_ id: UUID) async {
        snapshot.alerts.removeAll { $0.id == id }
        rebuildIndex()
        await persist()
    }
    func clearHistory() async {
        snapshot.history.removeAll()
        await persist()
    }
    func updateSettings(_ settings: AlertNotificationSettings) async {
        snapshot.settings = settings
        await persist()
    }

    func process(_ quote: MarketQuote, convertedPrice: Decimal? = nil, currency: PortfolioCurrency? = nil) async
        -> [AlertTriggerEvent]
    {
        guard quote.isFresh, let ids = index[quote.asset.key] else { return [] }
        var events: [AlertTriggerEvent] = []
        for id in ids {
            guard let i = snapshot.alerts.firstIndex(where: { $0.id == id }), snapshot.alerts[i].state == .active else {
                continue
            }
            var alert = snapshot.alerts[i]
            if let currency, alert.currency != currency { continue }
            let processedFingerprint = "\(quote.fingerprint):\(alert.currency.rawValue)"
            guard alert.lastFingerprint != processedFingerprint else { continue }
            let current = convertedPrice ?? quote.price
            guard let target = alert.condition.target else { continue }
            if let previous = alert.previousValue {
                let crossed =
                    alert.condition.crossesAbove
                    ? previous < target && current >= target
                    : previous > target && current <= target
                let opposite = alert.condition.crossesAbove ? current < target : current > target
                if !alert.armed && opposite { alert.armed = true }
                if alert.armed && crossed {
                    let event = AlertTriggerEvent(
                        id: UUID(), alertID: alert.id, asset: alert.asset,
                        observedValue: current, target: target, currency: alert.currency,
                        timestamp: quote.receivedAt, quoteFingerprint: quote.fingerprint)
                    events.append(event)
                    snapshot.history.append(event)
                    if alert.frequency == .once {
                        alert.state = .triggered
                        alert.armed = false
                    } else {
                        alert.armed = false
                    }
                }
            } else {
                alert.armed = alert.condition.crossesAbove ? current < target : current > target
            }
            alert.previousValue = current
            alert.lastFingerprint = processedFingerprint
            snapshot.alerts[i] = alert
        }
        if !events.isEmpty { rebuildIndex() }
        await persist()
        return events
    }

    /// Replays ordered quotes after wake/reconnect. A recovery intentionally emits at
    /// most one event per alert and never invents crossings beyond the 24-hour window.
    func processCatchUp(_ quotes: [MarketQuote], now: Date = Date()) async -> [AlertTriggerEvent] {
        let ordered = quotes.sorted { $0.sourceTimestamp < $1.sourceTimestamp }
            .filter { now.timeIntervalSince($0.sourceTimestamp) <= 86_400 }
        var emittedAlertIDs: Set<UUID> = []
        var result: [AlertTriggerEvent] = []
        for quote in ordered {
            let replay = MarketQuote(
                asset: quote.asset, price: quote.price, currency: quote.currency,
                sourceTimestamp: quote.sourceTimestamp, receivedAt: quote.sourceTimestamp,
                maximumAge: 86_400, fingerprint: quote.fingerprint)
            let events = await process(replay)
            for event in events {
                guard emittedAlertIDs.insert(event.alertID).inserted else {
                    snapshot.history.removeAll { $0.id == event.id }
                    continue
                }
                guard let i = snapshot.history.firstIndex(where: { $0.id == event.id }) else { continue }
                snapshot.history[i].origin = .catchUp
                result.append(snapshot.history[i])
            }
        }
        await persist()
        return result
    }

    func isIdentical(_ candidate: PriceAlert) -> Bool {
        snapshot.alerts.contains {
            $0.id != candidate.id && $0.asset.key == candidate.asset.key && $0.condition == candidate.condition
                && $0.currency == candidate.currency && $0.frequency == candidate.frequency
        }
    }

    private func rebuildIndex() {
        index.removeAll()
        for alert in snapshot.alerts where alert.state == .active {
            index[alert.asset.key, default: []].insert(alert.id)
        }
    }
    private func persist() async {
        snapshot.schemaVersion = AlertPersistenceSnapshot.currentSchemaVersion
        snapshot.revision &+= 1
        await repository.save(snapshot)
    }
}
