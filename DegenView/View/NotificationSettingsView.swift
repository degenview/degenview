import ServiceManagement
import SwiftUI

/// Settings ▸ Notifications: how price alerts are delivered, and the health of the background agent.
struct NotificationSettingsView: View {
    @StateObject private var store = AlertStore.shared

    var body: some View {
        SettingsPage(
            systemImage: "bell.badge.fill", title: "Notifications",
            subtitle: "Choose how DegenView delivers price alerts and check on background delivery."
        ) {
            SettingsSection(title: "Delivery") {
                SettingsCardRow(
                    title: "Price Alert Delivery", icon: "bell.badge.fill",
                    hint: "Evaluate armed alerts and deliver them when their conditions are met."
                ) { toggle("Price Alert Delivery", \.deliveryEnabled) }
                SettingsCardRow(
                    title: "Sound", icon: "speaker.wave.2.fill",
                    hint: "Play an alert sound when a notification is delivered."
                ) { toggle("Sound", \.soundEnabled) }
                SettingsCardRow(
                    title: "In-App Banners", icon: "rectangle.topthird.inset.filled",
                    hint: "Show alerts inside DegenView while the app is open."
                ) { toggle("In-App Banners", \.inAppBannersEnabled) }
                SettingsCardRow(
                    title: "macOS Notifications", icon: "macwindow.badge.plus",
                    hint: "Send alerts through Notification Center, including in the background."
                ) { toggle("macOS Notifications", \.macOSNotificationsEnabled) }
            }

            SettingsSection(
                title: "Background agent",
                subtitle: "Keeps alerts evaluating while DegenView is closed."
            ) {
                SettingsCard {
                    row("Status") {
                        SettingsStatusBadge(text: serviceLabel, tone: serviceTone)
                    }
                    Divider()
                    row("Notification permission") {
                        SettingsStatusBadge(text: notificationPermissionLabel, tone: permissionTone)
                    }
                    if let heartbeat = store.health.heartbeat {
                        Divider()
                        row("Last heartbeat") { Text(heartbeat, style: .relative).foregroundStyle(.secondary) }
                    }
                    ForEach(store.health.providers) { provider in
                        Divider()
                        row(provider.source.displayName) {
                            SettingsStatusBadge(
                                text: providerLabel(provider), tone: provider.lastError == nil ? .good : .bad)
                        }
                    }
                }

                if !store.health.unreconciledGaps.isEmpty {
                    NoticeCard(
                        systemImage: "exclamationmark.triangle.fill", tint: .orange,
                        title: "\(store.health.unreconciledGaps.count) unreconciled data "
                            + "\(store.health.unreconciledGaps.count == 1 ? "gap" : "gaps")")
                }

                HStack(spacing: 10) {
                    Button("Retry Registration") { Task { await store.retryBackgroundService() } }
                    Button("Login Items Settings") { SMAppService.openSystemSettingsLoginItems() }
                    Button("Notification Settings") {
                        NSWorkspace.shared.open(
                            URL(string: "x-apple.systempreferences:com.apple.Notifications-Settings.extension")!)
                    }
                }

                Text(
                    "If the background item is disabled or unavailable, alerts keep evaluating while DegenView is open."
                )
                .font(.caption)
                .foregroundStyle(.secondary)
            }
        }
    }

    private func row<Value: View>(_ title: String, @ViewBuilder value: () -> Value) -> some View {
        HStack {
            Text(title).font(.subheadline)
            Spacer()
            value()
        }
    }

    private func toggle(_ title: String, _ path: WritableKeyPath<AlertNotificationSettings, Bool>) -> some View {
        Toggle(title, isOn: setting(path))
            .labelsHidden()
            .toggleStyle(.switch)
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

    private var serviceTone: SettingsStatusBadge.Tone {
        switch store.health.serviceState {
        case .enabled: .good
        case .requiresApproval: .warning
        case .failed, .notFound: .bad
        case .disabled, .unavailable: .neutral
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

    private var permissionTone: SettingsStatusBadge.Tone {
        switch store.health.notificationPermission {
        case .authorized, .provisional, .ephemeral: .good
        case .denied: .bad
        case .notDetermined, .unknown: .neutral
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
