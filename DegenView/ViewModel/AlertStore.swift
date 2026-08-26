import AppKit
import Combine
import Foundation
import UserNotifications

@MainActor
final class AlertStore: ObservableObject {
    static let shared = AlertStore()
    @Published private(set) var alerts: [PriceAlert] = []
    @Published private(set) var history: [AlertTriggerEvent] = []
    @Published var settings = AlertNotificationSettings()
    @Published var bannerEvent: AlertTriggerEvent?
    @Published private(set) var latestQuotes: [String: MarketQuote] = [:]
    @Published private(set) var health = AlertRuntimeHealth()
    private let client = AlertRuntimeClient.shared
    private var fallbackHost: AlertRuntimeHost?
    private var observerTask: Task<Void, Never>?
    private let sessionStartedAt = Date()
    private var seenEventIDs: Set<UUID> = []
    private var snapshotRevision: UInt64 = 0

    private init() {
        observerTask = Task { [weak self] in
            await self?.reload()
            await self?.configureRuntime()
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(1))
                await self?.reload()
            }
        }
        NotificationCenter.default.addObserver(
            forName: NSApplication.didBecomeActiveNotification, object: nil, queue: .main
        ) { [weak self] _ in
            Task { @MainActor in
                await self?.reload()
                await self?.configureRuntime()
            }
        }
        NSWorkspace.shared.notificationCenter.addObserver(
            forName: NSWorkspace.didWakeNotification, object: nil, queue: .main
        ) { [weak self] _ in
            Task { @MainActor in await self?.reload() }
        }
    }

    var activeCount: Int { alerts.filter { $0.state == .active }.count }
    func alerts(for assetKey: String) -> [PriceAlert] { alerts.filter { $0.asset.key == assetKey } }
    func latestPrice(for asset: PortfolioAsset, currency: PortfolioCurrency) async -> Decimal? {
        let quote = await MarketQuoteCoordinator.shared.latestQuote(for: asset.key)
        guard let quote, quote.isFresh, let rate = await FXRateService.shared.rate(from: quote.currency, to: currency)
        else { return nil }
        return quote.price * rate
    }

    func save(_ alert: PriceAlert) async {
        let baseline = await latestPrice(for: alert.asset, currency: alert.currency)
        _ = try? await client.send(.save(alert, baseline: baseline), expectedRevision: snapshotRevision)
        _ = try? await client.send(.requestNotificationAuthorization)
        if alerts.isEmpty { _ = AlertBackgroundService.shared.register() }
        await requestNotificationAuthorizationIfNeeded()
        await configureRuntime()
        await awaitRevision()
    }
    func pause(_ id: UUID) async { await changeState(id, .paused) }
    func resume(_ id: UUID) async { await changeState(id, .active) }
    func reenable(_ id: UUID) async { await changeState(id, .active) }
    func delete(_ id: UUID) async {
        _ = try? await client.send(.delete(id), expectedRevision: snapshotRevision)
        await awaitRevision()
    }
    func clearHistory() async {
        _ = try? await client.send(.clearHistory, expectedRevision: snapshotRevision)
        await awaitRevision()
    }
    func updateSettings(_ value: AlertNotificationSettings) async {
        settings = value
        _ = try? await client.send(.updateSettings(value), expectedRevision: snapshotRevision)
        await awaitRevision()
    }
    func isIdentical(_ candidate: PriceAlert) async -> Bool {
        alerts.contains {
            $0.id != candidate.id && $0.asset.key == candidate.asset.key && $0.condition == candidate.condition
                && $0.currency == candidate.currency && $0.frequency == candidate.frequency
        }
    }
    func retryBackgroundService() async {
        _ = AlertBackgroundService.shared.register()
        await configureRuntime()
    }

    private func changeState(_ id: UUID, _ state: AlertState) async {
        guard let alert = alerts.first(where: { $0.id == id }) else { return }
        let baseline = state == .active ? await latestPrice(for: alert.asset, currency: alert.currency) : nil
        _ = try? await client.send(.setState(id, state, baseline: baseline), expectedRevision: snapshotRevision)
        await awaitRevision()
    }

    private func configureRuntime() async {
        let serviceState = AlertBackgroundService.shared.state
        if !alerts.isEmpty && serviceState == .disabled { _ = AlertBackgroundService.shared.register() }
        let currentState = AlertBackgroundService.shared.state
        if currentState == .enabled {
            await fallbackHost?.stop()
            fallbackHost = nil
        } else if fallbackHost == nil {
            let host = AlertRuntimeHost(role: .app)
            if await host.start() { fallbackHost = host }
        }
        health.serviceState = currentState
    }

    private func reload() async {
        let value = await client.snapshot()
        guard value.revision != snapshotRevision || (alerts.isEmpty && !value.alerts.isEmpty) else { return }
        let previousIDs = seenEventIDs
        snapshotRevision = value.revision
        alerts = value.alerts
        history = value.history.sorted { $0.timestamp > $1.timestamp }
        settings = value.settings
        health = value.health
        health.serviceState = AlertBackgroundService.shared.state
        seenEventIDs.formUnion(value.history.map(\.id))
        if settings.inAppBannersEnabled,
            let event = value.history.filter({ $0.timestamp >= sessionStartedAt && !previousIDs.contains($0.id) }).max(
                by: { $0.timestamp < $1.timestamp })
        {
            bannerEvent = event
        }
    }

    private func awaitRevision() async {
        let old = snapshotRevision
        for _ in 0..<30 {
            try? await Task.sleep(for: .milliseconds(100))
            await reload()
            if snapshotRevision > old { break }
        }
    }

    private func requestNotificationAuthorizationIfNeeded() async {
        guard !UserDefaults.standard.bool(forKey: "didExplainPriceAlerts") else { return }
        UserDefaults.standard.set(true, forKey: "didExplainPriceAlerts")
        _ = try? await UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound])
    }
}
