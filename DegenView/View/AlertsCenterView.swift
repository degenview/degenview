import AppKit
import SwiftUI

/// The Alerts window: every price alert by state, what has fired, and the Pine script alerts.
struct AlertsCenterView: View {
    enum Filter: String, CaseIterable {
        case active = "Active"
        case triggered = "Triggered"
        case paused = "Paused"
        case all = "All"
        case history = "History"
        case script = "Scripts"

        var systemImage: String {
            switch self {
            case .active: "bell.badge"
            case .triggered: "checkmark.circle"
            case .paused: "pause.circle"
            case .all: "list.bullet"
            case .history: "clock.arrow.circlepath"
            case .script: "curlybraces"
            }
        }
    }

    @StateObject private var store = AlertStore.shared
    @StateObject private var pineStore = PineAlertStore.shared
    @StateObject private var info = PortfolioAssetInfoViewModel()
    @StateObject private var unseen = UnseenAlertsStore.shared
    /// This view's window, kept so key-window changes can be matched to it.
    @State private var hostWindow = WeakWindow()
    @AppStorage("appTheme") private var appTheme: AppTheme = .system
    @State private var filter: Filter = .active
    @State private var search = ""
    @State private var editing: PriceAlert?
    @State private var pendingDelete: PriceAlert?
    @State private var confirmingClear = false
    /// The script alert the window was asked to show; `PineAlertListView` consumes it.
    @State private var focusedScriptAlert: UUID?

    var body: some View {
        VStack(spacing: 0) {
            header
            if store.health.notificationPermission == .denied { notificationsOffNotice }
            Divider()
            content
        }
        .frame(minWidth: 760, minHeight: 480)
        .background(Color(nsColor: .windowBackgroundColor))
        .background(
            WindowAccessor {
                hostWindow.window = $0
                if $0.isKeyWindow { unseen.windowDidBecomeActive() }
            }
        )
        .onReceive(NotificationCenter.default.publisher(for: NSWindow.didBecomeKeyNotification)) { note in
            if let window = note.object as? NSWindow, window === hostWindow.window { unseen.windowDidBecomeActive() }
        }
        .onReceive(NotificationCenter.default.publisher(for: NSWindow.didResignKeyNotification)) { note in
            if let window = note.object as? NSWindow, window === hostWindow.window { unseen.windowDidResign() }
        }
        .onReceive(NotificationCenter.default.publisher(for: NSWindow.willCloseNotification)) { note in
            if let window = note.object as? NSWindow, window === hostWindow.window { unseen.windowDidResign() }
        }
        .task(id: assetKeys) { info.load(assets) }
        .onAppear {
            if hostWindow.window?.isKeyWindow == true { unseen.windowDidBecomeActive() }
            if let id = WindowCoordinator.shared.takePendingAlertsFocus() { show(scriptAlert: id) }
        }
        .onReceive(NotificationCenter.default.publisher(for: .showScriptAlertInCenter)) { note in
            guard let id = note.object as? UUID else { return }
            // Delivered here, so a later fresh window must not replay it from the parked request.
            _ = WindowCoordinator.shared.takePendingAlertsFocus()
            show(scriptAlert: id)
        }
        .sheet(item: $editing) { PriceAlertEditor(asset: $0.asset, existing: $0) }
        .confirmationDialog(
            "Delete this alert?",
            isPresented: Binding(get: { pendingDelete != nil }, set: { if !$0 { pendingDelete = nil } }),
            presenting: pendingDelete
        ) { alert in
            Button("Delete", role: .destructive) { Task { await store.delete(alert.id) } }
        } message: { alert in
            Text("The \(alert.asset.displayTicker) alert is removed. Its trigger history is kept.")
        }
        .confirmationDialog("Clear all trigger history?", isPresented: $confirmingClear) {
            Button("Clear History", role: .destructive) { Task { await store.clearHistory() } }
        } message: {
            Text("This can't be undone. Your alerts are kept.")
        }
        .preferredColorScheme(appTheme.colorScheme)
    }

