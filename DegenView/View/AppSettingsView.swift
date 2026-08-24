import SwiftUI
import ServiceManagement

enum SettingsTab: String {
    case appearance
    case alpaca
    case notifications
}

struct AppSettingsView: View {
    @AppStorage("settingsTab") private var selectedTab: SettingsTab = .appearance

    var body: some View {
        TabView(selection: $selectedTab) {
            AppearanceSettingsView()
                .tabItem { Label("Appearance", systemImage: "paintbrush") }
                .tag(SettingsTab.appearance)
            AlpacaSettingsView()
                .tabItem { Label("Alpaca", systemImage: "chart.xyaxis.line") }
                .tag(SettingsTab.alpaca)
            NotificationSettingsView()
                .tabItem { Label("Notifications", systemImage: "bell") }
                .tag(SettingsTab.notifications)
        }
        .frame(width: 560, height: 500)
        .padding(20)
    }
}

private struct NotificationSettingsView: View {
    @StateObject private var store = AlertStore.shared
    var body: some View {
        Form {
            Toggle("Price alert delivery", isOn: setting(\.deliveryEnabled))
            Toggle("Sound", isOn: setting(\.soundEnabled))
            Toggle("In-app banners", isOn: setting(\.inAppBannersEnabled))
            Toggle("macOS notifications", isOn: setting(\.macOSNotificationsEnabled))
            Section("Background agent") {
                LabeledContent("Status", value: serviceLabel)
                LabeledContent("Notification permission", value: notificationPermissionLabel)
                if let heartbeat = store.health.heartbeat { LabeledContent("Last heartbeat") { Text(heartbeat, style: .relative) } }
                ForEach(store.health.providers) { provider in
                    LabeledContent(provider.source.displayName) {
                        Text(providerLabel(provider))
                            .foregroundStyle(provider.lastError == nil ? Color.secondary : Color.red)
                    }
                }
                if !store.health.unreconciledGaps.isEmpty {
                    Label("\(store.health.unreconciledGaps.count) unreconciled data gap(s)", systemImage: "exclamationmark.triangle").foregroundStyle(.orange)
                }
                HStack {
                    Button("Retry Registration") { Task { await store.retryBackgroundService() } }
                    Button("Open Login Items Settings") { SMAppService.openSystemSettingsLoginItems() }
                }
            }
            Text("If the background item is disabled or unavailable, alerts continue evaluating while DegenView is open.").font(.caption).foregroundStyle(.secondary)
            Button("Open System Notification Settings") { NSWorkspace.shared.open(URL(string: "x-apple.systempreferences:com.apple.Notifications-Settings.extension")!) }
        }.padding(.top, 12)
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
        Binding(get: { store.settings[keyPath: path] }, set: { value in var settings = store.settings; settings[keyPath: path] = value; Task { await store.updateSettings(settings) } })
    }
}

private struct AppearanceSettingsView: View {
    @AppStorage("appTheme") private var appTheme: AppTheme = .system

    var body: some View {
        Form {
            Picker("Appearance", selection: $appTheme) {
                ForEach(AppTheme.allCases) { theme in
                    Label(theme.rawValue, systemImage: theme.icon).tag(theme)
                }
            }
            .pickerStyle(.segmented)
        }
        .padding(.top, 24)
    }
}

private struct AlpacaSettingsView: View {
    @State private var keyID = AlpacaCredentialsStore.credentials.keyID
    @State private var secretKey = AlpacaCredentialsStore.credentials.secretKey
    @State private var status: String?

    var body: some View {
        Form {
            Section {
                TextField("API Key ID", text: $keyID)
                SecureField("Secret Key", text: $secretKey)
            }
            Section {
                Text("Create a free Alpaca account, open the API Keys section in the dashboard, then generate or regenerate a key. Copy both values here; Alpaca only shows the secret once.")
                    .foregroundStyle(.secondary)
                Link("Open the Alpaca dashboard", destination: URL(string: "https://app.alpaca.markets/")!)
            }
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
}
