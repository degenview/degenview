import ServiceManagement
import SwiftUI

enum SettingsTab: String, CaseIterable, Identifiable {
    case appearance
    case alpaca
    case coinMarketCap
    case notifications

    var id: Self { self }

    var title: String {
        switch self {
        case .appearance: "Appearance"
        case .alpaca: "Alpaca"
        case .coinMarketCap: "CoinMarketCap"
        case .notifications: "Notifications"
        }
    }

    var systemImage: String {
        switch self {
        case .appearance: "paintbrush"
        case .alpaca: "chart.xyaxis.line"
        case .coinMarketCap: "gauge.with.dots.needle.50percent"
        case .notifications: "bell"
        }
    }
}

struct AppSettingsView: View {
    @AppStorage("settingsTab") private var selectedTab: SettingsTab = .appearance

    var body: some View {
        HStack(spacing: 0) {
            List(SettingsTab.allCases, selection: $selectedTab) { tab in
                Label(tab.title, systemImage: tab.systemImage)
                    .tag(tab)
            }
            .listStyle(.sidebar)
            .frame(width: 180)

            Divider()

            selectedContent
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                .padding(.horizontal, 20)
                .padding(.bottom, 20)
        }
        .frame(width: 740, height: 520)
    }

    @ViewBuilder
    private var selectedContent: some View {
        switch selectedTab {
        case .appearance:
            AppearanceSettingsView()
        case .alpaca:
            AlpacaSettingsView()
        case .coinMarketCap:
            CoinMarketCapSettingsView()
        case .notifications:
            NotificationSettingsView()
        }
    }
}

private struct CoinMarketCapSettingsView: View {
    @State private var apiKey = ""
    @State private var configured = CoinMarketCapCredentialStore.isConfigured
    @State private var status: String?
    @State private var isTesting = false

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            SettingsPageHeader(
                title: "CoinMarketCap",
                subtitle: "Configure market sentiment access and optional higher API rate limits."
            )

            VStack(alignment: .leading, spacing: 12) {
                VStack(alignment: .leading, spacing: 10) {
                    Text("API Access").font(.subheadline.weight(.semibold))
                    SecureField("API Key", text: $apiKey)
                    LabeledContent("Status", value: configured ? "API key configured" : "Not configured")
                }
                .settingsCard()

                VStack(alignment: .leading, spacing: 8) {
                    Text(
                        "CoinMarketCap can be used without an API key. Adding a CoinMarketCap API key gives this app access to higher API rate limits."
                    )
                    .foregroundStyle(.secondary)
                    Link("Get API Key", destination: URL(string: "https://pro.coinmarketcap.com/signup/")!)
                }
                .settingsCard()

                HStack {
                    if let status { Text(status).font(.caption).foregroundStyle(.secondary) }
                    Spacer()
                    Button("Remove API Key", role: .destructive) {
                        do {
                            try CoinMarketCapCredentialStore.remove()
                            apiKey = ""
                            configured = false
                            status = "Removed. Public API mode is active."
                        } catch { status = "Could not remove the key: \(error.localizedDescription)" }
                    }.disabled(!configured)
                    Button("Test Connection") {
                        isTesting = true
                        Task {
                            do {
                                _ = try await CoinMarketCapDataProvider.shared.fearGreedLatest(force: true)
                                status = "Connection successful."
                            } catch { status = error.localizedDescription }
                            isTesting = false
                        }
                    }.disabled(isTesting)
                    Button("Save") {
                        do {
                            try CoinMarketCapCredentialStore.save(apiKey)
                            configured = true
                            apiKey = ""
                            status = "Saved securely in Keychain."
                        } catch { status = "Could not save: \(error.localizedDescription)" }
                    }.buttonStyle(.borderedProminent).disabled(
                        apiKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            }
        }
        .padding(.horizontal, 4)
        .padding(.top, 16)
    }
}

