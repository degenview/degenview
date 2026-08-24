import Darwin
import Foundation
import UserNotifications

/// The single writer/evaluator used by either the agent or the foreground fallback.
actor AlertRuntimeHost {
    enum Role: String, Sendable { case app, agent }

    private let role: Role
    private let persistence: AlertRuntimePersistence
    private var engine: LocalPriceAlertEngine?
    private var task: Task<Void, Never>?
    private var lockFD: Int32 = -1
    private var providerHealth: [DataSourceType: AlertProviderHealth] = [:]

    init(role: Role, persistence: AlertRuntimePersistence = .shared) {
        self.role = role; self.persistence = persistence
    }

    deinit { if lockFD >= 0 { flock(lockFD, LOCK_UN); close(lockFD) } }

    /// Nonblocking ownership prevents double polling during registration and handoff.
    func start() async -> Bool {
        guard task == nil, acquireOwnership() else { return false }
        let engine = await LocalPriceAlertEngine(repository: LocalAlertRepository())
        self.engine = engine
        await MarketQuoteCoordinator.shared.setQuoteHandler { [weak self] quote in await self?.receive(quote) }
        await refreshSubscriptions()
        task = Task { [weak self] in
            while !Task.isCancelled {
                await self?.tick()
                try? await Task.sleep(for: .seconds(1))
            }
        }
        await tick()
        return true
    }

    func stop() async {
        task?.cancel(); task = nil
        await MarketQuoteCoordinator.shared.unsubscribe(owner: "price-alert-runtime")
        if lockFD >= 0 { flock(lockFD, LOCK_UN); close(lockFD); lockFD = -1 }
    }

    private func acquireOwnership() -> Bool {
        let path = persistence.directory.appendingPathComponent("alert_runtime.lock").path
        let fd = open(path, O_CREAT | O_RDWR, S_IRUSR | S_IWUSR)
        guard fd >= 0, flock(fd, LOCK_EX | LOCK_NB) == 0 else { if fd >= 0 { close(fd) }; return false }
        lockFD = fd; return true
    }

    private func tick() async {
        guard let engine else { return }
        var changed = false
        for (url, command) in persistence.pendingCommands() {
            if case .requestNotificationAuthorization = command.payload { await requestNotificationAuthorization() }
            await engine.apply(command); persistence.acknowledge(url); changed = true
        }
        if changed { await refreshSubscriptions() }
        await deliverPending()
        var snapshot = await engine.currentSnapshot()
        var health = snapshot.health
        health.owner = role == .agent ? .agent : .app
        health.heartbeat = Date()
        health.providers = providerHealth.values.sorted { $0.source.rawValue < $1.source.rawValue }
        snapshot.health = health
        await engine.replaceHealth(health)
    }

    private func requestNotificationAuthorization() async {
        let center = UNUserNotificationCenter.current()
        _ = try? await center.requestAuthorization(options: [.alert, .sound])
        let settings = await center.notificationSettings()
        guard let engine else { return }
        var health = (await engine.currentSnapshot()).health
        switch settings.authorizationStatus {
        case .notDetermined: health.notificationPermission = .notDetermined
        case .denied: health.notificationPermission = .denied
        case .authorized: health.notificationPermission = .authorized
        case .provisional: health.notificationPermission = .provisional
        case .ephemeral: health.notificationPermission = .ephemeral
        @unknown default: health.notificationPermission = .unknown
        }
        await engine.replaceHealth(health)
    }

    private func refreshSubscriptions() async {
        guard let engine else { return }
        await MarketQuoteCoordinator.shared.subscribe(owner: "price-alert-runtime", assets: await engine.activeAssets())
    }

    private func receive(_ quote: MarketQuote) async {
        guard let engine else { return }
        var health = providerHealth[quote.asset.source] ?? AlertProviderHealth(source: quote.asset.source)
        health.lastSuccessfulQuote = Date(); health.lastError = nil; health.retryAt = nil
        providerHealth[quote.asset.source] = health
        let snapshot = await engine.currentSnapshot()
        for currency in Set(snapshot.alerts.filter { $0.asset.key == quote.asset.key }.map(\.currency)) {
            guard let rate = await FXRateService.shared.rate(from: quote.currency, to: currency) else { continue }
            _ = await engine.process(quote, convertedPrice: quote.price * rate, currency: currency)
        }
        await deliverPending()
        await refreshSubscriptions()
    }

    private func deliverPending() async {
        guard let engine else { return }
        let snapshot = await engine.currentSnapshot()
        for event in snapshot.history where event.delivery.state == .pending || (event.delivery.state == .failed && (event.delivery.nextRetryAt ?? .distantPast) <= Date()) {
            var record = event.delivery
            guard snapshot.settings.deliveryEnabled && snapshot.settings.macOSNotificationsEnabled else {
                record.state = .suppressed; await engine.markDelivery(eventID: event.id, record: record); continue
            }
            record.attemptCount += 1; record.lastAttemptAt = Date()
            let content = UNMutableNotificationContent()
            content.title = event.origin == .catchUp ? "Delayed \(event.asset.symbol) price alert" : "\(event.asset.symbol) price alert"
            content.body = "Reached \(event.target) \(event.currency.rawValue)"
            content.userInfo = ["alertID": event.alertID.uuidString, "eventID": event.id.uuidString]
            if snapshot.settings.soundEnabled { content.sound = .default }
            do {
                try await UNUserNotificationCenter.current().add(UNNotificationRequest(identifier: event.id.uuidString, content: content, trigger: nil))
                record.state = .delivered; record.error = nil; record.nextRetryAt = nil
            } catch {
                record.state = record.attemptCount >= 5 ? .suppressed : .failed
                record.error = String(describing: error)
                record.nextRetryAt = Date().addingTimeInterval(min(pow(2, Double(record.attemptCount)) * 15, 900))
            }
            await engine.markDelivery(eventID: event.id, record: record)
        }
    }
}

actor AlertRuntimeClient {
    static let shared = AlertRuntimeClient()
    private let persistence: AlertRuntimePersistence
    init(persistence: AlertRuntimePersistence = .shared) { self.persistence = persistence }
    func snapshot() -> AlertPersistenceSnapshot { persistence.loadSnapshot() ?? AlertPersistenceSnapshot() }
    func send(_ payload: AlertRuntimeCommandPayload, expectedRevision: UInt64? = nil) throws -> UUID {
        let command = AlertRuntimeCommand(expectedRevision: expectedRevision, payload: payload)
        try persistence.enqueue(command); return command.id
    }
}
