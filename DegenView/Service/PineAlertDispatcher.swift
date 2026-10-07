import Foundation

/// Fans a notification out to every channel. Channels run independently: one that throws or hangs
/// neither blocks the others nor reaches back into the script that raised the alert.
actor PineAlertDispatcher {
    private let channels: [any PineAlertChannel]

    init(
        channels: [any PineAlertChannel] = [
            UserNotificationChannel(), InAppBannerChannel(), WebhookPineAlertChannel(),
        ]
    ) {
        self.channels = channels
    }

    /// Returns once every channel has finished or failed.
    func dispatch(_ notification: PineAlertNotification) async {
        await withTaskGroup(of: Void.self) { group in
            for channel in channels {
                group.addTask {
                    do {
                        try await channel.deliver(notification)
                    } catch {
                        #if DEBUG
                            print("[PineAlert] \(channel.name) failed: \(error.localizedDescription)")
                        #endif
                    }
                }
            }
        }
    }
}