private struct NotificationSettingsView: View {
    @StateObject private var store = AlertStore.shared

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                SettingsPageHeader(
                    title: "Notifications",
                    subtitle: "Choose how DegenView delivers price alerts and monitor background delivery."
                )

                SettingsToggleRow(
                    title: "Price Alert Delivery",
                    subtitle: "Evaluate armed alerts and deliver them when their conditions are met.",
                    icon: "bell.badge.fill",
                    isOn: setting(\.deliveryEnabled)
                )
                SettingsToggleRow(
                    title: "Sound",
                    subtitle: "Play an alert sound when a notification is delivered.",
                    icon: "speaker.wave.2.fill",
                    isOn: setting(\.soundEnabled)
                )
                SettingsToggleRow(
                    title: "In-App Banners",
                    subtitle: "Show alerts inside DegenView while the app is open.",
                    icon: "rectangle.topthird.inset.filled",
                    isOn: setting(\.inAppBannersEnabled)
                )
                SettingsToggleRow(
                    title: "macOS Notifications",
                    subtitle: "Send alerts through Notification Center, including in the background.",
                    icon: "macwindow.badge.plus",
                    isOn: setting(\.macOSNotificationsEnabled)
                )

                VStack(alignment: .leading, spacing: 10) {
                    Label("Background Agent", systemImage: "bolt.horizontal.circle.fill")
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(.primary)

                    Divider()

                    LabeledContent("Status", value: serviceLabel)
                    LabeledContent("Notification permission", value: notificationPermissionLabel)
                    if let heartbeat = store.health.heartbeat {
                        LabeledContent("Last heartbeat") { Text(heartbeat, style: .relative) }
                    }
                    ForEach(store.health.providers) { provider in
                        LabeledContent(provider.source.displayName) {
                            Text(providerLabel(provider))
                                .foregroundStyle(provider.lastError == nil ? Color.secondary : Color.red)
                        }
                    }
                    if !store.health.unreconciledGaps.isEmpty {
                        Label(
                            "\(store.health.unreconciledGaps.count) unreconciled data gap(s)",
                            systemImage: "exclamationmark.triangle"
                        ).foregroundStyle(.orange)
                    }
                    HStack {
                        Button("Retry Registration") { Task { await store.retryBackgroundService() } }
                        Button("Open Login Items Settings") { SMAppService.openSystemSettingsLoginItems() }
                    }
                }
                .settingsCard()

                Text(
                    "If the background item is disabled or unavailable, alerts continue evaluating while DegenView is open."
                )
                .font(.caption)
                .foregroundStyle(.secondary)

                Button("Open System Notification Settings") {
                    NSWorkspace.shared.open(
                        URL(string: "x-apple.systempreferences:com.apple.Notifications-Settings.extension")!)
                }
            }
            .padding(.horizontal, 4)
            .padding(.top, 16)
        }
    }

    private var serviceLabel: String {
        switch store.health.serviceState {
        case .enabled: "Running in background"
        case .requiresApproval: "Needs approval in Login Items"
        case .disabled: "Foreground only"
        case .notFound: "Agent not embedded"
        case .failed: "Registration failed"
        case .unavailable: "Unavailable"
        }
    }
    private func providerLabel(_ provider: AlertProviderHealth) -> String {
        if let error = provider.lastError { return error }
        return provider.lastSuccessfulQuote == nil ? "Waiting for data" : "Healthy"
    }
    private var notificationPermissionLabel: String {
        switch store.health.notificationPermission {
        case .authorized: "Authorized"
        case .provisional: "Provisional"
        case .ephemeral: "Ephemeral"
        case .denied: "Denied"
        case .notDetermined: "Not requested"
        case .unknown: "Unknown"
        }
    }
    private func setting(_ path: WritableKeyPath<AlertNotificationSettings, Bool>) -> Binding<Bool> {
        Binding(
            get: { store.settings[keyPath: path] },
            set: { value in
                var settings = store.settings
                settings[keyPath: path] = value
                Task { await store.updateSettings(settings) }
            })
    }
}

