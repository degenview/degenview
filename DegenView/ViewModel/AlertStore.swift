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
    private var engine: LocalPriceAlertEngine?

    private init() {
        Task {
            let engine = await LocalPriceAlertEngine()
            self.engine = engine
            await MarketQuoteCoordinator.shared.setQuoteHandler { [weak self] quote in await self?.receive(quote) }
            await reload()
            await syncSubscriptions()
        }
    }

    var activeCount: Int { alerts.filter { $0.state == .active }.count }
    func alerts(for assetKey: String) -> [PriceAlert] { alerts.filter { $0.asset.key == assetKey } }
    func latestPrice(for asset: PortfolioAsset, currency: PortfolioCurrency) async -> Decimal? {
        guard let quote = latestQuotes[asset.key], quote.isFresh,
              let rate = await FXRateService.shared.rate(from: quote.currency, to: currency) else { return nil }
        return quote.price * rate
    }

    func save(_ alert: PriceAlert) async {
        guard let engine else { return }
        let baseline = await latestPrice(for: alert.asset, currency: alert.currency)
        await engine.saveAlert(alert, baseline: baseline); await reload(); await syncSubscriptions()
        if !UserDefaults.standard.bool(forKey: "didExplainPriceAlerts") {
            UserDefaults.standard.set(true, forKey: "didExplainPriceAlerts")
            _ = try? await UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound])
        }
    }
    func pause(_ id: UUID) async { await changeState(id, .paused) }
    func resume(_ id: UUID) async { await changeState(id, .active) }
    func reenable(_ id: UUID) async { await changeState(id, .active) }
    func delete(_ id: UUID) async { await engine?.delete(id); await reload(); await syncSubscriptions() }
    func clearHistory() async { await engine?.clearHistory(); await reload() }
    func updateSettings(_ value: AlertNotificationSettings) async { settings = value; await engine?.updateSettings(value); await reload() }
    func isIdentical(_ alert: PriceAlert) async -> Bool { await engine?.isIdentical(alert) ?? false }

    private func changeState(_ id: UUID, _ state: AlertState) async {
        guard let alert = alerts.first(where: { $0.id == id }) else { return }
        let baseline = state == .active ? await latestPrice(for: alert.asset, currency: alert.currency) : nil
        await engine?.setState(id, state: state, baseline: baseline); await reload(); await syncSubscriptions()
    }
    private func reload() async {
        guard let value = await engine?.currentSnapshot() else { return }
        alerts = value.alerts; history = value.history.sorted { $0.timestamp > $1.timestamp }; settings = value.settings
    }
    private func syncSubscriptions() async { await MarketQuoteCoordinator.shared.subscribe(owner: "price-alerts", assets: alerts.filter { $0.state == .active }.map(\.asset)) }
    private func receive(_ quote: MarketQuote) async {
        latestQuotes[quote.asset.key] = quote
        guard let engine else { return }
        for currency in Set(alerts.filter { $0.asset.key == quote.asset.key }.map(\.currency)) {
            guard let rate = await FXRateService.shared.rate(from: quote.currency, to: currency) else { continue }
            let events = await engine.process(quote, convertedPrice: quote.price * rate, currency: currency)
            for event in events { await deliver(event) }
        }
        await reload()
    }
    private func deliver(_ event: AlertTriggerEvent) async {
        guard settings.deliveryEnabled else { return }
        if settings.inAppBannersEnabled { bannerEvent = event }
        guard settings.macOSNotificationsEnabled else { return }
        let content = UNMutableNotificationContent()
        content.title = "\(event.asset.symbol) price alert"
        content.body = "Reached \(event.target) \(event.currency.rawValue)"
        content.userInfo = ["assetID": event.asset.key, "alertID": event.alertID.uuidString]
        if settings.soundEnabled { content.sound = .default }
        try? await UNUserNotificationCenter.current().add(UNNotificationRequest(identifier: event.id.uuidString, content: content, trigger: nil))
    }
}