    /// Switches to the Scripts tab, clears any search that would hide it, and points at one alert.
    private func show(scriptAlert id: UUID) {
        search = ""
        filter = .script
        focusedScriptAlert = id
    }

    // MARK: - Header

    private var header: some View {
        VStack(spacing: 14) {
            HStack(spacing: 14) {
                Image(systemName: "bell.badge")
                    .font(.system(size: 20, weight: .semibold))
                    .foregroundStyle(.tint)
                    .frame(width: 44, height: 44)
                    .background(
                        Color.accentColor.opacity(0.12), in: RoundedRectangle(cornerRadius: 12, style: .continuous)
                    )
                    .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: 2) {
                    Text("Alerts").font(.title2.weight(.semibold))
                    Text(summary).font(.subheadline).foregroundStyle(.secondary)
                }
                Spacer(minLength: 12)
                runtimeBadge
                PortfolioSearchField(text: $search, prompt: "Search asset or name")
            }
            IconTabBar(items: tabs, selection: $filter, isCompact: true, fillsWidth: true)
        }
        .padding(.horizontal, 20)
        .padding(.top, 18)
        .padding(.bottom, 14)
    }

    private var tabs: [IconTabBar<Filter>.Item] {
        Filter.allCases.map { tab in
            .init(value: tab, title: tab.rawValue, systemImage: tab.systemImage, count: count(of: tab))
        }
    }

    private func count(of tab: Filter) -> Int? {
        switch tab {
        case .active: store.alerts.filter { $0.state == .active }.count
        case .triggered: store.alerts.filter { $0.state == .triggered }.count
        case .paused: store.alerts.filter { $0.state == .paused }.count
        case .all: store.alerts.count
        case .history: store.history.count
        case .script: pineStore.subscriptions.count
        }
    }

    private var summary: String {
        let parts = [(Filter.active, "active"), (.triggered, "triggered"), (.paused, "paused")]
            .compactMap { tab, word in count(of: tab).flatMap { $0 > 0 ? "\($0) \(word)" : nil } }
        return parts.isEmpty ? "No price alerts yet" : parts.joined(separator: " · ")
    }

    private var runtimeBadge: some View {
        let background = store.health.serviceState == .enabled
        return SettingsStatusBadge(
            text: background ? "Background agent active" : "Foreground evaluation only",
            tone: background ? .good : .warning
        )
        .help(
            background
                ? "Alerts keep being checked when DegenView is closed."
                : "Alerts are only checked while DegenView is open.")
    }

    private var notificationsOffNotice: some View {
        NoticeCard(
            systemImage: "bell.slash.fill", tint: .orange, title: "Notifications are turned off",
            detail:
                "Alerts still fire and are logged here, but macOS will not show them. Allow DegenView in System Settings.",
            actionTitle: "Open Settings"
        ) {
            if let url = URL(string: "x-apple.systempreferences:com.apple.Notifications-Settings.extension") {
                NSWorkspace.shared.open(url)
            }
        }
        .padding(.horizontal, 20)
        .padding(.bottom, 14)
    }

    // MARK: - Content

    @ViewBuilder private var content: some View {
        switch filter {
        case .history: historyContent
        case .script: PineAlertListView(search: search, focused: $focusedScriptAlert)
        default: ruleContent
        }
    }

    private var ruleContent: some View {
        let fired = lastFiredByRule
        return AlertCardScroll {
            ForEach(filteredRules) { alert in
                AlertRuleRow(
                    alert: alert, info: info, quote: store.latestQuotes[alert.asset.key], lastFired: fired[alert.id],
                    onEdit: { editing = alert },
                    onToggle: { Task { await toggle(alert) } },
                    onDuplicate: { duplicate(alert) },
                    onDelete: { pendingDelete = alert })
            }
        }
        .overlay {
            if filteredRules.isEmpty { emptyState(hasAnyInTab: store.alerts.contains(where: matchesTab)) }
        }
    }

    private var historyContent: some View {
        let byID = Dictionary(store.alerts.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
        return VStack(spacing: 0) {
            AlertHistoryList(events: filteredHistory, info: info, rules: byID)
                .overlay {
                    if filteredHistory.isEmpty { emptyState(hasAnyInTab: !store.history.isEmpty) }
                }
            historyFooter
        }
    }

    private var historyFooter: some View {
        let count = store.history.count
        return AlertFooterBar(
            systemImage: "clock.arrow.circlepath",
            note: count == 0
                ? "Every time an alert fires it is logged here."
                : "\(count) \(count == 1 ? "trigger" : "triggers") logged. Clearing history keeps your alerts."
        ) {
            Button("Clear History", systemImage: "trash") { confirmingClear = true }
                .disabled(count == 0)
                .help("Remove every logged trigger, not just the ones matching the search")
        }
    }

    @ViewBuilder private func emptyState(hasAnyInTab: Bool) -> some View {
        if hasAnyInTab && !search.isEmpty {
            ContentUnavailableView.search(text: search)
        } else {
            switch filter {
            case .active:
                ContentUnavailableView(
                    "No Active Alerts", systemImage: "bell.slash",
                    description: Text("Click the bell on a chart to alert on a price."))
            case .triggered:
                ContentUnavailableView(
                    "Nothing Triggered", systemImage: "checkmark.circle",
                    description: Text("An alert that fires once waits here until you re-enable it."))
            case .paused:
                ContentUnavailableView(
                    "Nothing Paused", systemImage: "pause.circle",
                    description: Text("Paused alerts wait here until you resume them."))
            case .all, .script:
                ContentUnavailableView(
                    "No Alerts", systemImage: "bell",
                    description: Text("Click the bell on a chart to alert on a price."))
            case .history:
                ContentUnavailableView(
                    "No Trigger History", systemImage: "clock.arrow.circlepath",
                    description: Text("Every time an alert fires it is logged here."))
            }
        }
    }

    // MARK: - Data

    private var rules: [PriceAlert] {
        store.alerts.filter(matchesSearch).sorted { $0.updatedAt > $1.updatedAt }
    }

    /// The rules of this tab, before the search narrows them.
    private var filteredRules: [PriceAlert] { rules.filter(matchesTab) }

    private var filteredHistory: [AlertTriggerEvent] {
        store.history.filter { matchesSearch($0.asset) }
    }

    private var lastFiredByRule: [UUID: AlertTriggerEvent] {
        Dictionary(
            store.history.map { ($0.alertID, $0) }, uniquingKeysWith: { a, b in a.timestamp > b.timestamp ? a : b })
    }

    private func matchesTab(_ alert: PriceAlert) -> Bool {
        switch filter {
        case .active: alert.state == .active
        case .triggered: alert.state == .triggered
        case .paused: alert.state == .paused
        case .all: true
        case .history, .script: false
        }
    }

    private func matchesSearch(_ alert: PriceAlert) -> Bool { matchesSearch(alert.asset) }

    private func matchesSearch(_ asset: PortfolioAsset) -> Bool {
        search.isEmpty
            || [asset.symbol, asset.name, asset.displayTicker, info.subtitle(for: asset)].contains {
                $0.localizedCaseInsensitiveContains(search)
            }
    }

    /// Every asset the lists show, once: the rules' and the history's.
    private var assets: [PortfolioAsset] {
        var byKey: [String: PortfolioAsset] = [:]
        for asset in store.alerts.map(\.asset) + store.history.map(\.asset) where byKey[asset.key] == nil {
            byKey[asset.key] = asset
        }
        return Array(byKey.values)
    }

    private var assetKeys: [String] { assets.map(\.key).sorted() }

    // MARK: - Actions

    private func toggle(_ alert: PriceAlert) async {
        if alert.state == .active {
            await store.pause(alert.id)
        } else {
            await store.resume(alert.id)
        }
    }

    private func duplicate(_ alert: PriceAlert) {
        store.save(
            PriceAlert(
                asset: alert.asset, condition: alert.condition, currency: alert.currency,
                frequency: alert.frequency, note: alert.note,
                webhookEndpointIDs: alert.webhookEndpointIDs, webhookMessage: alert.webhookMessage))
    }
}

/// Holds a window without retaining it.
private final class WeakWindow {
    weak var window: NSWindow?
}