private struct AppearanceSettingsView: View {
    @AppStorage("appTheme") private var appTheme: AppTheme = .system

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            SettingsPageHeader(
                title: "Appearance",
                subtitle: "Choose how DegenView looks across every window and chart."
            )

            VStack(alignment: .leading, spacing: 12) {
                Label("App Theme", systemImage: "paintbrush.fill")
                    .font(.subheadline.weight(.semibold))

                Picker("Appearance", selection: $appTheme) {
                    ForEach(AppTheme.allCases) { theme in
                        Label(theme.rawValue, systemImage: theme.icon).tag(theme)
                    }
                }
                .labelsHidden()
                .pickerStyle(.segmented)
            }
            .settingsCard()

            Spacer()
        }
        .padding(.horizontal, 4)
        .padding(.top, 16)
    }
}

private struct SettingsPageHeader: View {
    let title: String
    let subtitle: String

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title)
                .font(.title3.weight(.semibold))
            Text(subtitle)
                .font(.subheadline)
                .foregroundStyle(.secondary)
        }
        .padding(.bottom, 4)
    }
}

private struct SettingsToggleRow: View {
    let title: String
    let subtitle: String
    let icon: String
    @Binding var isOn: Bool

    var body: some View {
        HStack(spacing: 14) {
            Image(systemName: icon)
                .font(.system(size: 14, weight: .semibold))
                .foregroundStyle(.tint)
                .frame(width: 30, height: 30)
                .background(.tint.opacity(0.12), in: RoundedRectangle(cornerRadius: 8))

            VStack(alignment: .leading, spacing: 3) {
                Text(title)
                    .font(.subheadline.weight(.semibold))
                Text(subtitle)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }

            Spacer(minLength: 16)

            Toggle(title, isOn: $isOn)
                .labelsHidden()
                .toggleStyle(.switch)
                .fixedSize()
        }
        .settingsCard()
    }
}

private extension View {
    func settingsCard() -> some View {
        padding(.horizontal, 14)
            .padding(.vertical, 12)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(.quaternary.opacity(0.45), in: RoundedRectangle(cornerRadius: 12))
            .overlay {
                RoundedRectangle(cornerRadius: 12)
                    .stroke(.separator.opacity(0.45), lineWidth: 1)
            }
    }
}

private struct AlpacaSettingsView: View {
    @State private var keyID = AlpacaCredentialsStore.credentials.keyID
    @State private var secretKey = AlpacaCredentialsStore.credentials.secretKey
    @State private var status: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            SettingsPageHeader(
                title: "Alpaca",
                subtitle: "Connect Alpaca to access stock market data in DegenView."
            )

            VStack(alignment: .leading, spacing: 12) {
                VStack(alignment: .leading, spacing: 10) {
                    Text("API Access").font(.subheadline.weight(.semibold))
                    TextField("API Key ID", text: $keyID)
                    SecureField("Secret Key", text: $secretKey)
                }
                .settingsCard()

                VStack(alignment: .leading, spacing: 8) {
                    Text(
                        "Create a free Alpaca account, open the API Keys section in the dashboard, then generate or regenerate a key. Copy both values here; Alpaca only shows the secret once."
                    )
                    .foregroundStyle(.secondary)
                    Link("Open the Alpaca dashboard", destination: URL(string: "https://app.alpaca.markets/")!)
                }
                .settingsCard()

                HStack {
                    if let status { Text(status).font(.caption).foregroundStyle(.secondary) }
                    Spacer()
                    Button("Save") {
                        do {
                            try AlpacaCredentialsStore.save(.init(keyID: keyID, secretKey: secretKey))
                            status = "Saved securely in Keychain."
                        } catch { status = "Could not save: \(error.localizedDescription)" }
                    }
                    .buttonStyle(.borderedProminent)
                    .disabled(keyID.trimmingCharacters(in: .whitespaces).isEmpty || secretKey.isEmpty)
                }
            }
        }
        .padding(.horizontal, 4)
        .padding(.top, 16)
    }
}
