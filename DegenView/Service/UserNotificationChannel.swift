import Foundation
import UserNotifications

/// Posts a macOS notification, honoring the price-alert notification settings.
struct UserNotificationChannel: PineAlertChannel {
    let name = "macOS notification"

    func deliver(_ notification: PineAlertNotification) async throws {
        let settings = await MainActor.run { AlertStore.shared.settings }
        guard settings.deliveryEnabled, settings.macOSNotificationsEnabled else { return }
        let content = UNMutableNotificationContent()
        content.title = notification.title
        content.body = notification.message.isEmpty ? "Alert" : notification.message
        content.userInfo = ["pineAlertSubscriptionID": notification.subscriptionID.uuidString]
        if settings.soundEnabled { content.sound = .default }
        try await UNUserNotificationCenter.current().add(
            UNNotificationRequest(identifier: notification.id.uuidString, content: content, trigger: nil))
    }
}
