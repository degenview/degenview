import SwiftUI

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
        .frame(width: 520, height: 330)
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
            Text("Evaluation and trigger history continue when delivery is off. Alerts run locally while DegenView is open.").font(.caption).foregroundStyle(.secondary)
            Button("Open System Notification Settings") { NSWorkspace.shared.open(URL(string: "x-apple.systempreferences:com.apple.Notifications-Settings.extension")!) }
        }.padding(.top, 12)
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
