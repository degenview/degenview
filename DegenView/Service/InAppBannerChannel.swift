import Foundation

/// Shows the alert in `GlobalAlertBanner`, honoring the price-alert notification settings.
struct InAppBannerChannel: PineAlertChannel {
    let name = "in-app banner"

    func deliver(_ notification: PineAlertNotification) async throws {
        await MainActor.run {
            let settings = AlertStore.shared.settings
            guard settings.deliveryEnabled, settings.inAppBannersEnabled else { return }
            PineAlertStore.shared.banner = notification
        }
    }
}
